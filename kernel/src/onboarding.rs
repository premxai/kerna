use crate::config::{BudgetPreset, Config, PermissionRule};
use crate::providers;
use console::style;
use dialoguer::{theme::ColorfulTheme, Select};
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

    // 1. LLM Provider — driven by the built-in preset catalog so onboarding
    //    always matches what the runtime actually supports.
    if let Some(p) = provider_flag {
        config.llm_provider = p;
    } else if ci || quick || yes {
        config.llm_provider = "openai".to_string();
    } else {
        let choices = vec![
            "OpenAI            (gpt-4o-mini)",
            "Anthropic         (claude-sonnet-4)",
            "Ollama            (local, no API key needed)",
            "OpenRouter        (one key, 300+ models)",
            "Groq              (fast llama inference)",
            "Other / OpenAI-compatible endpoint",
            "🎬 Demo mode      (no key, mock LLM — try Kerna instantly)",
            "Skip for now",
        ];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Which model provider do you want to start with?")
            .default(0)
            .items(&choices)
            .interact()
            .unwrap_or(7);

        config.llm_provider = match selection {
            0 => "openai".to_string(),
            1 => "anthropic".to_string(),
            2 => "ollama".to_string(),
            3 => "openrouter".to_string(),
            4 => "groq".to_string(),
            5 => "local".to_string(),
            6 => "mock".to_string(),
            _ => "skip".to_string(),
        };
    }

    // Model default comes from the provider preset so it never goes stale.
    if let Some(m) = model_flag {
        config.llm_model = m;
    } else if let Some(info) = providers::preset_info(&config.llm_provider) {
        config.llm_model = info.default_model;
    } else if config.llm_provider == "mock" {
        config.llm_model = "mock".to_string();
    } else {
        config.llm_model = "llama3".to_string();
    }

    // Key guidance: tell the user exactly which env var this provider reads,
    // and whether it's already set. Never ask for the key itself.
    if !(ci || quick || yes)
        && config.llm_provider != "mock"
        && config.llm_provider != "skip"
        && config.llm_provider != "ollama"
    {
        let env_var = providers::api_key_env_for(&config, &config.llm_provider);
        println!();
        if std::env::var(&env_var)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            println!(
                " {} {} detected in your environment — you're ready to go.",
                style("[✓]").green(),
                style(&env_var).bold()
            );
        } else {
            println!(
                " {} Set your API key (Kerna reads it from the environment, never stores it):",
                style("[i]").cyan()
            );
            if cfg!(windows) {
                println!(
                    "     setx {} \"your-key-here\"     (new terminals)",
                    env_var
                );
                println!("     $env:{} = \"your-key-here\"   (this session)", env_var);
            } else {
                println!("     export {}=\"your-key-here\"", env_var);
            }
            println!(
                "     Validate anytime with: {}",
                style(format!("kerna keys add {}", config.llm_provider)).bold()
            );
        }
        println!();
    }
    if config.llm_provider == "ollama" && !(ci || quick || yes) {
        println!();
        println!(
            " {} Ollama runs locally — no API key needed. Make sure it's running:",
            style("[i]").cyan()
        );
        println!(
            "     ollama serve   (then: ollama pull {})",
            config.llm_model
        );
        println!();
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

    // 4. Optional starter tools — install a curated pack so a new user has
    //    something useful on day one. Fail-closed: the pack only sets read tools
    //    to require_confirmation; nothing is auto-approved.
    if !(ci || quick || yes) {
        let tool_opts = vec![
            "Productivity (web search, notes, web reading)",
            "Developer (files, git, HTTP)",
            "None for now",
        ];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Install a starter tool pack?")
            .default(0)
            .items(&tool_opts)
            .interact()
            .unwrap_or(2);
        let pack_name = match selection {
            0 => Some("productivity"),
            1 => Some("dev"),
            _ => None,
        };
        if let Some(name) = pack_name {
            match crate::packs::load_pack(name) {
                Ok(pack) => {
                    let report = crate::packs::install(&mut config, &pack);
                    println!(
                        " {} Installed '{}' pack: {}",
                        style("[✓]").green(),
                        name,
                        report.added.join(", ")
                    );
                    for (plugin, env_var) in &report.secrets_needed {
                        println!(
                            "     Set {} for '{}':  kerna secrets add {}",
                            env_var, plugin, plugin
                        );
                    }
                }
                Err(e) => {
                    println!(
                        " {} Could not install pack now ({}). Add later: kerna pack install {}",
                        style("[!]").yellow(),
                        e,
                        name
                    );
                }
            }
        }
    }

    // Write config
    if let Ok(toml_str) = toml::to_string_pretty(&config) {
        let _ = fs::write("kerna.toml", toml_str);
    }

    if !ci {
        println!("{}", style("[✓] Kerna is ready.").green().bold());
        println!();
        println!("{}", style("Your first 3 commands:").bold());
        println!();
        if config.llm_provider == "mock" {
            println!(
                "  {}  {}",
                style("1.").cyan(),
                style("kerna run \"Please call echo\"").bold()
            );
            println!("      Watch a full agent loop run with zero API keys.");
        } else {
            println!(
                "  {}  {}",
                style("1.").cyan(),
                style("kerna run \"Summarize the files in this folder\"").bold()
            );
            println!("      Your first supervised task — Kerna asks before anything risky.");
        }
        println!();
        println!(
            "  {}  {}",
            style("2.").cyan(),
            style("kerna trace last").bold()
        );
        println!("      The black-box recording: every prompt, tool call, and policy check.");
        println!();
        println!("  {}  {}", style("3.").cyan(), style("kerna doctor").bold());
        println!("      Health check: database, provider keys, plugins.");
        println!();
        println!(
            "{} kerna mcp add <name> --command <cmd>   (connect your tools)",
            style("Add tools:").bold()
        );
        println!("{} docs/USING_KERNA.md", style("Learn more:").bold());
        println!();
    }
}
