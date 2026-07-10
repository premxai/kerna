//! Provider resolution: maps a provider *name* (either a built-in preset or a
//! user-defined `[providers.<name>]` entry in `kerna.toml`) to a concrete wire
//! protocol, base URL, API key, and default model.
//!
//! This is the single place that knows how to reach an LLM endpoint. The
//! scheduler asks `resolve()` for a `ResolvedProvider` and then dispatches on
//! `protocol` — it never hardcodes provider URLs.

use crate::config::Config;
use anyhow::{anyhow, Result};

/// The HTTP wire format a provider speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireProtocol {
    /// OpenAI `/v1/chat/completions` shape. Used by OpenAI, OpenRouter, Ollama,
    /// Groq, Together, DeepSeek, Mistral, xAI, Venice, and any compatible host.
    OpenAiCompat,
    /// Anthropic `/v1/messages` shape.
    Anthropic,
    /// In-process deterministic mock used by tests and the zero-key demo.
    Mock,
}

/// A fully resolved provider ready to call.
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub name: String,
    pub protocol: WireProtocol,
    /// Base URL *without* the trailing endpoint path (e.g. `https://api.openai.com/v1`).
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl ResolvedProvider {
    /// True when the endpoint is on the local machine (loopback host).
    /// Used to enforce `--privacy local-only` and to waive the API-key
    /// requirement for local runtimes like Ollama.
    pub fn is_local(&self) -> bool {
        let lower = self.base_url.to_lowercase();
        lower.contains("://localhost")
            || lower.contains("://127.0.0.1")
            || lower.contains("://0.0.0.0")
            || lower.contains("://[::1]")
            || lower.contains("://host.docker.internal")
    }
}

/// A built-in provider preset. `base_url`/`model` are defaults the user can
/// override via `[providers.<name>]` or `--model`.
struct Preset {
    protocol: WireProtocol,
    base_url: &'static str,
    api_key_env: &'static str,
    default_model: &'static str,
}

/// Returns the built-in preset for a well-known provider name, if any.
fn builtin_preset(name: &str) -> Option<Preset> {
    let p = match name {
        "openai" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.openai.com/v1",
            api_key_env: "OPENAI_API_KEY",
            default_model: "gpt-4o-mini",
        },
        "anthropic" => Preset {
            protocol: WireProtocol::Anthropic,
            base_url: "https://api.anthropic.com",
            api_key_env: "ANTHROPIC_API_KEY",
            default_model: "claude-sonnet-4-20250514",
        },
        "openrouter" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://openrouter.ai/api/v1",
            api_key_env: "OPENROUTER_API_KEY",
            default_model: "openai/gpt-4o-mini",
        },
        "ollama" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "http://localhost:11434/v1",
            api_key_env: "OLLAMA_API_KEY",
            default_model: "qwen2.5-coder",
        },
        "groq" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.groq.com/openai/v1",
            api_key_env: "GROQ_API_KEY",
            default_model: "llama-3.3-70b-versatile",
        },
        "together" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.together.xyz/v1",
            api_key_env: "TOGETHER_API_KEY",
            default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        },
        "deepseek" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.deepseek.com/v1",
            api_key_env: "DEEPSEEK_API_KEY",
            default_model: "deepseek-chat",
        },
        "mistral" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.mistral.ai/v1",
            api_key_env: "MISTRAL_API_KEY",
            default_model: "mistral-large-latest",
        },
        "xai" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.x.ai/v1",
            api_key_env: "XAI_API_KEY",
            default_model: "grok-2-latest",
        },
        "venice" => Preset {
            protocol: WireProtocol::OpenAiCompat,
            base_url: "https://api.venice.ai/api/v1",
            api_key_env: "VENICE_API_KEY",
            default_model: "llama-3.3-70b",
        },
        _ => return None,
    };
    Some(p)
}

/// Public view of a built-in preset, used to pre-fill `kerna provider add`.
pub struct PresetInfo {
    pub provider_type: String,
    pub base_url: String,
    pub api_key_env: String,
    pub default_model: String,
}

/// Returns the built-in preset details for a well-known provider name.
pub fn preset_info(name: &str) -> Option<PresetInfo> {
    builtin_preset(name).map(|p| PresetInfo {
        provider_type: match p.protocol {
            WireProtocol::Anthropic => "anthropic".to_string(),
            WireProtocol::OpenAiCompat => "openai_compatible".to_string(),
            WireProtocol::Mock => "mock".to_string(),
        },
        base_url: p.base_url.to_string(),
        api_key_env: p.api_key_env.to_string(),
        default_model: p.default_model.to_string(),
    })
}

/// The names of all built-in provider presets (for help text / `keys list`).
pub fn builtin_names() -> &'static [&'static str] {
    &[
        "openai",
        "anthropic",
        "openrouter",
        "ollama",
        "groq",
        "together",
        "deepseek",
        "mistral",
        "xai",
        "venice",
    ]
}

/// The environment variable a provider reads its key from — user override first,
/// then the built-in preset, then the generic `KERNA_LLM_API_KEY`.
pub fn api_key_env_for(config: &Config, name: &str) -> String {
    if let Some(user) = config.providers.get(name) {
        if let Some(env) = &user.api_key_env {
            return env.clone();
        }
    }
    if let Some(preset) = builtin_preset(name) {
        return preset.api_key_env.to_string();
    }
    "KERNA_LLM_API_KEY".to_string()
}

/// Resolve `provider_name` into a concrete endpoint.
///
/// Precedence for each field: explicit user `[providers.<name>]` config →
/// built-in preset → generic fallback. `model_override` (from `--model` or a
/// route) wins over the provider default.
pub fn resolve(
    config: &Config,
    provider_name: &str,
    model_override: Option<&str>,
    api_key: &str,
) -> Result<ResolvedProvider> {
    if provider_name == "mock" {
        return Ok(ResolvedProvider {
            name: "mock".to_string(),
            protocol: WireProtocol::Mock,
            base_url: "mock://local".to_string(),
            api_key: String::new(),
            model: model_override.unwrap_or("mock").to_string(),
        });
    }

    let user = config.providers.get(provider_name);
    let preset = builtin_preset(provider_name);

    if user.is_none() && preset.is_none() {
        return Err(anyhow!(
            "Unknown provider '{}'. Add it with `kerna provider add {} --base-url <url>` \
             or use a built-in: {}.",
            provider_name,
            provider_name,
            builtin_names().join(", ")
        ));
    }

    // Determine wire protocol: user's `type` if set, else preset, else openai_compat.
    let protocol = match user.map(|u| u.provider_type.as_str()) {
        Some("anthropic") => WireProtocol::Anthropic,
        Some("openai") | Some("openai_compatible") | Some("local") => WireProtocol::OpenAiCompat,
        Some("mock") => WireProtocol::Mock,
        _ => preset
            .as_ref()
            .map(|p| p.protocol.clone())
            .unwrap_or(WireProtocol::OpenAiCompat),
    };

    let base_url = user
        .and_then(|u| u.base_url.clone())
        .or_else(|| preset.as_ref().map(|p| p.base_url.to_string()))
        .ok_or_else(|| {
            anyhow!(
                "Provider '{}' has no base_url. Set it with `kerna provider add {} --base-url <url>`.",
                provider_name,
                provider_name
            )
        })?;

    let model = model_override
        .map(|s| s.to_string())
        .or_else(|| {
            user.map(|u| u.default_model.clone())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| preset.as_ref().map(|p| p.default_model.to_string()))
        .unwrap_or_else(|| "gpt-4o-mini".to_string());

    let resolved = ResolvedProvider {
        name: provider_name.to_string(),
        protocol,
        base_url,
        api_key: api_key.to_string(),
        model,
    };

    // Local runtimes (Ollama) don't require a key; remote ones do.
    if resolved.api_key.is_empty()
        && resolved.protocol != WireProtocol::Mock
        && !resolved.is_local()
    {
        return Err(anyhow!(
            "No API key for provider '{}'. Set the {} environment variable \
             (see `kerna keys add {}`).",
            provider_name,
            api_key_env_for(config, provider_name),
            provider_name
        ));
    }

    Ok(resolved)
}

/// Rough USD cost for a call, based on a small static price table
/// (USD per 1M tokens, blended input+output as an approximation).
/// Returns `None` when the model's pricing is unknown.
pub fn estimate_cost_usd(model: &str, total_tokens: u64) -> Option<f64> {
    let m = model.to_lowercase();
    // (substring, usd_per_million_tokens)
    let table: &[(&str, f64)] = &[
        ("gpt-4o-mini", 0.30),
        ("gpt-4o", 5.00),
        ("gpt-4.1-mini", 0.40),
        ("gpt-4.1", 5.00),
        ("o1-mini", 3.00),
        ("o1", 15.00),
        ("claude-3-5-haiku", 1.20),
        ("claude-3-5-sonnet", 6.00),
        ("claude-sonnet-4", 6.00),
        ("claude-3-opus", 30.00),
        ("claude-opus-4", 30.00),
        ("llama-3.3-70b", 0.60),
        ("deepseek-chat", 0.28),
        ("mistral-large", 3.00),
        ("grok-2", 4.00),
    ];
    for (needle, price_per_m) in table {
        if m.contains(needle) {
            return Some((total_tokens as f64) / 1_000_000.0 * price_per_m);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ProviderConfig};

    fn base_config() -> Config {
        Config::default()
    }

    #[test]
    fn resolves_builtin_openai() {
        let cfg = base_config();
        let r = resolve(&cfg, "openai", None, "sk-test").unwrap();
        assert_eq!(r.protocol, WireProtocol::OpenAiCompat);
        assert_eq!(r.base_url, "https://api.openai.com/v1");
        assert_eq!(r.model, "gpt-4o-mini");
    }

    #[test]
    fn resolves_builtin_anthropic_protocol() {
        let cfg = base_config();
        let r = resolve(&cfg, "anthropic", Some("claude-x"), "sk-ant").unwrap();
        assert_eq!(r.protocol, WireProtocol::Anthropic);
        assert_eq!(r.model, "claude-x");
    }

    #[test]
    fn ollama_is_local_and_needs_no_key() {
        let cfg = base_config();
        let r = resolve(&cfg, "ollama", None, "").unwrap();
        assert!(r.is_local());
        assert_eq!(r.protocol, WireProtocol::OpenAiCompat);
    }

    #[test]
    fn remote_provider_without_key_errors() {
        let cfg = base_config();
        let err = resolve(&cfg, "openai", None, "").unwrap_err();
        assert!(err.to_string().contains("No API key"));
    }

    #[test]
    fn unknown_provider_errors_with_suggestions() {
        let cfg = base_config();
        let err = resolve(&cfg, "totally-made-up", None, "k").unwrap_err();
        assert!(err.to_string().contains("Unknown provider"));
    }

    #[test]
    fn user_custom_base_url_overrides() {
        let mut cfg = base_config();
        cfg.providers.insert(
            "mylocal".to_string(),
            ProviderConfig {
                provider_type: "openai_compatible".to_string(),
                api_key_env: Some("MY_KEY".to_string()),
                default_model: "custom-model".to_string(),
                base_url: Some("http://localhost:9999/v1".to_string()),
            },
        );
        let r = resolve(&cfg, "mylocal", None, "").unwrap();
        assert_eq!(r.base_url, "http://localhost:9999/v1");
        assert_eq!(r.model, "custom-model");
        assert!(r.is_local());
    }

    #[test]
    fn mock_always_resolves() {
        let cfg = base_config();
        let r = resolve(&cfg, "mock", None, "").unwrap();
        assert_eq!(r.protocol, WireProtocol::Mock);
    }

    #[test]
    fn cost_known_and_unknown() {
        assert!(estimate_cost_usd("gpt-4o-mini", 1_000_000).is_some());
        assert!(estimate_cost_usd("some-random-model", 1000).is_none());
    }
}
