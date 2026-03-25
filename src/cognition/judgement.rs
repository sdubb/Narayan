use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    agent::{
        evaluator::{EvalReflection, EvalVerdict},
        executor::StepResult,
        planner::{Plan, PlannedStep},
    },
    compliance::sla::SlaStatus,
    segments::DomainProfile,
    state::AgentState,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JudgementRecommendation {
    Continue,
    Watch,
    Revise,
    Escalate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgementSignal {
    pub step_index: usize,
    pub step_description: String,
    pub job_type: Option<String>,
    pub profile: String,
    pub score: f64,
    pub confidence: f64,
    pub recommendation: JudgementRecommendation,
    pub summary: String,
    pub reasons: Vec<String>,
    pub timestamp: String,
}

pub struct JudgementContext<'a> {
    pub state: &'a AgentState,
    pub plan: &'a Plan,
    pub step: &'a PlannedStep,
    pub result: &'a StepResult,
    pub eval: &'a EvalReflection,
    pub eval_verdict: EvalVerdict,
    pub retry_count: u32,
}

#[derive(Debug, Default, Clone)]
pub struct JudgementEngine;

impl JudgementEngine {
    pub fn evaluate(&self, ctx: JudgementContext<'_>) -> JudgementSignal {
        let profile = DomainProfile::for_job_type(ctx.plan.job_type.as_deref());
        let tuning = profile.judgement;
        let mut score = 0.72_f64;
        let mut reasons = Vec::new();

        if ctx.result.success {
            score += 0.16;
        } else {
            score -= tuning.failure_penalty;
            reasons.push("the step did not succeed".into());
        }

        let failed_tools = ctx.result.tool_results.iter().filter(|tool_result| !tool_result.success).count();
        if failed_tools > 0 {
            score -= tuning.tool_failure_penalty;
            reasons.push(format!("{failed_tools} tool call(s) failed"));
        }

        if ctx.retry_count > 0 {
            let penalty = tuning.retry_penalty * (ctx.retry_count.min(3) as f64);
            score -= penalty;
            reasons.push(format!("retry count is {}", ctx.retry_count));
        }

        if ctx.eval.should_revise {
            score -= 0.14;
            reasons.push("evaluator suggested a plan revision".into());
        }

        match ctx.eval_verdict {
            EvalVerdict::Continue => {}
            EvalVerdict::GoalComplete => {
                score += 0.05;
                reasons.push("the evaluator marked the run complete".into());
            }
            EvalVerdict::Retry => {
                score -= 0.08;
                reasons.push("the evaluator requested another attempt".into());
            }
            EvalVerdict::Abort => {
                score -= 0.22;
                reasons.push("the evaluator aborted the step".into());
            }
        }

        if ctx.step.tool.is_none() && ctx.result.success {
            score += tuning.no_tool_bonus;
            reasons.push("the step completed without a tool call".into());
        }

        if ctx.plan.is_complete(ctx.state.current_step as usize + 1) {
            score += tuning.final_step_bonus;
            reasons.push("this was the final planned step".into());
        }

        if ctx.result.items_processed > 0 {
            score += tuning.item_bonus;
            reasons.push(format!("processed {} item(s)", ctx.result.items_processed));
        }

        if !ctx.result.connector_writes.is_empty() {
            score += tuning.connector_bonus;
            reasons.push(format!("wrote to {} connector(s)", ctx.result.connector_writes.len()));
        }

        let mut sla_warning = false;
        let mut sla_escalate = false;
        if let Some((pct_elapsed, breached)) = sla_pressure(ctx.state) {
            if breached || pct_elapsed >= tuning.sla_escalate_pct {
                score -= tuning.sla_escalate_penalty;
                sla_escalate = true;
                reasons.push(format!("SLA breach at {:.0}% elapsed", pct_elapsed));
            } else if pct_elapsed >= tuning.sla_watch_pct {
                score -= tuning.sla_watch_penalty;
                sla_warning = true;
                reasons.push(format!("SLA is at {:.0}% elapsed", pct_elapsed));
            }
        }

        score = score.clamp(0.0, 1.0);

        let recommendation = if sla_escalate
            || matches!(ctx.eval_verdict, EvalVerdict::Abort)
            || (!ctx.result.success && ctx.retry_count >= tuning.escalate_retry_limit)
        {
            JudgementRecommendation::Escalate
        } else if ctx.eval.should_revise || !ctx.result.success || score < tuning.revise_threshold {
            JudgementRecommendation::Revise
        } else if sla_warning || score < tuning.watch_threshold || ctx.retry_count > 0 {
            JudgementRecommendation::Watch
        } else {
            JudgementRecommendation::Continue
        };

        let confidence = match recommendation {
            JudgementRecommendation::Continue => (score + 0.08).clamp(0.0, 1.0),
            JudgementRecommendation::Watch => score,
            JudgementRecommendation::Revise => (score - 0.06).clamp(0.0, 1.0),
            JudgementRecommendation::Escalate => (score - 0.12).clamp(0.0, 1.0),
        };

        let summary = build_summary(&recommendation, confidence, &reasons);

        JudgementSignal {
            step_index: ctx.step.index,
            step_description: ctx.step.description.clone(),
            job_type: ctx.plan.job_type.clone(),
            profile: tuning.name.into(),
            score,
            confidence,
            recommendation,
            summary,
            reasons,
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

fn sla_pressure(state: &AgentState) -> Option<(f64, bool)> {
    let raw = state.metadata.get("sla_status")?;
    let status = serde_json::from_value::<SlaStatus>(raw.clone()).ok()?;
    let total_secs = (status.resolution_deadline - status.started_at).num_seconds().max(1) as f64;
    let elapsed_secs = (Utc::now() - status.started_at).num_seconds().max(0) as f64;
    let pct_elapsed = (elapsed_secs / total_secs) * 100.0;
    Some((pct_elapsed, status.breached))
}

fn build_summary(recommendation: &JudgementRecommendation, confidence: f64, reasons: &[String]) -> String {
    let headline = match recommendation {
        JudgementRecommendation::Continue => "Judgement: continue".to_string(),
        JudgementRecommendation::Watch => "Judgement: watch closely".to_string(),
        JudgementRecommendation::Revise => "Judgement: revise the plan".to_string(),
        JudgementRecommendation::Escalate => "Judgement: escalate for review".to_string(),
    };

    let confidence_text = format!("confidence {:.0}%", (confidence * 100.0).clamp(0.0, 100.0));
    let reason_text = reasons.first().cloned().unwrap_or_else(|| "no additional concerns".into());
    format!("{headline} - {confidence_text} - {reason_text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        agent::{
            evaluator::EvalReflection,
            evaluator::EvalVerdict,
            executor::StepResult,
            planner::{Plan, PlannedStep},
        },
        state::AgentState,
    };

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    fn make_plan() -> Plan {
        Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![PlannedStep {
                index: 0,
                description: "Inspect failing workflow".into(),
                tool: Some("file_read".into()),
                tool_args: None,
                success_criteria: "workflow reviewed".into(),
                condition: None,
            }],
            rationale: "inspect first".into(),
        }
    }

    fn make_result(success: bool) -> StepResult {
        StepResult {
            step_index: 0,
            success,
            output: "ok".into(),
            final_answer_candidate: Some("ok".into()),
            tool_results: vec![],
            tools_called: vec![],
            items_processed: 0,
            connector_writes: vec![],
        }
    }

    fn make_eval(verdict: EvalVerdict) -> EvalReflection {
        EvalReflection {
            verdict,
            summary: "step summary".into(),
            key_findings: vec![],
            should_revise: false,
            revision_feedback: String::new(),
        }
    }

    #[test]
    fn test_successful_step_stays_quiet() {
        let engine = JudgementEngine;
        let state = make_state();
        let plan = make_plan();
        let step = plan.steps[0].clone();
        let signal = engine.evaluate(JudgementContext {
            state: &state,
            plan: &plan,
            step: &step,
            result: &make_result(true),
            eval: &make_eval(EvalVerdict::Continue),
            eval_verdict: EvalVerdict::Continue,
            retry_count: 0,
        });

        assert_eq!(signal.recommendation, JudgementRecommendation::Continue);
        assert!(signal.score > 0.8);
        assert_eq!(signal.profile, "engineering");
    }

    #[test]
    fn test_failed_step_recommends_revision_or_escalation() {
        let engine = JudgementEngine;
        let mut state = make_state();
        state.metadata["sla_status"] = serde_json::json!({
            "agent_id": "agent-1",
            "policy_id": "policy-1",
            "started_at": Utc::now().to_rfc3339(),
            "first_response_at": null,
            "resolved_at": null,
            "first_response_deadline": (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
            "resolution_deadline": (Utc::now() + chrono::Duration::minutes(20)).to_rfc3339(),
            "breached": false,
            "escalation_triggered": false,
        });
        let plan = make_plan();
        let step = plan.steps[0].clone();
        let signal = engine.evaluate(JudgementContext {
            state: &state,
            plan: &plan,
            step: &step,
            result: &make_result(false),
            eval: &make_eval(EvalVerdict::Retry),
            eval_verdict: EvalVerdict::Retry,
            retry_count: 2,
        });

        assert!(matches!(signal.recommendation, JudgementRecommendation::Revise | JudgementRecommendation::Escalate));
        assert!(!signal.reasons.is_empty());
    }

    #[test]
    fn test_finance_profile_is_stricter() {
        let engine = JudgementEngine;
        let state = make_state();
        let mut plan = make_plan();
        plan.job_type = Some("finance_accounting".into());
        let step = plan.steps[0].clone();
        let signal = engine.evaluate(JudgementContext {
            state: &state,
            plan: &plan,
            step: &step,
            result: &make_result(true),
            eval: &make_eval(EvalVerdict::Continue),
            eval_verdict: EvalVerdict::Continue,
            retry_count: 1,
        });

        assert_eq!(signal.profile, "finance");
        assert!(matches!(signal.recommendation, JudgementRecommendation::Watch));
    }
}
