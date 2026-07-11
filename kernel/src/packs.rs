//! Curated tool packs. A pack (`plugins/packs/<name>.toml`) is a list of MCP
//! plugins to register in one command, with declared secrets and suggested
//! (fail-closed) permissions. `kerna pack install <name>` wires them up; nothing
//! is auto-approved — read tools become `require_confirmation`, and the user is
//! told which secrets to set.

use crate::config::{Config, McpServerConfig, PermissionRule};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Pack {
    pub pack: PackMeta,
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<PackPlugin>,
}

#[derive(Debug, Deserialize)]
pub struct PackMeta {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct PackPlugin {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Tools to suggest as `require_confirmation` on install (still opt-in-able).
    #[serde(default)]
    pub suggest_confirm: Vec<String>,
}

/// Resolve Kerna's plugins directory: `$KERNA_PLUGINS_DIR`, else the `plugins`
/// dir next to the repo the binary was built in, else `./plugins`.
pub fn plugins_dir() -> PathBuf {
    if let Ok(d) = std::env::var("KERNA_PLUGINS_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        // <repo>/target/<profile>/kerna(.exe) → <repo>/plugins
        if let Some(repo) = exe.ancestors().nth(3) {
            let cand = repo.join("plugins");
            if cand.exists() {
                return cand;
            }
        }
    }
    PathBuf::from("plugins")
}

fn packs_dir() -> PathBuf {
    plugins_dir().join("packs")
}

/// (name, description) for every pack found on disk.
pub fn list_packs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(packs_dir()) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().map(|x| x == "toml").unwrap_or(false) {
                if let Ok(pack) = load_pack_file(&path) {
                    out.push((pack.pack.name, pack.pack.description));
                }
            }
        }
    }
    out.sort();
    out
}

fn load_pack_file(path: &std::path::Path) -> Result<Pack> {
    let content = std::fs::read_to_string(path)?;
    let pack: Pack = toml::from_str(&content)?;
    Ok(pack)
}

pub fn load_pack(name: &str) -> Result<Pack> {
    let path = packs_dir().join(format!("{}.toml", name));
    if !path.exists() {
        return Err(anyhow!(
            "pack '{}' not found in {}. Try `kerna pack list`.",
            name,
            packs_dir().display()
        ));
    }
    load_pack_file(&path)
}

/// Expand `{plugins}` in a plugin arg to the resolved plugins dir.
fn expand(arg: &str, plugins: &str) -> String {
    arg.replace("{plugins}", plugins)
}

pub struct InstallReport {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
    pub secrets_needed: Vec<(String, String)>, // (plugin, env_var)
}

/// Register a pack's plugins into `config` (mutates it). Fail-closed: nothing is
/// auto-approved; suggested tools become `require_confirmation`. Caller saves.
pub fn install(config: &mut Config, pack: &Pack) -> InstallReport {
    let plugins = plugins_dir().to_string_lossy().to_string();
    let mut report = InstallReport {
        added: Vec::new(),
        skipped: Vec::new(),
        secrets_needed: Vec::new(),
    };

    for p in &pack.plugins {
        if config.mcp_servers.iter().any(|s| s.name == p.name) {
            report.skipped.push(p.name.clone());
        } else {
            config.mcp_servers.push(McpServerConfig {
                name: p.name.clone(),
                command: p.command.clone(),
                args: p.args.iter().map(|a| expand(a, &plugins)).collect(),
                enabled: true,
                capabilities: vec![],
                allowed_paths: vec![],
                approval_required: vec![],
                allow_tools: vec![],
                deny_tools: vec![],
                secrets: p.secrets.clone(),
                runtime_mode: "native".to_string(),
                docker_image: "ubuntu:latest".to_string(),
            });
            report.added.push(p.name.clone());
        }

        // Suggested (fail-closed) permissions: require_confirmation, only if the
        // tool has no rule yet — never downgrade an existing stricter rule.
        for tool in &p.suggest_confirm {
            if !config.permissions.iter().any(|r| r.tool == *tool) {
                config.permissions.push(PermissionRule {
                    tool: tool.clone(),
                    action: "require_confirmation".to_string(),
                });
            }
        }

        for s in &p.secrets {
            report.secrets_needed.push((p.name.clone(), s.clone()));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pack() -> Pack {
        Pack {
            pack: PackMeta {
                name: "test".into(),
                description: "test pack".into(),
            },
            plugins: vec![PackPlugin {
                name: "search".into(),
                command: "python".into(),
                args: vec!["{plugins}/search_mcp/mcp_server.py".into()],
                secrets: vec!["TAVILY_API_KEY".into()],
                suggest_confirm: vec!["web_search".into()],
            }],
        }
    }

    #[test]
    fn install_registers_server_secret_and_failclosed_permission() {
        let mut config = Config::default();
        let report = install(&mut config, &sample_pack());

        assert_eq!(report.added, vec!["search".to_string()]);
        let server = config
            .mcp_servers
            .iter()
            .find(|s| s.name == "search")
            .expect("server registered");
        assert_eq!(server.secrets, vec!["TAVILY_API_KEY".to_string()]);
        // {plugins} was expanded (no literal token remains).
        assert!(!server.args[0].contains("{plugins}"));
        // Suggested tool is require_confirmation — never auto_approve.
        let rule = config
            .permissions
            .iter()
            .find(|r| r.tool == "web_search")
            .expect("permission added");
        assert_eq!(rule.action, "require_confirmation");
    }

    #[test]
    fn install_is_idempotent_and_skips_existing() {
        let mut config = Config::default();
        install(&mut config, &sample_pack());
        let report = install(&mut config, &sample_pack());
        assert_eq!(report.skipped, vec!["search".to_string()]);
        assert_eq!(
            config
                .mcp_servers
                .iter()
                .filter(|s| s.name == "search")
                .count(),
            1
        );
    }
}
