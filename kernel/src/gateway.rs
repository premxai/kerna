//! MCP Policy-Gateway mode.
//!
//! `kerna gateway` turns Kerna into an MCP *server* over stdio. Any MCP client
//! (Claude Code, Cursor, Cline, …) points at `kerna gateway` as if it were a
//! normal MCP server. Kerna spawns the downstream MCP servers listed in
//! `kerna.toml`, aggregates their tools, and re-exposes them — but every
//! `tools/call` first passes through Kerna's fail-closed policy engine and is
//! recorded to the SQLite event log, so you get governance + a full audit trail
//! over tools you already use, without adopting a new agent runtime.
//!
//! Protocol note: stdout is the JSON-RPC channel, so *nothing* human-readable
//! may be written there. All diagnostics go to stderr (see `McpRegistry`'s quiet
//! mode).

use crate::config::Config;
use crate::events::{Event, EventSink};
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::permissions::{PermissionLevel, PermissionManager};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

pub struct Gateway {
    config: Config,
    registry: Arc<Mutex<McpRegistry>>,
    permissions: PermissionManager,
    memory: Arc<MemoryEngine>,
    task_id: Uuid,
    session_id: String,
    sequence: i64,
}

impl Gateway {
    pub fn new(
        config: Config,
        registry: Arc<Mutex<McpRegistry>>,
        memory: Arc<MemoryEngine>,
    ) -> Self {
        let permissions = PermissionManager::new(config.clone());
        let task_id = Uuid::new_v4();
        let session_id = format!("gateway-{}", Uuid::new_v4());
        Gateway {
            config,
            registry,
            permissions,
            memory,
            task_id,
            session_id,
            sequence: 0,
        }
    }

    /// Run the stdio JSON-RPC loop until EOF.
    pub async fn run(&mut self) -> Result<()> {
        // Record the gateway session as a task so `kerna trace <id>` works.
        // session_id is carried on each event, not on the task row (the tasks
        // table foreign-keys session_id to a sessions row we don't create here).
        if let Err(e) = self
            .memory
            .create_task(self.task_id, None, "MCP Gateway Session")
        {
            eprintln!("[gateway] warning: could not record gateway task: {}", e);
        }
        let _ = self.memory.update_task_status(self.task_id, "running");
        eprintln!(
            "[gateway] Kerna MCP policy-gateway online (task {}). Proxying {} downstream server(s). Ctrl+C to stop.",
            self.task_id,
            self.config.mcp_servers.len()
        );

        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break; // EOF — upstream client disconnected
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                Ok(req) => {
                    let id = req.id.clone();
                    match req.method.as_str() {
                        "initialize" => {
                            self.respond(
                                id,
                                Some(json!({
                                    "protocolVersion": "2024-11-05",
                                    "capabilities": { "tools": {} },
                                    "serverInfo": { "name": "kerna-gateway", "version": "0.1.0" }
                                })),
                                None,
                            )
                            .await;
                        }
                        // Notifications have no id and expect no response.
                        "notifications/initialized" | "notifications/cancelled" => {}
                        "ping" => {
                            self.respond(id, Some(json!({})), None).await;
                        }
                        "tools/list" => {
                            let tools = {
                                let registry = self.registry.lock().await;
                                registry.get_mcp_tools()
                            };
                            self.respond(id, Some(json!({ "tools": tools })), None)
                                .await;
                        }
                        "tools/call" => {
                            let result =
                                self.handle_tool_call(req.params.unwrap_or(json!({}))).await;
                            self.respond(id, Some(result), None).await;
                        }
                        other => {
                            self.respond(
                                id,
                                None,
                                Some(JsonRpcError {
                                    code: -32601,
                                    message: format!("Method not found: {}", other),
                                }),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    self.respond(
                        None,
                        None,
                        Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                        }),
                    )
                    .await;
                }
            }
        }

        let _ = self.memory.update_task_status(self.task_id, "completed");
        Ok(())
    }

    /// The governed tool-call path: policy check → record → forward → record.
    async fn handle_tool_call(&mut self, params: serde_json::Value) -> serde_json::Value {
        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        if tool_name.is_empty() {
            return error_result("Missing tool name in tools/call request.");
        }

        let server_name = {
            let registry = self.registry.lock().await;
            registry.get_server_for_tool(&tool_name)
        };

        // Unknown tool → fail closed.
        if server_name.is_none() {
            self.record(
                "tool.call.blocked",
                Some(&tool_name),
                "warning",
                Some("UnknownTool"),
                json!({ "reason": "no downstream server exposes this tool" }),
            );
            return error_result(&format!(
                "Kerna gateway: unknown tool '{}' (not exposed by any configured MCP server).",
                tool_name
            ));
        }

        self.record(
            "tool.call.requested",
            Some(&tool_name),
            "info",
            None,
            json!({ "arguments": arguments, "server": server_name }),
        );

        // Fail-closed policy check. A non-interactive server can't prompt, so
        // only auto-approved tools pass; deny and require-confirmation are both
        // blocked with a clear message.
        let level = self.permissions.check(&tool_name, server_name.as_deref());
        self.record(
            "tool.policy.checked",
            Some(&tool_name),
            if level == PermissionLevel::AutoApprove {
                "info"
            } else {
                "warning"
            },
            Some(&format!("{:?}", level)),
            json!({}),
        );

        if level != PermissionLevel::AutoApprove {
            let reason = match level {
                PermissionLevel::Deny => {
                    format!("Tool '{}' is denied by Kerna policy.", tool_name)
                }
                PermissionLevel::RequireConfirmation => format!(
                    "Tool '{}' requires human confirmation, which the gateway cannot prompt for. \
                     Grant it `auto_approve` in kerna.toml to allow it through the gateway.",
                    tool_name
                ),
                PermissionLevel::AutoApprove => unreachable!(),
            };
            self.record(
                "tool.call.blocked",
                Some(&tool_name),
                "warning",
                Some(&format!("{:?}", level)),
                json!({ "reason": reason }),
            );
            return error_result(&format!("Kerna gateway blocked this call. {}", reason));
        }

        // Forward to the downstream server (registry also enforces
        // allow_tools/deny_tools/capabilities filters).
        let forward = {
            let mut registry = self.registry.lock().await;
            registry.call_tool(&tool_name, arguments.clone()).await
        };

        match forward {
            Ok(result) => {
                self.record(
                    "tool.call.completed",
                    Some(&tool_name),
                    "info",
                    Some("AutoApprove"),
                    json!({ "result_preview": preview(&result) }),
                );
                // The downstream result is already an MCP tools/call result
                // (content blocks); pass it straight through.
                result
            }
            Err(e) => {
                self.record(
                    "tool.call.failed",
                    Some(&tool_name),
                    "error",
                    Some("AutoApprove"),
                    json!({ "error": e.to_string() }),
                );
                error_result(&format!("Downstream tool '{}' failed: {}", tool_name, e))
            }
        }
    }

    fn record(
        &mut self,
        event_type: &str,
        tool: Option<&str>,
        severity: &str,
        policy_decision: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.sequence += 1;
        let _ = self.memory.record(Event {
            event_id: Uuid::new_v4().to_string(),
            task_id: self.task_id.to_string(),
            session_id: Some(self.session_id.clone()),
            sequence: self.sequence,
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: event_type.to_string(),
            actor: "gateway".to_string(),
            severity: severity.to_string(),
            model: None,
            tool: tool.map(|t| t.to_string()),
            policy_decision: policy_decision.map(|p| p.to_string()),
            risk_score: None,
            parent_event_id: None,
            correlation_id: None,
            redaction_status: None,
            budget_snapshot_json: None,
            payload_json: payload,
        });
    }

    async fn respond(
        &self,
        id: Option<serde_json::Value>,
        result: Option<serde_json::Value>,
        error: Option<JsonRpcError>,
    ) {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            result,
            error,
        };
        if let Ok(mut s) = serde_json::to_string(&resp) {
            s.push('\n');
            let mut out = tokio::io::stdout();
            let _ = out.write_all(s.as_bytes()).await;
            let _ = out.flush().await;
        }
    }
}

/// Build an MCP tools/call error result (isError + text content).
fn error_result(message: &str) -> serde_json::Value {
    json!({
        "isError": true,
        "content": [{ "type": "text", "text": message }]
    })
}

/// Short preview of a downstream result for the event log (avoid storing huge
/// payloads verbatim).
fn preview(result: &serde_json::Value) -> String {
    let s = result.to_string();
    if s.chars().count() > 240 {
        let truncated: String = s.chars().take(240).collect();
        format!("{}…", truncated)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServerConfig, PermissionRule};
    use std::fs;

    fn kerna_bin() -> String {
        let test_exe = std::env::current_exe().unwrap();
        test_exe
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("kerna{}", std::env::consts::EXE_SUFFIX))
            .to_string_lossy()
            .to_string()
    }

    #[tokio::test]
    async fn gateway_governs_and_records_proxied_calls() {
        let db_path = "test_gateway.db".to_string();
        let _ = fs::remove_file(&db_path);
        let memory = Arc::new(MemoryEngine::new(&db_path).unwrap());

        let mut config = Config {
            db_path: db_path.clone(),
            ..Config::default()
        };
        config.mcp_servers.push(McpServerConfig {
            name: "mockmcp".to_string(),
            command: kerna_bin(),
            args: vec!["mockmcp".to_string()],
            enabled: true,
            runtime_mode: "local".to_string(),
            docker_image: String::new(),
            capabilities: vec![],
            allowed_paths: vec![],
            approval_required: vec![],
            allow_tools: vec![],
            deny_tools: vec![],
            secrets: vec![],
        });
        // echo is auto-approved; everything else denied by the wildcard.
        config.permissions.push(PermissionRule {
            tool: "echo".to_string(),
            action: "auto_approve".to_string(),
        });
        config.permissions.push(PermissionRule {
            tool: "*".to_string(),
            action: "deny".to_string(),
        });

        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        registry
            .lock()
            .await
            .initialize(&config.mcp_servers)
            .await
            .unwrap();

        let mut gw = Gateway::new(config, registry, memory.clone());
        let _ = memory.create_task(gw.task_id, None, "MCP Gateway Session");

        // Auto-approved tool is forwarded and returns the downstream result.
        let ok = gw
            .handle_tool_call(json!({"name": "echo", "arguments": {"text": "hi"}}))
            .await;
        assert!(
            ok.get("isError").is_none(),
            "echo should not error: {:?}",
            ok
        );
        assert_eq!(ok["content"][0]["text"], "hi");

        // Denied tool is blocked with an isError result — never reaches downstream.
        let blocked = gw
            .handle_tool_call(json!({"name": "secret_probe", "arguments": {}}))
            .await;
        assert_eq!(blocked["isError"], json!(true));

        // Unknown tool fails closed.
        let unknown = gw
            .handle_tool_call(json!({"name": "does_not_exist", "arguments": {}}))
            .await;
        assert_eq!(unknown["isError"], json!(true));

        // Everything is in the audit trail.
        let events = memory.get_events(&gw.task_id.to_string()).unwrap();
        assert!(events
            .iter()
            .any(|e| e.event_type == "tool.call.completed" && e.tool.as_deref() == Some("echo")));
        assert!(events.iter().any(|e| e.event_type == "tool.call.blocked"
            && e.tool.as_deref() == Some("secret_probe")
            && e.policy_decision.as_deref() == Some("Deny")));

        let _ = fs::remove_file(&db_path);
    }
}
