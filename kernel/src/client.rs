//! Client configuration snippets for Kerna's governed MCP gateway.
//!
//! This module prints configuration rather than editing an IDE's files. A
//! gateway command inherits the trust of the active project, so silently
//! adding it to a user's global client config would be the wrong boundary.

use anyhow::{bail, Context, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientConfigFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone, Copy)]
pub struct ClientAdapter {
    pub id: &'static str,
    pub format: ClientConfigFormat,
}

const ADAPTERS: &[ClientAdapter] = &[
    ClientAdapter {
        id: "codex",
        format: ClientConfigFormat::Toml,
    },
    ClientAdapter {
        id: "claude-code",
        format: ClientConfigFormat::Json,
    },
    ClientAdapter {
        id: "qoder",
        format: ClientConfigFormat::Json,
    },
    ClientAdapter {
        id: "generic",
        format: ClientConfigFormat::Json,
    },
];

pub fn adapter(client: &str) -> Result<ClientAdapter> {
    ADAPTERS
        .iter()
        .copied()
        .find(|candidate| candidate.id == client)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown client '{client}'. Available clients: {}.",
                ADAPTERS
                    .iter()
                    .map(|candidate| candidate.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

pub fn config(client: &str, workspace: &Path) -> Result<String> {
    let workspace = workspace.canonicalize().with_context(|| {
        format!(
            "Workspace '{}' does not exist. Create a contract there first.",
            workspace.display()
        )
    })?;
    if !workspace.join("kerna.toml").is_file() {
        bail!(
            "'{}' does not contain kerna.toml. Run `kerna contract init` first.",
            workspace.display()
        );
    }

    // `canonicalize` returns the Windows extended-length `\\?\` spelling on
    // some Rust builds. It is useful to Win32 but surprising to users and not
    // accepted by every client configuration parser.
    let mut cwd = workspace.to_string_lossy().into_owned();
    if let Some(normal) = cwd.strip_prefix(r"\\?\") {
        cwd = normal.to_string();
    }
    match adapter(client)?.format {
        // Every client receives the identical direct gateway process. Passing
        // the workspace explicitly avoids client-specific cwd semantics and
        // never requires a cmd.exe/sh wrapper.
        ClientConfigFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "kerna-governed-tools": {
                    "command": "kerna",
                    "args": ["gateway", "--workspace", cwd],
                }
            }
        }))
        .map_err(Into::into),
        ClientConfigFormat::Toml => Ok(format!(
            "[mcp_servers.kerna-governed-tools]\ncommand = {}\nargs = {}\n",
            toml::Value::String("kerna".to_string()),
            toml::Value::Array(
                ["gateway".to_string(), "--workspace".to_string(), cwd]
                    .into_iter()
                    .map(toml::Value::String)
                    .collect()
            )
        )),
    }
}

/// Start the exact generated gateway command and prove the client-independent
/// MCP lifecycle reaches `initialize` and `tools/list`. This never edits any
/// IDE configuration; it is a local readiness check for all adapters.
pub async fn doctor(client: &str, workspace: &Path) -> Result<Vec<String>> {
    let _rendered = config(client, workspace)?;
    let workspace = workspace.canonicalize()?;
    let executable = std::env::var("KERNA_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    let command = executable.to_string_lossy().to_string();
    let workspace_arg = workspace.to_string_lossy().to_string();
    let args = ["gateway", "--workspace", workspace_arg.as_str()];
    let mut gateway =
        crate::mcp::McpClient::spawn(&command, &args, "native", "", "none", None, &[])?;
    gateway.initialize().await?;
    let tools = gateway.list_tools().await?;
    gateway.close().await?;
    Ok(vec![
        format!("Adapter '{}': configuration format is valid.", client),
        "Gateway MCP initialize handshake completed.".to_string(),
        format!("Gateway exposed {} governed tool(s).", tools.len()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("kerna-client-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("kerna.toml"), "llm_provider = \"mock\"\n").unwrap();
        path
    }

    #[test]
    fn qoder_config_launches_the_direct_workspace_gateway() {
        let workspace = workspace();
        let rendered = config("qoder", &workspace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let server = &parsed["mcpServers"]["kerna-governed-tools"];
        assert_eq!(server["command"], "kerna");
        assert_eq!(server["args"][0], "gateway");
        assert_eq!(server["args"][1], "--workspace");
        assert!(server["args"][2]
            .as_str()
            .unwrap()
            .contains("kerna-client-"));
        assert!(server.get("cwd").is_none());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn refuses_a_workspace_without_a_contract() {
        let workspace = std::env::temp_dir().join("kerna-client-missing-contract");
        fs::create_dir_all(&workspace).unwrap();
        assert!(config("qoder", &workspace)
            .unwrap_err()
            .to_string()
            .contains("kerna.toml"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn claude_and_codex_configs_pin_the_contract_working_directory() {
        let workspace = workspace();
        let claude = config("claude-code", &workspace).unwrap();
        let codex = config("codex", &workspace).unwrap();
        assert!(claude.contains("--workspace"));
        assert!(codex.contains("--workspace"));
        assert!(claude.contains("kerna-client-"));
        assert!(codex.contains("kerna-client-"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn generic_config_uses_a_direct_gateway_command() {
        let workspace = workspace();
        let rendered = config("generic", &workspace).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let server = &parsed["mcpServers"]["kerna-governed-tools"];
        assert_eq!(server["command"], "kerna");
        assert_eq!(server["args"][0], "gateway");
        assert_eq!(server["args"][1], "--workspace");
        let _ = fs::remove_dir_all(workspace);
    }
}
