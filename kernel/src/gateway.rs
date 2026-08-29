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

use crate::budget::{BudgetConfig, BudgetTracker, Remaining};
use crate::config::Config;
use crate::events::{Event, EventSink};
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::permissions::{PermissionLevel, PermissionManager};
use anyhow::Result;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
        InitializeRequestParams, ListToolsResult, ProtocolVersion, ServerCapabilities, ServerInfo,
        Tool,
    },
    service::RequestContext,
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use uuid::Uuid;

const LEGACY_MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const CODEX_MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Select a protocol revision that this gateway can actually serve.
///
/// MCP clients may close the connection when the initialize response advertises
/// a revision they do not support. Codex currently initiates with 2025-06-18,
/// while the demo verifier and older IDE clients use 2024-11-05.
fn negotiate_protocol_version(params: Option<&serde_json::Value>) -> &'static str {
    match params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(serde_json::Value::as_str)
    {
        Some(CODEX_MCP_PROTOCOL_VERSION) => CODEX_MCP_PROTOCOL_VERSION,
        _ => LEGACY_MCP_PROTOCOL_VERSION,
    }
}

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
    client_name: Option<String>,
    client_version: Option<String>,
    /// Budgets for this client session.
    ///
    /// Only the three the gateway can actually observe are enforced here:
    /// tool calls, wall clock, and the bytes handed back to the client. The
    /// model runs in the customer's own client, so `max_llm_calls` and
    /// `max_cost_usd` are not ours to count -- and a budget reported as
    /// satisfied because nothing measured it is the same lie as a gate check
    /// passing on zero observations. `kerna run` remains the path that
    /// enforces all six.
    budget: BudgetTracker,
}

/// Official RMCP server wrapper. It owns MCP negotiation and stdio framing;
/// `Gateway` remains the single policy/containment implementation for every
/// listed or called tool.
#[derive(Clone)]
struct RmcpGateway {
    inner: Arc<Mutex<Gateway>>,
}

impl RmcpGateway {
    async fn exposed_tools(&self) -> Result<Vec<Tool>, McpError> {
        let gateway = self.inner.lock().await;
        let mut tools = {
            let registry = gateway.registry.lock().await;
            registry
                .get_mcp_tools()
                .into_iter()
                .filter(|tool| {
                    let name = tool
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    registry.tool_is_callable(name)
                        && gateway
                            .permissions
                            .check(name, registry.get_server_for_tool(name).as_deref())
                            != PermissionLevel::Deny
                })
                .map(|tool| {
                    serde_json::from_value::<Tool>(tool).map_err(|error| {
                        McpError::internal_error(
                            format!(
                                "Kerna rejected an invalid downstream tool declaration: {error}"
                            ),
                            None,
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        tools.push(
            serde_json::from_value(json!({
                "name": "kerna_session_status",
                "description": "Show Kerna containment, policy, and audit state for this MCP session.",
                "inputSchema": {"type": "object", "properties": {}}
            }))
            .map_err(|error| McpError::internal_error(error.to_string(), None))?,
        );
        Ok(tools)
    }
}

impl ServerHandler for RmcpGateway {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Kerna is a local policy gateway. It governs only MCP calls routed through this server.",
            );
        // rmcp's default `Implementation` is built from its own crate env, so a
        // client lists this server as "rmcp". The operator needs to see which
        // process is governing their tools.
        info.server_info = Implementation::new("kerna-gateway", env!("CARGO_PKG_VERSION"));
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[
            ProtocolVersion::V_2024_11_05,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_11_25,
        ])
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<ServerInfo, McpError> {
        let mut gateway = self.inner.lock().await;
        gateway.client_name = Some(request.client_info.name);
        gateway.client_version = Some(request.client_info.version);
        let _ = gateway.memory.identify_gateway_session(
            &gateway.session_id,
            gateway.client_name.as_deref(),
            gateway.client_version.as_deref(),
            request.protocol_version.as_str(),
        );
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        let mut result = ListToolsResult::default();
        result.tools = self.exposed_tools().await?;
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        let params = json!({
            "name": request.name,
            "arguments": request.arguments.unwrap_or_default(),
        });
        let mut gateway = self.inner.lock().await;
        let result = gateway.handle_tool_call(params).await;
        let result = serde_json::from_value::<CallToolResult>(result).map_err(|error| {
            McpError::internal_error(
                format!("Kerna generated an invalid governed tool response: {error}"),
                None,
            )
        })?;
        Ok(result.into())
    }
}

impl Gateway {
    pub fn new(
        config: Config,
        registry: Arc<Mutex<McpRegistry>>,
        memory: Arc<MemoryEngine>,
    ) -> Self {
        let permissions = PermissionManager::with_mode(
            config.clone(),
            if config.audit_only {
                crate::permissions::EnforcementMode::Observe
            } else {
                crate::permissions::EnforcementMode::Enforce
            },
        );
        let task_id = Uuid::new_v4();
        let session_id = format!("gateway-{}", Uuid::new_v4());
        // The two the gateway cannot see are set to zero rather than to the
        // configured value, so that a future caller reaching for them gets an
        // immediate, obvious failure instead of a number nothing measured.
        let budget = BudgetTracker::new(BudgetConfig {
            max_runtime_seconds: config.max_runtime_seconds,
            max_tool_calls: config.max_tool_calls,
            max_output_bytes: config.max_output_bytes,
            max_llm_calls: 0,
            max_cost_usd: 0.0,
            max_memory_writes: 0,
        });
        Gateway {
            config,
            registry,
            permissions,
            memory,
            task_id,
            session_id,
            sequence: 0,
            client_name: None,
            client_version: None,
            budget,
        }
    }

    /// Serve stdio through the official RMCP lifecycle until the client exits.
    pub async fn run(self) -> Result<()> {
        let shared = Arc::new(Mutex::new(self));
        {
            let mut gateway = shared.lock().await;
            gateway.start_session();
        }
        let service = RmcpGateway {
            inner: shared.clone(),
        };
        let server = service.serve(rmcp::transport::stdio()).await?;
        let outcome = server.waiting().await;
        {
            let mut gateway = shared.lock().await;
            gateway.finish_session();
        }
        outcome.map(|_| ()).map_err(Into::into)
    }

    fn start_session(&mut self) {
        // Record the gateway session as a task so `kerna trace <id>` works.
        if let Err(e) = self
            .memory
            .create_task(self.task_id, None, "MCP Gateway Session")
        {
            eprintln!("[gateway] warning: could not record gateway task: {}", e);
        }
        let _ = self.memory.update_task_status(self.task_id, "running");
        let workspace = std::env::current_dir()
            .unwrap_or_else(|_| Path::new(".").to_path_buf())
            .display()
            .to_string();
        let _ = self.memory.start_gateway_session(
            &self.session_id,
            &self.task_id.to_string(),
            &workspace,
        );
        eprintln!(
            "[gateway] Kerna MCP policy-gateway online (task {}). Proxying {} downstream server(s).",
            self.task_id,
            self.config.mcp_servers.len()
        );
    }

    fn finish_session(&mut self) {
        let _ = self.memory.update_task_status(self.task_id, "completed");
        let _ = self.memory.finish_gateway_session(&self.session_id);
    }

    /// Legacy parser retained only for reference while supporting old on-disk
    /// traces; production gateway execution calls `run` above.
    #[allow(dead_code)]
    async fn run_legacy_jsonrpc(&mut self) -> Result<()> {
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
        let workspace = std::env::current_dir()
            .unwrap_or_else(|_| Path::new(".").to_path_buf())
            .display()
            .to_string();
        let _ = self.memory.start_gateway_session(
            &self.session_id,
            &self.task_id.to_string(),
            &workspace,
        );
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
                            let protocol_version = negotiate_protocol_version(req.params.as_ref());
                            self.capture_client_identity(req.params.as_ref(), protocol_version);
                            self.respond(
                                id,
                                Some(json!({
                                    "protocolVersion": protocol_version,
                                    "capabilities": { "tools": {} },
                                    "serverInfo": { "name": "kerna-gateway", "version": env!("CARGO_PKG_VERSION") }
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
                            let mut tools = {
                                let registry = self.registry.lock().await;
                                registry
                                    .get_mcp_tools()
                                    .into_iter()
                                    .filter(|tool| {
                                        let name = tool
                                            .get("name")
                                            .and_then(|value| value.as_str())
                                            .unwrap_or_default();
                                        registry.tool_is_callable(name)
                                            && self.permissions.check(
                                                name,
                                                registry.get_server_for_tool(name).as_deref(),
                                            ) != PermissionLevel::Deny
                                    })
                                    .collect::<Vec<_>>()
                            };
                            tools.push(json!({
                                "name": "kerna_session_status",
                                "description": "Show Kerna containment, policy, and audit state for this MCP session.",
                                "inputSchema": {"type": "object", "properties": {}}
                            }));
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
        let _ = self.memory.finish_gateway_session(&self.session_id);
        Ok(())
    }

    /// The governed tool-call path: policy check → record → forward → record.
    async fn handle_tool_call(&mut self, params: serde_json::Value) -> serde_json::Value {
        let started = Instant::now();
        let call_id = Uuid::new_v4().to_string();
        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        if tool_name.is_empty() {
            return error_result("Missing tool name in tools/call request.");
        }

        if tool_name == "kerna_session_status" {
            let remaining = self.budget.remaining();
            return session_card(&self.config, &self.memory, &self.task_id, remaining);
        }

        // Wall clock first, and before anything else is looked at. A session
        // past its runtime limit has spent its budget whether or not the next
        // call would have been allowed, and answering it would extend a window
        // the operator already closed.
        if let Err(error) = self.budget.check_runtime() {
            return self.refuse_on_budget(&call_id, &tool_name, None, started, error);
        }

        let server_name = {
            let registry = self.registry.lock().await;
            registry.get_server_for_tool(&tool_name)
        };

        // Unknown tool → fail closed.
        if server_name.is_none() {
            let trace_id = self.record(
                "tool.call.blocked",
                Some(&tool_name),
                "warning",
                Some("UnknownTool"),
                json!({
                    "reason": "no downstream server exposes this tool",
                    "container": self.container_metadata(None)
                }),
            );
            self.start_receipt(&call_id, None, &tool_name, "UnknownTool");
            self.finish_receipt(
                &call_id,
                None,
                started.elapsed(),
                "blocked",
                trace_id.as_deref(),
                None,
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
            json!({
                "arguments": arguments,
                "server": server_name,
                "container": self.container_metadata(server_name.as_deref())
            }),
        );

        // Fail-closed policy check. Confirmation requests are durable and bound
        // to the exact retry; the downstream plugin never sees the first call.
        let level = self.permissions.check(&tool_name, server_name.as_deref());
        self.start_receipt(
            &call_id,
            server_name.as_deref(),
            &tool_name,
            &format!("{:?}", level),
        );
        self.record(
            "tool.policy.checked",
            Some(&tool_name),
            if level == PermissionLevel::AutoApprove {
                "info"
            } else {
                "warning"
            },
            Some(&format!("{:?}", level)),
            json!({ "container": self.container_metadata(server_name.as_deref()) }),
        );

        if level == PermissionLevel::RequireConfirmation {
            let binding = self.approval_binding(&tool_name, &arguments, server_name.as_deref());
            match self.memory.consume_gateway_approval(&binding) {
                Ok(true) => {
                    self.record(
                        "tool.approval.consumed",
                        Some(&tool_name),
                        "info",
                        Some("RequireConfirmation"),
                        json!({
                            "binding": binding,
                            "container": self.container_metadata(server_name.as_deref())
                        }),
                    );
                }
                Ok(false) => {
                    let approval_id = match self.memory.create_gateway_approval(
                        self.task_id,
                        &tool_name,
                        &canonical_json(&arguments),
                        &binding,
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            return error_result(&format!(
                                "Kerna could not queue approval: {}",
                                error
                            ))
                        }
                    };
                    let reason = format!(
                        "Tool '{}' requires approval. Run `kerna approval approve {}` and retry the exact call within 10 minutes.",
                        tool_name, approval_id
                    );
                    let trace_id = self.record(
                        "tool.approval.requested",
                        Some(&tool_name),
                        "warning",
                        Some("RequireConfirmation"),
                        json!({
                            "approval_id": approval_id,
                            "binding": binding,
                            "container": self.container_metadata(server_name.as_deref())
                        }),
                    );
                    self.finish_receipt(
                        &call_id,
                        Some(&approval_id),
                        started.elapsed(),
                        "approval_required",
                        trace_id.as_deref(),
                        None,
                    );
                    return error_result(&format!("Kerna gateway blocked this call. {}", reason));
                }
                Err(error) => {
                    self.finish_receipt(
                        &call_id,
                        None,
                        started.elapsed(),
                        "approval_error",
                        None,
                        None,
                    );
                    return error_result(&format!("Kerna could not check approval: {}", error));
                }
            }
        } else if level == PermissionLevel::Deny {
            let reason = match level {
                PermissionLevel::Deny => {
                    format!("Tool '{}' is denied by Kerna policy.", tool_name)
                }
                PermissionLevel::RequireConfirmation => unreachable!(),
                PermissionLevel::AutoApprove => unreachable!(),
            };
            let trace_id = self.record(
                "tool.call.blocked",
                Some(&tool_name),
                "warning",
                Some(&format!("{:?}", level)),
                json!({
                    "reason": reason,
                    "container": self.container_metadata(server_name.as_deref())
                }),
            );
            self.finish_receipt(
                &call_id,
                None,
                started.elapsed(),
                "blocked",
                trace_id.as_deref(),
                None,
            );
            return error_result(&format!("Kerna gateway blocked this call. {}", reason));
        }

        // Charged here, not earlier: a denied call and one waiting on approval
        // have done no work and reached no plugin. The budget bounds what an
        // agent gets to *do*, so refusing a call must not also spend from it --
        // otherwise a policy that denies loudly would exhaust the session it
        // was protecting.
        if let Err(error) = self.budget.record_tool_call() {
            return self.refuse_on_budget(
                &call_id,
                &tool_name,
                server_name.as_deref(),
                started,
                error,
            );
        }

        // Forward to the downstream server (registry also enforces
        // allow_tools/deny_tools/capabilities filters).
        let forward = {
            let mut registry = self.registry.lock().await;
            registry.call_tool(&tool_name, arguments.clone()).await
        };

        match forward {
            Ok(result) => {
                // The result exists, so the work is already done and the budget
                // cannot undo it. What it can still do is stop the payload
                // reaching the client, which is the thing `max_output_bytes`
                // is actually protecting: an agent's context, and the bill for
                // carrying it on every later turn.
                let size = serde_json::to_string(&result).map(|s| s.len()).unwrap_or(0) as u64;
                if let Err(error) = self.budget.record_output_bytes(size) {
                    return self.refuse_on_budget(
                        &call_id,
                        &tool_name,
                        server_name.as_deref(),
                        started,
                        error,
                    );
                }
                let result_preview = redacted_preview(&result);
                let trace_id = self.record(
                    "tool.call.completed",
                    Some(&tool_name),
                    "info",
                    Some("AutoApprove"),
                    json!({
                        "result_preview": result_preview,
                        "container": self.container_metadata(server_name.as_deref())
                    }),
                );
                self.finish_receipt(
                    &call_id,
                    None,
                    started.elapsed(),
                    "completed",
                    trace_id.as_deref(),
                    Some(&result_preview),
                );
                // The downstream result is already an MCP tools/call result
                // (content blocks); pass it straight through.
                result
            }
            Err(e) => {
                let error_text = e.to_string();
                let trace_id = self.record(
                    "tool.call.failed",
                    Some(&tool_name),
                    "error",
                    Some("AutoApprove"),
                    json!({
                        "error": error_text,
                        "container": self.container_metadata(server_name.as_deref())
                    }),
                );
                self.finish_receipt(
                    &call_id,
                    None,
                    started.elapsed(),
                    "failed",
                    trace_id.as_deref(),
                    Some(&redacted_text_preview(&error_text)),
                );
                error_result(&format!("Downstream tool '{}' failed: {}", tool_name, e))
            }
        }
    }

    fn approval_binding(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        server_name: Option<&str>,
    ) -> String {
        let image = server_name
            .and_then(|name| {
                self.config
                    .mcp_servers
                    .iter()
                    .find(|server| server.name == name)
            })
            .map(|server| server.image.as_str())
            .unwrap_or_default();
        let workspace = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
        let payload = format!(
            "workspace={}\\nimage={}\\ntool={}\\narguments={}",
            workspace.display(),
            image,
            tool_name,
            canonical_json(arguments)
        );
        format!("{:x}", Sha256::digest(payload.as_bytes()))
    }

    fn container_metadata(&self, server_name: Option<&str>) -> serde_json::Value {
        let Some(server) = server_name.and_then(|name| {
            self.config
                .mcp_servers
                .iter()
                .find(|server| server.name == name)
        }) else {
            return json!({ "mode": "none", "network": "none" });
        };
        json!({
            "mode": server.runtime_mode,
            "image_digest": server.image,
            "mount_scope": {
                "read_roots": server.read_roots,
                "write_roots": server.write_roots
            },
            "network": "none"
        })
    }

    fn capture_client_identity(
        &mut self,
        params: Option<&serde_json::Value>,
        protocol_version: &str,
    ) {
        self.client_name = params
            .and_then(|value| value.get("clientInfo"))
            .and_then(|value| value.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        self.client_version = params
            .and_then(|value| value.get("clientInfo"))
            .and_then(|value| value.get("version"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let _ = self.memory.identify_gateway_session(
            &self.session_id,
            self.client_name.as_deref(),
            self.client_version.as_deref(),
            protocol_version,
        );
    }

    fn start_receipt(
        &self,
        call_id: &str,
        server_name: Option<&str>,
        tool: &str,
        policy_decision: &str,
    ) {
        let server = server_name.and_then(|name| {
            self.config
                .mcp_servers
                .iter()
                .find(|server| server.name == name)
        });
        let _ = self.memory.start_tool_call_receipt(
            call_id,
            &self.session_id,
            &self.task_id.to_string(),
            self.client_name.as_deref(),
            server.map(|server| server.name.as_str()),
            server.and_then(|server| (!server.image.is_empty()).then_some(server.image.as_str())),
            tool,
            policy_decision,
        );
    }

    fn finish_receipt(
        &self,
        call_id: &str,
        approval_id: Option<&str>,
        elapsed: std::time::Duration,
        result_class: &str,
        trace_id: Option<&str>,
        preview: Option<&str>,
    ) {
        let _ = self.memory.finish_tool_call_receipt(
            call_id,
            approval_id,
            elapsed.as_millis().min(i64::MAX as u128) as i64,
            result_class,
            trace_id,
            preview,
        );
    }

    /// Refuse a call because a budget is spent, and say which one.
    ///
    /// Deliberately the same shape as a policy denial: an MCP error the client
    /// can read, a recorded event, and a closed receipt. An agent that is told
    /// *why* it stopped can report that to the person waiting; one that gets a
    /// bare failure retries, which is the worst available outcome for a limit
    /// designed to stop work.
    fn refuse_on_budget(
        &mut self,
        call_id: &str,
        tool_name: &str,
        server_name: Option<&str>,
        started: Instant,
        error: anyhow::Error,
    ) -> serde_json::Value {
        let reason = error.to_string();
        let snapshot = self.budget.get_snapshot_json();
        let remaining = self.budget.remaining();
        let trace_id = self.record(
            "budget.exceeded",
            Some(tool_name),
            "error",
            Some("BudgetExceeded"),
            json!({
                "reason": reason,
                "budget": snapshot,
                "remaining": {
                    "tool_calls": remaining.tool_calls,
                    "output_bytes": remaining.output_bytes,
                    "runtime_seconds": remaining.runtime_seconds,
                },
                "container": self.container_metadata(server_name)
            }),
        );
        // A receipt may not have been opened yet -- the runtime check runs
        // before the tool is even resolved -- so open one now if needed. A
        // refusal with no receipt is a refusal the dashboard cannot show.
        self.start_receipt(call_id, server_name, tool_name, "BudgetExceeded");
        self.finish_receipt(
            call_id,
            None,
            started.elapsed(),
            "budget_exceeded",
            trace_id.as_deref(),
            None,
        );
        error_result(&format!(
            "Kerna gateway stopped this call: {}. No further tool calls will be \
             served in this session -- start a new one, or raise the limit in kerna.toml.",
            reason
        ))
    }

    fn record(
        &mut self,
        event_type: &str,
        tool: Option<&str>,
        severity: &str,
        policy_decision: Option<&str>,
        payload: serde_json::Value,
    ) -> Option<String> {
        self.sequence += 1;
        let event_id = Uuid::new_v4().to_string();
        let result = self.memory.record(Event {
            event_id: event_id.clone(),
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
        result.ok().map(|_| event_id)
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

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", body)
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

fn session_card(
    config: &Config,
    memory: &MemoryEngine,
    task_id: &Uuid,
    remaining: Remaining,
) -> serde_json::Value {
    let pending = memory
        .list_pending_approvals()
        .map(|items| items.len())
        .unwrap_or_default();
    let contained = config
        .mcp_servers
        .iter()
        .filter(|server| server.enabled && server.runtime_mode == "docker")
        .count();
    let mounts = config
        .mcp_servers
        .iter()
        .filter(|server| server.enabled)
        .map(|server| {
            format!(
                "{}: ro=[{}], rw=[{}]",
                server.name,
                server.read_roots.join(","),
                server.write_roots.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let exposed = config
        .mcp_servers
        .iter()
        .filter(|server| server.enabled)
        .flat_map(|server| {
            if !server.allow_tools.is_empty() {
                server.allow_tools.clone()
            } else {
                server.capabilities.clone()
            }
        })
        .collect::<Vec<_>>();
    let write_state = if config
        .mcp_servers
        .iter()
        .any(|server| server.enabled && !server.write_roots.is_empty())
    {
        "approval-gated write roots"
    } else {
        "read-only (no write roots)"
    };
    let last_decision = memory
        .get_events(&task_id.to_string())
        .ok()
        .and_then(|events| {
            events
                .iter()
                .rev()
                .find(|event| event.policy_decision.is_some())
                .and_then(|event| event.policy_decision.clone())
        })
        .unwrap_or_else(|| "none".to_string());
    let text = format!(
        "Kerna session card\\nSandbox: Docker required; network disabled\\nContained plugins: {}\\nTools exposed: {}\\nWrite state: {}\\nMounts: {}\\nPending approvals: {}\\nLast decision: {}\\nBudget left: {} tool calls, {}s, {} bytes\\nTrace ID: {}",
        contained,
        if exposed.is_empty() { "none".to_string() } else { exposed.join(", ") },
        write_state,
        if mounts.is_empty() { "none" } else { &mounts },
        pending,
        last_decision,
        remaining.tool_calls,
        remaining.runtime_seconds,
        remaining.output_bytes,
        task_id
    );
    json!({ "content": [{ "type": "text", "text": text }] })
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

fn redacted_preview(value: &serde_json::Value) -> String {
    let (redacted, _) = crate::events::redact_payload(value);
    preview(&redacted)
}

pub(crate) fn redacted_text_preview(value: &str) -> String {
    let (redacted, _) =
        crate::events::redact_payload(&serde_json::Value::String(value.to_string()));
    preview(&redacted)
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

    #[test]
    fn negotiates_the_codex_protocol_revision() {
        assert_eq!(
            negotiate_protocol_version(Some(&json!({"protocolVersion": "2025-06-18"}))),
            CODEX_MCP_PROTOCOL_VERSION
        );
        assert_eq!(
            negotiate_protocol_version(Some(&json!({"protocolVersion": "2024-11-05"}))),
            LEGACY_MCP_PROTOCOL_VERSION
        );
    }

    #[tokio::test]
    async fn official_rmcp_stdio_service_negotiates_legacy_and_codex_clients() {
        let db_path = std::env::temp_dir().join(format!("kerna-rmcp-{}.db", Uuid::new_v4()));
        let memory = Arc::new(MemoryEngine::new(&db_path).unwrap());
        let gateway = Gateway::new(
            Config {
                db_path: db_path.to_string_lossy().to_string(),
                ..Config::default()
            },
            Arc::new(Mutex::new(McpRegistry::new())),
            memory,
        );
        let service = RmcpGateway {
            inner: Arc::new(Mutex::new(gateway)),
        };
        let (server_io, client_io) = tokio::io::duplex(16 * 1024);
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let server = service.serve(server_io).await.unwrap();
            let _ = shutdown_rx.await;
            server.cancel().await.unwrap();
        });
        let (read, mut write) = tokio::io::split(client_io);
        let mut read = BufReader::new(read);
        let mut line = String::new();

        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"Codex\",\"version\":\"test\"}}}\n")
            .await
            .unwrap();
        read.read_line(&mut line).await.unwrap();
        let initialized: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");

        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n")
            .await
            .unwrap();
        line.clear();
        read.read_line(&mut line).await.unwrap();
        let tools: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "kerna_session_status"));

        shutdown.send(()).unwrap();
        task.await.unwrap();
        let _ = std::fs::remove_file(db_path);
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
            image: String::new(),
            manifest_path: String::new(),
            manifest_sha256: String::new(),
            signing_public_key: String::new(),
            read_roots: vec![],
            write_roots: vec![],
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

    #[tokio::test]
    async fn gateway_approval_is_bound_to_one_exact_retry() {
        let db_path = format!("test_gateway_approval_{}.db", Uuid::new_v4());
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
            image:
                "fixture@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
            manifest_path: String::new(),
            manifest_sha256: String::new(),
            signing_public_key: String::new(),
            read_roots: vec![],
            write_roots: vec![],
            capabilities: vec![],
            allowed_paths: vec![],
            approval_required: vec![],
            allow_tools: vec![],
            deny_tools: vec![],
            secrets: vec![],
        });
        config.permissions.push(PermissionRule {
            tool: "echo".to_string(),
            action: "require_confirmation".to_string(),
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
        let mut gateway = Gateway::new(config, registry, memory.clone());
        memory
            .create_task(gateway.task_id, None, "MCP Gateway Session")
            .unwrap();

        let call = json!({"name": "echo", "arguments": {"text": "approved"}});
        let requested = gateway.handle_tool_call(call.clone()).await;
        assert_eq!(requested["isError"], json!(true));
        assert!(requested["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("approval approve"));
        let approval_id = memory.list_pending_approvals().unwrap()[0].0.clone();
        assert!(memory.decide_pending_approval(&approval_id, true).unwrap());

        let completed = gateway.handle_tool_call(call).await;
        assert_eq!(completed["content"][0]["text"], "approved");

        // The consumed approval cannot authorize a different argument hash.
        let changed = gateway
            .handle_tool_call(json!({"name": "echo", "arguments": {"text": "changed"}}))
            .await;
        assert_eq!(changed["isError"], json!(true));
        assert_eq!(memory.list_pending_approvals().unwrap().len(), 1);
        let _ = fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn the_tool_call_budget_stops_the_session_and_denials_do_not_spend_it() {
        let db_path = format!("test_gateway_budget_{}.db", Uuid::new_v4());
        let _ = fs::remove_file(&db_path);
        let memory = Arc::new(MemoryEngine::new(&db_path).unwrap());

        let mut config = Config {
            db_path: db_path.clone(),
            max_tool_calls: 2,
            ..Config::default()
        };
        config.mcp_servers.push(McpServerConfig {
            name: "mockmcp".to_string(),
            command: kerna_bin(),
            args: vec!["mockmcp".to_string()],
            enabled: true,
            runtime_mode: "local".to_string(),
            docker_image: String::new(),
            image: String::new(),
            manifest_path: String::new(),
            manifest_sha256: String::new(),
            signing_public_key: String::new(),
            read_roots: vec![],
            write_roots: vec![],
            capabilities: vec![],
            allowed_paths: vec![],
            approval_required: vec![],
            allow_tools: vec![],
            deny_tools: vec![],
            secrets: vec![],
        });
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

        // Denied calls must not consume the budget: a loud policy would
        // otherwise exhaust the session it exists to protect.
        for _ in 0..5 {
            let denied = gw
                .handle_tool_call(json!({"name": "network_probe", "arguments": {}}))
                .await;
            assert_eq!(denied["isError"], json!(true));
        }

        // Two forwarded calls fit the budget of two.
        for i in 0..2 {
            let ok = gw
                .handle_tool_call(json!({"name": "echo", "arguments": {"text": "hi"}}))
                .await;
            assert!(
                ok.get("isError").is_none(),
                "call {i} should succeed: {ok:?}"
            );
        }

        // The third is refused, and says which limit it hit.
        let stopped = gw
            .handle_tool_call(json!({"name": "echo", "arguments": {"text": "hi"}}))
            .await;
        assert_eq!(stopped["isError"], json!(true));
        let text = stopped["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("max tool calls of 2"),
            "refusal must name the budget: {text}"
        );

        // Exhaustion is durable -- a retry gets the same answer, not a slot
        // that quietly reopened.
        let again = gw
            .handle_tool_call(json!({"name": "echo", "arguments": {"text": "hi"}}))
            .await;
        assert_eq!(again["isError"], json!(true));

        // And it is on the record, not only in the response.
        let events = memory.get_events(&gw.task_id.to_string()).unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "budget.exceeded"),
            "a budget refusal must be auditable"
        );

        // The session card reports the remainder, so an agent can stop at a
        // boundary of its own choosing rather than be cut off at an arbitrary one.
        let card = gw
            .handle_tool_call(json!({"name": "kerna_session_status", "arguments": {}}))
            .await;
        assert!(card["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Budget left: 0 tool calls"));

        drop(gw);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn canonical_arguments_are_order_independent() {
        assert_eq!(
            canonical_json(&json!({"second": 2, "first": {"b": true, "a": false}})),
            canonical_json(&json!({"first": {"a": false, "b": true}, "second": 2}))
        );
    }

    #[tokio::test]
    async fn contained_filesystem_fixture_reads_and_approval_gates_real_writes() {
        if !crate::sandbox::docker_available() {
            eprintln!("skipping Docker acceptance test: Docker daemon is unavailable");
            return;
        }
        let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/filesystem-mcp");
        let mut config: Config =
            toml::from_str(&fs::read_to_string(workspace.join("kerna.toml")).unwrap()).unwrap();
        let db_path =
            std::env::temp_dir().join(format!("kerna-container-gateway-{}.db", Uuid::new_v4()));
        config.db_path = db_path.to_string_lossy().to_string();
        let memory = Arc::new(MemoryEngine::new(&config.db_path).unwrap());
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        registry
            .lock()
            .await
            .initialize_production(&config, &workspace)
            .await
            .unwrap();
        let mut gateway = Gateway::new(config, registry, memory.clone());
        memory
            .create_task(gateway.task_id, None, "contained fixture")
            .unwrap();

        let read = gateway
            .handle_tool_call(json!({"name": "read_file", "arguments": {"path": "hello.txt"}}))
            .await;
        assert!(read["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("real project file"));

        let write_call = json!({"name": "write_file", "arguments": {"path": "from_gateway.txt", "content": "written through Kerna"}});
        let requested = gateway.handle_tool_call(write_call.clone()).await;
        assert_eq!(requested["isError"], json!(true));
        assert!(!workspace.join("write/from_gateway.txt").exists());
        let approval = memory.list_pending_approvals().unwrap()[0].0.clone();
        assert!(memory.decide_pending_approval(&approval, true).unwrap());
        let written = gateway.handle_tool_call(write_call).await;
        assert_eq!(written["content"][0]["text"], "written");
        assert_eq!(
            fs::read_to_string(workspace.join("write/from_gateway.txt")).unwrap(),
            "written through Kerna"
        );

        let network = gateway
            .handle_tool_call(json!({"name": "network_probe", "arguments": {}}))
            .await;
        assert!(network["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("network unavailable"));

        let events = memory.get_events(&gateway.task_id.to_string()).unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "tool.approval.requested"));
        assert!(events
            .iter()
            .any(|event| event.event_type == "tool.call.completed"
                && event.tool.as_deref() == Some("write_file")));
        let _ = fs::remove_file(workspace.join("write/from_gateway.txt"));
        let _ = fs::remove_file(db_path);
    }
}
