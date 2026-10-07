//! `kerna init` — first-run onboarding exactly per the V1 plan: wordmark,
//! five-row machine check, hardware-aware model choice, cloud provider setup
//! with secure storage, then the ready screen that points at `kerna code .`.
//! Diagnostics deliberately do NOT live here; that is `kerna doctor`.

use crate::{cli_brand, cli_models, cli_providers, cli_system, credentials, guard_routing, models};
use anyhow::Result;
use dialoguer::{Confirm, Select};

/// Pure so the "aha" screen is testable without a terminal.
pub fn summary_lines(local: Option<&str>, connected: &[&str]) -> Vec<String> {
    let mut out = vec![
        "Kerna is ready.".to_string(),
        String::new(),
        "Local".to_string(),
    ];
    match local {
        Some(model) => out.push(format!("  ✓ {model}")),
        None => out.push("  — none (cloud models will be used)".to_string()),
    }
    out.push(String::new());
    out.push("Cloud".to_string());
    for provider in cli_providers::PROVIDERS {
        if connected.contains(&provider) {
            out.push(format!("  ✓ {}", cli_providers::label(provider)));
        } else {
            match cli_providers::source(provider) {
                credentials::CredentialSource::None => out.push(format!(
                    "  — {} (not configured)",
                    cli_providers::label(provider)
                )),
                _ => out.push(format!("  ✓ {}", cli_providers::label(provider))),
            }
        }
    }
    out.push(String::new());
    out.push("Try:".to_string());
    out.push(String::new());
    out.push("  kerna code .".to_string());
    out
}

pub async fn run() -> Result<()> {
    cli_brand::banner("Agent Runtime");
    println!("Checking your machine...\n");
    let report = cli_system::SystemReport::detect();
    for (label, value, ok) in report.rows() {
        println!("{label:<9} {value:<28} {}", if ok { "✓" } else { "—" });
    }
    println!();
    println!(
        "{}",
        if report.local_ai_supported() {
            "Local AI supported."
        } else {
            "No local AI hardware detected - cloud models work perfectly."
        }
    );

    let mut chosen_local: Option<String> = None;
    let installed = cli_models::installed_models().await;
    let recipes = models::recommend(&models::detect_hardware(), "coding").unwrap_or_default();
    if installed.is_empty() && recipes.is_empty() {
        println!("\nNo validated local model matches this machine yet.");
        println!("Kerna will use cloud models; add a local runtime later with `kerna model pull`.");
    } else {
        let mut items: Vec<(String, &str)> = installed
            .iter()
            .map(|name| (name.clone(), "Installed · ready"))
            .chain(recipes.iter().map(|recipe| {
                (
                    recipe.model_instance_id.clone(),
                    "Recommended for this machine",
                )
            }))
            .collect();
        items.push(("__skip__".to_string(), "Skip for now"));
        let labels: Vec<String> = items
            .iter()
            .map(|(id, note)| {
                if id == "__skip__" {
                    note.to_string()
                } else {
                    format!("{id}  · {note}")
                }
            })
            .collect();
        let pick = cli_brand_with_no_terminal(|| {
            Select::new()
                .with_prompt("Choose a local model")
                .items(&labels)
                .default(0)
                .interact_opt()
                .ok()
                .flatten()
        });
        if let Some(index) = pick {
            let chosen = items[index].0.clone();
            if chosen != "__skip__" {
                if !installed.contains(&chosen) {
                    println!("Preparing {chosen}...");
                    if cli_models::pull(&chosen).is_err() {
                        println!(
                            "Couldn't start local acceleration. Kerna can continue using cloud models.\nRun `kerna doctor` for details."
                        );
                    } else {
                        chosen_local = Some(chosen);
                    }
                } else {
                    chosen_local = Some(chosen);
                }
            }
        }
    }

    println!("\nConnect cloud AI");
    let mut connected: Vec<&'static str> = Vec::new();
    if cli_brand_with_no_terminal(|| {
        Confirm::new()
            .with_prompt("Add a provider API key now? (stored in the OS credential store)")
            .default(true)
            .interact()
            .unwrap_or(false)
    }) {
        for provider in cli_providers::PROVIDERS {
            let wants = cli_brand_with_no_terminal(|| {
                Confirm::new()
                    .with_prompt(format!("Connect {}?", cli_providers::label(provider)))
                    .default(provider == "anthropic")
                    .interact()
                    .unwrap_or(false)
            });
            if wants
                && cli_providers::connect_interactive(provider)
                    .await
                    .unwrap_or(false)
            {
                connected.push(provider);
            }
        }
    }

    if chosen_local.is_some() || !connected.is_empty() {
        let mut profile = guard_routing::load_demo_profile();
        if let Some(model) = &chosen_local {
            profile.local_model = Some(model.clone());
        }
        if !connected.is_empty() {
            profile.cloud_enabled = true;
        }
        guard_routing::save_demo_profile(&profile)?;
    }

    println!();
    for line in summary_lines(chosen_local.as_deref(), &connected) {
        println!("{line}");
    }
    Ok(())
}

/// Non-interactive contexts (pipes, CI, tests) must not hard-fail onboarding;
/// they take the default/skip branch instead of reading a terminal.
fn cli_brand_with_no_terminal<T: Default>(run: impl FnOnce() -> T) -> T {
    if console::Term::stdout().is_term() {
        run()
    } else {
        T::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_screen_lists_local_cloud_and_the_next_command() {
        let lines = summary_lines(Some("qwen2.5-coder:7b"), &["anthropic"]);
        let text = lines.join("\n");
        assert!(text.starts_with("Kerna is ready."));
        assert!(text.contains("✓ qwen2.5-coder:7b"));
        assert!(text.contains("✓ Anthropic"));
        assert!(text.contains("OpenAI (not configured)") || text.contains("— OpenAI"));
        assert!(text.contains("kerna code ."));
    }

    #[test]
    fn empty_setup_is_still_a_complete_screen() {
        let text = summary_lines(None, &[]).join("\n");
        assert!(text.contains("— none"));
        assert!(text.contains("kerna code ."));
    }
}
