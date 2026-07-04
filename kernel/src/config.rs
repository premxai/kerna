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
    
    pub enabled: bool,
    
    pub capabilities: Vec<String>,
    
    pub allowed_paths: Vec<String>,
    
    pub approval_required: Vec<String>,
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

impl Config {
    pub fn load() -> Self {
        // 1. Try to load from kerna.toml if it exists
        let toml_path = "kerna.toml";
        if Path::new(toml_path).exists() {
            if let Ok(content) = fs::read_to_string(toml_path) {
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    return config;
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
        }
    }

    /// Check if a tool action is allowed, requires confirmation, or is denied.
    pub fn check_permission(&self, tool_name: &str) -> &str {
        for rule in &self.permissions {
            if rule.tool == tool_name || rule.tool == "*" {
                return &rule.action;
            }
        }
        "deny" // Default: fail-closed security
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_fails_without_capabilities() {
        let toml_str = r#"
        name = "test"
        command = "echo"
        enabled = true
        "#;
        
        let result: Result<McpServerConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "Config should fail parsing if capabilities are missing to enforce fail-closed security");
    }

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
