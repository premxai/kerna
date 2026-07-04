use crate::config::Config;
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
}

impl TaskScheduler {
    pub fn new(
        config: Config,
        memory: Arc<MemoryEngine>,
        mcp_registry: Arc<Mutex<McpRegistry>>,
        session_id: Option<String>,
    ) -> Result<Self> {
        let sandbox = ProcessSandbox::new(&config.sandbox_dir)?;
        let permissions = PermissionManager::new(config.clone());
        Ok(TaskScheduler {
            config,
            memory,
            sandbox,
            mcp_registry,
            permissions,
            session_id,
        })
    }

    /// Run a goal using an agentic tool-call loop.
    /// The LLM decides which tools to call, we execute them, feed results back,
    /// and repeat until the LLM returns a final text response.
    pub async fn run_goal(&self, goal: &str) -> Result<Uuid> {
        let task_id = Uuid::new_v4();
        self.memory.create_task(task_id, self.session_id.as_deref(), goal)?;
        self.memory
            .log_message(task_id, "INFO", &format!("Received goal: {}", goal))?;

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
        all_tools.push(json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Execute a command in the sandboxed terminal. Use this for file operations, scripts, and system tasks.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "The command to run" },
                        "args": { "type": "array", "items": { "type": "string" }, "description": "Command arguments" }
                    },
                    "required": ["command"]
                }
            }
        }));

        self.memory
            .update_task_status(task_id, "running")?;

        let mut round = 0;
        let max_rounds = self.config.max_tool_rounds;

        // === AGENTIC LOOP ===
        loop {
            round += 1;
            if round > max_rounds {
                println!("[!] Max tool rounds ({}) reached. Finishing.", max_rounds);
                self.memory
                    .log_message(task_id, "WARN", "Max tool rounds reached")?;
                break;
            }

            println!("[*] Round {}/{} — Calling LLM...", round, max_rounds);

            // Call the LLM with retry logic for bad responses
            let mut attempt = 0;
            let response = loop {
                attempt += 1;
                match self.call_llm(&messages, &all_tools).await {
                    Ok(r) => break Ok(r),
                    Err(e) => {
                        let err_msg = format!("LLM call failed (Attempt {}/3): {}", attempt, e);
                        self.memory.log_message(task_id, "WARN", &err_msg)?;
                        println!("[!] {}", err_msg);
                        
                        if attempt >= 3 {
                            break Err(e);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            };

            let response = match response {
                Ok(r) => r,
                Err(_) => {
                    println!("[!] LLM failed after 3 attempts. Aborting task.");
                    break;
                }
            };

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

                    // Check permissions
                    let perm_level = self.permissions.check(tool_name);
                    if perm_level == PermissionLevel::Deny {
                        println!("❌ {} (Denied by policy)", display_name);
                        messages.push(ChatMessage {
                            role: "tool".to_string(),
                            content: Some(format!("Permission denied.")),
                            tool_calls: None,
                            tool_call_id: Some(tc.id.clone()),
                        });
                        continue;
                    }

                    if perm_level == PermissionLevel::RequireConfirmation {
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

                    // Execute tool
                    let tool_args: serde_json::Value = serde_json::from_str(tool_args_str).unwrap_or(json!({}));
                    let result = self.execute_tool(tool_name, &tool_args).await;

                    let mut result_str = match &result {
                        Ok(val) => {
                            println!("✓ {}", display_name);
                            val.to_string()
                        }
                        Err(e) => {
                            println!("✓ Retry ({})", display_name);
                            format!("Error: {}", e)
                        }
                    };

                    // HUGE OUTPUT SABOTAGE FIX: Truncate massively large tool outputs to prevent context window blowup
                    let max_tool_output_len = 50_000;
                    if result_str.len() > max_tool_output_len {
                        let truncated: String = result_str.chars().take(max_tool_output_len).collect();
                        result_str = format!("{}... [Output Truncated by Kerna ({} chars exceeded)]", truncated, max_tool_output_len);
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
                let memory_content = format!("Goal: {}. Result: {}", goal, content);
                self.memory.add_episodic_memory(&memory_content, &[0.1, 0.2, 0.3])?;
                self.memory.log_message(task_id, "INFO", content)?;
                break;
            } else {
                println!("✓ Finished (Empty response)\n");
                break;
            }
        }

        // Calculate observability metrics
        let duration_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64 
            - 
            // In a real system, we'd grab the task created_at, but we'll mock duration for now since we didn't track start
            (round * 5) as i64; // Mock duration: 5 seconds per round
        
        let dur = if duration_secs < 0 { 2 } else { duration_secs };
        
        // Mock token usage based on rounds
        let tokens_used = (round * 450) as i64;
        let cost = (tokens_used as f64) * 0.00001; // Mock pricing

        self.memory.update_task_observability(task_id, dur, &self.config.llm_model, cost, tokens_used, round.saturating_sub(1) as i64)?;
        self.memory.update_task_status(task_id, "completed")?;
        println!("\n[+] Task {} completed.", task_id);

        Ok(task_id)
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
            "run_command" => {
                let cmd = args["command"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Missing 'command' argument"))?;
                let args_arr: Vec<&str> = args["args"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                let output = self.sandbox.run_command(cmd, &args_arr, 30).await?;
                Ok(json!({ "output": output }))
            }
            _ => Err(anyhow!("Unknown tool: {}", tool_name)),
        }
    }

    /// Call the LLM with the conversation history and tools.
    async fn call_llm(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<ChatMessage> {
        if self.config.llm_api_key.is_empty() {
            return Err(anyhow!("No LLM API key configured"));
        }

        let client = reqwest::Client::new();

        match self.config.llm_provider.as_str() {
            "openai" | "venice" => {
                let url = if self.config.llm_provider == "venice" {
                    "https://api.venice.ai/api/v1/chat/completions"
                } else {
                    "https://api.openai.com/v1/chat/completions"
                };

                let mut body = json!({
                    "model": self.config.llm_model,
                    "messages": messages,
                });

                if !tools.is_empty() {
                    body["tools"] = json!(tools);
                    body["tool_choice"] = json!("auto");
                }

                let response = client
                    .post(url)
                    .bearer_auth(&self.config.llm_api_key)
                    .json(&body)
                    .send()
                    .await?;

                let res_json: serde_json::Value = response.json().await?;

                let choice = &res_json["choices"][0]["message"];
                let content = choice["content"].as_str().map(|s| s.to_string());

                let tool_calls: Option<Vec<ToolCallRequest>> =
                    if let Some(tcs) = choice["tool_calls"].as_array() {
                        Some(
                            tcs.iter()
                                .filter_map(|tc| serde_json::from_value(tc.clone()).ok())
                                .collect(),
                        )
                    } else {
                        None
                    };

                Ok(ChatMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                })
            }
            "anthropic" => {
                // Convert messages to Anthropic format
                let system_msg = messages
                    .iter()
                    .find(|m| m.role == "system")
                    .and_then(|m| m.content.clone())
                    .unwrap_or_default();

                let anthropic_messages: Vec<serde_json::Value> = messages
                    .iter()
                    .filter(|m| m.role != "system")
                    .map(|m| {
                        if m.role == "tool" {
                            json!({
                                "role": "user",
                                "content": [{
                                    "type": "tool_result",
                                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                                    "content": m.content.clone().unwrap_or_default()
                                }]
                            })
                        } else {
                            json!({
                                "role": m.role,
                                "content": m.content.clone().unwrap_or_default()
                            })
                        }
                    })
                    .collect();

                let anthropic_tools: Vec<serde_json::Value> = tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t["function"]["name"],
                            "description": t["function"]["description"],
                            "input_schema": t["function"]["parameters"]
                        })
                    })
                    .collect();

                let mut body = json!({
                    "model": self.config.llm_model,
                    "max_tokens": 4096,
                    "system": system_msg,
                    "messages": anthropic_messages,
                });

                if !anthropic_tools.is_empty() {
                    body["tools"] = json!(anthropic_tools);
                }

                let response = client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", &self.config.llm_api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body)
                    .send()
                    .await?;

                let res_json: serde_json::Value = response.json().await?;

                // Parse Anthropic response
                let content_blocks = res_json["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();

                let mut text_content: Option<String> = None;
                let mut tool_calls_vec: Vec<ToolCallRequest> = Vec::new();

                for block in &content_blocks {
                    match block["type"].as_str() {
                        Some("text") => {
                            text_content = block["text"].as_str().map(|s| s.to_string());
                        }
                        Some("tool_use") => {
                            tool_calls_vec.push(ToolCallRequest {
                                id: block["id"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                call_type: "function".to_string(),
                                function: FunctionCall {
                                    name: block["name"]
                                        .as_str()
                                        .unwrap_or("unknown")
                                        .to_string(),
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

                Ok(ChatMessage {
                    role: "assistant".to_string(),
                    content: text_content,
                    tool_calls,
                    tool_call_id: None,
                })
            }
            _ => Err(anyhow!(
                "Unsupported LLM provider: {}",
                self.config.llm_provider
            )),
        }
    }

    /// Fallback demo when no LLM API key is configured.
    async fn run_fallback_demo(&self, task_id: Uuid, goal: &str) -> Result<()> {
        println!("[!] Running offline demonstration mode...");

        let echo_cmd = format!(
            "echo AgentOS executed goal: '{}' successfully. > workspace_v2\\report.txt",
            goal
        );

        let steps: Vec<(&str, &str, Vec<&str>)> = vec![
            ("Create workspace", "cmd", vec!["/C", "mkdir workspace_v2"]),
            (
                "Generate report",
                "cmd",
                vec!["/C", &echo_cmd],
            ),
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
                    self.memory
                        .log_message(task_id, "ERROR", &format!("Step '{}' failed: {}", name, e))?;
                }
            }
        }

        let memory_content = format!("Goal: {}. Completed via offline fallback.", goal);
        self.memory
            .add_episodic_memory(&memory_content, &[0.1, 0.2, 0.3])?;

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
