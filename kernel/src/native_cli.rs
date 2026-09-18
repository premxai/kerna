use anyhow::Result;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum NativeEvent {
    #[serde(rename = "session.started")]
    SessionStarted {
        session_id: String,
        provider: String,
        model: String,
        tool_authority: &'static str,
    },
    #[serde(rename = "assistant.delta")]
    AssistantDelta { session_id: String, text: String },
    #[serde(rename = "context.cleared")]
    ContextCleared { session_id: String },
    #[serde(rename = "proposal.preflight")]
    ProposalPreflight {
        session_id: String,
        preflight: crate::native_code::ProposalPreflight,
    },
    #[serde(rename = "session.completed")]
    SessionCompleted { session_id: String, tokens: u64 },
    #[serde(rename = "session.interrupted")]
    SessionInterrupted {
        session_id: String,
        reason: &'static str,
    },
    #[serde(rename = "session.failed")]
    SessionFailed {
        session_id: String,
        error_class: &'static str,
    },
}

pub struct EventRenderer {
    json: bool,
}

impl EventRenderer {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn emit(&mut self, event: &NativeEvent) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string(event)?);
            io::stdout().flush()?;
            return Ok(());
        }
        match event {
            NativeEvent::AssistantDelta { text, .. } => {
                print!("{text}");
                io::stdout().flush()?;
            }
            NativeEvent::ContextCleared { .. } => {
                eprintln!("[i] in-memory context cleared");
            }
            NativeEvent::ProposalPreflight { preflight, .. } => {
                eprintln!(
                    "[i] proposal preflight - {} actions - no execution, no receipt requested",
                    preflight.actions.len()
                );
                for action in &preflight.actions {
                    eprintln!(
                        "    [{}] {} {} via {} ({})",
                        action.policy_effect,
                        action.proposed_kind,
                        action
                            .canonical_resource
                            .as_deref()
                            .unwrap_or("(no single resource)"),
                        action.raw_tool_name,
                        action.required_containment
                    );
                }
            }
            NativeEvent::SessionCompleted { tokens, .. } => {
                println!();
                eprintln!("[i] tool-less response - {tokens} tokens - prompt not persisted");
            }
            NativeEvent::SessionInterrupted { .. } => {
                eprintln!("\n[-] model request interrupted; broker cleanup is running");
            }
            NativeEvent::SessionFailed { .. } => {
                eprintln!("[-] model request failed safely; prompt not persisted");
            }
            NativeEvent::SessionStarted { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_is_stable_and_contains_no_prompt_field() {
        let event = NativeEvent::SessionStarted {
            session_id: "session-1".to_string(),
            provider: "anthropic".to_string(),
            model: "model-1".to_string(),
            tool_authority: "none",
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "session.started");
        assert_eq!(value["tool_authority"], "none");
        assert!(value.get("prompt").is_none());
    }

    #[test]
    fn terminal_event_vocabulary_is_explicit() {
        let completed = serde_json::to_value(NativeEvent::SessionCompleted {
            session_id: "session-1".to_string(),
            tokens: 7,
        })
        .unwrap();
        let interrupted = serde_json::to_value(NativeEvent::SessionInterrupted {
            session_id: "session-1".to_string(),
            reason: "ctrl_c",
        })
        .unwrap();
        let failed = serde_json::to_value(NativeEvent::SessionFailed {
            session_id: "session-1".to_string(),
            error_class: "provider_error",
        })
        .unwrap();
        assert_eq!(completed["type"], "session.completed");
        assert_eq!(interrupted["type"], "session.interrupted");
        assert_eq!(failed["type"], "session.failed");
    }

    #[test]
    fn proposal_preflight_event_is_explicitly_non_executing() {
        let preflight = crate::native_code::ProposalPreflight {
            schema_version: 1,
            mode: "preflight_only",
            receipt_state: "preflight_only_not_requested",
            actions: vec![],
        };
        let value = serde_json::to_value(NativeEvent::ProposalPreflight {
            session_id: "session-1".to_string(),
            preflight,
        })
        .unwrap();
        assert_eq!(value["type"], "proposal.preflight");
        assert_eq!(value["preflight"]["mode"], "preflight_only");
        assert_eq!(
            value["preflight"]["receipt_state"],
            "preflight_only_not_requested"
        );
        assert!(value.get("prompt").is_none());
    }

    #[test]
    fn context_clear_event_is_explicit_and_contains_no_transcript() {
        let value = serde_json::to_value(NativeEvent::ContextCleared {
            session_id: "session-1".to_string(),
        })
        .unwrap();
        assert_eq!(value["type"], "context.cleared");
        assert!(value.get("messages").is_none());
        assert!(value.get("prompt").is_none());
    }
}
