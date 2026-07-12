//! Messaging-channel bridges (Telegram, Discord).
//!
//! An inbound message from an **allowlisted** sender becomes a goal that runs
//! through Kerna's normal fail-closed pipeline (permissions, budgets, trace);
//! the final assistant reply is sent back to that chat. Anyone not on the
//! allowlist is logged and ignored — the bot never acts on them. Channels only
//! listen while `kerna daemon` is running.
//!
//! Runs that come in over a channel are `non_interactive`: there is no terminal
//! to approve a `require_confirmation` tool, so such tools are denied fail-closed
//! (see `TaskScheduler::non_interactive`).

use crate::config::{ChatChannelConfig, Config};
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Spawn a background listener for every enabled channel. Returns immediately;
/// each channel runs in its own tokio task for the life of the daemon.
pub fn start_channels(
    config: Config,
    memory: Arc<MemoryEngine>,
    mcp_registry: Arc<Mutex<McpRegistry>>,
) {
    for channel in config.channels.iter().filter(|c| c.enabled).cloned() {
        let config = config.clone();
        let memory = memory.clone();
        let registry = mcp_registry.clone();
        match channel.platform.as_str() {
            "telegram" => {
                tokio::spawn(async move {
                    if let Err(e) = run_telegram(channel.clone(), config, memory, registry).await {
                        eprintln!(
                            "[channel:{}] telegram listener stopped: {}",
                            channel.name, e
                        );
                    }
                });
            }
            other => {
                eprintln!(
                    "[channel:{}] unsupported platform '{}' — skipping.",
                    channel.name, other
                );
            }
        }
    }
}

/// Is this sender/chat id allowed to trigger runs on this channel?
/// Fail-closed: an empty allowlist means nobody is allowed.
fn is_allowed(channel: &ChatChannelConfig, from_id: &str, chat_id: &str) -> bool {
    channel
        .allowed_ids
        .iter()
        .any(|id| id == from_id || id == chat_id)
}

/// Resolve the bot token from the channel's declared env var name.
fn token_for(channel: &ChatChannelConfig) -> Result<String, String> {
    let tok = std::env::var(&channel.token_env).unwrap_or_default();
    if tok.trim().is_empty() {
        return Err(format!(
            "{} is not set (kerna secrets-style env var for this channel's bot token)",
            channel.token_env
        ));
    }
    Ok(tok)
}

/// Run one goal that arrived over a channel and return the assistant's reply
/// text. Every such run is non-interactive and fully recorded.
async fn run_channel_goal(
    goal: &str,
    session_id: &str,
    config: &Config,
    memory: &Arc<MemoryEngine>,
    mcp_registry: &Arc<Mutex<McpRegistry>>,
) -> String {
    let scheduler = match TaskScheduler::new(
        config.clone(),
        memory.clone(),
        mcp_registry.clone(),
        Some(session_id.to_string()),
    ) {
        Ok(s) => s.non_interactive(),
        Err(e) => return format!("Kerna could not start: {}", e),
    };
    match scheduler.run_goal(goal).await {
        Ok(task_id) => memory
            .get_task_result(&task_id.to_string())
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Done.".to_string()),
        Err(e) => format!("Task failed: {}", e),
    }
}

// ─── Telegram ────────────────────────────────────────────────────────────

async fn run_telegram(
    channel: ChatChannelConfig,
    config: Config,
    memory: Arc<MemoryEngine>,
    mcp_registry: Arc<Mutex<McpRegistry>>,
) -> Result<(), String> {
    let token = token_for(&channel)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(70))
        .build()
        .map_err(|e| e.to_string())?;
    let base = format!("https://api.telegram.org/bot{}", token);

    eprintln!(
        "[channel:{}] telegram online — {} allowlisted id(s).",
        channel.name,
        channel.allowed_ids.len()
    );

    let mut offset: i64 = 0;
    loop {
        // Long-poll for updates (30s server-side hold).
        let url = format!("{}/getUpdates?timeout=30&offset={}", base, offset);
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[channel:{}] getUpdates error: {}", channel.name, e);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[channel:{}] bad update payload: {}", channel.name, e);
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };

        let updates = body["result"].as_array().cloned().unwrap_or_default();
        for update in updates {
            offset = update["update_id"].as_i64().unwrap_or(offset) + 1;
            let msg = &update["message"];
            let text = msg["text"].as_str().unwrap_or("").to_string();
            if text.trim().is_empty() {
                continue;
            }
            let chat_id = msg["chat"]["id"]
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let from_id = msg["from"]["id"]
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_default();

            if !is_allowed(&channel, &from_id, &chat_id) {
                // Logged, never acted on.
                let audit = Uuid::new_v4();
                let _ = memory.log_message(
                    audit,
                    "WARN",
                    &format!(
                        "[channel:{}] ignored message from non-allowlisted id (from={}, chat={})",
                        channel.name, from_id, chat_id
                    ),
                );
                continue;
            }

            let session = format!("telegram-{}-{}", channel.name, chat_id);
            let reply = run_channel_goal(&text, &session, &config, &memory, &mcp_registry).await;
            send_telegram(&client, &base, &chat_id, &reply).await;
        }
    }
}

async fn send_telegram(client: &reqwest::Client, base: &str, chat_id: &str, text: &str) {
    // Telegram caps a message at 4096 chars.
    let clipped: String = text.chars().take(4000).collect();
    let url = format!("{}/sendMessage", base);
    let _ = client
        .post(&url)
        .json(&serde_json::json!({ "chat_id": chat_id, "text": clipped }))
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(ids: &[&str]) -> ChatChannelConfig {
        ChatChannelConfig {
            platform: "telegram".to_string(),
            name: "test".to_string(),
            token_env: "X".to_string(),
            allowed_ids: ids.iter().map(|s| s.to_string()).collect(),
            enabled: true,
        }
    }

    #[test]
    fn allowlist_matches_sender_or_chat_and_is_fail_closed_when_empty() {
        let c = channel(&["111", "222"]);
        assert!(is_allowed(&c, "111", "999")); // sender allowed
        assert!(is_allowed(&c, "999", "222")); // chat allowed
        assert!(!is_allowed(&c, "999", "888")); // neither
                                                // Empty allowlist => nobody allowed.
        assert!(!is_allowed(&channel(&[]), "111", "222"));
    }
}
