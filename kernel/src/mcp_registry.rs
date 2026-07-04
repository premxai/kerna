use crate::config::McpServerConfig;
use crate::mcp::{McpClient, McpTool};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Manages the lifecycle and routing of all registered MCP servers.
pub struct McpRegistry {
    /// Map from tool name → server name that owns it
    tool_to_server: HashMap<String, String>,
    /// Map from server name → active client connection
    clients: HashMap<String, McpClient>,
    /// Full list of all discovered tools across all servers
    all_tools: Vec<McpTool>,
}

impl McpRegistry {
    pub fn new() -> Self {
        McpRegistry {
            tool_to_server: HashMap::new(),
            clients: HashMap::new(),
            all_tools: Vec::new(),
        }
    }

    /// Spawn all configured MCP servers and discover their tools.
    pub async fn initialize(&mut self, configs: &[McpServerConfig]) -> Result<()> {
        for config in configs {
            if !config.enabled {
                println!("[MCP] Skipping disabled server: {}", config.name);
                continue;
            }

            println!("[MCP] Spawning server: {} ({})", config.name, config.command);

            let args_ref: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

            match McpClient::spawn(&config.command, &args_ref) {
                Ok(mut client) => {
                    // Discover tools from this server
                    match client.list_tools().await {
                        Ok(tools) => {
                            println!(
                                "[MCP] Server '{}' registered {} tools:",
                                config.name,
                                tools.len()
                            );
                            for tool in &tools {
                                println!("  → {}", tool.name);
                                self.tool_to_server
                                    .insert(tool.name.clone(), config.name.clone());
                                self.all_tools.push(tool.clone());
                            }
                            self.clients.insert(config.name.clone(), client);
                        }
                        Err(e) => {
                            eprintln!(
                                "[MCP] Warning: Failed to list tools from '{}': {}",
                                config.name, e
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[MCP] Warning: Failed to spawn server '{}': {}",
                        config.name, e
                    );
                }
            }
        }

        Ok(())
    }

    /// Get all available tools across all registered MCP servers.
    /// Returns tool definitions formatted for LLM function calling.
    pub fn get_tool_definitions(&self) -> Vec<serde_json::Value> {
        self.all_tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description.clone().unwrap_or_default(),
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect()
    }

    /// Route a tool call to the correct MCP server and return the result.
    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let server_name = self
            .tool_to_server
            .get(tool_name)
            .ok_or_else(|| anyhow!("No MCP server registered for tool '{}'", tool_name))?
            .clone();

        let client = self
            .clients
            .get_mut(&server_name)
            .ok_or_else(|| anyhow!("MCP server '{}' not connected", server_name))?;

        client.call_tool(tool_name, arguments).await
    }

    /// Check if a tool is available.
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tool_to_server.contains_key(tool_name)
    }

    /// Get all tool names.
    pub fn tool_names(&self) -> Vec<String> {
        self.tool_to_server.keys().cloned().collect()
    }
}
