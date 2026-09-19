//! `kerna model list|pull|use` — Kerna orchestrates an existing local runtime
//! (Ollama) and the curated hardware catalog; it never builds an inference
//! engine and never pretends a model exists that the machine lacks.

use crate::guard_routing;
use crate::models;
use anyhow::{anyhow, Result};

pub fn parse_tags(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("models")
        .and_then(|models| models.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.get("name").and_then(|name| name.as_str()))
                .map(|name| name.to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn installed_models() -> Vec<String> {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    else {
        return Vec::new();
    };
    let Ok(response) = client
        .get(format!(
            "{}/api/tags",
            guard_routing::DEFAULT_LOCAL_BASE_URL
        ))
        .send()
        .await
    else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    match response.json::<serde_json::Value>().await {
        Ok(payload) => parse_tags(&payload),
        Err(_) => Vec::new(),
    }
}

fn ollama_binary() -> Option<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from("ollama")];
    candidates.push(std::path::PathBuf::from(
        r"C:\ProgramData\Ollama\ollama.exe",
    ));
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(std::path::PathBuf::from(local).join(r"Programs\Ollama\ollama.exe"));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file() || version_check(candidate))
}

fn version_check(candidate: &std::path::Path) -> bool {
    std::process::Command::new(candidate)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub async fn list() -> Result<()> {
    let runtime_up = installed_models().await;
    println!("Local runtime");
    if runtime_up.is_empty() {
        println!("  No local model runtime is reachable on this machine.");
        println!("  Install Ollama to enable `kerna model pull`, or continue with cloud models.");
    } else {
        for name in runtime_up {
            println!("  ✓ {name}");
        }
    }
    let hardware = models::detect_hardware();
    let recipes = models::recommend(&hardware, "coding")?;
    println!("\nCatalog recommendations for this machine");
    if recipes.is_empty() {
        println!("  No validated catalog recipe matches this hardware.");
    }
    for recipe in recipes {
        println!(
            "  {}  (min {} GB VRAM, engine {}, tools {})",
            recipe.model_instance_id,
            recipe.min_vram_gb,
            recipe.engine,
            if recipe.tools { "yes" } else { "no" }
        );
    }
    Ok(())
}

pub fn pull(name: &str) -> Result<()> {
    validate_model_name(name)?;
    let binary = ollama_binary().ok_or_else(|| {
        anyhow!("Ollama is not installed here, so there is no local runtime to pull into.")
    })?;
    println!("Downloading {name}...");
    let status = std::process::Command::new(&binary)
        .args(["pull", name])
        .status()
        .map_err(|_| {
            anyhow!(
                "couldn't start local acceleration - Kerna can continue using cloud models.\nRun `kerna doctor` for details."
            )
        })?;
    if !status.success() {
        return Err(anyhow!(
            "the local model download did not complete - cloud models keep working.\nRun `kerna doctor` for details."
        ));
    }
    println!("✓ Local model ready");
    Ok(())
}

pub async fn use_model(name: &str) -> Result<()> {
    validate_model_name(name)?;
    let installed = installed_models().await;
    if !installed.iter().any(|tag| tag == name) {
        return Err(anyhow!(
            "{name} is not installed yet - try `kerna model pull {name}` first"
        ));
    }
    let mut profile = guard_routing::load_demo_profile();
    profile.local_model = Some(name.to_string());
    guard_routing::save_demo_profile(&profile)?;
    println!("✓ {name} is now the active local model");
    Ok(())
}

fn validate_model_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'));
    if ok {
        Ok(())
    } else {
        Err(anyhow!("invalid model name"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_parsing_tolerates_missing_or_malformed_payloads() {
        assert_eq!(parse_tags(&serde_json::json!({})), Vec::<String>::new());
        assert_eq!(
            parse_tags(&serde_json::json!({"models":[{"name":"qwen2.5-coder:7b"},{"nope":1}]})),
            vec!["qwen2.5-coder:7b".to_string()]
        );
    }

    #[test]
    fn model_names_are_validated_before_reaching_any_process() {
        assert!(validate_model_name("qwen2.5-coder:7b").is_ok());
        assert!(validate_model_name("qwen; rm -rf /").is_err());
        assert!(validate_model_name("").is_err());
    }

    #[tokio::test]
    async fn use_model_requires_an_installed_tag() {
        let installed = installed_models().await;
        if installed.is_empty() {
            // Honest refusal on a machine without the local runtime.
            let error = use_model("anything:1").await.unwrap_err().to_string();
            assert!(error.contains("not installed"), "{error}");
        }
    }
}
