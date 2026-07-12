use crate::budget::{BudgetConfig, BudgetTracker};
use crate::config::Config;
use crate::events::{Event, EventSink};
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::permissions::{PermissionLevel, PermissionManager};
use crate::sandbox::ProcessSandbox;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Represents an individual message in the LLM conversation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub struct TaskScheduler {
    config: Config,
    memory: Arc<MemoryEngine>,
    sandbox: ProcessSandbox,
    mcp_registry: Arc<Mutex<McpRegistry>>,
    permissions: PermissionManager,
    session_id: Option<String>,
    /// Shared HTTP client — reused across LLM rounds so TLS connections pool.
    http_client: reqwest::Client,
}

impl TaskScheduler {
    pub fn new(
        config: Config,
        memory: Arc<MemoryEngine>,
        mcp_registry: Arc<Mutex<McpRegistry>>,
        session_id: Option<String>,
    ) -> Result<Self> {
        let sandbox = ProcessSandbox::new(
            &config.sandbox_dir,
            config.runtime_mode.clone(),
            config.allow_dynamic_installs,
            config.network_mode.clone(),
            config.egress_proxy.clone(),
        )?;
        let permissions = PermissionManager::new(config.clone());
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(TaskScheduler {
            config,
            memory,
            sandbox,
            mcp_registry,
            permissions,
            session_id,
            http_client,
        })
    }

    /// Run a goal using an agentic tool-call loop.
    /// The LLM decides which tools to call, we execute them, feed results back,
    /// and repeat until the LLM returns a final text response.
    pub fn run_goal<'a>(
        &'a self,
        goal: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Uuid>> + Send + 'a>> {
        Box::pin(async move {
            let task_started_at = std::time::Instant::now();
            let task_id = Uuid::new_v4();
            self.memory
                .create_task(task_id, self.session_id.as_deref(), goal)?;
            self.memory
                .log_message(task_id, "INFO", &format!("Received goal: {}", goal))?;

            let mut tool_failures: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();
            let mut total_tool_failures = 0;
            let mut event_seq: i64 = 0;

            println!("\n╔══════════════════════════════════════════════════════════════╗");
            println!("║  Kerna Task Runner                                          ║");
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Task ID: {}  ║", task_id);
            println!("║  Goal: {:<53} ║", truncate_str(goal, 53));
            println!("╚══════════════════════════════════════════════════════════════╝\n");

            // Build system prompt with full context injection (memories + preferences + facts)
            let context_str = self.memory.gather_context(goal).unwrap_or_default();

            let system_prompt = format!(
                "You are Kerna, an autonomous AI agent runtime. \
                You help users accomplish goals by using available tools. \
                Execute the user's goal step by step. Use tools when needed. \
                When the goal is fully complete, respond with a final summary. \
                Be concise and action-oriented.\n\n{}",
                context_str
            );

            // Build the conversation
            let mut messages: Vec<ChatMessage> = vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: Some(system_prompt),
                    tool_calls: None,
                    tool_call_id: None,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: Some(goal.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                },
            ];

            // Get tool definitions from MCP registry
            let tool_defs = {
                let registry = self.mcp_registry.lock().await;
                registry.get_tool_definitions()
            };

            // Add built-in sandbox tools
            let mut all_tools = tool_defs;
            all_tools.extend(crate::tool_packs::get_tool_definitions());

            // Delegate task is not in packs yet, so add it here temporarily
            all_tools.push(json!({
            "type": "function",
            "function": {
                "name": "delegate_task",
                "description": "Delegate a subtask to a new sandboxed agent instance. Useful for parallelization or isolated tasks.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "goal": { "type": "string", "description": "The specific goal for the subagent to complete" }
                    },
                    "required": ["goal"]
                }
            }
        }));

            self.memory.update_task_status(task_id, "running")?;

            let mut round = 0;
            let max_rounds = self.config.max_tool_rounds;
            let max_retries = self.config.max_retries;

            let budget_config = BudgetConfig {
                max_runtime_seconds: self.config.max_runtime_seconds,
                max_tool_calls: self.config.max_tool_calls,
                max_llm_calls: self.config.max_llm_calls,
                max_cost_usd: self.config.max_cost_usd,
                max_output_bytes: self.config.max_output_bytes,
                max_memory_writes: self.config.max_memory_writes,
            };
            let mut budget = BudgetTracker::new(budget_config);
            let mut total_tokens_used: u64 = 0;
            let mut total_cost_usd: f64 = 0.0;

            // === AGENTIC LOOP ===
            let loop_result: Result<()> = tokio::select! {
                res = async {
                    loop {
                round += 1;
                if round > max_rounds {
                    println!("[!] Max tool rounds ({}) reached. Finishing.", max_rounds);
                    self.memory
                        .log_message(task_id, "WARN", "Max tool rounds reached")?;
                    let _ = self.memory.update_task_status(task_id, "failed");
                    return Err(anyhow::anyhow!("Task failed: Max tool rounds reached"));
                }

                println!("[*] Round {}/{} — Calling LLM...", round, max_rounds);

                // Call the LLM with retry logic for bad responses
                let mut attempt = 0;
                let response = loop {
                    attempt += 1;
                    match self.call_llm(&messages, &all_tools).await {
                        Ok(r) => break Ok(r),
                        Err(e) => {
                            let err_msg = format!("LLM call failed (Attempt {}/{}): {}", attempt, max_retries, e);
                            let _ = self.memory.log_message(task_id, "WARN", &err_msg);
                            println!("[!] {}", err_msg);

                            if attempt >= max_retries {
                                break Err(e);
                            }

                            // Exponential backoff
                            let backoff_secs = 2u64.pow(attempt);
                            println!("[*] Retrying in {} seconds...", backoff_secs);
                            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        }
                    }
                };

                let (response, tokens_used) = match response {
                    Ok(r) => r,
                    Err(e) => {
                        println!("[!] LLM failed after {} attempts. Aborting task.", max_retries);
                        return Err(e);
                    }
                };

                // Update budget with real token usage and model-based pricing.
                let cost_increment =
                    crate::providers::estimate_cost_usd(&self.config.llm_model, tokens_used)
                        .unwrap_or(0.0);
                total_tokens_used += tokens_used;
                total_cost_usd += cost_increment;
                if let Err(e) = budget.record_llm_call(cost_increment) {
                    let _ = self.memory.log_message(task_id, "ERROR", &e.to_string());
                    let _ = self.memory.update_task_status(task_id, "failed");
                    return Err(e);
                }

                // Check if the LLM returned tool calls
                if let Some(tool_calls) = &response.tool_calls {
                    messages.push(response.clone());

                    for tc in tool_calls {
                        let tool_name = &tc.function.name;
                        let tool_args_str = &tc.function.arguments;

                        let display_name = match tool_name.as_str() {
                            "run_command" => "Terminal",
                            "fs_list_dir" => "Filesystem",
                            "desktop_click" | "desktop_type" => "Desktop",
                            "voice_speak" | "voice_listen" => "Voice",
                            "web_search" | "web_navigate" => "Browser",
                            _ => "Planning",
                        };

                        let server_name = {
                            let registry = self.mcp_registry.lock().await;
                            registry.get_server_for_tool(tool_name)
                        };

                        if self.config.enable_supervisor {
                            let supervisor_prompt = format!(
                                "You are a Supervisor Agent. Does this tool call look safe and logical for a background agent? Tool: {}, Args: {}. Reply exactly with 'APPROVE' or 'REJECT'.",
                                tool_name, tool_args_str
                            );
                            let sup_msg = ChatMessage {
                                role: "user".to_string(),
                                content: Some(supervisor_prompt),
                                tool_calls: None,
                                tool_call_id: None,
                            };
                            println!("[Supervisor] Checking {}...", tool_name);
                            match self.call_llm(&[sup_msg], &[]).await {
                                Ok((res, _)) => {
                                    let decision = res.content.unwrap_or_default().trim().to_uppercase();
                                    if decision.contains("REJECT") {
                                        println!("❌ {} (Rejected by Supervisor)", display_name);
                                        messages.push(ChatMessage {
                                            role: "tool".to_string(),
                                            content: Some("Supervisor rejected this action because it appears unsafe or illogical.".to_string()),
                                            tool_calls: None,
                                            tool_call_id: Some(tc.id.clone()),
                                        });
                                        continue;
                                    }
                                }
                                Err(e) => {
                                    println!("[!] Supervisor check failed: {}", e);
                                }
                            }
                        }

                        // 1. Emit tool.call.requested
                        event_seq += 1;
                        let parent_evt_id = uuid::Uuid::new_v4().to_string();
                        let _ = self.memory.record(Event {
                            event_id: parent_evt_id.clone(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: "tool.call.requested".to_string(),
                            actor: "llm".to_string(),
                            severity: "info".to_string(),
                            model: Some(self.config.llm_model.clone()),
                            tool: Some(tool_name.clone()),
                            policy_decision: None,
                            risk_score: None,
                            parent_event_id: None,
                            correlation_id: Some(tc.id.clone()),
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({ "args": tool_args_str }),
                        });

                        let perm_level = self.permissions.check(tool_name, server_name.as_deref());

                        // 2. Emit tool.policy.checked
                        event_seq += 1;
                        let _ = self.memory.record(Event {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: "tool.policy.checked".to_string(),
                            actor: "kerna".to_string(),
                            severity: if perm_level == PermissionLevel::Deny { "warning".to_string() } else { "info".to_string() },
                            model: None,
                            tool: Some(tool_name.clone()),
                            policy_decision: Some(format!("{:?}", perm_level)),
                            risk_score: None,
                            parent_event_id: Some(parent_evt_id.clone()),
                            correlation_id: Some(tc.id.clone()),
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({ "args": tool_args_str }),
                        });

                        if perm_level == PermissionLevel::Deny {
                            println!("❌ {} (Denied by policy)", display_name);
                            messages.push(ChatMessage {
                                role: "tool".to_string(),
                                content: Some("Permission denied.".to_string()),
                                tool_calls: None,
                                tool_call_id: Some(tc.id.clone()),
                            });
                            continue;
                        }

                        if self.config.converse || perm_level == PermissionLevel::RequireConfirmation {
                            println!("⚠️ ACTION REQUIRES APPROVAL: {} {}", tool_name, tool_args_str);
                            let approved = PermissionManager::prompt_approval(tool_name, tool_args_str)?;
                            if !approved {
                                println!("❌ {} (Rejected)", display_name);
                                messages.push(ChatMessage {
                                    role: "tool".to_string(),
                                    content: Some("User rejected this action.".to_string()),
                                    tool_calls: None,
                                    tool_call_id: Some(tc.id.clone()),
                                });
                                continue;
                            }
                        }

                        if let Err(e) = budget.record_tool_call() {
                            event_seq += 1;
                            let _ = self.memory.record(Event {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                task_id: task_id.to_string(),
                                session_id: self.session_id.clone(),
                                sequence: event_seq,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                event_type: "budget.exceeded".to_string(),
                                actor: "kerna".to_string(),
                                severity: "error".to_string(),
                                model: None,
                                tool: Some(tool_name.clone()),
                                policy_decision: None,
                                risk_score: None,
                                parent_event_id: Some(parent_evt_id.clone()),
                                correlation_id: Some(tc.id.clone()),
                                redaction_status: None,
                                budget_snapshot_json: Some(budget.get_snapshot_json()),
                                payload_json: json!({ "error": e.to_string() }),
                            });
                            let _ = self.memory.log_message(task_id, "ERROR", &e.to_string());
                            let _ = self.memory.update_task_status(task_id, "failed");
                            return Err(e);
                        } else {
                            event_seq += 1;
                            let _ = self.memory.record(Event {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                task_id: task_id.to_string(),
                                session_id: self.session_id.clone(),
                                sequence: event_seq,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                event_type: "budget.checked".to_string(),
                                actor: "kerna".to_string(),
                                severity: "info".to_string(),
                                model: None,
                                tool: Some(tool_name.clone()),
                                policy_decision: None,
                                risk_score: None,
                                parent_event_id: Some(parent_evt_id.clone()),
                                correlation_id: Some(tc.id.clone()),
                                redaction_status: None,
                                budget_snapshot_json: Some(budget.get_snapshot_json()),
                                payload_json: json!({}),
                            });
                        }

                        // Execute tool
                        event_seq += 1;
                        let _ = self.memory.record(Event {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: "tool.call.started".to_string(),
                            actor: "kerna".to_string(),
                            severity: "info".to_string(),
                            model: None,
                            tool: Some(tool_name.clone()),
                            policy_decision: None,
                            risk_score: None,
                            parent_event_id: Some(parent_evt_id.clone()),
                            correlation_id: Some(tc.id.clone()),
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({}),
                        });

                        let tool_args: serde_json::Value = serde_json::from_str(tool_args_str).unwrap_or(json!({}));

                        let mut is_checkpointable = false;
                        if tool_name == "run_command" {
                            if let Some(cmd) = tool_args.get("command").and_then(|v| v.as_str()) {
                                let mut args_vec = Vec::new();
                                if let Some(args_arr) = tool_args.get("args").and_then(|v| v.as_array()) {
                                    for a in args_arr {
                                        if let Some(s) = a.as_str() {
                                            args_vec.push(s);
                                        }
                                    }
                                }
                                if self.config.workspace.checkpoint_enabled && self.sandbox.is_trusted_for_rollback(cmd, &args_vec) {
                                    is_checkpointable = true;
                                }
                            }
                        } else if tool_name == "execute_code" || tool_name == "execute_wasm" {
                            is_checkpointable = self.config.workspace.checkpoint_enabled;
                        }

                        if is_checkpointable {
                            match self.sandbox.snapshot() {
                                Ok(_) => {
                                    event_seq += 1;
                                    let _ = self.memory.record(Event {
                                        event_id: uuid::Uuid::new_v4().to_string(),
                                        task_id: task_id.to_string(),
                                        session_id: self.session_id.clone(),
                                        sequence: event_seq,
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        event_type: "checkpoint.created".to_string(),
                                        actor: "kerna".to_string(),
                                        severity: "info".to_string(),
                                        model: None,
                                        tool: Some(tool_name.clone()),
                                        policy_decision: None,
                                        risk_score: None,
                                        parent_event_id: Some(parent_evt_id.clone()),
                                        correlation_id: Some(tc.id.clone()),
                                        redaction_status: None,
                                        budget_snapshot_json: Some(json!({})),
                                        payload_json: json!({}),
                                    });
                                }
                                Err(e) => {
                                    println!("Snapshot failed: {}", e);
                                    is_checkpointable = false; // Snapshot failed
                                    event_seq += 1;
                                    let _ = self.memory.record(Event {
                                    event_id: uuid::Uuid::new_v4().to_string(),
                                    task_id: task_id.to_string(),
                                    session_id: self.session_id.clone(),
                                    sequence: event_seq,
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    event_type: "checkpoint.failed".to_string(),
                                    actor: "kerna".to_string(),
                                    severity: "warning".to_string(),
                                    model: None,
                                    tool: Some(tool_name.clone()),
                                    policy_decision: None,
                                    risk_score: None,
                                    parent_event_id: Some(parent_evt_id.clone()),
                                    correlation_id: Some(tc.id.clone()),
                                    redaction_status: None,
                                    budget_snapshot_json: Some(json!({})),
                                    payload_json: json!({}),
                                });
                            }
                        }
                    }

                        let result = self.execute_tool(tool_name, &tool_args).await;

                        if is_checkpointable && result.is_err() {
                            event_seq += 1;
                            let _ = self.memory.record(Event {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                task_id: task_id.to_string(),
                                session_id: self.session_id.clone(),
                                sequence: event_seq,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                event_type: "workspace.rollback.started".to_string(),
                                actor: "kerna".to_string(),
                                severity: "warning".to_string(),
                                model: None,
                                tool: Some(tool_name.clone()),
                                policy_decision: None,
                                risk_score: None,
                                parent_event_id: Some(parent_evt_id.clone()),
                                correlation_id: Some(tc.id.clone()),
                                redaction_status: None,
                                budget_snapshot_json: Some(json!({})),
                                payload_json: json!({}),
                            });

                            let rollback_event = if self.sandbox.rollback().is_ok() {
                                "workspace.rollback.completed"
                            } else {
                                "workspace.rollback.failed"
                            };

                            event_seq += 1;
                            let _ = self.memory.record(Event {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                task_id: task_id.to_string(),
                                session_id: self.session_id.clone(),
                                sequence: event_seq,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                event_type: rollback_event.to_string(),
                                actor: "kerna".to_string(),
                                severity: "warning".to_string(),
                                model: None,
                                tool: Some(tool_name.clone()),
                                policy_decision: None,
                                risk_score: None,
                                parent_event_id: Some(parent_evt_id.clone()),
                                correlation_id: Some(tc.id.clone()),
                                redaction_status: None,
                                budget_snapshot_json: Some(json!({})),
                                payload_json: json!({}),
                            });
                        } else if is_checkpointable && result.is_ok() {
                            event_seq += 1;
                            let _ = self.memory.record(Event {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                task_id: task_id.to_string(),
                                session_id: self.session_id.clone(),
                                sequence: event_seq,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                event_type: "checkpoint.discarded".to_string(),
                                actor: "kerna".to_string(),
                                severity: "info".to_string(),
                                model: None,
                                tool: Some(tool_name.clone()),
                                policy_decision: None,
                                risk_score: None,
                                parent_event_id: Some(parent_evt_id.clone()),
                                correlation_id: Some(tc.id.clone()),
                                redaction_status: None,
                                budget_snapshot_json: Some(budget.get_snapshot_json()),
                                payload_json: json!({"reason": "tool_execution_succeeded"}),
                            });
                        }

                        let mut result_str = match &result {
                            Ok(val) => {
                                println!("✔️ {}", display_name);
                                val.to_string()
                            }
                            Err(e) => {
                                println!("❌ Retry ({})", display_name);
                                let e_str = e.to_string();
                                let count = tool_failures.entry(tool_name.to_string()).or_insert(0);
                                *count += 1;
                                total_tool_failures += 1;

                                if (e_str.contains("timeout") || e_str.contains("Timeout"))
                                    && *count >= 2 {
                                        return Err(anyhow!("Task failed: Tool '{}' timed out {} times.", tool_name, count));
                                    }

                                if total_tool_failures >= 5 {
                                    return Err(anyhow!("Task failed: Max total tool failures (5) reached."));
                                }

                                format!("Error: {}", e_str)
                            }
                        };

                        event_seq += 1;
                        let _ = self.memory.record(Event {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: if result.is_ok() { "tool.call.completed".to_string() } else { "tool.call.failed".to_string() },
                            actor: "tool".to_string(),
                            severity: if result.is_ok() { "info".to_string() } else { "error".to_string() },
                            model: None,
                            tool: Some(tool_name.clone()),
                            policy_decision: None,
                            risk_score: None,
                            parent_event_id: Some(parent_evt_id.clone()),
                            correlation_id: Some(tc.id.clone()),
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({ "result_len": result_str.len() }),
                        });

                        // HUGE OUTPUT SABOTAGE FIX: Truncate massively large tool outputs to prevent context window blowup
                        let max_tool_output_len = self.config.max_output_bytes as usize;
                        if result_str.len() > max_tool_output_len {
                            let truncated: String = result_str.chars().take(max_tool_output_len).collect();
                            result_str = format!("{}... [Output Truncated by Kerna ({} bytes exceeded)]", truncated, max_tool_output_len);
                        }

                        if let Err(e) = budget.record_output_bytes(result_str.len() as u64) {
                            let _ = self.memory.log_message(task_id, "ERROR", &e.to_string());
                            let _ = self.memory.update_task_status(task_id, "failed");
                            return Err(e);
                        }

                        // SECURITY: Prompt Injection Middleware
                        if crate::security::PromptInjectionDetector::is_prompt_injection(&result_str) {
                            println!("⚠️ SECURITY: Prompt Injection detected in tool output. Stripping output.");
                            result_str = "[SYSTEM: The tool output was stripped because it matched known prompt injection heuristics.]".to_string();
                        }

                        self.memory.log_message(
                            task_id,
                            if result.is_ok() { "INFO" } else { "ERROR" },
                            &format!("Tool [{}]: {}", tool_name, truncate_str(&result_str, 500)),
                        )?;

                        messages.push(ChatMessage {
                            role: "tool".to_string(),
                            content: Some(result_str),
                            tool_calls: None,
                            tool_call_id: Some(tc.id.clone()),
                        });
                    }
                } else if let Some(content) = &response.content {
                    // Goal done
                    println!("✓ Finished\n");

                    // Store as episodic memory
                    // Store as episodic memory
                    let memory_content = format!("Goal: {}. Result: {}", goal, content);

                    event_seq += 1;
                    let _ = self.memory.record(Event {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        task_id: task_id.to_string(),
                        session_id: self.session_id.clone(),
                        sequence: event_seq,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        event_type: "memory.write.proposed".to_string(),
                        actor: "llm".to_string(),
                        severity: "info".to_string(),
                        model: None,
                        tool: None,
                        policy_decision: None,
                        risk_score: None,
                        parent_event_id: None,
                        correlation_id: None,
                        redaction_status: None,
                        budget_snapshot_json: Some(budget.get_snapshot_json()),
                        payload_json: json!({ "content_len": memory_content.len() }),
                    });

                    if let Err(e) = budget.record_memory_write() {
                        event_seq += 1;
                        let _ = self.memory.record(Event {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: "memory.write.skipped".to_string(),
                            actor: "kerna".to_string(),
                            severity: "warning".to_string(),
                            model: None,
                            tool: None,
                            policy_decision: None,
                            risk_score: None,
                            parent_event_id: None,
                            correlation_id: None,
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({ "error": e.to_string() }),
                        });
                        println!("⚠️ Memory write skipped (Budget exceeded)");
                    } else {
                        // TODO: Replace dummy embedding with actual embedding model call
                        let mem_id = self.memory.add_episodic_memory(&memory_content)?;
                        println!("⚠️ Memory proposal STAGED for approval (ID: {})", mem_id);

                        event_seq += 1;
                        let _ = self.memory.record(Event {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.to_string(),
                            session_id: self.session_id.clone(),
                            sequence: event_seq,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            event_type: "memory.write.staged".to_string(),
                            actor: "kerna".to_string(),
                            severity: "info".to_string(),
                            model: None,
                            tool: None,
                            policy_decision: None,
                            risk_score: None,
                            parent_event_id: None,
                            correlation_id: None,
                            redaction_status: None,
                            budget_snapshot_json: Some(budget.get_snapshot_json()),
                            payload_json: json!({ "content_len": memory_content.len(), "memory_id": mem_id }),
                        });
                    }

                    self.memory.log_message(task_id, "INFO", content)?;
                    let _ = self.memory.set_task_result(task_id, content);
                    break;
                } else {
                    println!("✓ Finished (Empty response)\n");
                    break;
                }
            }
                    Ok(())
                } => res,
                _ctrl_c_res = tokio::signal::ctrl_c() => {
                    println!("\n[!] Ctrl+C detected. Cancelling task...");
                    Err(anyhow::anyhow!("Task interrupted by user"))
                }
            };

            if let Err(e) = loop_result {
                let status = if e.to_string().contains("interrupted") {
                    "failed: interrupted"
                } else {
                    "failed"
                };
                let _ = self.memory.update_task_status(task_id, status);
                return Err(e);
            }

            // Calculate observability metrics from real accumulated usage.
            let duration_secs = task_started_at.elapsed().as_secs() as i64;
            let dur = if duration_secs < 0 { 0 } else { duration_secs };

            self.memory.update_task_observability(
                task_id,
                dur,
                &self.config.llm_model,
                total_cost_usd,
                total_tokens_used as i64,
                round.saturating_sub(1) as i64,
            )?;
            self.memory.update_task_status(task_id, "completed")?;
            println!("\n[+] Task {} completed.", task_id);

            Ok(task_id)
        })
    }

    /// Execute a tool call — either via MCP registry or built-in sandbox.
    async fn execute_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Check MCP registry first
        {
            let mut registry = self.mcp_registry.lock().await;
            if registry.has_tool(tool_name) {
                return registry.call_tool(tool_name, args.clone()).await;
            }
        }

        // Built-in tools
        match tool_name {
            "delegate_task" => {
                let sub_goal = args["goal"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'goal' argument"))?;

                let mut sub_config = self.config.clone();
                // Isolate the subagent's budget to a strict bound
                sub_config.max_tool_calls = std::cmp::min(self.config.max_tool_calls, 10);
                sub_config.max_llm_calls = std::cmp::min(self.config.max_llm_calls, 10);

                // Spawn a new TaskScheduler for the subagent
                // We pass a new session_id to isolate its working memory
                let sub_scheduler = TaskScheduler::new(
                    sub_config,
                    self.memory.clone(),
                    self.mcp_registry.clone(),
                    Some(uuid::Uuid::new_v4().to_string()),
                )?;

                // Run the sub-goal
                let sub_task_id = sub_scheduler.run_goal(sub_goal).await?;
                Ok(
                    json!({ "status": "Subagent completed task", "sub_task_id": sub_task_id.to_string() }),
                )
            }
            _ => {
                // Try tool packs
                crate::tool_packs::execute_tool(tool_name, args, &self.sandbox, &self.config).await
            }
        }
    }

    /// Call the LLM with the conversation history and tools, handling fallbacks and credential pooling.
    async fn call_llm(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, u64)> {
        let provider_name = self.config.llm_provider.clone();
        let model = self.config.llm_model.clone();

        // Try the primary key, then any credentials in the rotation pool.
        let mut keys = vec![self.config.llm_api_key.clone()];
        keys.extend(self.config.credential_pool.clone());

        let mut last_error = anyhow!("No credentials attempted");

        for key in &keys {
            let resolved =
                match crate::providers::resolve(&self.config, &provider_name, Some(&model), key) {
                    Ok(r) => r,
                    Err(e) => {
                        // Resolution failures (unknown provider, missing key) are not
                        // fixed by trying a different key — surface immediately.
                        last_error = e;
                        break;
                    }
                };

            match self.execute_resolved(&resolved, messages, tools).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    let err_str = e.to_string();
                    last_error = e;
                    if err_str.contains("429") || err_str.contains("401") {
                        println!(
                            "[!] Rate limited or unauthorized. Trying next credential in pool..."
                        );
                        continue;
                    } else {
                        break;
                    }
                }
            }
        }

        if let (Some(fb_prov), Some(fb_key)) = (
            &self.config.llm_fallback_provider,
            &self.config.llm_fallback_api_key,
        ) {
            println!(
                "[!] Primary LLM failed. Trying fallback provider: {}",
                fb_prov
            );
            let resolved = crate::providers::resolve(&self.config, fb_prov, None, fb_key)?;
            return self.execute_resolved(&resolved, messages, tools).await;
        }

        Err(anyhow!(
            "LLM call failed after all attempts: {}",
            last_error
        ))
    }

    /// Dispatch a resolved provider to the correct wire implementation.
    async fn execute_resolved(
        &self,
        resolved: &crate::providers::ResolvedProvider,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, u64)> {
        use crate::providers::WireProtocol;
        match resolved.protocol {
            WireProtocol::Mock => call_mock(messages),
            WireProtocol::OpenAiCompat => self.call_openai_compat(resolved, messages, tools).await,
            WireProtocol::Anthropic => self.call_anthropic(resolved, messages, tools).await,
        }
    }

    /// OpenAI-compatible `/chat/completions` wire format (OpenAI, OpenRouter,
    /// Ollama, Groq, Together, DeepSeek, Mistral, xAI, Venice, ...).
    async fn call_openai_compat(
        &self,
        resolved: &crate::providers::ResolvedProvider,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, u64)> {
        let client = &self.http_client;

        let url = format!(
            "{}/chat/completions",
            resolved.base_url.trim_end_matches('/')
        );

        let mut body = json!({
            "model": resolved.model,
            "messages": messages,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }

        let mut req = client.post(&url).json(&body);
        if !resolved.api_key.is_empty() {
            req = req.bearer_auth(&resolved.api_key);
        }

        let response = req.send().await?;

        let status = response.status();
        if !status.is_success() {
            let err_text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(anyhow!(
                    "401 Unauthorized: check the API key for this provider."
                ));
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(anyhow!("429 Rate Limited: please slow down."));
            } else {
                return Err(anyhow!("HTTP Error {}: {}", status, err_text));
            }
        }

        let res_json: serde_json::Value = response.json().await?;

        if let Some(err) = res_json.get("error") {
            return Err(anyhow!("API Error: {}", err));
        }

        let choice = &res_json["choices"][0]["message"];
        let content = choice["content"].as_str().map(|s| s.to_string());

        let tool_calls: Option<Vec<ToolCallRequest>> = choice["tool_calls"].as_array().map(|tcs| {
            tcs.iter()
                .filter_map(|tc| serde_json::from_value(tc.clone()).ok())
                .collect()
        });

        let total_tokens = res_json["usage"]["total_tokens"].as_u64().unwrap_or(0);

        Ok((
            ChatMessage {
                role: "assistant".to_string(),
                content,
                tool_calls,
                tool_call_id: None,
            },
            total_tokens,
        ))
    }

    /// Anthropic `/v1/messages` wire format, with correct multi-turn tool use:
    /// assistant `tool_calls` become `tool_use` blocks and `tool`-role results
    /// become `tool_result` blocks in the following user turn.
    async fn call_anthropic(
        &self,
        resolved: &crate::providers::ResolvedProvider,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<(ChatMessage, u64)> {
        let client = &self.http_client;

        let (system_prompt, anthropic_messages) = convert_to_anthropic(messages);

        let mut body = json!({
            "model": resolved.model,
            "max_tokens": 4096,
            "system": system_prompt,
            "messages": anthropic_messages,
        });

        if !tools.is_empty() {
            let anthropic_tools: Vec<_> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t["function"]["name"],
                        "description": t["function"]["description"],
                        "input_schema": t["function"]["parameters"]
                    })
                })
                .collect();
            body["tools"] = json!(anthropic_tools);
        }

        let url = format!("{}/v1/messages", resolved.base_url.trim_end_matches('/'));

        let response = client
            .post(&url)
            .header("x-api-key", &resolved.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let err_text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(anyhow!("401 Unauthorized: check your Anthropic API key."));
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(anyhow!("429 Rate Limited: please slow down."));
            }
            return Err(anyhow!("Anthropic HTTP Error {}: {}", status, err_text));
        }

        let res_json: serde_json::Value = response.json().await?;

        if let Some(err) = res_json.get("error") {
            return Err(anyhow!("Anthropic API Error: {}", err));
        }

        let content_blocks = res_json["content"].as_array().cloned().unwrap_or_default();

        let mut text_content: Option<String> = None;
        let mut tool_calls_vec: Vec<ToolCallRequest> = Vec::new();

        for block in &content_blocks {
            match block["type"].as_str() {
                Some("text") => {
                    text_content = block["text"].as_str().map(|s| s.to_string());
                }
                Some("tool_use") => {
                    tool_calls_vec.push(ToolCallRequest {
                        id: block["id"].as_str().unwrap_or("unknown").to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: block["name"].as_str().unwrap_or("unknown").to_string(),
                            arguments: block["input"].to_string(),
                        },
                    });
                }
                _ => {}
            }
        }

        let tool_calls = if tool_calls_vec.is_empty() {
            None
        } else {
            Some(tool_calls_vec)
        };

        let total_tokens = res_json["usage"]["input_tokens"].as_u64().unwrap_or(0)
            + res_json["usage"]["output_tokens"].as_u64().unwrap_or(0);

        Ok((
            ChatMessage {
                role: "assistant".to_string(),
                content: text_content,
                tool_calls,
                tool_call_id: None,
            },
            total_tokens,
        ))
    }

    /// Fallback demo when no LLM API key is configured.
    #[allow(dead_code)]
    async fn run_fallback_demo(&self, task_id: Uuid, goal: &str) -> Result<()> {
        println!("[!] Running offline demonstration mode...");

        let echo_cmd = format!(
            "echo Kerna executed goal: '{}' successfully. > workspace_v2\\report.txt",
            goal
        );

        let steps: Vec<(&str, &str, Vec<&str>)> = vec![
            ("Create workspace", "cmd", vec!["/C", "mkdir workspace_v2"]),
            ("Generate report", "cmd", vec!["/C", &echo_cmd]),
            (
                "Verify output",
                "cmd",
                vec!["/C", "type workspace_v2\\report.txt"],
            ),
        ];

        for (i, (name, cmd, args)) in steps.iter().enumerate() {
            println!("[*] Step {}/{}: {}", i + 1, steps.len(), name);
            let args_ref: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
            match self.sandbox.run_command(cmd, &args_ref, 30).await {
                Ok(output) => {
                    self.memory.log_message(
                        task_id,
                        "INFO",
                        &format!("Step '{}': {}", name, output),
                    )?;
                }
                Err(e) => {
                    self.memory.log_message(
                        task_id,
                        "ERROR",
                        &format!("Step '{}' failed: {}", name, e),
                    )?;
                }
            }
        }

        let memory_content = format!("Goal: {}. Completed via offline fallback.", goal);
        let mem_id = self.memory.add_episodic_memory(&memory_content)?;
        println!("⚠️ Memory proposal STAGED for approval (ID: {})", mem_id);

        Ok(())
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// Flush any pending Anthropic `tool_result` blocks as a single user turn.
fn flush_tool_results(pending: &mut Vec<serde_json::Value>, out: &mut Vec<serde_json::Value>) {
    if !pending.is_empty() {
        out.push(json!({ "role": "user", "content": std::mem::take(pending) }));
    }
}

/// Convert Kerna's OpenAI-shaped message list into Anthropic's `(system, messages)`
/// form. Assistant `tool_calls` become `tool_use` blocks; consecutive `tool`-role
/// results are coalesced into one following user turn of `tool_result` blocks so
/// the required user/assistant alternation is preserved.
pub fn convert_to_anthropic(messages: &[ChatMessage]) -> (String, Vec<serde_json::Value>) {
    let mut system_prompt = String::new();
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                if !system_prompt.is_empty() {
                    system_prompt.push_str("\n\n");
                }
                system_prompt.push_str(msg.content.as_deref().unwrap_or(""));
            }
            "tool" => {
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": msg.content.clone().unwrap_or_default(),
                }));
            }
            "user" => {
                flush_tool_results(&mut pending_tool_results, &mut out);
                out.push(json!({
                    "role": "user",
                    "content": msg.content.clone().unwrap_or_default()
                }));
            }
            "assistant" => {
                flush_tool_results(&mut pending_tool_results, &mut out);
                if let Some(tcs) = &msg.tool_calls {
                    let mut blocks: Vec<serde_json::Value> = Vec::new();
                    if let Some(text) = &msg.content {
                        if !text.is_empty() {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    for tc in tcs {
                        let input: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.function.name,
                            "input": input,
                        }));
                    }
                    out.push(json!({ "role": "assistant", "content": blocks }));
                } else {
                    out.push(json!({
                        "role": "assistant",
                        "content": msg.content.clone().unwrap_or_default()
                    }));
                }
            }
            _ => {}
        }
    }
    flush_tool_results(&mut pending_tool_results, &mut out);
    (system_prompt, out)
}

/// In-process deterministic mock model used by tests and the zero-key demo.
/// Chooses a MockMCP tool based on keywords in the latest user goal and returns
/// a finishing message once a successful tool result comes back.
fn call_mock(messages: &[ChatMessage]) -> Result<(ChatMessage, u64)> {
    // If the last message is a successful tool result, finish the interaction.
    if let Some(last_msg) = messages.last() {
        if last_msg.role == "tool" {
            let content = last_msg.content.as_deref().unwrap_or("");
            if !content.contains("Error:") && !content.contains("Supervisor rejected") {
                return Ok((
                    ChatMessage {
                        role: "assistant".to_string(),
                        content: Some("Mock finished".to_string()),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    10,
                ));
            }
        }
    }

    let last_user_msg = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_deref())
        .unwrap_or("");

    let (cmd, args) = if last_user_msg.contains("echo") {
        ("echo".to_string(), "{}".to_string())
    } else if last_user_msg.contains("hang") {
        ("hang".to_string(), "{}".to_string())
    } else if last_user_msg.contains("huge_output") {
        ("huge_output".to_string(), "{}".to_string())
    } else if last_user_msg.contains("invalid_json") {
        ("invalid_json".to_string(), "{}".to_string())
    } else if last_user_msg.contains("fail_once_then_pass") {
        ("fail_once_then_pass".to_string(), "{}".to_string())
    } else if last_user_msg.contains("malicious") {
        ("malicious".to_string(), "{}".to_string())
    } else if last_user_msg.contains("Please fail") {
        (
            "run_command".to_string(),
            "{\"command\": \"rm\", \"args\": [\"does_not_exist_file_12345.txt\"]}".to_string(),
        )
    } else if last_user_msg.contains("Please delegate") {
        (
            "delegate_task".to_string(),
            "{\"goal\": \"subtask\"}".to_string(),
        )
    } else if let Some(rest) = last_user_msg.strip_prefix("MOCK_FS_READ ") {
        // Test-only trigger: "MOCK_FS_READ <root> <path>" drives a real
        // fs.read tool call so integration tests can exercise folder grants
        // end-to-end through the scheduler, not just unit-test the resolver.
        let mut parts = rest.splitn(2, ' ');
        let root = parts.next().unwrap_or("workspace");
        let path = parts.next().unwrap_or("");
        (
            "fs.read".to_string(),
            serde_json::json!({ "root": root, "path": path }).to_string(),
        )
    } else if let Some(rest) = last_user_msg.strip_prefix("MOCK_FS_WRITE ") {
        let mut parts = rest.splitn(3, ' ');
        let root = parts.next().unwrap_or("workspace");
        let path = parts.next().unwrap_or("");
        let content = parts.next().unwrap_or("");
        (
            "fs.write".to_string(),
            serde_json::json!({ "root": root, "path": path, "content": content }).to_string(),
        )
    } else if last_user_msg.contains("memory_writes") {
        return Ok((
            ChatMessage {
                role: "assistant".to_string(),
                content: Some("I am attempting to write to memory now.".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            10,
        ));
    } else {
        return Ok((
            ChatMessage {
                role: "assistant".to_string(),
                content: Some("Mock finished".to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            10,
        ));
    };

    let tc = ToolCallRequest {
        id: "call_mock".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: cmd,
            arguments: args,
        },
    };

    Ok((
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![tc]),
            tool_call_id: None,
        },
        10,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: Option<&str>) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.map(|s| s.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn anthropic_conversion_extracts_system_and_roles() {
        let messages = vec![
            msg("system", Some("You are Kerna.")),
            msg("user", Some("do the thing")),
        ];
        let (system, out) = convert_to_anthropic(&messages);
        assert_eq!(system, "You are Kerna.");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
    }

    #[test]
    fn anthropic_conversion_maps_tool_use_and_result() {
        // assistant makes a tool call, then a tool result comes back.
        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallRequest {
                id: "call_1".to_string(),
                call_type: "function".to_string(),
                function: FunctionCall {
                    name: "echo".to_string(),
                    arguments: "{\"text\":\"hi\"}".to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let tool_result = ChatMessage {
            role: "tool".to_string(),
            content: Some("hi".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
        };
        let messages = vec![
            msg("system", Some("sys")),
            msg("user", Some("say hi")),
            assistant,
            tool_result,
        ];
        let (_system, out) = convert_to_anthropic(&messages);
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(out[1]["content"][0]["type"], "tool_use");
        assert_eq!(out[1]["content"][0]["id"], "call_1");
        assert_eq!(out[2]["role"], "user");
        assert_eq!(out[2]["content"][0]["type"], "tool_result");
        assert_eq!(out[2]["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn anthropic_coalesces_multiple_tool_results() {
        // Two tool results after one assistant turn must land in ONE user turn.
        let mk_tool = |id: &str| ChatMessage {
            role: "tool".to_string(),
            content: Some(format!("result {}", id)),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
        };
        let messages = vec![
            msg("user", Some("go")),
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_calls: Some(vec![
                    ToolCallRequest {
                        id: "a".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "t".to_string(),
                            arguments: "{}".to_string(),
                        },
                    },
                    ToolCallRequest {
                        id: "b".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "t".to_string(),
                            arguments: "{}".to_string(),
                        },
                    },
                ]),
                tool_call_id: None,
            },
            mk_tool("a"),
            mk_tool("b"),
        ];
        let (_s, out) = convert_to_anthropic(&messages);
        // user, assistant, user(with 2 tool_result blocks)
        assert_eq!(out.len(), 3);
        let last = &out[2];
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"].as_array().unwrap().len(), 2);
    }
}
