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
    #[serde(rename = "inspection.requested")]
    InspectionRequested {
        session_id: String,
        action_id: String,
        proposed_kind: String,
        path: String,
        canonical_action_digest: String,
        policy_effect: String,
        policy_rule: Option<String>,
    },
    #[serde(rename = "inspection.released")]
    InspectionReleased {
        session_id: String,
        action_id: String,
        call_id: String,
        path: String,
        containment: &'static str,
    },
    #[serde(rename = "inspection.result_observed")]
    InspectionResultObserved {
        session_id: String,
        action_id: String,
        call_id: String,
        path: String,
        bytes_read: u64,
        content_sha256: String,
        truncated: bool,
        content: String,
    },
    #[serde(rename = "inspection.blocked")]
    InspectionBlocked {
        session_id: String,
        action_id: String,
        path: String,
        reason: &'static str,
        policy_effect: &'static str,
    },
    #[serde(rename = "inspection.outcome_unknown")]
    InspectionOutcomeUnknown {
        session_id: String,
        action_id: String,
        call_id: Option<String>,
        path: String,
        reason: &'static str,
    },
    #[serde(rename = "action.executed")]
    ActionExecuted {
        session_id: String,
        action_id: String,
        kind: String,
        status: String,
        detail: String,
        canonical_action_digest: String,
        policy_effect: String,
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
            NativeEvent::InspectionRequested {
                path,
                policy_effect,
                ..
            } => {
                eprintln!("[i] inspection requested - file_read {path} (policy {policy_effect})");
            }
            NativeEvent::InspectionReleased {
                path,
                call_id,
                containment,
                ..
            } => {
                eprintln!("[+] inspection released - {path} receipt {call_id} ({containment})");
            }
            NativeEvent::InspectionResultObserved {
                path,
                bytes_read,
                content_sha256,
                truncated,
                content,
                ..
            } => {
                if *truncated {
                    eprintln!(
                        "[+] read {path} - {bytes_read} bytes sha256:{content_sha256} (preview truncated)",
                    );
                } else {
                    eprintln!("[+] read {path} - {bytes_read} bytes sha256:{content_sha256}");
                }
                print!("{content}");
                io::stdout().flush()?;
            }
            NativeEvent::InspectionBlocked { path, reason, .. } => {
                eprintln!("[-] inspection blocked - {path}: {reason}");
            }
            NativeEvent::InspectionOutcomeUnknown { path, reason, .. } => {
                eprintln!("[!] inspection outcome unknown - {path}: {reason}");
            }
            NativeEvent::ActionExecuted {
                kind,
                status,
                detail,
                ..
            } => {
                eprintln!("[{status}] {kind} - {detail}");
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
    fn inspection_events_expose_the_full_lifecycle_vocabulary() {
        let requested = serde_json::to_value(NativeEvent::InspectionRequested {
            session_id: "code-1".to_string(),
            action_id: "proposal_1".to_string(),
            proposed_kind: "file_read".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_action_digest: "sha256:action".to_string(),
            policy_effect: "allow".to_string(),
            policy_rule: Some("balanced.allow_read".to_string()),
        })
        .unwrap();
        assert_eq!(requested["type"], "inspection.requested");
        assert_eq!(requested["proposed_kind"], "file_read");
        assert_eq!(requested["policy_effect"], "allow");
        assert!(requested.get("prompt").is_none());

        let released = serde_json::to_value(NativeEvent::InspectionReleased {
            session_id: "code-1".to_string(),
            action_id: "proposal_1".to_string(),
            call_id: "code-1:proposal_1".to_string(),
            path: "src/lib.rs".to_string(),
            containment: "trusted_cli_worktree_read",
        })
        .unwrap();
        assert_eq!(released["type"], "inspection.released");
        assert_eq!(released["containment"], "trusted_cli_worktree_read");

        let observed = serde_json::to_value(NativeEvent::InspectionResultObserved {
            session_id: "code-1".to_string(),
            action_id: "proposal_1".to_string(),
            call_id: "code-1:proposal_1".to_string(),
            path: "src/lib.rs".to_string(),
            bytes_read: 3,
            content_sha256: "abc".to_string(),
            truncated: false,
            content: "abc".to_string(),
        })
        .unwrap();
        assert_eq!(observed["type"], "inspection.result_observed");
        assert_eq!(observed["bytes_read"], 3);
        assert_eq!(observed["truncated"], false);

        let blocked = serde_json::to_value(NativeEvent::InspectionBlocked {
            session_id: "code-1".to_string(),
            action_id: "proposal_2".to_string(),
            path: "C:/Windows/win.ini".to_string(),
            reason: "absolute_path",
            policy_effect: "allow",
        })
        .unwrap();
        assert_eq!(blocked["type"], "inspection.blocked");
        assert_eq!(blocked["reason"], "absolute_path");

        let unknown = serde_json::to_value(NativeEvent::InspectionOutcomeUnknown {
            session_id: "code-1".to_string(),
            action_id: "proposal_3".to_string(),
            call_id: Some("code-1:proposal_3".to_string()),
            path: "src/lib.rs".to_string(),
            reason: "read_failed",
        })
        .unwrap();
        assert_eq!(unknown["type"], "inspection.outcome_unknown");
        assert_eq!(unknown["reason"], "read_failed");
    }

    #[test]
    fn inspection_result_event_content_is_bounded_by_the_caller() {
        // The event carries only what the inspection path supplies; the
        // inspection layer caps the preview and never persists it.
        let event = NativeEvent::InspectionResultObserved {
            session_id: "code-1".to_string(),
            action_id: "proposal_1".to_string(),
            call_id: "code-1:proposal_1".to_string(),
            path: "src/lib.rs".to_string(),
            bytes_read: 9,
            content_sha256: "digest".to_string(),
            truncated: true,
            content: "preview".to_string(),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["truncated"], true);
        assert_eq!(value["content"], "preview");
        assert!(value.get("prompt").is_none());
        assert!(value.get("file_bytes").is_none());
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
