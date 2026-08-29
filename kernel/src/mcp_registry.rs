use crate::config::{Config, McpServerConfig};
use crate::mcp::{McpClient, McpTool};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;

/// Manages the lifecycle and routing of all registered MCP servers.
pub struct McpRegistry {
    /// Map from tool name → server name that owns it
    tool_to_server: HashMap<String, String>,
    /// Map from server name → active client connection
    clients: HashMap<String, McpClient>,
    /// Server configs for capability enforcement
    server_configs: HashMap<String, McpServerConfig>,
    /// Full list of all discovered tools across all servers
    all_tools: Vec<McpTool>,
    /// When true, route status output to stderr instead of stdout. Required in
    /// gateway mode, where stdout is the MCP JSON-RPC channel and must not be
    /// polluted with human-readable diagnostics.
    quiet: bool,
}

impl McpRegistry {
    pub fn new() -> Self {
        McpRegistry {
            tool_to_server: HashMap::new(),
            clients: HashMap::new(),
            server_configs: HashMap::new(),
            all_tools: Vec::new(),
            quiet: false,
        }
    }

    /// Route status output to stderr (used by the MCP gateway, where stdout is
    /// the protocol channel).
    pub fn set_quiet(&mut self, quiet: bool) {
        self.quiet = quiet;
    }

    /// Emit a status line to stdout normally, or stderr in quiet mode.
    fn status(&self, msg: &str) {
        if self.quiet {
            eprintln!("{}", msg);
        } else {
            println!("{}", msg);
        }
    }

    /// MCP `tools/list`-format view of every discovered tool, for re-exposing
    /// downstream tools through the gateway.
    pub fn get_mcp_tools(&self) -> Vec<serde_json::Value> {
        self.all_tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description.clone().unwrap_or_default(),
                    "inputSchema": tool.input_schema,
                })
            })
            .collect()
    }

    /// Spawn all configured MCP servers and discover their tools.
    pub async fn initialize(&mut self, configs: &[McpServerConfig]) -> Result<()> {
        for configured_server in configs {
            if !configured_server.enabled {
                self.status(&format!(
                    "[MCP] Skipping disabled server: {}",
                    configured_server.name
                ));
                continue;
            }

            // Apply manifest declarations here too, so registry users outside
            // the CLI main path cannot accidentally bypass the contract.
            let mut config = configured_server.clone();
            match crate::plugin_manifest::apply_to_server(&mut config) {
                Ok(Some(path)) => self.status(&format!(
                    "[MCP] Enforcing manifest for '{}': {}",
                    config.name,
                    path.display()
                )),
                Ok(None) => self.status(&format!(
                    "[MCP] Development-only legacy warning: Plugin '{}' lacks a manifest.toml. Production gateway will refuse it.",
                    config.name
                )),
                Err(e) => {
                    eprintln!(
                        "[MCP] Refusing to start '{}' because its manifest is invalid: {}",
                        config.name, e
                    );
                    continue;
                }
            }

            let args_ref: Vec<&str> = config.args.iter().map(|s| s.as_str()).collect();

            match McpClient::spawn(
                &config.command,
                &args_ref,
                &config.runtime_mode,
                &config.docker_image,
                "bridge",
                None,
                &config.secrets,
            ) {
                Ok(mut client) => {
                    // Initialize the client
                    if let Err(e) = client.initialize().await {
                        eprintln!(
                            "[MCP] Warning: Failed to initialize server '{}': {}",
                            config.name, e
                        );
                    }

                    // Discover tools from this server
                    match client.list_tools().await {
                        Ok(tools) => {
                            self.status(&format!(
                                "[MCP] Server '{}' registered {} tools:",
                                config.name,
                                tools.len()
                            ));
                            for tool in &tools {
                                if self.tool_to_server.contains_key(&tool.name) {
                                    eprintln!(
                                        "[MCP] Warning: Tool '{}' from server '{}' conflicts with an existing tool. Skipping duplicate registration.",
                                        tool.name, config.name
                                    );
                                    continue;
                                }
                                self.status(&format!("  ✔️ {}", tool.name));
                                self.tool_to_server
                                    .insert(tool.name.clone(), config.name.clone());
                                self.all_tools.push(tool.clone());
                            }
                            self.clients.insert(config.name.clone(), client);
                            self.server_configs
                                .insert(config.name.clone(), config.clone());
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

    /// Production gateway initialization. Unlike the legacy developer helper
    /// above, this path has no native-process fallback: every configured MCP
    /// server must have a verified manifest and a digest-pinned OCI image.
    pub async fn initialize_production(&mut self, config: &Config, workspace: &Path) -> Result<()> {
        if !crate::sandbox::docker_available() {
            return Err(anyhow!(
                "Docker is required for production MCP plugins but is unavailable"
            ));
        }
        for configured in &config.mcp_servers {
            if !configured.enabled {
                continue;
            }
            let mut server = configured.clone();
            let manifest = crate::plugin_manifest::verify_production_server(&server, workspace)?;
            if !image_available(&server.image) {
                return Err(anyhow!(
                    "contained plugin image for '{}' is not available locally: {}. Pull and review it before starting the gateway",
                    server.name,
                    server.image
                ));
            }
            crate::plugin_manifest::apply_manifest_to_server(&mut server, &manifest)?;
            let mut client = McpClient::spawn_container(&server, workspace).map_err(|error| {
                anyhow!(
                    "failed to start contained plugin '{}': {}",
                    server.name,
                    error
                )
            })?;
            client.initialize().await.map_err(|error| {
                anyhow!(
                    "plugin '{}' failed MCP initialization: {}",
                    server.name,
                    error
                )
            })?;
            let tools = client.list_tools().await.map_err(|error| {
                anyhow!("plugin '{}' failed tools/list: {}", server.name, error)
            })?;
            self.status(&format!(
                "[MCP] Contained server '{}' registered {} tools:",
                server.name,
                tools.len()
            ));
            for tool in tools {
                if self.tool_to_server.contains_key(&tool.name) {
                    return Err(anyhow!(
                        "tool '{}' from '{}' conflicts with an already configured plugin",
                        tool.name,
                        server.name
                    ));
                }
                self.tool_to_server
                    .insert(tool.name.clone(), server.name.clone());
                self.all_tools.push(tool);
            }
            self.clients.insert(server.name.clone(), client);
            self.server_configs.insert(server.name.clone(), server);
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

        if let Some(config) = self.server_configs.get(&server_name) {
            // 1. Enforce deny_tools instantly
            if config.deny_tools.contains(&tool_name.to_string())
                || config.deny_tools.contains(&"*".to_string())
            {
                return Err(anyhow!(
                    "Policy Violation: Tool '{}' on server '{}' is explicitly blocked by deny_tools filter.",
                    tool_name,
                    server_name
                ));
            }

            // 2. Enforce allow_tools instantly
            if !config.allow_tools.is_empty()
                && !config.allow_tools.contains(&tool_name.to_string())
                && !config.allow_tools.contains(&"*".to_string())
            {
                return Err(anyhow!(
                    "Policy Violation: Tool '{}' on server '{}' is not present in the allow_tools whitelist.",
                    tool_name,
                    server_name
                ));
            }

            // 3. Enforce capabilities (if defined)
            if !config.capabilities.is_empty()
                && !config.capabilities.contains(&tool_name.to_string())
                && !config.capabilities.contains(&"*".to_string())
            {
                return Err(anyhow!(
                    "Server '{}' does not have capability to run tool '{}'",
                    server_name,
                    tool_name
                ));
            }
        }

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

    pub fn get_server_for_tool(&self, tool_name: &str) -> Option<String> {
        self.tool_to_server.get(tool_name).cloned()
    }

    /// Whether a discovered tool survives server-level allow/deny/capability
    /// filters and is therefore safe to advertise to an MCP client.
    pub fn tool_is_callable(&self, tool_name: &str) -> bool {
        let Some(server_name) = self.tool_to_server.get(tool_name) else {
            return false;
        };
        let Some(config) = self.server_configs.get(server_name) else {
            return false;
        };
        !(config.deny_tools.contains(&tool_name.to_string())
            || config.deny_tools.contains(&"*".to_string()))
            && (config.allow_tools.is_empty()
                || config.allow_tools.contains(&tool_name.to_string())
                || config.allow_tools.contains(&"*".to_string()))
            && (config.capabilities.is_empty()
                || config.capabilities.contains(&tool_name.to_string())
                || config.capabilities.contains(&"*".to_string()))
    }

    /// Get all tool names.
    #[allow(dead_code)]
    pub fn tool_names(&self) -> Vec<String> {
        self.tool_to_server.keys().cloned().collect()
    }
}

pub fn image_available(image: &str) -> bool {
    std::process::Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
