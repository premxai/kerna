use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub task_id: String,
    pub session_id: Option<String>,
    pub sequence: i64,
    pub timestamp: String,
    pub event_type: String,
    pub actor: String,
    pub severity: String,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub policy_decision: Option<String>,
    pub risk_score: Option<f64>,
    pub parent_event_id: Option<String>,
    pub correlation_id: Option<String>,
    pub redaction_status: Option<String>,
    pub budget_snapshot_json: Option<serde_json::Value>,
    pub payload_json: serde_json::Value,
}

pub trait EventSink: Send + Sync {
    fn record(&self, event: Event) -> Result<()>;
}

/// Marker persisted in event payloads in place of credentials. It is explicit
/// enough for an operator to understand why a value is absent without leaving
/// recoverable secret material in the trace database.
pub const REDACTED_VALUE: &str = "[REDACTED]";

/// Redact credential-shaped values from an event payload before it reaches
/// durable storage. Tool arguments are sometimes stored as JSON encoded inside
/// a string, so those strings are parsed and redacted recursively as well.
///
/// This is intentionally a storage boundary, not the only secret control: MCP
/// child processes still receive only configured environment-variable names.
pub fn redact_payload(payload: &Value) -> (Value, bool) {
    redact_value(payload, false)
}

fn redact_value(value: &Value, force_redact: bool) -> (Value, bool) {
    if force_redact {
        return (Value::String(REDACTED_VALUE.to_string()), true);
    }

    match value {
        Value::Object(values) => {
            let mut redacted = serde_json::Map::new();
            let mut changed = false;
            for (key, value) in values {
                let (safe_value, value_changed) = redact_value(value, is_sensitive_key(key));
                redacted.insert(key.clone(), safe_value);
                changed |= value_changed;
            }
            (Value::Object(redacted), changed)
        }
        Value::Array(values) => {
            let mut changed = false;
            let redacted = values
                .iter()
                .map(|value| {
                    let (safe_value, value_changed) = redact_value(value, false);
                    changed |= value_changed;
                    safe_value
                })
                .collect();
            (Value::Array(redacted), changed)
        }
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                let (redacted, changed) = redact_value(&parsed, false);
                if changed {
                    return (Value::String(redacted.to_string()), true);
                }
            }
            (Value::String(text.clone()), false)
        }
        _ => (value.clone(), false),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        normalized.as_str(),
        "authorization"
            | "token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "api_key"
            | "apikey"
            | "secret"
            | "client_secret"
            | "password"
            | "credential"
            | "credentials"
            | "cookie"
            | "set_cookie"
            | "private_key"
            | "access_key"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested_credentials_and_json_encoded_arguments() {
        let payload = json!({
            "args": r#"{"query":"report","api_key":"top-secret"}"#,
            "headers": {"Authorization": "Bearer top-secret"},
            "safe": "keep me"
        });

        let (redacted, changed) = redact_payload(&payload);

        assert!(changed);
        assert_eq!(redacted["headers"]["Authorization"], REDACTED_VALUE);
        assert_eq!(redacted["safe"], "keep me");
        let args: Value = serde_json::from_str(redacted["args"].as_str().unwrap()).unwrap();
        assert_eq!(args["query"], "report");
        assert_eq!(args["api_key"], REDACTED_VALUE);
    }
}
