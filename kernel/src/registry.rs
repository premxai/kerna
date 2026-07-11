//! Plugin registry. A JSON index (`plugins/registry.json`) of installable MCP
//! plugins — Kerna's own stdlib plugins plus wrapped official servers. Powers
//! `kerna plugins search/install <name>`. Reuses the pack install logic so a
//! single plugin is registered fail-closed (suggested tools → require_confirmation).

use crate::config::Config;
use crate::packs::{self, InstallReport, Pack, PackMeta, PackPlugin};
use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub plugins: Vec<RegistryPlugin>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryPlugin {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub suggest_confirm: Vec<String>,
}

fn registry_path() -> std::path::PathBuf {
    packs::plugins_dir().join("registry.json")
}

pub fn load() -> Result<Registry> {
    let path = registry_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow!("could not read registry {}: {}", path.display(), e))?;
    let reg: Registry = serde_json::from_str(&content)?;
    Ok(reg)
}

/// Plugins whose name/description/tags contain `query` (empty query = all).
pub fn search(reg: &Registry, query: &str) -> Vec<RegistryPlugin> {
    let q = query.to_lowercase();
    reg.plugins
        .iter()
        .filter(|p| {
            q.is_empty()
                || p.name.to_lowercase().contains(&q)
                || p.description.to_lowercase().contains(&q)
                || p.tags.iter().any(|t| t.to_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

pub fn find<'a>(reg: &'a Registry, name: &str) -> Option<&'a RegistryPlugin> {
    reg.plugins.iter().find(|p| p.name == name)
}

/// Install a single registry plugin into `config` (fail-closed). Caller saves.
pub fn install(config: &mut Config, plugin: &RegistryPlugin) -> InstallReport {
    // Reuse the pack installer by wrapping the single plugin in a one-item pack.
    let pack = Pack {
        pack: PackMeta {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
        },
        plugins: vec![PackPlugin {
            name: plugin.name.clone(),
            command: plugin.command.clone(),
            args: plugin.args.clone(),
            secrets: plugin.secrets.clone(),
            suggest_confirm: plugin.suggest_confirm.clone(),
        }],
    };
    packs::install(config, &pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Registry {
        Registry {
            plugins: vec![
                RegistryPlugin {
                    name: "search".into(),
                    description: "Web search via Tavily".into(),
                    command: "python".into(),
                    args: vec!["{plugins}/search_mcp/mcp_server.py".into()],
                    secrets: vec!["TAVILY_API_KEY".into()],
                    tags: vec!["productivity".into(), "research".into()],
                    suggest_confirm: vec!["web_search".into()],
                },
                RegistryPlugin {
                    name: "slack".into(),
                    description: "Slack messaging".into(),
                    command: "npx".into(),
                    args: vec![],
                    secrets: vec!["SLACK_BOT_TOKEN".into()],
                    tags: vec!["messaging".into()],
                    suggest_confirm: vec![],
                },
            ],
        }
    }

    #[test]
    fn search_matches_name_desc_and_tag() {
        let reg = sample();
        assert_eq!(search(&reg, "search").len(), 1);
        assert_eq!(search(&reg, "messaging").len(), 1); // by tag
        assert_eq!(search(&reg, "").len(), 2); // empty = all
        assert_eq!(search(&reg, "nomatch").len(), 0);
    }

    #[test]
    fn install_registers_registry_plugin_failclosed() {
        let reg = sample();
        let mut config = Config::default();
        let plugin = find(&reg, "search").unwrap();
        let report = install(&mut config, plugin);
        assert_eq!(report.added, vec!["search".to_string()]);
        assert!(config
            .mcp_servers
            .iter()
            .any(|s| s.name == "search" && s.secrets == vec!["TAVILY_API_KEY".to_string()]));
        assert!(config
            .permissions
            .iter()
            .any(|r| r.tool == "web_search" && r.action == "require_confirmation"));
    }
}
