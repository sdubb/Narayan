//! Canonical domain profiles for segment plugins and downstream runtime policy.
//!
//! This keeps domain knowledge in one place so segment plugins, judgment,
//! plan mode, and UI surfaces can all describe the same domain consistently.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JudgementTuning {
    pub name: &'static str,
    pub watch_threshold: f64,
    pub revise_threshold: f64,
    pub escalate_retry_limit: u32,
    pub retry_penalty: f64,
    pub failure_penalty: f64,
    pub tool_failure_penalty: f64,
    pub no_tool_bonus: f64,
    pub final_step_bonus: f64,
    pub item_bonus: f64,
    pub connector_bonus: f64,
    pub sla_watch_pct: f64,
    pub sla_escalate_pct: f64,
    pub sla_watch_penalty: f64,
    pub sla_escalate_penalty: f64,
}

impl JudgementTuning {
    pub const fn new(
        name: &'static str,
        watch_threshold: f64,
        revise_threshold: f64,
        escalate_retry_limit: u32,
        retry_penalty: f64,
        failure_penalty: f64,
        tool_failure_penalty: f64,
        no_tool_bonus: f64,
        final_step_bonus: f64,
        item_bonus: f64,
        connector_bonus: f64,
        sla_watch_pct: f64,
        sla_escalate_pct: f64,
        sla_watch_penalty: f64,
        sla_escalate_penalty: f64,
    ) -> Self {
        Self {
            name,
            watch_threshold,
            revise_threshold,
            escalate_retry_limit,
            retry_penalty,
            failure_penalty,
            tool_failure_penalty,
            no_tool_bonus,
            final_step_bonus,
            item_bonus,
            connector_bonus,
            sla_watch_pct,
            sla_escalate_pct,
            sla_watch_penalty,
            sla_escalate_penalty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DomainProfile {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub primary_connectors: &'static [&'static str],
    pub requires_review: bool,
    pub requires_evidence: bool,
    pub pii_sensitive: bool,
    pub judgement: JudgementTuning,
}

impl DomainProfile {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        summary: &'static str,
        primary_connectors: &'static [&'static str],
        requires_review: bool,
        requires_evidence: bool,
        pii_sensitive: bool,
        judgement: JudgementTuning,
    ) -> Self {
        Self { id, label, summary, primary_connectors, requires_review, requires_evidence, pii_sensitive, judgement }
    }

    pub const fn general() -> Self {
        Self::new(
            "general",
            "General",
            "General-purpose assistant with no special compliance assumptions.",
            &[],
            false,
            false,
            false,
            JudgementTuning::new(
                "general", 0.80, 0.55, 2, 0.05, 0.28, 0.12, 0.05, 0.04, 0.03, 0.04, 80.0, 100.0, 0.10, 0.18,
            ),
        )
    }

    pub const fn engineering() -> Self {
        Self::new(
            "engineering",
            "Engineering Maintenance",
            "Code review, CI/CD, repo maintenance, and engineering incident response.",
            &["github"],
            true,
            false,
            false,
            JudgementTuning::new(
                "engineering",
                0.82,
                0.60,
                2,
                0.05,
                0.28,
                0.12,
                0.04,
                0.04,
                0.03,
                0.05,
                75.0,
                95.0,
                0.10,
                0.18,
            ),
        )
    }

    pub const fn customer_support() -> Self {
        Self::new(
            "customer_support",
            "Customer Support",
            "Ticket handling, escalation, SLA enforcement, and response drafting.",
            &["zendesk", "intercom", "freshdesk"],
            true,
            false,
            true,
            JudgementTuning::new(
                "support", 0.84, 0.60, 1, 0.06, 0.30, 0.15, 0.03, 0.03, 0.02, 0.04, 75.0, 95.0, 0.10, 0.20,
            ),
        )
    }

    pub const fn compliance_ops() -> Self {
        Self::new(
            "compliance_ops",
            "Compliance Ops",
            "Citation-first workflows, evidence packaging, and regulatory review.",
            &["servicenow"],
            true,
            true,
            true,
            JudgementTuning::new(
                "compliance",
                0.86,
                0.65,
                1,
                0.05,
                0.31,
                0.14,
                0.03,
                0.03,
                0.02,
                0.04,
                70.0,
                95.0,
                0.12,
                0.22,
            ),
        )
    }

    pub const fn sales_revops() -> Self {
        Self::new(
            "sales_revops",
            "Sales & RevOps",
            "Prospect research, CRM enrichment, outreach, and pipeline intelligence.",
            &["salesforce"],
            true,
            false,
            true,
            JudgementTuning::new(
                "sales", 0.79, 0.56, 2, 0.05, 0.28, 0.12, 0.03, 0.03, 0.03, 0.04, 80.0, 100.0, 0.10, 0.18,
            ),
        )
    }

    pub const fn finance_accounting() -> Self {
        Self::new(
            "finance_accounting",
            "Finance & Accounting",
            "Invoice processing, reconciliation, expense categorisation, and month-end close.",
            &["quickbooks", "stripe"],
            true,
            true,
            true,
            JudgementTuning::new(
                "finance", 0.86, 0.68, 1, 0.08, 0.32, 0.16, 0.03, 0.03, 0.02, 0.05, 70.0, 95.0, 0.12, 0.22,
            ),
        )
    }

    pub const fn hr_people_ops() -> Self {
        Self::new(
            "hr_people_ops",
            "HR & People Ops",
            "Candidate screening, onboarding, policy Q&A, and performance data.",
            &["greenhouse"],
            true,
            false,
            true,
            JudgementTuning::new("hr", 0.84, 0.67, 1, 0.06, 0.30, 0.15, 0.03, 0.03, 0.02, 0.04, 75.0, 95.0, 0.10, 0.20),
        )
    }

    pub const fn legal_contract() -> Self {
        Self::new(
            "legal_contract",
            "Legal & Contract Ops",
            "Contract review, clause extraction, redlining, and due diligence.",
            &["docusign"],
            true,
            true,
            true,
            JudgementTuning::new(
                "legal", 0.88, 0.70, 1, 0.07, 0.34, 0.18, 0.02, 0.03, 0.02, 0.04, 70.0, 95.0, 0.12, 0.22,
            ),
        )
    }

    pub const fn it_ops_itsm() -> Self {
        Self::new(
            "it_ops_itsm",
            "IT Ops & ITSM",
            "Incident runbooks, change advisory, health checks, and postmortems.",
            &["pagerduty", "servicenow"],
            true,
            true,
            false,
            JudgementTuning::new(
                "itsm", 0.82, 0.66, 1, 0.06, 0.28, 0.14, 0.02, 0.03, 0.02, 0.05, 65.0, 90.0, 0.14, 0.24,
            ),
        )
    }

    pub const fn research_intelligence() -> Self {
        Self::new(
            "research_intelligence",
            "Research & Intelligence",
            "Market research, competitive intel, due diligence, and synthesis.",
            &["notion"],
            true,
            true,
            false,
            JudgementTuning::new(
                "research", 0.80, 0.58, 2, 0.05, 0.26, 0.12, 0.03, 0.03, 0.03, 0.04, 80.0, 100.0, 0.10, 0.18,
            ),
        )
    }

    pub const fn data_analytics() -> Self {
        Self::new(
            "data_analytics",
            "Data & Analytics Ops",
            "Pipeline monitoring, data quality checks, scheduled reports, and schema migrations.",
            &["dbt_cloud"],
            true,
            true,
            true,
            JudgementTuning::new(
                "data", 0.81, 0.60, 2, 0.05, 0.28, 0.14, 0.03, 0.03, 0.02, 0.04, 75.0, 95.0, 0.10, 0.20,
            ),
        )
    }

    pub const fn marketing_growth() -> Self {
        Self::new(
            "marketing_growth",
            "Marketing & Growth",
            "SEO audits, competitor monitoring, content research, and campaign reporting.",
            &["hubspot"],
            true,
            false,
            true,
            JudgementTuning::new(
                "marketing",
                0.78,
                0.54,
                2,
                0.05,
                0.27,
                0.12,
                0.03,
                0.03,
                0.03,
                0.04,
                80.0,
                100.0,
                0.10,
                0.18,
            ),
        )
    }

    pub const fn procurement_vendor_ops() -> Self {
        Self::new(
            "procurement_vendor_ops",
            "Procurement & Vendor Ops",
            "Vendor intake, purchase approvals, contract routing, invoice matching, and renewal management.",
            &["docusign", "quickbooks", "stripe", "notion"],
            true,
            true,
            true,
            JudgementTuning::new(
                "procurement",
                0.87,
                0.69,
                1,
                0.08,
                0.32,
                0.16,
                0.03,
                0.03,
                0.02,
                0.05,
                70.0,
                95.0,
                0.12,
                0.22,
            ),
        )
    }

    pub const fn security_ops_grc() -> Self {
        Self::new(
            "security_ops_grc",
            "Security Ops & GRC",
            "Access reviews, security evidence, incident response, risk tracking, and audit readiness.",
            &["servicenow", "pagerduty", "github", "notion"],
            true,
            true,
            true,
            JudgementTuning::new(
                "security", 0.90, 0.72, 1, 0.08, 0.34, 0.16, 0.02, 0.03, 0.02, 0.05, 60.0, 90.0, 0.14, 0.24,
            ),
        )
    }

    pub const fn customer_success_renewals() -> Self {
        Self::new(
            "customer_success_renewals",
            "Customer Success & Renewals",
            "Account health, renewals, churn risk, QBR prep, and escalation follow-up.",
            &["salesforce", "hubspot", "zendesk", "intercom", "freshdesk"],
            true,
            true,
            true,
            JudgementTuning::new(
                "customer_success",
                0.81,
                0.58,
                2,
                0.06,
                0.28,
                0.13,
                0.03,
                0.03,
                0.03,
                0.04,
                80.0,
                100.0,
                0.10,
                0.18,
            ),
        )
    }

    pub fn for_slug(slug: &str) -> Self {
        match slug.trim().to_ascii_lowercase().as_str() {
            "engineering" | "software_engineer" | "devops" => Self::engineering(),
            "customer_support" => Self::customer_support(),
            "compliance_ops" => Self::compliance_ops(),
            "sales_revops" => Self::sales_revops(),
            "finance_accounting" => Self::finance_accounting(),
            "hr_people_ops" => Self::hr_people_ops(),
            "legal_contract" => Self::legal_contract(),
            "it_ops_itsm" => Self::it_ops_itsm(),
            "research_intelligence" | "research_analyst" => Self::research_intelligence(),
            "data_analytics" | "data_extraction" => Self::data_analytics(),
            "marketing_growth" | "marketing" => Self::marketing_growth(),
            "procurement_vendor_ops" | "procurement" => Self::procurement_vendor_ops(),
            "security_ops_grc" | "security_ops" | "grc" => Self::security_ops_grc(),
            "customer_success_renewals" | "customer_success" => Self::customer_success_renewals(),
            _ => Self::general(),
        }
    }

    pub fn for_job_type(job_type: Option<&str>) -> Self {
        Self::for_slug(job_type.unwrap_or("general"))
    }

    pub fn judgement_tuning(&self) -> JudgementTuning {
        self.judgement
    }
}
