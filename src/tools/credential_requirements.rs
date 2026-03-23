//! Credential scanning for the plan-approval gate.
//!
//! ## Single source of truth
//!
//! Previously this file maintained a separate TOOL_CREDENTIALS map with
//! fantasy tool names like "salesforce_query" that never existed in the
//! registry. That list was guaranteed to drift from connector_tool::ALL_CONNECTORS.
//!
//! Now credential requirements are derived directly from ALL_CONNECTORS:
//!   - If a step's tool name matches a connector name → that connector's
//!     credential is required
//!   - If the connector isn't installed → red dot + flagged as missing
//!
//! The only exception is tools with multiple accepted providers
//! (email: gmail OR outlook). These are declared in MULTI_PROVIDER_TOOLS below.
//! Everything else derives automatically from ALL_CONNECTORS.
//!
//! ## Adding a new connector
//!
//! Add it to connector_tool::ALL_CONNECTORS. Nothing else needs to change here.
//! The credential check will work automatically because the tool name IS the
//! credential provider name for all connectors.
//!
//! ## Adding a non-connector tool with credentials
//!
//! Add it to MULTI_PROVIDER_TOOLS below with its accepted provider list.

use crate::tools::connector_tool::ALL_CONNECTORS;

/// Tools that accept any one of several credential providers.
/// The check passes if at least one of the listed providers is installed.
/// Only needed for tools NOT in ALL_CONNECTORS (i.e. non-connector tools
/// that have flexible auth like email).
static MULTI_PROVIDER_TOOLS: &[(&str, &[&str])] = &[
    ("email_send", &["gmail", "outlook"]),
    ("email_read", &["gmail", "outlook"]),
    ("email",      &["gmail", "outlook"]),
];

/// Returns the credential provider(s) required for a given tool name.
/// Returns None if the tool has no credential requirement.
pub fn required_credentials(tool_name: &str) -> Option<Vec<&'static str>> {
    // Check multi-provider tools first
    for (name, providers) in MULTI_PROVIDER_TOOLS {
        if *name == tool_name {
            return Some(providers.to_vec());
        }
    }

    // Check ALL_CONNECTORS — connector name IS the credential name
    for def in ALL_CONNECTORS {
        if def.name == tool_name {
            // Strip "connector/" prefix from category to get the credential key
            // e.g. connector/crm → connector name "salesforce" → credential "salesforce"
            return Some(vec![def.name]);
        }
    }

    None
}

/// Determine confidence colour for a single plan step.
///
/// - "green" → a skill in the registry covers this step exactly
/// - "amber" → tool is known but no skill match (LLM improvises)
/// - "red"   → required credential is not installed
fn step_confidence(
    tool: Option<&str>,
    skill_names: &[String],
    description: &str,
    tenant_credentials: &[String],
) -> &'static str {
    if let Some(tool_name) = tool {
        if let Some(required) = required_credentials(tool_name) {
            let any_present = required
                .iter()
                .any(|cred| tenant_credentials.iter().any(|tc| tc == cred));
            if !any_present {
                return "red";
            }
        }
    }

    // Green: a skill in the registry covers this step
    let desc_lower = description.to_lowercase();
    if skill_names.iter().any(|s| desc_lower.contains(&s.to_lowercase())) {
        return "green";
    }

    "amber"
}

/// Scan a plan for missing credentials and compute per-step confidence colours.
///
/// # Arguments
/// * `planned_tools`      – `step.tool` for each planned step (may be `None`)
/// * `tenant_credentials` – credential provider names installed for this tenant
///                          (from both ConnectorInstallStore and TenantConfig credentials)
/// * `skill_names`        – skill names from the registry (for green confidence)
/// * `step_descriptions`  – description text for each step
///
/// # Returns
/// `(missing_credentials, step_confidence_colours)`
///
/// `missing_credentials` is deduplicated and sorted — these are the provider
/// names the frontend shows in the CredentialGap banners.
/// `step_confidence_colours` has one entry per step in input order.
pub fn scan_plan_credentials(
    planned_tools: &[Option<String>],
    tenant_credentials: &[String],
    skill_names: &[String],
    step_descriptions: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut missing: std::collections::HashSet<String> = Default::default();
    let mut confidences: Vec<String> = Vec::with_capacity(planned_tools.len());

    for (i, tool_opt) in planned_tools.iter().enumerate() {
        let description = step_descriptions.get(i).map(String::as_str).unwrap_or("");

        if let Some(tool_name) = tool_opt.as_deref() {
            if let Some(required) = required_credentials(tool_name) {
                let any_present = required
                    .iter()
                    .any(|cred| tenant_credentials.iter().any(|tc| tc == cred));
                if !any_present {
                    for cred in &required {
                        missing.insert(cred.to_string());
                    }
                }
            }
        }

        let colour = step_confidence(
            tool_opt.as_deref(),
            skill_names,
            description,
            tenant_credentials,
        );
        confidences.push(colour.to_string());
    }

    let mut missing_sorted: Vec<String> = missing.into_iter().collect();
    missing_sorted.sort();

    (missing_sorted, confidences)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── required_credentials ───────────────────────────────────────────────

    #[test]
    fn test_connector_tool_names_resolve_to_themselves() {
        // Every connector in ALL_CONNECTORS should resolve to its own name
        for def in ALL_CONNECTORS {
            let creds = required_credentials(def.name)
                .unwrap_or_else(|| panic!("connector '{}' has no credential requirement", def.name));
            assert_eq!(creds, vec![def.name],
                "connector '{}' should require credential '{}'", def.name, def.name);
        }
    }

    #[test]
    fn test_email_accepts_either_provider() {
        let creds = required_credentials("email_send").unwrap();
        assert!(creds.contains(&"gmail"));
        assert!(creds.contains(&"outlook"));
    }

    #[test]
    fn test_unknown_tool_returns_none() {
        assert!(required_credentials("shell").is_none());
        assert!(required_credentials("file_read").is_none());
        assert!(required_credentials("web_search_tool").is_none());
        assert!(required_credentials("code_run").is_none());
    }

    // ── scan_plan_credentials ─────────────────────────────────────────────

    #[test]
    fn test_connector_tool_red_when_not_installed() {
        // Real connector tool names — the ones the LLM actually uses now
        let tools = vec![Some("salesforce".into()), Some("slack".into())];
        let creds: Vec<String> = vec![];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.contains(&"salesforce".to_string()));
        assert!(missing.contains(&"slack".to_string()));
        assert!(confidence.iter().all(|c| c == "red"));
    }

    #[test]
    fn test_connector_tool_green_when_installed() {
        let tools = vec![Some("salesforce".into()), Some("github".into())];
        let creds = vec!["salesforce".into(), "github".into()];
        let (missing, _) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_intercom_red_when_not_installed() {
        // intercom is in ALL_CONNECTORS → auto-detected, no manual entry needed
        let tools = vec![Some("intercom".into())];
        let creds: Vec<String> = vec![];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.contains(&"intercom".to_string()));
        assert_eq!(confidence[0], "red");
    }

    #[test]
    fn test_intercom_green_when_installed() {
        let tools = vec![Some("intercom".into())];
        let creds = vec!["intercom".into()];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.is_empty());
        assert_eq!(confidence[0], "amber"); // amber because no skill match
    }

    #[test]
    fn test_no_missing_for_core_tools() {
        // Core tools (shell, file_read etc.) don't need credentials
        let tools = vec![
            Some("shell".into()),
            Some("file_read".into()),
            Some("web_search_tool".into()),
        ];
        let creds: Vec<String> = vec![];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.is_empty());
        assert!(confidence.iter().all(|c| c == "amber"));
    }

    #[test]
    fn test_email_send_with_outlook_only() {
        let tools = vec![Some("email_send".into())];
        let creds = vec!["outlook".into()];
        let (missing, _) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_email_send_no_provider() {
        let tools = vec![Some("email_send".into())];
        let creds: Vec<String> = vec![];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        // Both gmail and outlook listed as missing
        assert!(missing.contains(&"gmail".to_string()));
        assert!(missing.contains(&"outlook".to_string()));
        assert_eq!(confidence[0], "red");
    }

    #[test]
    fn test_skill_match_gives_green_regardless_of_tool() {
        let tools = vec![None];
        let creds: Vec<String> = vec![];
        let skills = vec!["deploy service".into()];
        let descs = vec!["deploy service to kubernetes".into()];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &skills, &descs);
        assert!(missing.is_empty());
        assert_eq!(confidence[0], "green");
    }

    #[test]
    fn test_all_connectors_in_registry_are_scannable() {
        // Regression: every connector in ALL_CONNECTORS must produce a red dot
        // when its credential is absent, ensuring frontend gaps are accurate.
        for def in ALL_CONNECTORS {
            let tools = vec![Some(def.name.to_string())];
            let creds: Vec<String> = vec![];
            let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
            assert!(
                missing.contains(&def.name.to_string()),
                "connector '{}' should appear in missing credentials when not installed",
                def.name
            );
            assert_eq!(
                confidence[0], "red",
                "connector '{}' should get red confidence when credential absent",
                def.name
            );
        }
    }

    #[test]
    fn test_mixed_plan_partial_creds() {
        // salesforce installed, jira not
        let tools = vec![
            Some("salesforce".into()),
            Some("jira".into()),
            Some("web_search_tool".into()),
        ];
        let creds = vec!["salesforce".into()];
        let (missing, confidence) = scan_plan_credentials(&tools, &creds, &[], &[]);
        assert!(!missing.contains(&"salesforce".to_string()));
        assert!(missing.contains(&"jira".to_string()));
        assert_eq!(confidence[0], "amber"); // salesforce installed, no skill → amber
        assert_eq!(confidence[1], "red");   // jira missing
        assert_eq!(confidence[2], "amber"); // web_search has no credential requirement
    }
}
