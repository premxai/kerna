use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use sha2::{Sha256, Digest};

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

fn default_source() -> String { "local".to_string() }
fn default_trust() -> String { "untrusted".to_string() }
fn default_max_output_bytes() -> u64 { 50000 }

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
        let risk_level = if p.trust == "untrusted" && (!p.secrets.is_empty() || !p.network_allowlist.is_empty()) {
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
