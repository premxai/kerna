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
    #[serde(rename = "session.completed")]
    SessionCompleted { session_id: String, tokens: u64 },
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
            NativeEvent::SessionCompleted { tokens, .. } => {
                println!();
                eprintln!("[i] tool-less response · {tokens} tokens · prompt not persisted");
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
        let failed = serde_json::to_value(NativeEvent::SessionFailed {
            session_id: "session-1".to_string(),
            error_class: "provider_error",
        })
        .unwrap();
        assert_eq!(completed["type"], "session.completed");
        assert_eq!(failed["type"], "session.failed");
    }
}
