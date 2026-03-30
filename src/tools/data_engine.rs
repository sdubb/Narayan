use std::time::Instant;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::tools::{ParameterSchema, Tool, ToolResult};

const DEFAULT_MAX_DEPTH: usize = 3;
const DEFAULT_MAX_REGEX_LENGTH: usize = 128;
const DEFAULT_MAX_FORMULA_LENGTH: usize = 100;
const DEFAULT_TOP_N: usize = 1000;

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct EngineOptions {
    strict: bool,
    max_depth: usize,
    max_regex_length: usize,
    max_formula_length: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct PipelineStep {
    op: String,
    #[serde(default)]
    condition: Option<ConditionSpec>,
    #[serde(default)]
    assign: Option<Map<String, Value>>,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    formula: Option<String>,
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    rules: Option<Vec<RuleSpec>>,
    #[serde(default)]
    group_by: Option<Vec<String>>,
    #[serde(default)]
    metrics: Option<Map<String, Value>>,
    #[serde(default)]
    top_n: Option<usize>,
    #[serde(default)]
    source_field: Option<String>,
    #[serde(default)]
    schema: Option<Map<String, Value>>,
    #[serde(default)]
    required_fields: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ConditionSpec {
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    exists: Option<bool>,
    #[serde(default)]
    not_exists: Option<bool>,
    #[serde(default)]
    truthy: Option<bool>,
    #[serde(default)]
    falsy: Option<bool>,
    #[serde(default)]
    equals: Option<Value>,
    #[serde(default)]
    not_equals: Option<Value>,
    #[serde(default)]
    contains: Option<Value>,
    #[serde(default)]
    nonempty: Option<bool>,
    #[serde(default)]
    empty: Option<bool>,
    #[serde(default)]
    gt: Option<Value>,
    #[serde(default)]
    gte: Option<Value>,
    #[serde(default)]
    lt: Option<Value>,
    #[serde(default)]
    lte: Option<Value>,
    #[serde(default)]
    regex: Option<String>,
    #[serde(default)]
    all_of: Option<Vec<ConditionSpec>>,
    #[serde(default)]
    any_of: Option<Vec<ConditionSpec>>,
    #[serde(rename = "not", default)]
    not_condition: Option<Box<ConditionSpec>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct RuleSpec {
    #[serde(rename = "if")]
    condition: ConditionSpec,
    then: RuleActionSpec,
    #[serde(rename = "else", default)]
    else_action: Option<RuleActionSpec>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct RuleActionSpec {
    #[serde(default)]
    set: Option<Map<String, Value>>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    drop: Option<bool>,
    #[serde(default)]
    remove_fields: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct CleanConfig {
    #[serde(default)]
    trim_strings: bool,
    #[serde(default)]
    lowercase_fields: Vec<String>,
    #[serde(default)]
    null_values: Vec<Value>,
    #[serde(default)]
    type_coercion: Map<String, Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct RankConfig {
    #[serde(default)]
    score_field: Option<String>,
    #[serde(default)]
    score_formula: Option<String>,
    #[serde(default)]
    top_n: Option<usize>,
    #[serde(default = "default_rank_descending")]
    descending: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct AggregateConfig {
    #[serde(default)]
    group_by: Vec<String>,
    #[serde(default)]
    metrics: Map<String, Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct ExtractFieldSpec {
    pattern: String,
    #[serde(default = "default_extract_type")]
    r#type: String,
    #[serde(default)]
    required: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct ExtractConfig {
    #[serde(default)]
    source_field: Option<String>,
    #[serde(default)]
    schema: Map<String, Value>,
    #[serde(default)]
    required_fields: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct RulesConfig {
    #[serde(default = "default_rule_mode")]
    mode: String,
    rules: Vec<RuleSpec>,
}

pub struct DataEngineTool;

#[async_trait]
impl Tool for DataEngineTool {
    fn name(&self) -> &str {
        "data_engine"
    }

    fn description(&self) -> &str {
        "Deterministic data engine for tenant workflows. Use it for typed record pipelines and single-op data tasks."
    }

    fn category(&self) -> &'static str {
        "data"
    }

    fn input_contract(&self) -> Option<String> {
        Some(
            "Use either pipeline mode or single-op mode. Pipeline mode: { records, pipeline, options? } where each step is a typed op such as filter, map, compute, clean, apply_rules, rank, aggregate, or extract_structured_data. Single-op mode: { records, op, config, options? }. DSL rules are per-record, side-effect-free, bounded-depth, and deterministic.".into(),
        )
    }

    fn output_contract(&self) -> Option<String> {
        Some(
            "Returns { records, meta, warnings, errors }. meta includes input_count, output_count, dropped_count, derived_fields, ops_applied, execution_time_ms, used_llm, confidence, fallback_needed, and missing_fields.".into(),
        )
    }

    fn when_to_use(&self) -> Option<String> {
        Some(
            "Use for filtering, mapping, cleaning, scoring, ranking, grouping, aggregation, schema alignment, and deterministic structured extraction.".into(),
        )
    }

    fn when_not_to_use(&self) -> Option<String> {
        Some(
            "Do not use for free-form scripts, browser automation, remote execution, arbitrary custom code, or workflows that need stateful side effects.".into(),
        )
    }

    fn examples(&self) -> Vec<String> {
        vec![
            r#"{"records":[{"email":"a@x.com","revenue":12000}],"pipeline":[{"op":"map","assign":{"priority":{"if":{"field":"revenue","gt":10000},"then":"high","else":"low"}}},{"op":"compute","field":"score","formula":"(revenue * 0.3) + 10"}]}"#.into(),
            r#"{"records":[{"text":"Invoice INV-123 total $42"}],"op":"extract_structured_data","config":{"source_field":"text","schema":{"invoice_id":{"pattern":"INV-[0-9]+","type":"string","required":true},"total":{"pattern":"\\$[0-9]+","type":"string","required":false}},"required_fields":["invoice_id"]}}"#.into(),
        ]
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("records", "array", "Array of input record objects. Each item must be an object."),
            ParameterSchema::optional(
                "pipeline",
                "array",
                "Typed pipeline steps for deterministic transforms. Use when chaining row-wise or dataset ops in order.",
            ),
            ParameterSchema::optional(
                "op",
                "string",
                "Single operation mode: transform_records | clean_data | compute_formula | apply_rules | rank_items | aggregate_records | extract_structured_data.",
            ),
            ParameterSchema::optional("config", "object", "Operation-specific config for single-op mode."),
            ParameterSchema::optional(
                "options",
                "object",
                "Engine options: { strict, max_depth, max_regex_length, max_formula_length }.",
            ),
        ]
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let start = Instant::now();
        let options = parse_options(&args["options"]);
        let records = match parse_records(&args["records"]) {
            Ok(records) => records,
            Err(err) => return Ok(ToolResult::err(err)),
        };

        let has_pipeline = args.get("pipeline").and_then(Value::as_array).map(|items| !items.is_empty()).unwrap_or(false);
        let op = args["op"].as_str().map(str::trim).unwrap_or_default().to_string();
        let mut warnings = Vec::<String>::new();
        let mut errors = Vec::<String>::new();
        let mut ops_applied = Vec::<String>::new();

        let result = if has_pipeline {
            if !op.is_empty() && op != "transform_records" {
                return Ok(ToolResult::err("pipeline mode requires op='transform_records' or no op"));
            }
            let steps: Vec<PipelineStep> = match serde_json::from_value(args["pipeline"].clone()) {
                Ok(steps) => steps,
                Err(err) => return Ok(ToolResult::err(format!("invalid pipeline: {err}"))),
            };
            execute_pipeline(records, &steps, &options, &mut warnings, &mut errors, &mut ops_applied)
                .map_err(anyhow::Error::msg)?
        } else {
            if op.is_empty() {
                return Ok(ToolResult::err("either 'pipeline' or 'op' is required"));
            }
            let config = args.get("config").cloned().unwrap_or(Value::Null);
            execute_single_op(&op, &config, records, &options, &mut warnings, &mut errors, &mut ops_applied)
                .map_err(anyhow::Error::msg)?
        };

        let execution_time_ms = start.elapsed().as_millis() as u64;
        let output_count = result.len() as u64;
        let input_count = args["records"].as_array().map(|a| a.len()).unwrap_or(0) as u64;
        let dropped_count = input_count.saturating_sub(output_count);
        let missing_fields = dedupe_strings(
            &warnings
                .iter()
                .filter_map(|w| w.strip_prefix("missing_field:").map(|s| s.trim().to_string()))
                .collect::<Vec<_>>(),
        );
        let meta = serde_json::json!({
            "input_count": input_count,
            "output_count": output_count,
            "dropped_count": dropped_count,
            "derived_fields": dedupe_strings(&ops_applied),
            "ops_applied": ops_applied,
            "execution_time_ms": execution_time_ms,
            "used_llm": false,
            "confidence": confidence_score(input_count, output_count, &warnings, &errors),
            "fallback_needed": !missing_fields.is_empty() || warnings.iter().any(|w| w.contains("fallback_needed")),
            "missing_fields": missing_fields,
        });

        Ok(ToolResult::ok(serde_json::json!({
            "records": result,
            "meta": meta,
            "warnings": warnings,
            "errors": errors,
        })))
    }
}

fn execute_pipeline(
    mut records: Vec<Map<String, Value>>,
    steps: &[PipelineStep],
    options: &EngineOptions,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
    ops_applied: &mut Vec<String>,
) -> Result<Vec<Map<String, Value>>, String> {
    for step in steps {
        validate_step(step, options)?;
        let (next, step_warnings, step_errors) = apply_step(records, step, options)?;
        warnings.extend(step_warnings);
        errors.extend(step_errors);
        ops_applied.push(step.op.clone());
        records = next;
    }
    Ok(records)
}

fn execute_single_op(
    op: &str,
    config: &Value,
    records: Vec<Map<String, Value>>,
    options: &EngineOptions,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
    ops_applied: &mut Vec<String>,
) -> Result<Vec<Map<String, Value>>, String> {
    let step = PipelineStep {
        op: op.to_string(),
        condition: None,
        assign: None,
        field: None,
        formula: None,
        config: if config.is_null() { None } else { Some(config.clone()) },
        mode: None,
        rules: None,
        group_by: None,
        metrics: None,
        top_n: None,
        source_field: None,
        schema: None,
        required_fields: None,
    };
    validate_step(&step, options)?;
    let (next, step_warnings, step_errors) = apply_step(records, &step, options)?;
    warnings.extend(step_warnings);
    errors.extend(step_errors);
    ops_applied.push(step.op.clone());
    Ok(next)
}

fn apply_step(
    records: Vec<Map<String, Value>>,
    step: &PipelineStep,
    options: &EngineOptions,
) -> Result<(Vec<Map<String, Value>>, Vec<String>, Vec<String>), String> {
    match step.op.as_str() {
        "filter" => {
            let condition = step.condition.as_ref().ok_or_else(|| "filter step requires condition".to_string())?;
            let mut kept = Vec::new();
            for record in records {
                if evaluate_condition(condition, &record, options)? {
                    kept.push(record);
                }
            }
            Ok((kept, vec![], vec![]))
        }
        "map" => {
            let assign = step.assign.as_ref().ok_or_else(|| "map step requires assign".to_string())?;
            let mut out = Vec::with_capacity(records.len());
            for mut record in records {
                for (field, value_spec) in assign {
                    let value = evaluate_assignment_value(value_spec, &record, options)?;
                    set_path(&mut record, field, value);
                }
                out.push(record);
            }
            Ok((out, vec![], vec![]))
        }
        "compute" | "compute_formula" => {
            let field = required_field_name(step.field.as_deref(), "compute step requires field")?;
            let formula = required_formula(step.formula.as_deref(), "compute step requires formula")?;
            let mut out = Vec::with_capacity(records.len());
            for mut record in records {
                let value = evaluate_formula(formula, &record, options)?;
                set_path(&mut record, field, value);
                out.push(record);
            }
            Ok((out, vec![], vec![]))
        }
        "clean" | "clean_data" => {
            let config: CleanConfig = parse_config(step.config.as_ref(), "clean_data")?;
            let mut out = Vec::with_capacity(records.len());
            for record in records {
                out.push(clean_record(record, &config));
            }
            Ok((out, vec![], vec![]))
        }
        "apply_rules" => {
            let config = parse_rules_config(step)?;
            let mut out = Vec::with_capacity(records.len());
            for record in records {
                if let Some(record) = apply_rules_to_record(record, &config.rules, &config.mode, options)? {
                    out.push(record);
                }
            }
            Ok((out, vec![], vec![]))
        }
        "rank" | "rank_items" => {
            let config: RankConfig = parse_config(step.config.as_ref(), "rank_items")?;
            Ok((rank_records(records, &config, options)?, vec![], vec![]))
        }
        "aggregate" | "aggregate_records" => {
            let config: AggregateConfig = parse_config(step.config.as_ref(), "aggregate_records")?;
            Ok((aggregate_records(records, &config, options)?, vec![], vec![]))
        }
        "extract_structured_data" => {
            let config: ExtractConfig = parse_config(step.config.as_ref(), "extract_structured_data")?;
            extract_structured_data(records, &config, options)
        }
        "transform_records" => Err("transform_records is a pipeline alias; provide nested pipeline steps".into()),
        other => Err(format!("unsupported op '{other}'")),
    }
}

fn parse_options(raw: &Value) -> EngineOptions {
    let max_depth = raw.get("max_depth").and_then(Value::as_u64).unwrap_or(DEFAULT_MAX_DEPTH as u64) as usize;
    let max_regex_length =
        raw.get("max_regex_length").and_then(Value::as_u64).unwrap_or(DEFAULT_MAX_REGEX_LENGTH as u64) as usize;
    let max_formula_length =
        raw.get("max_formula_length").and_then(Value::as_u64).unwrap_or(DEFAULT_MAX_FORMULA_LENGTH as u64) as usize;
    EngineOptions {
        strict: raw.get("strict").and_then(Value::as_bool).unwrap_or(true),
        max_depth: max_depth.max(1),
        max_regex_length: max_regex_length.max(1),
        max_formula_length: max_formula_length.max(1),
    }
}

fn parse_records(raw: &Value) -> Result<Vec<Map<String, Value>>, String> {
    let arr = raw.as_array().ok_or_else(|| "'records' must be an array".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, value) in arr.iter().enumerate() {
        let obj = value.as_object().ok_or_else(|| format!("record {idx} must be an object"))?;
        out.push(obj.clone());
    }
    Ok(out)
}

fn parse_config<T: for<'de> Deserialize<'de>>(raw: Option<&Value>, label: &str) -> Result<T, String> {
    let value = raw.cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|err| format!("invalid {label} config: {err}"))
}

fn parse_rules_config(step: &PipelineStep) -> Result<RulesConfig, String> {
    let mode = step.mode.clone().unwrap_or_else(|| "first_match".into());
    if mode != "first_match" && mode != "all_match" {
        return Err("apply_rules mode must be 'first_match' or 'all_match'".into());
    }
    let rules = if let Some(rules) = &step.rules {
        rules.clone()
    } else if let Some(config) = &step.config {
        serde_json::from_value::<RulesConfig>(config.clone())
            .map_err(|err| format!("invalid apply_rules config: {err}"))?
            .rules
    } else {
        return Err("apply_rules requires rules".into());
    };
    if rules.is_empty() {
        return Err("apply_rules requires at least one rule".into());
    }
    Ok(RulesConfig { mode, rules })
}

fn validate_step(step: &PipelineStep, options: &EngineOptions) -> Result<(), String> {
    if step.op.trim().is_empty() {
        return Err("step op cannot be empty".into());
    }
    if let Some(formula) = &step.formula {
        if formula.len() > options.max_formula_length {
            return Err(format!("formula too long ({} > {})", formula.len(), options.max_formula_length));
        }
    }
    if let Some(condition) = &step.condition {
        validate_condition(condition, 1, options)?;
    }
    if let Some(rules) = &step.rules {
        for rule in rules {
            validate_condition(&rule.condition, 1, options)?;
        }
    }
    if let Some(schema) = &step.schema {
        for (_field, spec) in schema {
            if let Some(spec) = spec.as_object() {
                if let Some(pattern) = spec.get("pattern").and_then(Value::as_str) {
                    if pattern.len() > options.max_regex_length {
                        return Err(format!("pattern too long ({} > {})", pattern.len(), options.max_regex_length));
                    }
                    Regex::new(pattern).map_err(|err| format!("invalid pattern: {err}"))?;
                }
            }
        }
    }
    Ok(())
}

fn validate_condition(condition: &ConditionSpec, depth: usize, options: &EngineOptions) -> Result<(), String> {
    if depth > options.max_depth {
        return Err(format!("condition depth exceeds max_depth={}", options.max_depth));
    }
    if condition.not_condition.is_some() && (condition.field.is_some() || condition.all_of.is_some() || condition.any_of.is_some()) {
        return Err("condition 'not' cannot be combined with atomic or grouped conditions".into());
    }
    if condition.all_of.is_some() && condition.any_of.is_some() {
        return Err("condition cannot have both all_of and any_of".into());
    }

    let atomic = [
        condition.exists.is_some(),
        condition.not_exists.is_some(),
        condition.truthy.is_some(),
        condition.falsy.is_some(),
        condition.equals.is_some(),
        condition.not_equals.is_some(),
        condition.contains.is_some(),
        condition.nonempty.is_some(),
        condition.empty.is_some(),
        condition.gt.is_some(),
        condition.gte.is_some(),
        condition.lt.is_some(),
        condition.lte.is_some(),
        condition.regex.is_some(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count();

    if condition.not_condition.is_none() && condition.all_of.is_none() && condition.any_of.is_none() {
        if condition.field.is_none() {
            return Err("atomic condition requires field".into());
        }
        if atomic != 1 {
            return Err("atomic condition must define exactly one comparator".into());
        }
    }

    if let Some(regex) = &condition.regex {
        if regex.len() > options.max_regex_length {
            return Err(format!("regex too long ({} > {})", regex.len(), options.max_regex_length));
        }
        Regex::new(regex).map_err(|err| format!("invalid regex: {err}"))?;
    }
    if let Some(all_of) = &condition.all_of {
        if all_of.is_empty() {
            return Err("all_of cannot be empty".into());
        }
        for item in all_of {
            validate_condition(item, depth + 1, options)?;
        }
    }
    if let Some(any_of) = &condition.any_of {
        if any_of.is_empty() {
            return Err("any_of cannot be empty".into());
        }
        for item in any_of {
            validate_condition(item, depth + 1, options)?;
        }
    }
    if let Some(inner) = &condition.not_condition {
        validate_condition(inner, depth + 1, options)?;
    }
    Ok(())
}

fn evaluate_condition(condition: &ConditionSpec, record: &Map<String, Value>, options: &EngineOptions) -> Result<bool, String> {
    if let Some(inner) = &condition.not_condition {
        return Ok(!evaluate_condition(inner, record, options)?);
    }
    if let Some(all_of) = &condition.all_of {
        return all_of
            .iter()
            .try_fold(true, |acc, item| Ok(acc && evaluate_condition(item, record, options)?));
    }
    if let Some(any_of) = &condition.any_of {
        return any_of
            .iter()
            .try_fold(false, |acc, item| Ok(acc || evaluate_condition(item, record, options)?));
    }

    let field = condition.field.as_ref().ok_or_else(|| "atomic condition requires field".to_string())?;
    let value = get_path(record, field);

    if let Some(expected) = condition.exists {
        return Ok(expected == value.is_some() && !value.unwrap_or(&Value::Null).is_null());
    }
    if let Some(expected) = condition.not_exists {
        return Ok(expected == (value.is_none() || value.unwrap_or(&Value::Null).is_null()));
    }
    if let Some(expected) = condition.truthy {
        return Ok(expected == is_truthy(value));
    }
    if let Some(expected) = condition.falsy {
        return Ok(expected == !is_truthy(value));
    }
    if let Some(expected) = &condition.equals {
        return Ok(value == Some(expected));
    }
    if let Some(expected) = &condition.not_equals {
        return Ok(value != Some(expected));
    }
    if let Some(expected) = &condition.contains {
        return Ok(contains_value(value, expected));
    }
    if condition.nonempty == Some(true) {
        return Ok(!is_empty_value(value));
    }
    if condition.empty == Some(true) {
        return Ok(is_empty_value(value));
    }
    if let Some(expected) = &condition.regex {
        let Some(value) = value.and_then(Value::as_str) else {
            return Ok(false);
        };
        let re = Regex::new(expected).map_err(|err| format!("invalid regex: {err}"))?;
        return Ok(re.is_match(value));
    }
    if let Some(expected) = &condition.gt {
        return compare_numeric(value, expected, |a, b| a > b);
    }
    if let Some(expected) = &condition.gte {
        return compare_numeric(value, expected, |a, b| a >= b);
    }
    if let Some(expected) = &condition.lt {
        return compare_numeric(value, expected, |a, b| a < b);
    }
    if let Some(expected) = &condition.lte {
        return compare_numeric(value, expected, |a, b| a <= b);
    }
    Err("unsupported condition".into())
}

fn evaluate_assignment_value(value_spec: &Value, record: &Map<String, Value>, options: &EngineOptions) -> Result<Value, String> {
    if let Some(obj) = value_spec.as_object() {
        if obj.contains_key("if") && obj.contains_key("then") {
            let condition: ConditionSpec =
                serde_json::from_value(obj.get("if").cloned().unwrap_or(Value::Null)).map_err(|err| format!("invalid assign condition: {err}"))?;
            let branch = if evaluate_condition(&condition, record, options)? {
                obj.get("then")
            } else {
                obj.get("else")
            };
            return Ok(branch.cloned().unwrap_or(Value::Null));
        }
        if obj.contains_key("formula") && obj.len() == 1 {
            let formula = obj
                .get("formula")
                .and_then(Value::as_str)
                .ok_or_else(|| "formula assignment requires string formula".to_string())?;
            return evaluate_formula(formula, record, options);
        }
    }
    Ok(value_spec.clone())
}

fn apply_rules_to_record(
    mut record: Map<String, Value>,
    rules: &[RuleSpec],
    mode: &str,
    options: &EngineOptions,
) -> Result<Option<Map<String, Value>>, String> {
    let mut matched_any = false;
    for rule in rules {
        let matched = evaluate_condition(&rule.condition, &record, options)?;
        if matched {
            matched_any = true;
            let dropped = apply_action(&mut record, &rule.then, options)?;
            if mode == "first_match" {
                return Ok(if dropped { None } else { Some(record) });
            }
        } else if let Some(else_action) = &rule.else_action {
            let dropped = apply_action(&mut record, else_action, options)?;
            if mode == "first_match" {
                return Ok(if dropped { None } else { Some(record) });
            }
        }
    }
    if mode == "first_match" && !matched_any {
        Ok(Some(record))
    } else {
        Ok(Some(record))
    }
}

fn apply_action(record: &mut Map<String, Value>, action: &RuleActionSpec, options: &EngineOptions) -> Result<bool, String> {
    if let Some(set) = &action.set {
        for (field, spec) in set {
            let value = evaluate_assignment_value(spec, record, options)?;
            set_path(record, field, value);
        }
    }
    if let Some(tag) = &action.tag {
        append_tag(record, tag);
    }
    if let Some(tags) = &action.tags {
        for tag in tags {
            append_tag(record, tag);
        }
    }
    if let Some(remove_fields) = &action.remove_fields {
        for field in remove_fields {
            remove_path(record, field);
        }
    }
    Ok(action.drop.unwrap_or(false))
}

fn clean_record(mut record: Map<String, Value>, config: &CleanConfig) -> Map<String, Value> {
    let lowercase: std::collections::HashSet<&str> = config.lowercase_fields.iter().map(String::as_str).collect();
    let null_values: std::collections::HashSet<String> = config.null_values.iter().filter_map(value_to_string).collect();
    let coercions = &config.type_coercion;

    let keys: Vec<String> = record.keys().cloned().collect();
    for key in keys {
        if let Some(value) = record.get_mut(&key) {
            clean_value(value, &key, config.trim_strings, &lowercase, &null_values, coercions);
        }
    }
    record
}

fn clean_value(
    value: &mut Value,
    field_path: &str,
    trim_strings: bool,
    lowercase_fields: &std::collections::HashSet<&str>,
    null_values: &std::collections::HashSet<String>,
    coercions: &Map<String, Value>,
) {
    match value {
        Value::String(s) => {
            if trim_strings {
                *s = s.trim().to_string();
            }
            if null_values.contains(s) {
                *value = Value::Null;
                return;
            }
            if lowercase_fields.contains(field_path) {
                *s = s.to_lowercase();
            }
            if let Some(target) = coercions.get(field_path).and_then(Value::as_str) {
                *value = coerce_string(s, target).unwrap_or_else(|| Value::String(s.clone()));
            }
        }
        Value::Array(items) => {
            for item in items {
                clean_value(item, field_path, trim_strings, lowercase_fields, null_values, coercions);
            }
        }
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(item) = map.get_mut(&key) {
                    let nested_path = format!("{field_path}.{key}");
                    clean_value(item, &nested_path, trim_strings, lowercase_fields, null_values, coercions);
                }
            }
        }
        _ => {}
    }
}

fn coerce_string(value: &str, target: &str) -> Option<Value> {
    match target.to_ascii_lowercase().as_str() {
        "number" | "float" | "f64" => value.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number),
        "integer" | "int" | "i64" => value.parse::<i64>().ok().map(|n| Value::Number(n.into())),
        "boolean" | "bool" => value.parse::<bool>().ok().map(Value::Bool),
        "string" => Some(Value::String(value.to_string())),
        _ => None,
    }
}

fn rank_records(records: Vec<Map<String, Value>>, config: &RankConfig, options: &EngineOptions) -> Result<Vec<Map<String, Value>>, String> {
    let score_field = config.score_field.clone().unwrap_or_else(|| "score".into());
    let mut scored: Vec<(f64, String, Map<String, Value>)> = Vec::with_capacity(records.len());
    for mut record in records {
        let score = if let Some(formula) = config.score_formula.as_deref() {
            let value = evaluate_formula(formula, &record, options)?;
            as_f64(&value).ok_or_else(|| "rank formula must evaluate to a number".to_string())?
        } else {
            let value = get_path(&record, &score_field).ok_or_else(|| format!("missing score field '{score_field}'"))?;
            as_f64(value).ok_or_else(|| format!("score field '{score_field}' must be numeric"))?
        };
        set_path(&mut record, &score_field, number_to_value(score)?);
        scored.push((score, stable_json_string(&Value::Object(record.clone())), record));
    }

    scored.sort_by(|a, b| {
        let primary = if config.descending { b.0.partial_cmp(&a.0) } else { a.0.partial_cmp(&b.0) };
        primary.unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.1.cmp(&b.1))
    });

    let mut ranked: Vec<Map<String, Value>> =
        scored.into_iter().enumerate().map(|(idx, (_score, _, mut record))| {
            record.insert("rank".into(), Value::Number((idx + 1).into()));
            record
        }).collect();

    let top_n = config.top_n.unwrap_or(DEFAULT_TOP_N);
    if ranked.len() > top_n {
        ranked.truncate(top_n);
    }
    Ok(ranked)
}

fn aggregate_records(
    records: Vec<Map<String, Value>>,
    config: &AggregateConfig,
    _options: &EngineOptions,
) -> Result<Vec<Map<String, Value>>, String> {
    if config.group_by.is_empty() {
        return Err("aggregate_records requires group_by".into());
    }
    if config.metrics.is_empty() {
        return Err("aggregate_records requires metrics".into());
    }

    #[derive(Default, Clone)]
    struct Bucket {
        group_values: Map<String, Value>,
        rows: Vec<Map<String, Value>>,
    }

    let mut buckets: std::collections::BTreeMap<String, Bucket> = std::collections::BTreeMap::new();
    for record in records {
        let key_value = Value::Array(
            config
                .group_by
                .iter()
                .map(|field| get_path(&record, field).cloned().unwrap_or(Value::Null))
                .collect(),
        );
        let key = stable_json_string(&key_value);
        let mut bucket = buckets.remove(&key).unwrap_or_default();
        for field in &config.group_by {
            bucket.group_values.insert(field.clone(), get_path(&record, field).cloned().unwrap_or(Value::Null));
        }
        bucket.rows.push(record);
        buckets.insert(key, bucket);
    }

    let mut out = Vec::new();
    for (_key, bucket) in buckets {
        let mut row = bucket.group_values;
        for (metric_name, expr_value) in &config.metrics {
            let expr = expr_value
                .as_str()
                .ok_or_else(|| format!("metric '{metric_name}' must be a string expression"))?;
            let metric_value = evaluate_metric(expr, &bucket.rows)?;
            row.insert(metric_name.clone(), metric_value);
        }
        out.push(row);
    }
    Ok(out)
}

fn evaluate_metric(expr: &str, rows: &[Map<String, Value>]) -> Result<Value, String> {
    let expr = expr.trim();
    let open = expr.find('(').ok_or_else(|| format!("invalid metric expression '{expr}'"))?;
    let close = expr.rfind(')').ok_or_else(|| format!("invalid metric expression '{expr}'"))?;
    if close <= open {
        return Err(format!("invalid metric expression '{expr}'"));
    }
    let name = expr[..open].trim().to_ascii_lowercase();
    let arg = expr[open + 1..close].trim();
    match name.as_str() {
        "count" => {
            if arg == "*" {
                Ok(Value::Number((rows.len() as i64).into()))
            } else {
                Ok(Value::Number((rows.iter().filter(|row| get_path(row, arg).map(|v| !v.is_null()).unwrap_or(false)).count() as i64).into()))
            }
        }
        "sum" | "avg" | "min" | "max" => {
            let mut values = Vec::new();
            for row in rows {
                if let Some(value) = get_path(row, arg).and_then(as_f64) {
                    values.push(value);
                }
            }
            if values.is_empty() {
                return Ok(Value::Null);
            }
            let result = match name.as_str() {
                "sum" => values.iter().sum(),
                "avg" => values.iter().sum::<f64>() / values.len() as f64,
                "min" => values.iter().fold(f64::INFINITY, |a, b| a.min(*b)),
                "max" => values.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b)),
                _ => unreachable!(),
            };
            Ok(number_to_value(result)?)
        }
        _ => Err(format!("unsupported metric '{name}'")),
    }
}

fn extract_structured_data(
    records: Vec<Map<String, Value>>,
    config: &ExtractConfig,
    options: &EngineOptions,
) -> Result<(Vec<Map<String, Value>>, Vec<String>, Vec<String>), String> {
    let source_field = config.source_field.clone().unwrap_or_else(|| "content".into());
    let schema = config.schema.clone();
    let mut required_fields: std::collections::HashSet<String> = config.required_fields.iter().cloned().collect();
    let mut field_specs = Vec::<(String, ExtractFieldSpec)>::new();
    for (field, raw_spec) in schema {
        let spec: ExtractFieldSpec = if raw_spec.is_object() {
            serde_json::from_value(raw_spec).map_err(|err| format!("invalid schema spec for '{field}': {err}"))?
        } else if let Some(pattern) = raw_spec.as_str() {
            ExtractFieldSpec { pattern: pattern.to_string(), r#type: default_extract_type(), required: false }
        } else {
            return Err(format!("schema spec for '{field}' must be an object or string pattern"));
        };
        if spec.required {
            required_fields.insert(field.clone());
        }
        field_specs.push((field, spec));
    }

    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let errors = Vec::new();
    let mut missing_fields = std::collections::BTreeSet::new();
    let mut found_required = 0usize;
    let required_total = required_fields.len().max(1);

    for mut record in records {
        let source = get_path(&record, &source_field)
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        if source.is_empty() {
            warnings.push(format!("missing_field:{source_field}"));
        }
        for (field, spec) in &field_specs {
            let value = extract_field_value(&source, spec, options)?;
            if value.is_null() {
                if required_fields.contains(field) {
                    missing_fields.insert(field.clone());
                }
                continue;
            }
            if required_fields.contains(field) {
                found_required += 1;
            }
            set_path(&mut record, field, value);
        }
        out.push(record);
    }

    if !missing_fields.is_empty() {
        for field in &missing_fields {
            warnings.push(format!("missing_field:{field}"));
        }
        warnings.push("fallback_needed: deterministic extraction missing required fields".into());
    }

    let _confidence = (found_required as f64 / required_total as f64).clamp(0.0, 1.0);
    Ok((out, warnings, errors))
}

fn extract_field_value(source: &str, spec: &ExtractFieldSpec, options: &EngineOptions) -> Result<Value, String> {
    if spec.pattern.len() > options.max_regex_length {
        return Err(format!("pattern too long ({} > {})", spec.pattern.len(), options.max_regex_length));
    }
    let regex = Regex::new(&spec.pattern).map_err(|err| format!("invalid pattern: {err}"))?;
    let Some(captures) = regex.captures(source) else {
        return Ok(Value::Null);
    };
    let matched = captures.get(1).or_else(|| captures.get(0)).map(|m| m.as_str()).unwrap_or("");
    if matched.is_empty() {
        return Ok(Value::Null);
    }
    match spec.r#type.to_ascii_lowercase().as_str() {
        "number" | "float" | "f64" => {
            let normalized = matched.replace(',', "");
            let number = normalized.parse::<f64>().map_err(|err| format!("failed to parse number '{matched}': {err}"))?;
            number_to_value(number)
        }
        "integer" | "int" | "i64" => {
            let normalized = matched.replace(',', "");
            let number = normalized.parse::<i64>().map_err(|err| format!("failed to parse integer '{matched}': {err}"))?;
            Ok(Value::Number(number.into()))
        }
        "boolean" | "bool" => matched.parse::<bool>().map(Value::Bool).map_err(|err| format!("failed to parse bool '{matched}': {err}")),
        _ => Ok(Value::String(matched.to_string())),
    }
}

fn evaluate_formula(formula: &str, record: &Map<String, Value>, options: &EngineOptions) -> Result<Value, String> {
    let expr = parse_formula(formula)?;
    if expr_depth(&expr) > options.max_depth {
        return Err(format!("formula expression depth exceeds max_depth={}", options.max_depth));
    }
    evaluate_expr(&expr, record)
}

fn parse_formula(formula: &str) -> Result<Expr, String> {
    let mut parser = FormulaParser::new(formula);
    let expr = parser.parse_expression()?;
    parser.expect_end()?;
    Ok(expr)
}

fn evaluate_expr(expr: &Expr, record: &Map<String, Value>) -> Result<Value, String> {
    match expr {
        Expr::Literal(value) => Ok(value.clone()),
        Expr::Field(field) => Ok(get_path(record, field).cloned().unwrap_or(Value::Null)),
        Expr::UnaryMinus(inner) => {
            let number = as_f64(&evaluate_expr(inner, record)?).ok_or_else(|| "unary minus expects numeric value".to_string())?;
            number_to_value(-number)
        }
        Expr::Binary(left, op, right) => {
            let left = evaluate_expr(left, record)?;
            let right = evaluate_expr(right, record)?;
            let l = as_f64(&left).ok_or_else(|| "arithmetic expects numeric values".to_string())?;
            let r = as_f64(&right).ok_or_else(|| "arithmetic expects numeric values".to_string())?;
            match op {
                BinOp::Add => number_to_value(l + r),
                BinOp::Sub => number_to_value(l - r),
                BinOp::Mul => number_to_value(l * r),
                BinOp::Div => {
                    if r == 0.0 {
                        Err("division by zero".into())
                    } else {
                        number_to_value(l / r)
                    }
                }
            }
        }
        Expr::Call(name, args) => evaluate_call(name, args, record),
    }
}

fn evaluate_call(name: &str, args: &[Expr], record: &Map<String, Value>) -> Result<Value, String> {
    let evaluated: Vec<Value> = args.iter().map(|arg| evaluate_expr(arg, record)).collect::<Result<_, _>>()?;
    match name.to_ascii_lowercase().as_str() {
        "coalesce" => Ok(evaluated.into_iter().find(|value| !value.is_null()).unwrap_or(Value::Null)),
        "abs" => {
            let value = evaluated.get(0).ok_or_else(|| "abs expects one argument".to_string())?;
            number_to_value(as_f64(value).ok_or_else(|| "abs expects numeric value".to_string())?.abs())
        }
        "min" => {
            let values = evaluated.iter().map(as_f64).collect::<Option<Vec<_>>>().ok_or_else(|| "min expects numeric values".to_string())?;
            let first = values.first().copied().ok_or_else(|| "min expects at least one argument".to_string())?;
            number_to_value(values.into_iter().fold(first, f64::min))
        }
        "max" => {
            let values = evaluated.iter().map(as_f64).collect::<Option<Vec<_>>>().ok_or_else(|| "max expects numeric values".to_string())?;
            let first = values.first().copied().ok_or_else(|| "max expects at least one argument".to_string())?;
            number_to_value(values.into_iter().fold(first, f64::max))
        }
        "round" => {
            let value = evaluated.get(0).ok_or_else(|| "round expects at least one argument".to_string())?;
            let digits = evaluated.get(1).and_then(Value::as_i64).unwrap_or(0).clamp(0, 10) as i32;
            let n = as_f64(value).ok_or_else(|| "round expects numeric value".to_string())?;
            let scale = 10f64.powi(digits);
            number_to_value((n * scale).round() / scale)
        }
        other => Err(format!("unsupported formula helper '{other}'")),
    }
}

fn expr_depth(expr: &Expr) -> usize {
    match expr {
        Expr::Literal(_) | Expr::Field(_) => 1,
        Expr::UnaryMinus(inner) => 1 + expr_depth(inner),
        Expr::Binary(left, _, right) => 1 + expr_depth(left).max(expr_depth(right)),
        Expr::Call(_, args) => 1 + args.iter().map(expr_depth).max().unwrap_or(0),
    }
}

#[derive(Debug, Clone)]
enum Expr {
    Literal(Value),
    Field(String),
    UnaryMinus(Box<Expr>),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

struct FormulaParser<'a> {
    chars: Vec<char>,
    pos: usize,
    _source: &'a str,
}

impl<'a> FormulaParser<'a> {
    fn new(source: &'a str) -> Self {
        Self { chars: source.chars().collect(), pos: 0, _source: source }
    }

    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_add_sub()
    }

    fn parse_add_sub(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_mul_div()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => BinOp::Add,
                Some('-') => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_mul_div()?;
            expr = Expr::Binary(Box::new(expr), op, Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_mul_div(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => BinOp::Mul,
                Some('/') => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            expr = Expr::Binary(Box::new(expr), op, Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(Expr::UnaryMinus(Box::new(self.parse_unary()?)))
            }
            Some('+') => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        self.skip_ws();
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let expr = self.parse_expression()?;
                self.skip_ws();
                self.expect(')')?;
                Ok(expr)
            }
            Some('"') | Some('\'') => self.parse_string().map(Expr::Literal),
            Some(ch) if ch.is_ascii_digit() || ch == '.' => self.parse_number().map(Expr::Literal),
            Some(ch) if is_ident_start(ch) => {
                let ident = self.parse_identifier();
                self.skip_ws();
                if self.peek() == Some('(') {
                    self.pos += 1;
                    let mut args = Vec::new();
                    self.skip_ws();
                    if self.peek() != Some(')') {
                        loop {
                            args.push(self.parse_expression()?);
                            self.skip_ws();
                            match self.peek() {
                                Some(',') => self.pos += 1,
                                Some(')') => break,
                                _ => return Err("expected ',' or ')' in function call".into()),
                            }
                        }
                    }
                    self.expect(')')?;
                    Ok(Expr::Call(ident, args))
                } else {
                    Ok(Expr::Field(ident))
                }
            }
            Some(other) => Err(format!("unexpected character '{other}' in formula")),
            None => Err("unexpected end of formula".into()),
        }
    }

    fn parse_identifier(&mut self) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if is_ident_continue(ch) {
                out.push(ch);
                self.pos += 1;
            } else {
                break;
            }
        }
        out
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                out.push(ch);
                self.pos += 1;
            } else {
                break;
            }
        }
        let number = out.parse::<f64>().map_err(|err| format!("invalid number '{out}': {err}"))?;
        number_to_value(number)
    }

    fn parse_string(&mut self) -> Result<Value, String> {
        let quote = self.peek().ok_or_else(|| "unexpected end of string".to_string())?;
        self.pos += 1;
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            self.pos += 1;
            if ch == quote {
                return Ok(Value::String(out));
            }
            if ch == '\\' {
                let escaped = self.peek().ok_or_else(|| "unterminated escape sequence".to_string())?;
                self.pos += 1;
                match escaped {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    '\\' => out.push('\\'),
                    other => out.push(other),
                }
            } else {
                out.push(ch);
            }
        }
        Err("unterminated string literal".into())
    }

    fn expect(&mut self, ch: char) -> Result<(), String> {
        self.skip_ws();
        match self.peek() {
            Some(current) if current == ch => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(format!("expected '{ch}'")),
        }
    }

    fn expect_end(&mut self) -> Result<(), String> {
        self.skip_ws();
        if self.peek().is_some() {
            Err("unexpected trailing characters".into())
        } else {
            Ok(())
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'
}

fn required_field_name<'a>(field: Option<&'a str>, message: &str) -> Result<&'a str, String> {
    field.filter(|s| !s.trim().is_empty()).ok_or_else(|| message.to_string())
}

fn required_formula<'a>(formula: Option<&'a str>, message: &str) -> Result<&'a str, String> {
    formula.filter(|s| !s.trim().is_empty()).ok_or_else(|| message.to_string())
}

fn default_rank_descending() -> bool {
    true
}

fn default_extract_type() -> String {
    "string".into()
}

fn default_rule_mode() -> String {
    "first_match".into()
}

fn number_to_value(number: f64) -> Result<Value, String> {
    serde_json::Number::from_f64(number).map(Value::Number).ok_or_else(|| "invalid numeric value".into())
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Bool(true) => Some(1.0),
        Value::Bool(false) => Some(0.0),
        _ => None,
    }
}

fn compare_numeric<F>(left: Option<&Value>, right: &Value, f: F) -> Result<bool, String>
where
    F: FnOnce(f64, f64) -> bool,
{
    let left = left.and_then(as_f64).ok_or_else(|| "numeric comparison requires numeric field value".to_string())?;
    let right = as_f64(right).ok_or_else(|| "numeric comparison requires numeric comparison value".to_string())?;
    Ok(f(left, right))
}

fn contains_value(left: Option<&Value>, right: &Value) -> bool {
    match left {
        Some(Value::String(text)) => right.as_str().map(|needle| text.contains(needle)).unwrap_or(false),
        Some(Value::Array(items)) => items.iter().any(|item| item == right),
        Some(Value::Object(map)) => right.as_str().map(|key| map.contains_key(key)).unwrap_or(false),
        _ => false,
    }
}

fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(v)) => *v,
        Some(Value::Number(n)) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(Value::Null) | None => false,
    }
}

fn is_empty_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Null) | None => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(map)) => map.is_empty(),
        _ => false,
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn set_path(record: &mut Map<String, Value>, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    set_path_parts(record, &parts, value);
}

fn set_path_parts(map: &mut Map<String, Value>, parts: &[&str], value: Value) {
    if parts.len() == 1 {
        map.insert(parts[0].to_string(), value);
        return;
    }
    let entry = map.entry(parts[0].to_string()).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let child = entry.as_object_mut().expect("object just created");
    set_path_parts(child, &parts[1..], value);
}

fn get_path<'a>(record: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut current: Option<&Value> = None;
    for (idx, part) in path.split('.').enumerate() {
        if idx == 0 {
            current = record.get(part);
        } else if let Some(Value::Object(next)) = current {
            current = next.get(part);
        } else {
            return None;
        }
    }
    current
}

fn remove_path(record: &mut Map<String, Value>, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    remove_path_parts(record, &parts);
}

fn remove_path_parts(map: &mut Map<String, Value>, parts: &[&str]) {
    if parts.len() == 1 {
        map.remove(parts[0]);
        return;
    }
    if let Some(Value::Object(child)) = map.get_mut(parts[0]) {
        remove_path_parts(child, &parts[1..]);
    }
}

fn append_tag(record: &mut Map<String, Value>, tag: &str) {
    let entry = record.entry("tags".to_string()).or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(items) = entry {
        if !items.iter().any(|item| item.as_str() == Some(tag)) {
            items.push(Value::String(tag.to_string()));
        }
    } else {
        *entry = Value::Array(vec![Value::String(tag.to_string())]);
    }
}

fn dedupe_strings(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    values.iter().filter(|value| seen.insert((*value).clone())).cloned().collect()
}

fn confidence_score(input_count: u64, output_count: u64, warnings: &[String], errors: &[String]) -> f64 {
    if input_count == 0 {
        return 1.0;
    }
    let base = output_count as f64 / input_count as f64;
    let penalty = ((warnings.len() + errors.len()) as f64 * 0.05).min(0.5);
    (base - penalty).clamp(0.0, 1.0)
}

fn stable_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(stable_json_string).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let mut items: Vec<(&String, &Value)> = map.iter().collect();
            items.sort_by(|a, b| a.0.cmp(b.0));
            let inner: Vec<String> = items
                .into_iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), stable_json_string(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    #[tokio::test]
    async fn test_pipeline_filter_map_compute() {
        let tool = DataEngineTool;
        let result = tool
            .execute(serde_json::json!({
                "records": [
                    {"name": "a", "email": "a@example.com", "revenue": 100, "engagement": 0.5},
                    {"name": "b", "revenue": 50, "engagement": 0.1}
                ],
                "pipeline": [
                    {"op": "filter", "condition": {"field": "email", "exists": true}},
                    {"op": "map", "assign": {"priority": {"if": {"field": "revenue", "gt": 80}, "then": "high", "else": "low"}}},
                    {"op": "compute", "field": "score", "formula": "(revenue * 0.3) + (engagement * 0.7)"}
                ]
            }))
            .await
            .unwrap();

        assert!(result.success);
        let records = result.output["records"].as_array().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["priority"], "high");
        assert_eq!(result.output["meta"]["ops_applied"], serde_json::json!(["filter", "map", "compute"]));
    }

    #[tokio::test]
    async fn test_clean_data_explicit_rules() {
        let tool = DataEngineTool;
        let result = tool
            .execute(serde_json::json!({
                "records": [{"email": "  USER@EXAMPLE.COM  ", "revenue": "100"}],
                "op": "clean_data",
                "config": {
                    "trim_strings": true,
                    "lowercase_fields": ["email"],
                    "null_values": ["", "N/A"],
                    "type_coercion": {"revenue": "number"}
                }
            }))
            .await
            .unwrap();

        assert!(result.success);
        let record = &result.output["records"][0];
        assert_eq!(record["email"], "user@example.com");
        assert_eq!(record["revenue"], serde_json::json!(100.0));
    }

    #[tokio::test]
    async fn test_compute_formula_rejects_bad_formula() {
        let tool = DataEngineTool;
        let result = tool
            .execute(serde_json::json!({
                "records": [{"revenue": 100}],
                "op": "compute_formula",
                "config": {"field": "score", "formula": "revenue / 0"}
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("division by zero"));
    }

    #[tokio::test]
    async fn test_apply_rules_first_match_and_all_match() {
        let tool = DataEngineTool;
        let first = tool
            .execute(serde_json::json!({
                "records": [{"revenue": 120, "tags": []}],
                "op": "apply_rules",
                "config": {
                    "mode": "first_match",
                    "rules": [
                        {"if": {"field": "revenue", "gt": 100}, "then": {"set": {"priority": "high"}, "tag": "tier1"}},
                        {"if": {"field": "revenue", "gt": 50}, "then": {"set": {"priority": "medium"}, "tag": "tier2"}}
                    ]
                }
            }))
            .await
            .unwrap();
        assert!(first.success);
        let record = &first.output["records"][0];
        assert_eq!(record["priority"], "high");
        assert_eq!(record["tags"], serde_json::json!(["tier1"]));

        let all = tool
            .execute(serde_json::json!({
                "records": [{"revenue": 120, "tags": []}],
                "op": "apply_rules",
                "config": {
                    "mode": "all_match",
                    "rules": [
                        {"if": {"field": "revenue", "gt": 100}, "then": {"set": {"priority": "high"}, "tag": "tier1"}},
                        {"if": {"field": "revenue", "gt": 50}, "then": {"set": {"segment": "warm"}, "tag": "tier2"}}
                    ]
                }
            }))
            .await
            .unwrap();
        assert!(all.success);
        let record = &all.output["records"][0];
        assert_eq!(record["priority"], "high");
        assert_eq!(record["segment"], "warm");
        assert_eq!(record["tags"], serde_json::json!(["tier1", "tier2"]));
    }

    #[tokio::test]
    async fn test_aggregate_records_order_invariant() {
        let tool = DataEngineTool;
        let a = tool
            .execute(serde_json::json!({
                "records": [
                    {"region": "US", "revenue": 10},
                    {"region": "EU", "revenue": 20},
                    {"region": "US", "revenue": 30}
                ],
                "op": "aggregate_records",
                "config": {
                    "group_by": ["region"],
                    "metrics": {"total_revenue": "sum(revenue)", "count_all": "count(*)"}
                }
            }))
            .await
            .unwrap();
        let b = tool
            .execute(serde_json::json!({
                "records": [
                    {"region": "US", "revenue": 30},
                    {"region": "US", "revenue": 10},
                    {"region": "EU", "revenue": 20}
                ],
                "op": "aggregate_records",
                "config": {
                    "group_by": ["region"],
                    "metrics": {"total_revenue": "sum(revenue)", "count_all": "count(*)"}
                }
            }))
            .await
            .unwrap();

        assert!(a.success && b.success);
        assert_eq!(a.output["records"], b.output["records"]);
    }

    #[tokio::test]
    async fn test_rank_items_order_invariant() {
        let tool = DataEngineTool;
        let a = tool
            .execute(serde_json::json!({
                "records": [
                    {"name": "a", "score": 1},
                    {"name": "b", "score": 5},
                    {"name": "c", "score": 3}
                ],
                "op": "rank_items",
                "config": {"score_field": "score", "top_n": 2, "descending": true}
            }))
            .await
            .unwrap();
        let b = tool
            .execute(serde_json::json!({
                "records": [
                    {"name": "c", "score": 3},
                    {"name": "a", "score": 1},
                    {"name": "b", "score": 5}
                ],
                "op": "rank_items",
                "config": {"score_field": "score", "top_n": 2, "descending": true}
            }))
            .await
            .unwrap();

        assert!(a.success && b.success);
        assert_eq!(a.output["records"], b.output["records"]);
    }

    #[tokio::test]
    async fn test_extract_structured_data_reports_missing_fields() {
        let tool = DataEngineTool;
        let result = tool
            .execute(serde_json::json!({
                "records": [{"content": "invoice INV-123 amount 4500"}],
                "op": "extract_structured_data",
                "config": {
                    "source_field": "content",
                    "required_fields": ["invoice_id", "amount"],
                    "schema": {
                        "invoice_id": {"pattern": "(INV-[0-9]+)", "type": "string", "required": true},
                        "amount": {"pattern": "amount\\s+([0-9]+)", "type": "number", "required": true}
                    }
                }
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.output["meta"]["fallback_needed"], true);
        let missing = result.output["meta"]["missing_fields"].as_array().unwrap();
        assert!(!missing.is_empty());
    }

    #[tokio::test]
    async fn test_pipeline_meta_populated() {
        let tool = DataEngineTool;
        let result = tool
            .execute(serde_json::json!({
                "records": [{"name": "a", "score": 1}, {"name": "b", "score": 2}],
                "pipeline": [{"op": "rank_items", "config": {"score_field": "score", "top_n": 2}}]
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output["meta"]["execution_time_ms"].as_u64().unwrap() > 0);
        assert_eq!(result.output["meta"]["ops_applied"], serde_json::json!(["rank_items"]));
    }
}
