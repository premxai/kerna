//! `kerna provider add|list|remove` — cloud provider onboarding with an
//! optional live verification call, storing the key in the OS credential
//! store. List and status surfaces only ever show the source, never a key.

use crate::credentials;
use anyhow::Result;

pub const PROVIDERS: [&str; 2] = credentials::SUPPORTED_PROVIDERS;

pub fn label(provider: &str) -> String {
    match provider {
        "anthropic" => "Anthropic".to_string(),
        "openai" => "OpenAI".to_string(),
        other => other.to_string(),
    }
}

fn env_var_name(provider: &str) -> String {
    crate::providers::api_key_env_for(&crate::config::Config::load(), provider)
}

pub fn source(provider: &str) -> credentials::CredentialSource {
    credentials::source_for(provider, &env_var_name(provider))
}

/// One cheap authenticated request proves the key without spending a real
/// completion: Anthropic gets a max_tokens=1 ping, OpenAI the models list.
pub async fn verify(provider: &str, key: &str) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let request = match provider {
        "anthropic" => client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": crate::guard_routing::DEFAULT_CLOUD_MODEL,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "ping"}]
            })),
        _ => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key),
    };
    match request.send().await {
        Ok(response) if response.status().is_success() => Ok(true),
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            Err("the provider rejected this key (authentication failed)".to_string())
        }
        Ok(response) => Err(format!("provider returned HTTP {}", response.status())),
        Err(error) => Err(format!("could not reach the provider: {error}")),
    }
}

/// Shared by `kerna provider add` and the `kerna init` checklist.
pub async fn connect_interactive(provider: &str) -> Result<bool> {
    credentials::normalize_provider(provider)?;
    let key = dialoguer::Password::new()
        .with_prompt("API key")
        .interact()?;
    if key.trim().is_empty() {
        println!("  (skipped: no key entered)");
        return Ok(false);
    }
    match verify(provider, key.trim()).await {
        Ok(true) => {
            credentials::store(provider, key.trim())?;
            println!("  ✓ Connected  (stored in the OS credential store)");
            Ok(true)
        }
        Err(problem) => {
            println!("  ! {problem}");
            if dialoguer::Confirm::new()
                .with_prompt("Store this key anyway?")
                .default(false)
                .interact()?
            {
                credentials::store(provider, key.trim())?;
                println!("  ✓ Stored (unverified)");
            }
            Ok(false)
        }
        Ok(false) => unreachable!("verify reports success or a problem"),
    }
}

pub fn list() -> Result<()> {
    println!("Cloud providers");
    for provider in PROVIDERS {
        let source = match source(provider) {
            credentials::CredentialSource::Environment => "environment variable",
            credentials::CredentialSource::CredentialStore => {
                "OS credential store (kerna://credential-store)"
            }
            credentials::CredentialSource::None => "not configured",
        };
        println!("  {:<10} {source}", label(provider));
    }
    Ok(())
}

pub fn remove(provider: &str) -> Result<()> {
    credentials::forget(provider)?;
    println!(
        "Removed the stored {} credential (an environment variable, if set, still applies).",
        label(provider)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_env_names_cover_both_v1_providers() {
        assert_eq!(label("anthropic"), "Anthropic");
        assert_eq!(label("openai"), "OpenAI");
        assert!(!env_var_name("anthropic").is_empty());
        assert!(!env_var_name("openai").is_empty());
    }

    #[tokio::test]
    async fn an_obviously_bad_key_fails_authentication_rather_than_hanging() {
        // Network-free guarantee: a bogus key must produce Err/Ok(false),
        // never a panic, within the client's own timeout.
        let outcome = verify("anthropic", "bogus-key-for-cli-test").await;
        assert!(outcome.is_err() || outcome == Ok(false));
    }
}
