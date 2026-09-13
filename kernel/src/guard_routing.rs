use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_CLOUD_MODEL: &str = "claude-sonnet-5";
pub const DEFAULT_LOCAL_MODEL: &str = "qwen2.5-coder:7b";
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    Auto,
    Local,
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub session_id: String,
    pub requested_mode: RouteMode,
    pub provider: String,
    pub model: String,
    pub reason: String,
    pub privacy: String,
    pub shadow_provider: Option<String>,
    pub sticky: bool,
}

impl RouteDecision {
    pub fn upstream_base_url(&self) -> &'static str {
        if self.provider == "ollama" {
            DEFAULT_LOCAL_BASE_URL
        } else {
            "https://api.anthropic.com"
        }
    }

    pub fn is_local(&self) -> bool {
        self.provider == "ollama"
    }
}

pub fn decide_route(
    session_id: impl Into<String>,
    requested_mode: RouteMode,
    initial_task: &str,
    local_available: bool,
    shadow_requested: bool,
) -> Result<RouteDecision> {
    let session_id = session_id.into();
    let (provider, model, reason, privacy) = match requested_mode {
        RouteMode::Local => {
            if !local_available {
                return Err(anyhow!(
                    "local-only routing requested, but Ollama is unavailable or the configured model is not installed"
                ));
            }
            (
                "ollama",
                DEFAULT_LOCAL_MODEL,
                "explicit_local",
                "local_only",
            )
        }
        RouteMode::Cloud => (
            "anthropic",
            DEFAULT_CLOUD_MODEL,
            "explicit_cloud",
            "cloud_allowed",
        ),
        RouteMode::Auto if local_available && is_small_read_only_task(initial_task) => (
            "ollama",
            DEFAULT_LOCAL_MODEL,
            "auto_small_read_only",
            "local_preferred",
        ),
        RouteMode::Auto if !local_available => (
            "anthropic",
            DEFAULT_CLOUD_MODEL,
            "auto_local_unavailable",
            "cloud_allowed",
        ),
        RouteMode::Auto => (
            "anthropic",
            DEFAULT_CLOUD_MODEL,
            "auto_complex_or_ambiguous",
            "cloud_allowed",
        ),
    };
    let shadow_provider = (shadow_requested && provider == "anthropic" && local_available)
        .then(|| "ollama".to_string());
    Ok(RouteDecision {
        session_id,
        requested_mode,
        provider: provider.to_string(),
        model: model.to_string(),
        reason: reason.to_string(),
        privacy: privacy.to_string(),
        shadow_provider,
        sticky: true,
    })
}

pub fn initial_user_task(body: &[u8]) -> String {
    let Ok(payload) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    payload
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|message| message.get("content"))
        .map(content_text)
        .unwrap_or_default()
}

pub fn rewrite_model(body: &[u8], model: &str, remove_tools: bool) -> Result<Vec<u8>> {
    let mut payload: Value = serde_json::from_slice(body)?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| anyhow!("Anthropic request body must be a JSON object"))?;
    object.insert("model".to_string(), Value::String(model.to_string()));
    if remove_tools {
        object.remove("tools");
        object.remove("tool_choice");
    }
    Ok(serde_json::to_vec(&payload)?)
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_small_read_only_task(task: &str) -> bool {
    let normalized = task.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 4_096 {
        return false;
    }
    let mutating = [
        "add ",
        "build ",
        "change ",
        "create ",
        "delete ",
        "deploy ",
        "edit ",
        "fix ",
        "implement ",
        "install ",
        "modify ",
        "patch ",
        "publish ",
        "refactor ",
        "remove ",
        "rename ",
        "run tests",
        "update ",
        "write ",
    ];
    if mutating.iter().any(|term| normalized.contains(term)) {
        return false;
    }
    let read_only = [
        "describe",
        "explain",
        "inspect",
        "list",
        "read",
        "review",
        "summarize",
        "what ",
        "where ",
        "which ",
    ];
    read_only.iter().any(|term| normalized.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_local_fails_closed_when_unavailable() {
        assert!(decide_route("s", RouteMode::Local, "explain this", false, false).is_err());
    }

    #[test]
    fn auto_routes_small_read_only_work_locally() {
        let decision = decide_route(
            "s",
            RouteMode::Auto,
            "Summarize the security policy without changing files.",
            true,
            true,
        )
        .unwrap();
        assert_eq!(decision.provider, "ollama");
        assert_eq!(decision.shadow_provider, None);
    }

    #[test]
    fn auto_routes_mutation_to_cloud_with_local_shadow() {
        let decision = decide_route(
            "s",
            RouteMode::Auto,
            "Fix the parser and run tests.",
            true,
            true,
        )
        .unwrap();
        assert_eq!(decision.provider, "anthropic");
        assert_eq!(decision.shadow_provider.as_deref(), Some("ollama"));
    }

    #[test]
    fn rewriting_a_shadow_request_removes_tool_authority() {
        let body = br#"{"model":"old","messages":[],"tools":[{"name":"Bash"}],"tool_choice":{"type":"auto"}}"#;
        let rewritten = rewrite_model(body, DEFAULT_LOCAL_MODEL, true).unwrap();
        let value: Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(value["model"], DEFAULT_LOCAL_MODEL);
        assert!(value.get("tools").is_none());
        assert!(value.get("tool_choice").is_none());
    }

    #[test]
    fn extracts_the_last_user_text_block() {
        let body = br#"{"messages":[{"role":"user","content":"first"},{"role":"assistant","content":"ok"},{"role":"user","content":[{"type":"text","text":"final"}]}]}"#;
        assert_eq!(initial_user_task(body), "final");
    }
}
