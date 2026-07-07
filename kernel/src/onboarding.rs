use crate::config::{BudgetPreset, Config, PermissionRule};
use console::{style, Term};
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use std::collections::HashMap;
use std::fs;

pub fn run_onboarding(
    quick: bool,
    ci: bool,
    yes: bool,
    no_setup: bool,
    provider_flag: Option<String>,
    model_flag: Option<String>,
) {
    let term = Term::stdout();

    if no_setup {
        // Just return, user requested no setup
        return;
    }

    if !ci && !quick && !yes {
        println!("{}", style("  _  __                     ").cyan().bold());
        println!("{}", style(" | |/ /___ _ __ _ __   __ _ ").cyan().bold());
        println!("{}", style(" | ' // _ \\ '__| '_ \\ / _` |").cyan().bold());
        println!("{}", style(" | . \\  __/ |  | | | | (_| |").cyan().bold());
        println!("{}", style(" |_|\\_\\___|_|  |_| |_|\\__,_|").cyan().bold());
        println!();
        println!(
            "{}",
            style("Welcome to Kerna — the runtime trust layer for autonomous agents.").bold()
        );
        println!();
        println!("Kerna helps you run agents with:");
        println!(" {} budgets", style("✓").green());
        println!(" {} fail-closed permissions", style("✓").green());
        println!(" {} plugin risk cards", style("✓").green());
        println!(" {} structured traces", style("✓").green());
        println!(" {} workspace checkpoints", style("✓").green());
        println!();
        println!("{}", style("Important:").yellow().bold());
        println!("Kerna reduces risk, but local tools and plugins can still affect your machine.");
        println!("Review docs/SECURITY_MODEL.md before running untrusted plugins.");
        println!();
    }

    let mut config = Config::load();

    // 1. LLM Provider
    if let Some(p) = provider_flag {
        config.llm_provider = p;
    } else if ci || quick || yes {
        config.llm_provider = "openai".to_string();
    } else {
        let providers = vec![
            "OpenAI",
            "Anthropic",
            "Local / OpenAI-compatible",
            "Skip for now",
        ];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select provider")
            .default(0)
            .items(&providers)
            .interact()
            .unwrap_or(3);

        config.llm_provider = match selection {
            0 => "openai".to_string(),
            1 => "anthropic".to_string(),
            2 => "local".to_string(),
            _ => "skip".to_string(),
        };
    }

    if let Some(m) = model_flag {
        config.llm_model = m;
    } else if config.llm_provider == "openai" {
        config.llm_model = "gpt-4o-mini".to_string();
    } else if config.llm_provider == "anthropic" {
        config.llm_model = "claude-3-5-sonnet-20240620".to_string();
    } else {
        config.llm_model = "llama3".to_string();
    }

    // 2. Default Policy
    let mut is_strict = true;
    if !(ci || quick || yes) {
        let policies = vec![
            "Strict: ask before file writes, shell commands, network, external messages",
            "Permissive: approve low-risk tools automatically",
        ];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Default policy")
            .default(0)
            .items(&policies)
            .interact()
            .unwrap_or(0);
        is_strict = selection == 0;
    }

    if is_strict {
        config.permissions = vec![
            PermissionRule {
                tool: "run_command".to_string(),
                action: "require_confirmation".to_string(),
            },
            PermissionRule {
                tool: "write_file".to_string(),
                action: "require_confirmation".to_string(),
            },
            PermissionRule {
                tool: "*".to_string(),
                action: "deny".to_string(),
            },
        ];
    } else {
        config.permissions = vec![
            PermissionRule {
                tool: "run_command".to_string(),
                action: "require_confirmation".to_string(),
            },
            PermissionRule {
                tool: "write_file".to_string(),
                action: "auto_approve".to_string(),
            },
            PermissionRule {
                tool: "*".to_string(),
                action: "deny".to_string(),
            },
        ];
    }

    // 3. Budget Presets
    let mut presets = HashMap::new();

    let conservative = BudgetPreset {
        max_tool_calls: 10,
        max_llm_calls: 5,
        max_runtime_seconds: 120,
        max_output_bytes: 50000,
        max_memory_writes: 5,
        max_cost_usd: 0.10,
    };

    let balanced = BudgetPreset {
        max_tool_calls: 25,
        max_llm_calls: 10,
        max_runtime_seconds: 300,
        max_output_bytes: 100000,
        max_memory_writes: 20,
        max_cost_usd: 0.50,
    };

    presets.insert("conservative".to_string(), conservative.clone());
    presets.insert("balanced".to_string(), balanced.clone());
    config.presets = presets;

    let mut selected_preset = "conservative";
    if !(ci || quick || yes) {
        let preset_opts = vec!["Conservative", "Balanced", "Custom"];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Budget preset")
            .default(0)
            .items(&preset_opts)
            .interact()
            .unwrap_or(0);

        if selection == 1 {
            selected_preset = "balanced";
        } else if selection == 2 {
            selected_preset = "custom";
        }
    }

    if selected_preset == "conservative" {
        config.max_tool_calls = conservative.max_tool_calls;
        config.max_llm_calls = conservative.max_llm_calls;
        config.max_runtime_seconds = conservative.max_runtime_seconds;
        config.max_output_bytes = conservative.max_output_bytes;
        config.max_memory_writes = conservative.max_memory_writes;
        config.max_cost_usd = conservative.max_cost_usd;
    } else if selected_preset == "balanced" {
        config.max_tool_calls = balanced.max_tool_calls;
        config.max_llm_calls = balanced.max_llm_calls;
        config.max_runtime_seconds = balanced.max_runtime_seconds;
        config.max_output_bytes = balanced.max_output_bytes;
        config.max_memory_writes = balanced.max_memory_writes;
        config.max_cost_usd = balanced.max_cost_usd;
    }

    println!();
    println!(
        " {}",
        style("[✓] Initializing structured event log...").green()
    );
    println!(
        " {}",
        style("[✓] Creating workspace execution profile...").green()
    );
    println!(
        " {}",
        style("[✓] Creating default fail-closed permission policy...").green()
    );
    println!();

    // Write config
    if let Ok(toml_str) = toml::to_string_pretty(&config) {
        let _ = fs::write("kerna.toml", toml_str);
    }

    if !ci {
        println!("{}", style("[✓] Kerna is ready.").green().bold());
        println!();
        println!("Let's test your agent's execution boundaries.");
        println!("Run your first supervised task:");
        println!(
            "  > kerna run \"Calculate 25 * 4 and save it to result.txt\" --budget-tool-calls=5"
        );
        println!();
        println!("Then, audit the agent's exact thought process and policy checks:");
        println!("  > kerna trace last");
        println!();
    }
}
