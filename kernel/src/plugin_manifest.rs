use crate::config::{Config, McpServerConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub kind: String, // "tool.mcp", "tool.native", etc.
    pub entrypoint: String,

    #[serde(default = "default_source")]
    pub source: String,

    #[serde(default = "default_trust")]
    pub trust: String, // "untrusted", "verified", "core"

    #[serde(default)]
    pub capabilities: Vec<String>,

    #[serde(default)]
    pub requires_approval: Vec<String>,

    #[serde(default)]
    pub secrets: Vec<String>,

    #[serde(default)]
    pub allowed_paths: Vec<String>,

    #[serde(default)]
    pub network_allowlist: Vec<String>,

    #[serde(default)]
    pub declared_outputs: Vec<String>,

    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: u64,

    pub manifest_sha256: Option<String>,
    pub signature: Option<String>,
}

fn default_source() -> String {
    "local".to_string()
}
fn default_trust() -> String {
    "untrusted".to_string()
}
fn default_max_output_bytes() -> u64 {
    50000
}

impl PluginManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut manifest: PluginManifest = toml::from_str(&content)?;

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        manifest.plugin.manifest_sha256 = Some(format!("{:x}", hasher.finalize()));

        Ok(manifest)
    }

    pub fn print_risk_card(&self) {
        let p = &self.plugin;

        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║  PLUGIN RISK CARD: {:<40}  ║", p.name);
        println!("╠══════════════════════════════════════════════════════════════╣");

        let trust_icon = match p.trust.as_str() {
            "core" | "verified" => "✔️",
            _ => "⚠️",
        };
        println!("║  Trust: {} {:<47} ║", trust_icon, p.trust);

        println!("║                                                              ║");
        println!("║  Capabilities:                                               ║");
        for cap in &p.capabilities {
            if p.requires_approval.contains(cap) {
                println!("║    ⚠️ {:<52} ║", cap);
            } else {
                println!("║    ✔️ {:<52} ║", cap);
            }
        }

        if !p.secrets.is_empty() {
            println!("║                                                              ║");
            println!("║  Secrets Requested:                                          ║");
            for secret in &p.secrets {
                println!("║    🔒 {:<52} ║", secret);
            }
        }

        if !p.network_allowlist.is_empty() {
            println!("║                                                              ║");
            println!("║  Network Access:                                             ║");
            for net in &p.network_allowlist {
                println!("║    🌐 {:<52} ║", net);
            }
        }

        if !p.requires_approval.is_empty() {
            println!("║                                                              ║");
            println!("║  Approval Required For:                                      ║");
            for req in &p.requires_approval {
                println!("║    ✋ {:<52} ║", req);
            }
        }

        println!("║                                                              ║");
        let risk_level = if p.trust == "untrusted"
            && (!p.secrets.is_empty() || !p.network_allowlist.is_empty())
        {
            "High"
        } else if p.trust == "untrusted" {
            "Medium"
        } else {
            "Low"
        };
        println!("║  Overall Risk: {:<45} ║", risk_level);
        println!("╚══════════════════════════════════════════════════════════════╝\n");
    }
}

/// Locate a manifest for a configured MCP server. Prefer a manifest adjacent to
/// the configured entrypoint, then fall back to Kerna's shipped plugin layout.
/// This keeps manifests working for both local paths and installed packs.
pub fn find_for_server(server: &McpServerConfig) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    for arg in &server.args {
        let path = PathBuf::from(arg);
        if path.is_file() {
            if path.file_name().and_then(|name| name.to_str()) == Some("manifest.toml") {
                candidates.push(path);
            } else if let Some(parent) = path.parent() {
                candidates.push(parent.join("manifest.toml"));
            }
        } else if path.is_dir() {
            candidates.push(path.join("manifest.toml"));
        }
    }

    let command_path = PathBuf::from(&server.command);
    if command_path.is_file() {
        if let Some(parent) = command_path.parent() {
            candidates.push(parent.join("manifest.toml"));
        }
    }

    let plugins_dir = crate::packs::plugins_dir();
    candidates.push(plugins_dir.join(&server.name).join("manifest.toml"));
    candidates.push(
        plugins_dir
            .join(format!("{}_mcp", server.name))
            .join("manifest.toml"),
    );

    candidates.into_iter().find(|path| path.is_file())
}

/// Load the manifest that belongs to a configured server, if it has one.
/// A manifest that is found but malformed is an error: silently treating a
/// malformed declaration as legacy would weaken the security boundary.
pub fn load_for_server(server: &McpServerConfig) -> Result<Option<(PathBuf, PluginManifest)>> {
    match find_for_server(server) {
        Some(path) => Ok(Some((path.clone(), PluginManifest::load(&path)?))),
        None => Ok(None),
    }
}

/// Apply the manifest's declarations to a configured server as additional
/// restrictions. Configuration can only narrow a manifest declaration; it can
/// never expand the tools or secrets a manifest permits.
pub fn apply_to_server(server: &mut McpServerConfig) -> Result<Option<PathBuf>> {
    let Some((path, manifest)) = load_for_server(server)? else {
        return Ok(None);
    };

    let declared_tools = &manifest.plugin.capabilities;
    server.capabilities = intersect_or_use_declared(&server.capabilities, declared_tools);
    server.allow_tools = intersect_or_use_declared(&server.allow_tools, declared_tools);

    // A manifest with no tool capabilities is a valid declaration for a
    // resource-only server. It must not accidentally grant every discovered
    // tool through empty-list semantics.
    if declared_tools.is_empty() && !server.deny_tools.iter().any(|tool| tool == "*") {
        server.deny_tools.push("*".to_string());
    }

    append_unique(
        &mut server.approval_required,
        &manifest.plugin.requires_approval,
    );

    // Passing a secret needs two independent declarations: the user-configured
    // server entry and the plugin manifest. This prevents a plugin update from
    // receiving an unrelated environment value merely because its name appears
    // in config.
    server
        .secrets
        .retain(|secret| manifest.plugin.secrets.contains(secret));

    Ok(Some(path))
}

/// Apply every configured plugin's manifest restrictions to the in-memory
/// config. The saved config remains the user's explicit intent; this derives
/// an effective runtime policy for the current process.
pub fn apply_to_config(config: &mut Config) -> Result<Vec<(String, PathBuf)>> {
    let mut applied = Vec::new();
    for server in &mut config.mcp_servers {
        if let Some(path) = apply_to_server(server)? {
            applied.push((server.name.clone(), path));
        }
    }
    Ok(applied)
}

fn intersect_or_use_declared(configured: &[String], declared: &[String]) -> Vec<String> {
    if configured.iter().any(|tool| tool == "*") {
        return declared.to_vec();
    }
    if configured.is_empty() {
        return declared.to_vec();
    }
    configured
        .iter()
        .filter(|tool| declared.contains(*tool))
        .cloned()
        .collect()
}

fn append_unique(target: &mut Vec<String>, additions: &[String]) {
    for value in additions {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn server(entrypoint: PathBuf) -> McpServerConfig {
        McpServerConfig {
            name: "test-plugin".to_string(),
            command: "python".to_string(),
            args: vec![entrypoint.to_string_lossy().to_string()],
            enabled: true,
            capabilities: vec!["read".to_string(), "outside".to_string()],
            allowed_paths: vec![],
            approval_required: vec!["configured_approval".to_string()],
            allow_tools: vec!["read".to_string(), "outside".to_string()],
            deny_tools: vec![],
            secrets: vec![
                "DECLARED_SECRET".to_string(),
                "UNDECLARED_SECRET".to_string(),
            ],
            runtime_mode: "native".to_string(),
            docker_image: "ubuntu:latest".to_string(),
        }
    }

    #[test]
    fn manifest_adjacent_to_entrypoint_is_found_and_enforced() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kerna_manifest_test_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let entrypoint = dir.join("mcp_server.py");
        fs::write(&entrypoint, "# test entrypoint").unwrap();
        fs::write(
            dir.join("manifest.toml"),
            r#"[plugin]
name = "test-plugin"
version = "1.0.0"
kind = "tool.mcp"
entrypoint = "mcp_server.py"
capabilities = ["read", "write"]
requires_approval = ["write"]
secrets = ["DECLARED_SECRET"]
"#,
        )
        .unwrap();

        let mut configured = server(entrypoint);
        let path = apply_to_server(&mut configured).unwrap().unwrap();

        assert_eq!(path, dir.join("manifest.toml"));
        assert_eq!(configured.capabilities, vec!["read"]);
        assert_eq!(configured.allow_tools, vec!["read"]);
        assert!(configured
            .approval_required
            .contains(&"configured_approval".to_string()));
        assert!(configured.approval_required.contains(&"write".to_string()));
        assert_eq!(configured.secrets, vec!["DECLARED_SECRET"]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_manifest_capabilities_blocks_all_tools() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kerna_manifest_empty_test_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let entrypoint = dir.join("mcp_server.py");
        fs::write(&entrypoint, "# test entrypoint").unwrap();
        fs::write(
            dir.join("manifest.toml"),
            r#"[plugin]
name = "test-plugin"
version = "1.0.0"
kind = "tool.mcp"
entrypoint = "mcp_server.py"
"#,
        )
        .unwrap();

        let mut configured = server(entrypoint);
        apply_to_server(&mut configured).unwrap();

        assert!(configured.capabilities.is_empty());
        assert!(configured.allow_tools.is_empty());
        assert!(configured.deny_tools.contains(&"*".to_string()));

        let _ = fs::remove_dir_all(dir);
    }
}
