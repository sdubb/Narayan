use anyhow::Result;

use crate::boundry::{BoundaryHandshake, BoundaryStep, TypedSchema};
use crate::boundry::data_barrier::DataBarrierViolation;

/// Validation errors surfaced as compiler errors.
#[derive(Debug, Clone)]
pub enum BoundaryValidationError {
    HandshakeNotFound { handshake_id: String },
    HandshakeNotAccepted { handshake_id: String, pending_party: String },
    HandshakeFrozen { handshake_id: String },
    HandshakeRevoked { handshake_id: String },
    HandshakeExpired { handshake_id: String, expired_at: chrono::DateTime<chrono::Utc> },
    HandshakeNotStarted { handshake_id: String, valid_from: chrono::DateTime<chrono::Utc> },
    HandshakeVersionMismatch { step_version: u32, current_version: u32 },
    RequestSchemaMismatch { unexpected_fields: Vec<String>, missing_required_fields: Vec<String> },
    ResponseSchemaMismatch { mismatched_fields: Vec<String> },
    MissingPeerResource { resource_id: String },
    PeerEndpointMismatch { step_endpoint: String, handshake_endpoint: String },
    DataBarrierViolation { violations: Vec<DataBarrierViolation> },
}

impl std::fmt::Display for BoundaryValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HandshakeNotFound { handshake_id } =>
                write!(f, "handshake not found: {}", handshake_id),
            Self::HandshakeNotAccepted { handshake_id, pending_party } =>
                write!(f, "handshake {} not yet accepted by {}", handshake_id, pending_party),
            Self::HandshakeFrozen { handshake_id } =>
                write!(f, "handshake {} is frozen — unfreeze before compiling", handshake_id),
            Self::HandshakeRevoked { handshake_id } =>
                write!(f, "handshake {} has been permanently revoked — create a new handshake", handshake_id),
            Self::HandshakeExpired { handshake_id, expired_at } =>
                write!(f, "handshake {} expired at {}", handshake_id, expired_at),
            Self::HandshakeNotStarted { handshake_id, valid_from } =>
                write!(f, "handshake {} is not yet valid (valid from {})", handshake_id, valid_from),
            Self::HandshakeVersionMismatch { step_version, current_version } =>
                write!(f, "step uses handshake version {} but current version is {}", step_version, current_version),
            Self::RequestSchemaMismatch { unexpected_fields, missing_required_fields } =>
                write!(f, "request schema mismatch — unexpected: {:?}, missing required: {:?}",
                    unexpected_fields, missing_required_fields),
            Self::ResponseSchemaMismatch { mismatched_fields } =>
                write!(f, "response schema mismatch — fields: {:?}", mismatched_fields),
            Self::MissingPeerResource { resource_id } =>
                write!(f, "no acp_peer resource bound for id: {}", resource_id),
            Self::PeerEndpointMismatch { step_endpoint, handshake_endpoint } =>
                write!(f, "peer endpoint mismatch — step: {}, handshake: {}", step_endpoint, handshake_endpoint),
            Self::DataBarrierViolation { violations } =>
                write!(f, "data barrier violations: {}", violations.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("; ")),
        }
    }
}

impl std::error::Error for BoundaryValidationError {}

// ── Schema validation ─────────────────────────────────────────────────────────

/// Validate a JSON payload against the TypedSchema declared in the handshake.
/// Returns all field path errors accumulated (not short-circuit).
pub fn validate_payload_against_schema(
    payload: &serde_json::Value,
    schema: &TypedSchema,
) -> Result<()> {
    let errors = validate_recursive(payload, schema, "");
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("schema validation failed:\n{}", errors.join("\n")))
    }
}

fn validate_recursive(
    value: &serde_json::Value,
    schema: &TypedSchema,
    path: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    match schema {
        TypedSchema::String => {
            if !value.is_string() {
                errors.push(format!("{}: expected string, got {}", path, type_name(value)));
            }
        }
        TypedSchema::Number => {
            if !value.is_number() {
                errors.push(format!("{}: expected number, got {}", path, type_name(value)));
            }
        }
        TypedSchema::Boolean => {
            if !value.is_boolean() {
                errors.push(format!("{}: expected boolean, got {}", path, type_name(value)));
            }
        }
        TypedSchema::Array { items } => {
            if let Some(arr) = value.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    let item_path = format!("{}.{}", path, i);
                    errors.extend(validate_recursive(item, items, &item_path));
                }
            } else {
                errors.push(format!("{}: expected array, got {}", path, type_name(value)));
            }
        }
        TypedSchema::Object { properties } => {
            if let Some(obj) = value.as_object() {
                for (field, typed_field) in properties {
                    let field_path = if path.is_empty() {
                        field.clone()
                    } else {
                        format!("{}.{}", path, field)
                    };
                    if let Some(field_value) = obj.get(field) {
                        errors.extend(validate_recursive(field_value, &typed_field.schema, &field_path));
                    } else if typed_field.required {
                        errors.push(format!("{}: required field missing", field_path));
                    }
                }
            } else {
                errors.push(format!("{}: expected object, got {}", path, type_name(value)));
            }
        }
    }
    errors
}

fn type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ── Step validation (compiler pass) ──────────────────────────────────────────

/// Full validation pass for an `acp_boundary` step.
/// Called by the workflow compiler before saving a workflow.
pub fn validate_boundary_step_full(
    step: &BoundaryStep,
    handshake: &BoundaryHandshake,
) -> Result<(), BoundaryValidationError> {
    use crate::boundry::{BoundaryRole, BoundaryScope};
    use crate::boundry::governance::RevocationState;

    // Governance: revocation state
    match &handshake.revocation_state {
        RevocationState::Frozen { .. } =>
            return Err(BoundaryValidationError::HandshakeFrozen {
                handshake_id: handshake.handshake_id.clone(),
            }),
        RevocationState::Revoked { .. } =>
            return Err(BoundaryValidationError::HandshakeRevoked {
                handshake_id: handshake.handshake_id.clone(),
            }),
        RevocationState::Active => {}
    }

    // Temporal window
    let now = chrono::Utc::now();
    if let Some(valid_from) = handshake.valid_from {
        if now < valid_from {
            return Err(BoundaryValidationError::HandshakeNotStarted {
                handshake_id: handshake.handshake_id.clone(),
                valid_from,
            });
        }
    }
    if let Some(valid_until) = handshake.valid_until {
        if now > valid_until {
            return Err(BoundaryValidationError::HandshakeExpired {
                handshake_id: handshake.handshake_id.clone(),
                expired_at: valid_until,
            });
        }
    }

    // Version check
    if step.handshake_version != handshake.handshake_version {
        return Err(BoundaryValidationError::HandshakeVersionMismatch {
            step_version: step.handshake_version,
            current_version: handshake.handshake_version,
        });
    }

    // Acceptance check
    if !handshake.requester_accepted || !handshake.responder_accepted {
        let pending = if !handshake.requester_accepted {
            handshake.requester.display_name.clone()
        } else {
            handshake.responder.display_name.clone()
        };
        return Err(BoundaryValidationError::HandshakeNotAccepted {
            handshake_id: handshake.handshake_id.clone(),
            pending_party: pending,
        });
    }

    // Endpoint check (only for CrossEnterprise)
    if matches!(handshake.scope, BoundaryScope::CrossEnterprise) {
        let expected = match step.role {
            BoundaryRole::Requester => &handshake.responder.acp_endpoint,
            BoundaryRole::Responder => &handshake.requester.acp_endpoint,
        };
        if &step.peer_endpoint != expected {
            return Err(BoundaryValidationError::PeerEndpointMismatch {
                step_endpoint: step.peer_endpoint.clone(),
                handshake_endpoint: expected.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_simple_object_ok() {
        let mut props = std::collections::HashMap::new();
        props.insert("name".to_string(), crate::boundry::TypedField {
            schema: TypedSchema::String,
            required: true,
            description: None,
        });
        let schema = TypedSchema::Object { properties: props };
        let payload = serde_json::json!({ "name": "Alice" });
        assert!(validate_payload_against_schema(&payload, &schema).is_ok());
    }

    #[test]
    fn validate_missing_required_field() {
        let mut props = std::collections::HashMap::new();
        props.insert("amount".to_string(), crate::boundry::TypedField {
            schema: TypedSchema::Number,
            required: true,
            description: None,
        });
        let schema = TypedSchema::Object { properties: props };
        let payload = serde_json::json!({ "other": "value" });
        assert!(validate_payload_against_schema(&payload, &schema).is_err());
    }

    #[test]
    fn validate_wrong_type() {
        let schema = TypedSchema::Number;
        let payload = serde_json::json!("not a number");
        assert!(validate_payload_against_schema(&payload, &schema).is_err());
    }
}
