use crate::config::{Config, McpServerConfig};
use anyhow::{bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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

    /// Stable bytes signed by a plugin publisher. The manifest's runtime file
    /// hash is intentionally not part of this representation: it is the
    /// separately pinned review fingerprint in `kerna.toml`.
    fn signing_payload(&self) -> Result<Vec<u8>> {
        let mut unsigned = self.clone();
        unsigned.plugin.signature = None;
        unsigned.plugin.manifest_sha256 = None;
        Ok(toml::to_string(&unsigned)?.into_bytes())
    }

    pub fn verify_signature(&self, public_key_b64: &str) -> Result<()> {
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(public_key_b64)
            .context("signing_public_key is not valid base64")?;
        let signature = self
            .plugin
            .signature
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("manifest is unsigned"))?;
        let signature = base64::engine::general_purpose::STANDARD
            .decode(signature)
            .context("manifest signature is not valid base64")?;
        let key: [u8; 32] = public_key
            .try_into()
            .map_err(|_| anyhow::anyhow!("signing_public_key must decode to 32 bytes"))?;
        let signature: [u8; 64] = signature
            .try_into()
            .map_err(|_| anyhow::anyhow!("manifest signature must decode to 64 bytes"))?;
        VerifyingKey::from_bytes(&key)
            .context("invalid Ed25519 signing_public_key")?
            .verify(&self.signing_payload()?, &Signature::from_bytes(&signature))
            .map_err(|_| anyhow::anyhow!("manifest signature verification failed"))
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

/// Sign a publisher-owned manifest with an Ed25519 seed supplied out of band.
/// The key is never written to the manifest or printed by the CLI. The result
/// contains the complete file fingerprint and public key needed by `mcp add`.
pub fn sign_manifest(path: &Path, secret_key_b64: &str) -> Result<(String, String)> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("could not read manifest '{}'", path.display()))?;
    let mut manifest: PluginManifest = toml::from_str(&raw)
        .with_context(|| format!("could not parse manifest '{}'", path.display()))?;
    let secret_key = base64::engine::general_purpose::STANDARD
        .decode(secret_key_b64)
        .context("signing key is not valid base64")?;
    let secret_key: [u8; 32] = secret_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must decode to a 32-byte Ed25519 seed"))?;
    let signer = SigningKey::from_bytes(&secret_key);

    // These two fields are excluded from the signed representation. The
    // fingerprint below binds the final serialized file in the contract.
    manifest.plugin.manifest_sha256 = None;
    manifest.plugin.signature = None;
    let signature = signer.sign(&manifest.signing_payload()?);
    manifest.plugin.signature =
        Some(base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()));
    let serialized = toml::to_string(&manifest)?;
    let fingerprint = format!("{:x}", Sha256::digest(serialized.as_bytes()));
    fs::write(path, serialized)
        .with_context(|| format!("could not write manifest '{}'", path.display()))?;
    Ok((
        fingerprint,
        base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes()),
    ))
}

/// Locate a manifest for a configured MCP server. Prefer a manifest adjacent to
/// the configured entrypoint, then fall back to Kerna's shipped plugin layout.
/// This keeps manifests working for both local paths and installed packs.
pub fn find_for_server(server: &McpServerConfig) -> Option<PathBuf> {
    if let Some(path) = server.manifest_candidate() {
        return path.is_file().then_some(path);
    }
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

/// Validate a production plugin before it is handed to the container runner.
/// Signature validation and both fingerprints are checked before any image is
/// started, eliminating a native or mutable-image downgrade path.
pub fn verify_production_server(
    server: &McpServerConfig,
    workspace: &Path,
) -> Result<PluginManifest> {
    server.validate_container_contract(workspace)?;
    let path = workspace.join(&server.manifest_path);
    let manifest = PluginManifest::load(&path)
        .with_context(|| format!("plugin '{}' is missing manifest.toml", server.name))?;
    if manifest.plugin.name != server.name {
        bail!(
            "manifest '{}' does not match configured plugin '{}'",
            manifest.plugin.name,
            server.name
        );
    }
    let observed = manifest
        .plugin
        .manifest_sha256
        .as_deref()
        .unwrap_or_default();
    if !observed.eq_ignore_ascii_case(&server.manifest_sha256) {
        bail!(
            "manifest fingerprint mismatch for '{}': {}",
            server.name,
            path.display()
        );
    }
    manifest.verify_signature(&server.signing_public_key)?;
    Ok(manifest)
}

/// Apply the manifest's declarations to a configured server as additional
/// restrictions. Configuration can only narrow a manifest declaration; it can
/// never expand the tools or secrets a manifest permits.
pub fn apply_to_server(server: &mut McpServerConfig) -> Result<Option<PathBuf>> {
    let Some((path, manifest)) = load_for_server(server)? else {
        return Ok(None);
    };

    apply_manifest_to_server(server, &manifest)?;
    Ok(Some(path))
}

/// Apply a manifest that has already been verified against an explicit
/// workspace. Production initialization uses this form so relative manifest
/// paths do not depend on the process current directory.
pub fn apply_manifest_to_server(
    server: &mut McpServerConfig,
    manifest: &PluginManifest,
) -> Result<()> {
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

    Ok(())
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
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

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
            image: String::new(),
            manifest_path: String::new(),
            manifest_sha256: String::new(),
            signing_public_key: String::new(),
            read_roots: vec![],
            write_roots: vec![],
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

    #[test]
    fn production_server_requires_a_valid_signed_manifest() {
        let dir = std::env::temp_dir().join(format!("kerna-signed-manifest-{}", Uuid::new_v4()));
        fs::create_dir_all(dir.join("input")).unwrap();
        let manifest_path = dir.join("manifest.toml");
        let mut manifest = PluginManifest {
            plugin: PluginMetadata {
                name: "test-plugin".to_string(),
                version: "1.0.0".to_string(),
                kind: "tool.mcp".to_string(),
                entrypoint: "server".to_string(),
                source: default_source(),
                trust: default_trust(),
                capabilities: vec!["read".to_string()],
                requires_approval: vec![],
                secrets: vec![],
                allowed_paths: vec![],
                network_allowlist: vec![],
                declared_outputs: vec![],
                max_output_bytes: default_max_output_bytes(),
                manifest_sha256: None,
                signature: None,
            },
        };
        let signer = SigningKey::from_bytes(&[7; 32]);
        let signature = signer.sign(&manifest.signing_payload().unwrap());
        manifest.plugin.signature =
            Some(base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()));
        let content = toml::to_string(&manifest).unwrap();
        fs::write(&manifest_path, &content).unwrap();
        let fingerprint = format!("{:x}", Sha256::digest(content.as_bytes()));
        let mut configured = server(manifest_path.clone());
        configured.runtime_mode = "docker".to_string();
        configured.image =
            "example/test@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string();
        configured.manifest_path = "manifest.toml".to_string();
        configured.manifest_sha256 = fingerprint;
        configured.signing_public_key =
            base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes());
        configured.read_roots = vec!["input".to_string()];
        configured.capabilities = vec![];
        configured.allow_tools = vec![];
        assert!(verify_production_server(&configured, &dir).is_ok());
        configured.manifest_sha256 = "0".repeat(64);
        assert!(verify_production_server(&configured, &dir).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn signer_outputs_a_manifest_that_verifies_with_its_reported_public_key() {
        let dir = std::env::temp_dir().join(format!("kerna-manifest-sign-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.toml");
        fs::write(
            &path,
            r#"[plugin]
name = "signed-fixture"
version = "1.0.0"
kind = "tool.mcp"
entrypoint = "server"
"#,
        )
        .unwrap();
        let secret = base64::engine::general_purpose::STANDARD.encode([9u8; 32]);
        let (fingerprint, public_key) = sign_manifest(&path, &secret).unwrap();
        let loaded = PluginManifest::load(&path).unwrap();
        assert_eq!(
            loaded.plugin.manifest_sha256.as_deref(),
            Some(fingerprint.as_str())
        );
        assert!(loaded.verify_signature(&public_key).is_ok());
        let _ = fs::remove_dir_all(dir);
    }
}
