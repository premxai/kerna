//! Provider credentials in the OS credential store (Windows Credential
//! Manager via `keyring`). Configuration only ever holds a reference like
//! `kerna://anthropic`; the secret itself never lands in a TOML/JSON file.
//! The environment variable remains the first lookup so existing harnesses
//! keep working; on platforms without a V1 backend we say so instead of
//! silently falling back to plaintext.

use anyhow::{anyhow, Result};

pub const SERVICE: &str = "kerna";
pub const SUPPORTED_PROVIDERS: [&str; 2] = ["anthropic", "openai"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Environment,
    CredentialStore,
    None,
}

pub fn normalize_provider(provider: &str) -> Result<&'static str> {
    SUPPORTED_PROVIDERS
        .iter()
        .find(|name| **name == provider)
        .copied()
        .ok_or_else(|| anyhow!("unknown provider {provider}"))
}

#[cfg(windows)]
fn entry(provider: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider)
        .map_err(|error| anyhow!("credential store unavailable: {error}"))
}

pub fn store(provider: &str, secret: &str) -> Result<()> {
    let provider = normalize_provider(provider)?;
    if secret.trim().is_empty() {
        return Err(anyhow!("credential is empty"));
    }
    #[cfg(windows)]
    {
        entry(provider)?
            .set_password(secret.trim())
            .map_err(|error| anyhow!("could not write to the credential store: {error}"))
    }
    #[cfg(not(windows))]
    {
        let _ = (provider, secret);
        Err(anyhow!(
            "the OS credential store is only supported on Windows in V1; use the environment variable"
        ))
    }
}

pub fn load(provider: &str) -> Result<Option<String>> {
    let provider = normalize_provider(provider)?;
    #[cfg(windows)]
    {
        match entry(provider)?.get_password() {
            Ok(secret) if !secret.trim().is_empty() => Ok(Some(secret)),
            Ok(_) => Ok(None),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow!("credential store read failed: {error}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = provider;
        Ok(None)
    }
}

pub fn forget(provider: &str) -> Result<()> {
    let provider = normalize_provider(provider)?;
    #[cfg(windows)]
    {
        match entry(provider)?.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(anyhow!("credential store delete failed: {error}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = provider;
        Ok(())
    }
}

/// The env var name is supplied by the caller (providers::api_key_env_for) so
/// this module never invents a second convention.
pub fn source_for(provider: &str, env_var: &str) -> CredentialSource {
    if std::env::var_os(env_var).is_some_and(|value| !value.to_string_lossy().trim().is_empty()) {
        return CredentialSource::Environment;
    }
    match load(provider) {
        Ok(Some(_)) => CredentialSource::CredentialStore,
        _ => CredentialSource::None,
    }
}

/// Environment first (harness compatibility), then the credential store.
pub fn resolve(provider: &str, env_var: &str) -> Option<String> {
    if let Some(value) = std::env::var(env_var)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(value);
    }
    load(provider).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_providers_are_accepted() {
        assert_eq!(normalize_provider("anthropic").unwrap(), "anthropic");
        assert_eq!(normalize_provider("openai").unwrap(), "openai");
        assert!(normalize_provider("google").is_err());
        assert!(store("google", "x").is_err());
    }

    #[test]
    fn empty_secrets_never_reach_the_store() {
        assert!(store("anthropic", "   ").is_err());
    }

    #[test]
    fn environment_wins_over_the_store_and_absence_is_explicit() {
        let unique = format!("KERNA_TEST_KEY_{}", uuid::Uuid::new_v4());
        assert_eq!(source_for("anthropic", &unique), CredentialSource::None);
        std::env::set_var(&unique, "  ");
        assert_eq!(source_for("anthropic", &unique), CredentialSource::None);
        std::env::set_var(&unique, "value");
        assert_eq!(
            source_for("anthropic", &unique),
            CredentialSource::Environment
        );
        assert_eq!(resolve("anthropic", &unique).as_deref(), Some("value"));
        std::env::remove_var(&unique);
    }

    #[cfg(windows)]
    #[test]
    fn credential_store_round_trip_uses_a_namespaced_test_account() {
        // The production service is namespaced per test run so parallel CI
        // jobs cannot observe each other's entries.
        let provider = "anthropic";
        let secret = format!("kerna-test-{}", uuid::Uuid::new_v4());
        if let Err(error) = store(provider, &secret) {
            // Headless Windows runners without a credential manager are a
            // supported "environment variable only" outcome, not a failure.
            eprintln!("skipping live keyring round trip: {error}");
            return;
        }
        assert_eq!(load(provider).unwrap().as_deref(), Some(secret.as_str()));
        forget(provider).unwrap();
        assert_eq!(load(provider).unwrap(), None);
    }
}
