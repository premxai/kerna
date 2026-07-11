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

    #[serde(default)]
    pub allow_tools: Vec<String>,

    #[serde(default)]
    pub deny_tools: Vec<String>,

    /// Names of environment variables this plugin needs (e.g. API tokens). Only
    /// the NAMES live in config; values are read from the environment at spawn
    /// and injected into the plugin's process — never written to disk.
    #[serde(default)]
    pub secrets: Vec<String>,

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

/// Configuration for an LLM provider (BYOK)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String, // "openai", "anthropic", "openai_compatible"
    pub api_key_env: Option<String>,
    pub default_model: String,
    pub base_url: Option<String>,
}

/// Workspace configuration for bounding checkpoints and execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_root")]
    pub root: String,
    #[serde(default = "default_true")]
    pub checkpoint_enabled: bool,
    #[serde(default = "default_checkpoint_max_bytes")]
    pub checkpoint_max_bytes: u64,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            root: default_workspace_root(),
            checkpoint_enabled: default_true(),
            checkpoint_max_bytes: default_checkpoint_max_bytes(),
        }
    }
}

/// A named budget preset to quickly switch limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPreset {
    pub max_tool_calls: u64,
    pub max_llm_calls: u64,
    pub max_runtime_seconds: u64,
    pub max_output_bytes: u64,
    pub max_memory_writes: u64,
    pub max_cost_usd: f64,
}

/// Root application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub llm_provider: String,

    #[serde(skip_serializing, default)]
    pub llm_api_key: String,

    pub llm_model: String,

    #[serde(default)]
    pub providers: std::collections::HashMap<String, ProviderConfig>,

    #[serde(default)]
    pub model_routes: std::collections::HashMap<String, String>,

    #[serde(default)]
    pub privacy_routes: std::collections::HashMap<String, String>,

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

    #[serde(default)]
    pub presets: std::collections::HashMap<String, BudgetPreset>,

    #[serde(default)]
    pub workspace: WorkspaceConfig,

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

    #[serde(skip_serializing, default)]
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

fn default_workspace_root() -> String {
    ".".to_string()
}

fn default_checkpoint_max_bytes() -> u64 {
    100_000_000
}

fn default_network_mode() -> String {
    "none".to_string()
}

fn default_max_runtime_seconds() -> u64 {
    300
}
fn default_max_tool_calls() -> u64 {
    25
}
fn default_max_llm_calls() -> u64 {
    10
}
fn default_max_cost_usd() -> f64 {
    0.25
}
fn default_max_output_bytes() -> u64 {
    50000
}
fn default_max_memory_writes() -> u64 {
    20
}

impl Config {
    pub fn load() -> Self {
        let mut config = Self::default();

        // 1. Try to load from kerna.toml if it exists
        let toml_path = "kerna.toml";
        if Path::new(toml_path).exists() {
            match fs::read_to_string(toml_path) {
                Ok(content) => match toml::from_str::<Config>(&content) {
                    Ok(c) => {
                        config = c;
                    }
                    Err(e) => {
                        eprintln!("[-] Fatal: Failed to parse kerna.toml: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("[-] Fatal: Failed to read kerna.toml: {}", e);
                    std::process::exit(1);
                }
            }
        }

        // 2. Fall back to environment variables for critical missing fields
        if config.llm_provider.is_empty() {
            config.llm_provider =
                env::var("KERNA_LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string());
        }

        if config.llm_api_key.is_empty() {
            config.llm_api_key = env::var("KERNA_LLM_API_KEY")
                .or_else(|_| env::var("OPENAI_API_KEY"))
                .or_else(|_| env::var("ANTHROPIC_API_KEY"))
                .unwrap_or_default();
        }

        if config.llm_model.is_empty() {
            config.llm_model = env::var("KERNA_LLM_MODEL").unwrap_or_else(|_| {
                match config.llm_provider.as_str() {
                    "anthropic" => "claude-sonnet-4-20250514".to_string(),
                    _ => "gpt-4o-mini".to_string(),
                }
            });
        }

        if config.db_path.is_empty() {
            config.db_path = env::var("KERNA_DB_PATH").unwrap_or_else(|_| "kerna.db".to_string());
        }

        if config.sandbox_dir.is_empty() {
            config.sandbox_dir =
                env::var("KERNA_SANDBOX_DIR").unwrap_or_else(|_| "sandbox".to_string());
        }

        config
    }

    pub fn save(&self) {
        if let Ok(toml) = toml::to_string(self) {
            let _ = fs::write("kerna.toml", toml);
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

    #[test]
    fn test_provider_config_parses_correctly() {
        let toml_str = r#"
        llm_provider = "openai"
        llm_api_key = "123"
        llm_model = "gpt-4"
        db_path = ""
        sandbox_dir = ""
        memory_backend = ""
        
        [providers.local]
        type = "openai_compatible"
        api_key_env = "LOCAL_API_KEY"
        default_model = "qwen"
        base_url = "http://localhost"
        
        [model_routes]
        cheap = "local/qwen"
        
        [privacy_routes]
        local_only = "cheap"
        "#;

        let result: Result<Config, _> = toml::from_str(toml_str);
        if let Err(e) = &result {
            println!("Parse error: {}", e);
        }
        assert!(result.is_ok());
        let conf = result.unwrap();
        assert_eq!(conf.providers.len(), 1);
        assert_eq!(conf.providers["local"].provider_type, "openai_compatible");
        assert_eq!(conf.model_routes["cheap"], "local/qwen");
        assert_eq!(conf.privacy_routes["local_only"], "cheap");
    }

    #[test]
    fn test_raw_api_keys_are_never_written_to_kerna_toml() {
        let mut conf = Config::default();
        conf.llm_api_key = "secret_sk_123456789".to_string();
        conf.llm_fallback_api_key = Some("fallback_sk_987".to_string());

        let toml_str = toml::to_string(&conf).unwrap();

        // Assert the raw string keys do NOT appear anywhere in the output
        assert!(!toml_str.contains("secret_sk"));
        assert!(!toml_str.contains("fallback_sk"));
        assert!(!toml_str.contains("llm_api_key"));
        assert!(!toml_str.contains("llm_fallback_api_key"));
    }
}
