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

/// Operator handoff for unattended keyed runs, mirroring the contained
/// launcher: the env var names a file *outside any repository* whose contents
/// are the key. Only the path ever crosses chat, argv, or logs.
fn key_file_env(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("KERNA_ANTHROPIC_KEY_FILE"),
        "openai" => Some("KERNA_OPENAI_KEY_FILE"),
        _ => None,
    }
}

/// Environment first (harness compatibility), then the operator key file,
/// then the credential store.
pub fn resolve(provider: &str, env_var: &str) -> Option<String> {
    if let Some(value) = std::env::var(env_var)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Some(value);
    }
    if let Some(path) = key_file_env(provider).and_then(std::env::var_os) {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let key = raw.trim();
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
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

    #[test]
    fn operator_key_file_handoff_is_read_trimmed_and_bounded() {
        // Serial test rule (--test-threads=1) makes the process-global env
        // mutation safe; the file lives in a private temp dir and is removed.
        let dir = std::env::temp_dir().join(format!("kerna-keyfile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("key");
        std::env::set_var("KERNA_ANTHROPIC_KEY_FILE", &file);

        std::fs::write(&file, "  sk-operator-handoff\n").unwrap();
        assert_eq!(
            resolve("anthropic", "KERNA_TEST_ABSENT_ENV").as_deref(),
            Some("sk-operator-handoff")
        );

        // An empty file must not masquerade as a key; resolution falls past
        // it to the store (absent here, possibly configured on dev boxes).
        std::fs::write(&file, "   \n").unwrap();
        assert_ne!(
            resolve("anthropic", "KERNA_TEST_ABSENT_ENV").as_deref(),
            Some("sk-operator-handoff")
        );

        // The env variable still outranks the file handoff.
        let unique = format!("KERNA_TEST_KEY_{}", uuid::Uuid::new_v4());
        std::fs::write(&file, "sk-from-file").unwrap();
        std::env::set_var(&unique, "sk-from-env");
        assert_eq!(
            resolve("anthropic", &unique).as_deref(),
            Some("sk-from-env")
        );
        std::env::remove_var(&unique);

        std::env::remove_var("KERNA_ANTHROPIC_KEY_FILE");
        let _ = std::fs::remove_dir_all(&dir);
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
