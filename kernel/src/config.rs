use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;

/// Configuration for a single MCP server that Kerna can spawn and route tool calls to.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    
    #[serde(default)]
    pub args: Vec<String>,
    
    #[serde(default = "default_true")]
    pub enabled: bool,
    
    #[serde(default)]
    pub capabilities: Vec<String>,
    
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    
    #[serde(default)]
    pub approval_required: Vec<String>,
    
    #[serde(default = "default_runtime_mode")]
    pub runtime_mode: String,
    
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
}

/// Configuration for a scheduled recurring goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub cron: String,
    pub goal: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Permission policy for a specific tool action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub action: String, // "auto_approve" | "require_confirmation" | "deny"
}

/// Root application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub llm_provider: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub db_path: String,
    pub sandbox_dir: String,
    pub memory_backend: String, // "sqlite" | "mem0" | "chroma" | "qdrant"
    #[serde(default)]
    pub allowed_directories: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub schedules: Vec<ScheduleConfig>,
    #[serde(default)]
    pub permissions: Vec<PermissionRule>,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    
    #[serde(default = "default_runtime_mode")]
    pub runtime_mode: String,
    
    #[serde(default = "default_false")]
    pub allow_dynamic_installs: bool,
    
    // Budget Envelope (Phase 1)
    #[serde(default = "default_max_runtime_seconds")]
    pub max_runtime_seconds: u64,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u64,
    #[serde(default = "default_max_llm_calls")]
    pub max_llm_calls: u64,
    #[serde(default = "default_max_cost_usd")]
    pub max_cost_usd: f64,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: u64,
    #[serde(default = "default_max_memory_writes")]
    pub max_memory_writes: u64,
    
    #[serde(default = "default_network_mode")]
    pub network_mode: String,
    
    #[serde(default)]
    pub egress_proxy: Option<String>,
    
    #[serde(default = "default_false")]
    pub enable_supervisor: bool,
    
    // v1.1 Hermes Parity Features
    #[serde(default)]
    pub converse: bool,
    
    #[serde(default)]
    pub llm_fallback_provider: Option<String>,
    
    #[serde(default)]
    pub llm_fallback_api_key: Option<String>,
    
    #[serde(default)]
    pub credential_pool: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_max_retries() -> u32 {
    3
}

fn default_max_tool_rounds() -> u32 {
    15
}

fn default_runtime_mode() -> String {
    "native".to_string()
}

fn default_docker_image() -> String {
    "ubuntu:latest".to_string()
}

fn default_false() -> bool {
    false
}

fn default_network_mode() -> String {
    "none".to_string()
}

fn default_max_runtime_seconds() -> u64 { 300 }
fn default_max_tool_calls() -> u64 { 25 }
fn default_max_llm_calls() -> u64 { 10 }
fn default_max_cost_usd() -> f64 { 0.25 }
fn default_max_output_bytes() -> u64 { 50000 }
fn default_max_memory_writes() -> u64 { 20 }

impl Config {
    pub fn load() -> Self {
        // 1. Try to load from kerna.toml if it exists
        let toml_path = "kerna.toml";
        if Path::new(toml_path).exists() {
            match fs::read_to_string(toml_path) {
                Ok(content) => {
                    match toml::from_str::<Config>(&content) {
                        Ok(config) => return config,
                        Err(e) => {
                            eprintln!("[-] Fatal: Failed to parse kerna.toml: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[-] Fatal: Failed to read kerna.toml: {}", e);
                    std::process::exit(1);
                }
            }
        }

        // 2. Fall back to environment variables with sensible defaults
        let llm_provider =
            env::var("KERNA_LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string());

        let llm_api_key = env::var("KERNA_LLM_API_KEY")
            .or_else(|_| env::var("OPENAI_API_KEY"))
            .or_else(|_| env::var("ANTHROPIC_API_KEY"))
            .unwrap_or_default();

        let llm_model =
            env::var("KERNA_LLM_MODEL").unwrap_or_else(|_| match llm_provider.as_str() {
                "anthropic" => "claude-sonnet-4-20250514".to_string(),
                _ => "gpt-4o-mini".to_string(),
            });

        let db_path = env::var("KERNA_DB_PATH").unwrap_or_else(|_| "kerna.db".to_string());
        let sandbox_dir =
            env::var("KERNA_SANDBOX_DIR").unwrap_or_else(|_| "sandbox".to_string());

        Config {
            llm_provider,
            llm_api_key,
            llm_model,
            db_path,
            sandbox_dir,
            memory_backend: "sqlite".to_string(),
            allowed_directories: vec![],
            mcp_servers: vec![],
            schedules: vec![],
            permissions: vec![],
            max_retries: 3,
            max_tool_rounds: 15,
            runtime_mode: "native".to_string(),
            allow_dynamic_installs: false,
            max_runtime_seconds: 300,
            max_tool_calls: 25,
            max_llm_calls: 10,
            max_cost_usd: 0.25,
            max_output_bytes: 50000,
            max_memory_writes: 20,
            network_mode: "none".to_string(),
            egress_proxy: None,
            enable_supervisor: false,
            converse: false,
            credential_pool: vec![],
            llm_fallback_provider: None,
            llm_fallback_api_key: None,
        }
    }

    /// Check if a tool action is allowed, requires confirmation, or is denied.
    pub fn check_permission(&self, tool_name: &str) -> &str {
        let mut wildcard_action = None;
        for rule in &self.permissions {
            if rule.tool == tool_name {
                return &rule.action;
            } else if rule.tool == "*" {
                wildcard_action = Some(&rule.action);
            }
        }
        if let Some(action) = wildcard_action {
            return action;
        }
        "deny" // Default: fail-closed security
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_parses_with_boundaries() {
        let toml_str = r#"
        name = "test"
        command = "echo"
        enabled = true
        capabilities = ["read"]
        allowed_paths = ["/tmp"]
        approval_required = ["write"]
        "#;
        
        let result: Result<McpServerConfig, _> = toml::from_str(toml_str);
        assert!(result.is_ok());
        let conf = result.unwrap();
        assert_eq!(conf.capabilities.len(), 1);
        assert_eq!(conf.allowed_paths[0], "/tmp");
    }
}
