//! Provider resolution: maps a provider *name* (either a built-in preset or a
//! user-defined `[providers.<name>]` entry in `kerna.toml`) to a concrete wire
//! protocol, base URL, API key, and default model.
//!
//! This is the single place that knows how to reach an LLM endpoint. The
//! scheduler asks `resolve()` for a `ResolvedProvider` and then dispatches on
//! `protocol` — it never hardcodes provider URLs.

use crate::config::Config;
use anyhow::{anyhow, Result};
use serde_json::Value;

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

/// A deterministic model selection from a named privacy policy.  This is kept
/// separate from `ResolvedProvider`: selection happens before the endpoint is
/// contacted, while `resolve` turns the selection into a usable connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelRoute {
    pub privacy_mode: String,
    pub route_name: String,
    pub provider: String,
    pub model: String,
}

/// A model reported by a local OpenAI-compatible or Ollama runtime.  This is a
/// live inventory, not a hard-coded catalogue: an installed local model is the
/// only model Kerna is allowed to claim is available.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocalModel {
    pub id: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
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

/// Validate and split `provider/model`.  Only the first slash is structural:
/// model identifiers such as `openai/gpt-4o-mini` are valid model names when
/// addressed through a provider such as OpenRouter.
pub fn parse_model_route_target(config: &Config, target: &str) -> Result<(String, String)> {
    let (provider, model) = target.split_once('/').ok_or_else(|| {
        anyhow!(
            "Invalid route target '{}'. Expected provider/model (for example ollama/qwen2.5-coder).",
            target
        )
    })?;
    if provider.trim().is_empty() || model.trim().is_empty() {
        return Err(anyhow!(
            "Invalid route target '{}'. Provider and model must both be non-empty.",
            target
        ));
    }
    if provider != "mock"
        && !config.providers.contains_key(provider)
        && builtin_preset(provider).is_none()
    {
        return Err(anyhow!(
            "Unknown provider '{}' in route target. Add it first or use one of: {}.",
            provider,
            builtin_names().join(", ")
        ));
    }
    Ok((provider.to_string(), model.to_string()))
}

/// Resolve a privacy label to an explicit route. Missing policy or route names
/// are errors; falling back to an unrelated remote model would violate the
/// privacy promise implied by `--privacy`.
pub fn resolve_model_route(config: &Config, privacy_mode: &str) -> Result<ResolvedModelRoute> {
    let alternate = if privacy_mode.contains('-') {
        privacy_mode.replace('-', "_")
    } else {
        privacy_mode.replace('_', "-")
    };
    let route_name = config
        .privacy_routes
        .get(privacy_mode)
        .or_else(|| config.privacy_routes.get(&alternate))
        .ok_or_else(|| {
            anyhow!(
                "No privacy route named '{}'. Define [privacy_routes].{} = \"<model-route>\" in kerna.toml.",
                privacy_mode,
                privacy_mode.replace('-', "_")
            )
        })?;
    let target = config.model_routes.get(route_name).ok_or_else(|| {
        anyhow!(
            "Privacy route '{}' refers to missing model route '{}'.",
            privacy_mode,
            route_name
        )
    })?;
    let (provider, model) = parse_model_route_target(config, target)?;
    Ok(ResolvedModelRoute {
        privacy_mode: privacy_mode.to_string(),
        route_name: route_name.clone(),
        provider,
        model,
    })
}

/// Resolve a privacy route and its concrete endpoint in one fail-closed step.
/// The caller can safely hand the returned provider/model to the scheduler.
pub fn resolve_privacy_route(
    config: &Config,
    privacy_mode: &str,
    api_key: &str,
) -> Result<(ResolvedModelRoute, ResolvedProvider)> {
    let route = resolve_model_route(config, privacy_mode)?;
    let provider = resolve(config, &route.provider, Some(&route.model), api_key)?;
    if (privacy_mode == "local-only" || privacy_mode == "local_only") && !provider.is_local() {
        return Err(anyhow!(
            "Privacy violation: --privacy {} selected non-local endpoint {}.",
            privacy_mode,
            provider.base_url
        ));
    }
    Ok((route, provider))
}

/// Parse both response formats used by common local runtimes without treating
/// network data as trusted configuration. Ollama returns `{models:[...]}`;
/// OpenAI-compatible hosts return `{data:[...]}`.
pub fn parse_local_models(payload: &Value) -> Result<Vec<LocalModel>> {
    let entries = payload
        .get("models")
        .or_else(|| payload.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Local model endpoint returned no models/data array."))?;
    let mut models = Vec::new();
    for entry in entries {
        let id = entry
            .get("name")
            .or_else(|| entry.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| anyhow!("Local model entry has no name or id."))?;
        models.push(LocalModel {
            id: id.to_string(),
            size_bytes: entry.get("size").and_then(Value::as_u64),
            modified_at: entry
                .get("modified_at")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// Discover models actually installed in a local provider. This intentionally
/// does not register an arbitrary cloud catalogue as "local".
pub async fn discover_local_models(
    config: &Config,
    provider_name: &str,
) -> Result<Vec<LocalModel>> {
    let resolved = resolve(config, provider_name, None, "")?;
    if !resolved.is_local() {
        return Err(anyhow!(
            "Provider '{}' is not a local loopback endpoint; refusing local model discovery.",
            provider_name
        ));
    }
    let endpoint = if provider_name == "ollama" {
        format!("{}/api/tags", resolved.base_url.trim_end_matches("/v1"))
    } else {
        format!("{}/models", resolved.base_url.trim_end_matches('/'))
    };
    let response = reqwest::Client::new()
        .get(&endpoint)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|error| anyhow!("Could not reach local provider at {}: {}", endpoint, error))?
        .error_for_status()
        .map_err(|error| anyhow!("Local provider rejected model discovery: {}", error))?;
    parse_local_models(&response.json::<Value>().await?)
}

/// Ensure a selected local route names a model reported by its runtime. This
/// prevents a reassuring `local-only` decision from proceeding with an absent
/// or misspelled model identifier.
pub fn ensure_local_model_available(models: &[LocalModel], model: &str) -> Result<()> {
    if let Some(candidate) = models.iter().find(|candidate| candidate.id == model) {
        // A local loopback runtime can expose a cloud-backed model name. Do not
        // convert that transport detail into a misleading `local-only` claim.
        if !candidate.id.to_ascii_lowercase().ends_with(":cloud") {
            return Ok(());
        }
        return Err(anyhow!(
            "Local model '{}' is explicitly cloud-backed and cannot satisfy local-only privacy.",
            model
        ));
    }
    let available = models
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!(
        "Local model '{}' is not installed. Available models: {}.",
        model,
        if available.is_empty() {
            "(none)"
        } else {
            &available
        }
    ))
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

    // Prefer the selected provider's declared environment variable. A global
    // OpenAI key must never silently become the credential for a route that
    // selected another provider.
    let provider_key = std::env::var(api_key_env_for(config, provider_name))
        .ok()
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| api_key.to_string());
    let resolved = ResolvedProvider {
        name: provider_name.to_string(),
        protocol,
        base_url,
        api_key: provider_key,
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
        // GPT-4.1 nano is $0.10 input / $0.40 output per 1M tokens.
        // Keep the table's documented blended-estimate convention.
        ("gpt-4.1-nano", 0.25),
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
    fn synthetic_privacy_route_selects_local_model() {
        let mut cfg = base_config();
        cfg.model_routes.insert(
            "offline-coding".to_string(),
            "ollama/qwen2.5-coder:7b".to_string(),
        );
        cfg.privacy_routes
            .insert("local-only".to_string(), "offline-coding".to_string());

        let route = resolve_model_route(&cfg, "local-only").unwrap();
        assert_eq!(route.route_name, "offline-coding");
        assert_eq!(route.provider, "ollama");
        assert_eq!(route.model, "qwen2.5-coder:7b");
        assert!(resolve(&cfg, &route.provider, Some(&route.model), "")
            .unwrap()
            .is_local());
    }

    #[test]
    fn synthetic_openrouter_route_preserves_slashes_in_model_id() {
        let cfg = base_config();
        let (provider, model) =
            parse_model_route_target(&cfg, "openrouter/openai/gpt-4o-mini").unwrap();
        assert_eq!(provider, "openrouter");
        assert_eq!(model, "openai/gpt-4o-mini");
    }

    #[test]
    fn synthetic_missing_privacy_route_fails_closed() {
        let cfg = base_config();
        let err = resolve_model_route(&cfg, "private").unwrap_err();
        assert!(err.to_string().contains("No privacy route"));
    }

    #[test]
    fn synthetic_privacy_route_cannot_reference_missing_model_route() {
        let mut cfg = base_config();
        cfg.privacy_routes
            .insert("private".to_string(), "does-not-exist".to_string());
        let err = resolve_model_route(&cfg, "private").unwrap_err();
        assert!(err.to_string().contains("missing model route"));
    }

    #[test]
    fn synthetic_local_only_route_rejects_remote_endpoint() {
        let mut cfg = base_config();
        cfg.llm_api_key = "synthetic-key".to_string();
        cfg.model_routes
            .insert("unsafe".to_string(), "openai/gpt-4o-mini".to_string());
        cfg.privacy_routes
            .insert("local-only".to_string(), "unsafe".to_string());
        let err = resolve_privacy_route(&cfg, "local-only", &cfg.llm_api_key).unwrap_err();
        assert!(err.to_string().contains("Privacy violation"));
    }

    #[test]
    fn synthetic_local_model_payload_supports_ollama_and_openai_shapes() {
        let ollama = serde_json::json!({
            "models": [{"name": "qwen2.5-coder:7b", "size": 123, "modified_at": "today"}]
        });
        let openai_compatible = serde_json::json!({"data": [{"id": "phi-4-mini"}]});
        assert_eq!(
            parse_local_models(&ollama).unwrap()[0].id,
            "qwen2.5-coder:7b"
        );
        assert_eq!(
            parse_local_models(&openai_compatible).unwrap()[0].id,
            "phi-4-mini"
        );
    }

    #[test]
    fn synthetic_local_model_availability_is_exact() {
        let models = vec![LocalModel {
            id: "qwen2.5-coder:7b".to_string(),
            size_bytes: None,
            modified_at: None,
        }];
        assert!(ensure_local_model_available(&models, "qwen2.5-coder:7b").is_ok());
        assert!(ensure_local_model_available(&models, "qwen2.5-coder:14b")
            .unwrap_err()
            .to_string()
            .contains("not installed"));
    }

    #[test]
    fn cloud_labelled_ollama_models_cannot_satisfy_local_only() {
        let models = vec![LocalModel {
            id: "minimax-m2.5:cloud".to_string(),
            size_bytes: Some(337),
            modified_at: None,
        }];
        assert!(ensure_local_model_available(&models, "minimax-m2.5:cloud")
            .unwrap_err()
            .to_string()
            .contains("cloud-backed"));
    }

    #[test]
    fn cost_known_and_unknown() {
        assert!(estimate_cost_usd("gpt-4o-mini", 1_000_000).is_some());
        assert!(estimate_cost_usd("gpt-4.1-nano", 1_000_000).is_some());
        assert!(estimate_cost_usd("some-random-model", 1000).is_none());
    }
}
