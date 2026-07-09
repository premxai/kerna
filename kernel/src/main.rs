pub mod budget;
mod config;
mod cron;
pub mod events;
mod gateways;
mod mcp;
mod mcp_governance;
mod mcp_registry;
mod memory;
mod mockmcp;
mod onboarding;
mod permissions;
pub mod plugin_manifest;
mod sandbox;
mod scheduler;
mod security;
mod server;
mod tool_packs;
mod watchdog;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use tokio::sync::Mutex;

use config::Config;
use cron::CronEngine;
use mcp_registry::McpRegistry;
use memory::MemoryEngine;
use scheduler::TaskScheduler;
use watchdog::WatchdogEngine;

#[derive(Parser, Debug)]
#[command(name = "kerna")]
#[command(
    about = "Kerna — The Developer Runtime for Autonomous AI Agents",
    long_about = "Kerna is the runtime for autonomous AI agents. Build them, run them, remember everything, and stay in control."
)]
#[command(version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize the Kerna runtime trust layer
    Init {
        #[arg(long)]
        quick: bool,
        #[arg(long)]
        ci: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        no_setup: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },

    /// Start the Kerna background daemon (Cron, Watchdog)
    Daemon,

    /// Start the OpenAI-compatible API Server
    Serve {
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Run the MockMCP deterministic integration test server
    Mockmcp {
        #[arg(long, default_value = "normal")]
        mode: String,
    },

    /// Execute a goal using the agentic tool-call loop and exit
    Run {
        /// The objective or goal to fulfill
        #[arg(index = 1)]
        goal: String,

        /// Enable Converse Mode to pause for user confirmation before executing tools
        #[arg(long)]
        converse: bool,

        /// Privacy routing mode (e.g. "public", "project", "private", "local-only")
        #[arg(long)]
        privacy: Option<String>,
    },

    /// Inspect a specific task's execution trace and observability metrics
    Inspect {
        /// Task ID
        #[arg(index = 1)]
        task_id: String,
    },

    /// Explain the reasoning chain for a task step-by-step
    Explain {
        /// Task ID
        #[arg(index = 1)]
        task_id: String,
    },

    /// View structured events for a specific task execution
    Trace {
        /// Task ID
        #[arg(index = 1)]
        task_id: String,
    },

    /// Task management (list, show, replay)
    Task {
        #[command(subcommand)]
        action: TaskCommands,
    },

    /// Manage or query persistent memory
    Memory {
        #[command(subcommand)]
        action: Option<MemoryCommands>,
    },

    /// List or manage MCP plugins
    Mcp {
        #[command(subcommand)]
        action: Option<McpCommands>,
    },

    /// Show the path to the current configuration file
    Config {
        #[command(subcommand)]
        action: Option<ConfigCommands>,
    },

    /// Top-like observability dashboard for AI agents
    Top,

    /// View system health and configuration
    Doctor,

    /// Watch a target continuously (Daemon must be running)
    Watch {
        #[arg(short, long)]
        url: String,

        #[arg(short, long, default_value = "5m")]
        interval: String,
    },

    /// View or test security and execution policies
    Policy {
        #[command(subcommand)]
        action: PolicyCommands,
    },

    /// Manage BYOK LLM Providers
    Provider {
        #[command(subcommand)]
        action: ProviderCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProviderCommands {
    /// Add a new provider to config
    Add {
        #[arg(index = 1)]
        name: String,
        
        #[arg(long, default_value = "openai")]
        provider_type: String,

        #[arg(long)]
        api_key_env: Option<String>,

        #[arg(long, default_value = "gpt-4o-mini")]
        default_model: String,

        #[arg(long)]
        base_url: Option<String>,
    },
    /// List configured providers
    List,
    /// Test a provider's connection
    Test {
        #[arg(index = 1)]
        name: String,
    },
    /// Manage model routing
    Route {
        #[command(subcommand)]
        action: RouteCommands,
    }
}

#[derive(Subcommand, Debug)]
pub enum RouteCommands {
    /// List all model routes
    List,
    /// Set a model route
    Set {
        #[arg(index = 1)]
        route_name: String,
        
        #[arg(index = 2)]
        target: String, // e.g. "anthropic/claude-3-5-sonnet-latest"
    }
}

#[derive(Subcommand, Debug)]
pub enum MemoryCommands {
    /// Search memory using a query
    Search {
        /// Search term
        #[arg(index = 1)]
        query: String,
    },
    /// List all staged (unapproved) memory writes
    Staged,
    /// Approve a staged memory write
    Approve {
        #[arg(index = 1)]
        id: String,
    },
    /// Reject a staged memory write
    Reject {
        #[arg(index = 1)]
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PolicyCommands {
    /// Dry-run a tool call against the current policy and workspace boundaries
    Simulate {
        /// The tool name to simulate (e.g., "run_command")
        #[arg(index = 1)]
        tool: String,

        /// The JSON arguments for the tool
        #[arg(index = 2)]
        args: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum McpCommands {
    /// List configured plugins
    List,
    /// Add a new plugin to config
    Add {
        #[arg(index = 1)]
        name: String,
        
        #[arg(index = 2)]
        command: String,
        
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Probe an MCP server for its raw capabilities
    Probe {
        #[arg(index = 1)]
        name: String,
    },
    /// Inspect an MCP server and show its raw tools
    Inspect {
        #[arg(index = 1)]
        name: String,
    },
    /// Generate a Human-readable Risk Card for an MCP server
    Risk {
        #[arg(index = 1)]
        name: String,
    },
    /// Run diagnostics on an MCP server
    Doctor {
        #[arg(index = 1)]
        name: String,
    },
    /// Enable an MCP server
    Enable {
        #[arg(index = 1)]
        name: String,
    },
    /// Disable an MCP server
    Disable {
        #[arg(index = 1)]
        name: String,
    },
    /// Manage tool filters for an MCP server
    Filter {
        #[command(subcommand)]
        action: FilterCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum FilterCommands {
    /// Add a tool to the allow list
    Allow {
        #[arg(index = 1)]
        server_name: String,
        
        #[arg(index = 2)]
        tool_name: String,
    },
    /// Add a tool to the deny list
    Deny {
        #[arg(index = 1)]
        server_name: String,
        
        #[arg(index = 2)]
        tool_name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Show the absolute path to the configuration file
    Path,
}

#[derive(Subcommand, Debug)]
enum TaskCommands {
    /// List all tasks
    List,
    /// Replay a task execution trace
    Replay { task_id: String },
    /// Export a task run
    Export {
        task_id: String,

        #[arg(long, default_value = "md")]
        format: String,

        #[arg(long)]
        out: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // We rely on the local ctrl_c wait in Daemon instead of global exit(0)

    let cli = Cli::parse();
    let mut config = Config::load();

    // Initialize Memory Engine
    let memory = Arc::new(MemoryEngine::new(&config.db_path)?);

    // Initialize MCP Registry
    let mcp_registry = Arc::new(Mutex::new(McpRegistry::new()));

    // Spawn registered MCP servers
    if !config.mcp_servers.is_empty() {
        let mut registry = mcp_registry.lock().await;
        if let Err(e) = registry.initialize(&config.mcp_servers).await {
            eprintln!("[!] Plugin initialization warning: {}", e);
        }
        drop(registry);
    }

    match cli.command {
        Some(Commands::Init {
            quick,
            ci,
            yes,
            no_setup,
            provider,
            model,
        }) => {
            onboarding::run_onboarding(quick, ci, yes, no_setup, provider, model);
        }
        Some(Commands::Daemon) => {
            let watchdog = WatchdogEngine::new(memory.clone(), config.clone());
            if let Err(e) = watchdog.start().await {
                eprintln!("[!] Watchdog engine failed to start: {}", e);
            }

            let mut cron =
                CronEngine::new(config.clone(), memory.clone(), mcp_registry.clone()).await?;
            if let Err(e) = cron.start().await {
                eprintln!("[!] Cron engine failed to start: {}", e);
            }

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                  Kerna Daemon v0.1.0                        ║");
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Database:    {:<45} ║", config.db_path);
            println!(
                "║  LLM:        {:<45} ║",
                format!("{} / {}", config.llm_provider, config.llm_model)
            );
            println!(
                "║  Plugins:    {:<45} ║",
                format!("{} installed", config.mcp_servers.len())
            );
            println!(
                "║  Schedules:  {:<45} ║",
                format!("{} cron jobs", config.schedules.len())
            );
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Daemon running. Press Ctrl+C to stop.                      ║");
            println!("╚══════════════════════════════════════════════════════════════╝");

            tokio::signal::ctrl_c().await?;
            println!("\n[+] Daemon stopped cleanly.");
        }

        Some(Commands::Serve { port }) => {
            let state = server::AppState {
                config: config.clone(),
                memory: memory.clone(),
                mcp_registry: mcp_registry.clone(),
            };
            if let Err(e) = server::start_server(state, port).await {
                eprintln!("[-] Server failed: {}", e);
            }
        }

        Some(Commands::Mockmcp { mode }) => {
            let mut server = mockmcp::MockMcpServer::new(&mode);
            if let Err(e) = server.run().await {
                eprintln!("[-] MockMCP failed: {}", e);
            }
        }

        Some(Commands::Run { goal, converse, privacy }) => {
            if converse {
                config.converse = true;
            }

            if let Some(priv_mode) = privacy {
                let route_target = match priv_mode.as_str() {
                    "public" => config.privacy_routes.get("public").map(|s| s.as_str()).unwrap_or("default"),
                    "project" => config.privacy_routes.get("project").map(|s| s.as_str()).unwrap_or("coding"),
                    "private" => config.privacy_routes.get("private").map(|s| s.as_str()).unwrap_or("private"),
                    "local-only" => "local-only",
                    _ => &priv_mode,
                };

                let target_route = if route_target == "local-only" {
                    // Enforce local provider exists
                    let has_local = config.providers.values().any(|p| p.provider_type == "openai_compatible" || p.provider_type == "local");
                    if !has_local {
                        eprintln!("No local provider configured for local-only privacy mode.\nRun: kerna provider add local --base-url http://localhost:11434/v1");
                        std::process::exit(1);
                    }
                    // For now, if local-only, we expect a 'local' provider or 'private' route to be local
                    config.model_routes.get("private").cloned().unwrap_or_else(|| "local/qwen2.5-coder".to_string())
                } else {
                    config.model_routes.get(route_target).cloned().unwrap_or_else(|| "openai/gpt-4o-mini".to_string())
                };

                // Split into provider and model
                let parts: Vec<&str> = target_route.split('/').collect();
                if parts.len() == 2 {
                    config.llm_provider = parts[0].to_string();
                    config.llm_model = parts[1].to_string();
                }
            }

            let mut final_goal = goal.clone();

            // Basic @ injection parsing
            let words: Vec<String> = final_goal
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            for word in &words {
                if let Some(path_or_url) = word.strip_prefix("@") {
                    if path_or_url.starts_with("http") {
                        if let Ok(content) = reqwest::get(path_or_url)
                            .await
                            .and_then(|r| r.error_for_status())
                        {
                            if let Ok(text) = content.text().await {
                                final_goal = final_goal.replace(
                                    word,
                                    &format!(
                                        "\n\n--- Content from {} ---\n{}\n--- End ---\n\n",
                                        path_or_url, text
                                    ),
                                );
                            }
                        }
                    } else if std::path::Path::new(path_or_url).exists() {
                        if let Ok(text) = std::fs::read_to_string(path_or_url) {
                            final_goal = final_goal.replace(
                                word,
                                &format!(
                                    "\n\n--- Content from {} ---\n{}\n--- End ---\n\n",
                                    path_or_url, text
                                ),
                            );
                        }
                    }
                }
            }

            let scheduler = TaskScheduler::new(config, memory.clone(), mcp_registry.clone(), None)?;
            match scheduler.run_goal(&final_goal).await {
                Ok(task_id) => println!("[+] Task completed: {}", task_id),
                Err(e) => {
                    eprintln!("[-] Task failed: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Inspect { task_id }) => {
            match memory.get_task_observability(&task_id) {
                Ok((goal, status, _created, dur, llm, cost, _tokens, retries)) => {
                    println!("Goal:\n{}\n", goal);
                    println!("Status:\n{}\n", status);

                    let dur_str = if dur > 0 {
                        format!("{}s", dur)
                    } else {
                        "N/A".to_string()
                    };
                    println!("Duration:\n{}\n", dur_str);

                    println!("LLM:\n{}\n", if llm.is_empty() { "Unknown" } else { &llm });

                    // Count tools used from logs
                    let logs = memory.get_task_logs(&task_id).unwrap_or_default();
                    let mut tools_used = std::collections::HashSet::new();
                    let mut timeline = String::new();

                    for (ts, lvl, msg) in &logs {
                        if msg.starts_with("Tool [") {
                            let parts: Vec<&str> = msg.split("]:").collect();
                            if parts.len() > 1 {
                                let t_name = parts[0].replace("Tool [", "");
                                tools_used.insert(t_name);
                            }
                        }
                        // Simple timeline extraction (hh:mm:ss)
                        let time_only = ts
                            .split(' ')
                            .next_back()
                            .unwrap_or("")
                            .split('.')
                            .next()
                            .unwrap_or("");
                        let action = if msg.starts_with("Tool") {
                            "Action"
                        } else if lvl == "ERROR" {
                            "Retry"
                        } else {
                            "Planning"
                        };
                        timeline.push_str(&format!("{} {}\n", time_only, action));
                    }

                    println!("Tools Used:");
                    for t in tools_used {
                        println!("✓ {}", t);
                    }
                    if logs.is_empty() {
                        println!("None");
                    }
                    println!();

                    println!("Retries:\n{}\n", retries);
                    println!("Estimated Cost:\n${:.4}\n", cost);
                    println!(
                        "Timeline:\n{}",
                        if timeline.is_empty() {
                            "No timeline recorded.\n".to_string()
                        } else {
                            timeline
                        }
                    );
                }
                Err(_) => {
                    eprintln!("[-] Task ID not found: {}", task_id);
                }
            }
        }

        Some(Commands::Explain { task_id }) => {
            println!("Reasoning Chain for Task {}:\n", task_id);
            if let Ok(logs) = memory.get_task_logs(&task_id) {
                if logs.is_empty() {
                    println!("No logs found for this task.");
                    return Ok(());
                }

                let mut explanation = vec!["Goal".to_string()];

                for (_ts, lvl, msg) in logs {
                    if msg.starts_with("Received goal:") {
                        explanation.push(
                            "Planning (Analyzing objective and breaking down steps)".to_string(),
                        );
                    } else if msg.starts_with("Tool [") {
                        let parts: Vec<&str> = msg.split("]:").collect();
                        if parts.len() > 1 {
                            let tool = parts[0].replace("Tool [", "");
                            explanation
                                .push(format!("Action (Decided to use {} to execute step)", tool));
                        }
                    } else if lvl == "ERROR" {
                        explanation.push(
                            "Self-Correction (Previous step failed, re-evaluating approach)"
                                .to_string(),
                        );
                    }
                }
                explanation.push("Final Answer".to_string());

                for (i, step) in explanation.iter().enumerate() {
                    println!("{}", step);
                    if i < explanation.len() - 1 {
                        println!("↓");
                    }
                }
            } else {
                eprintln!("[-] Task ID not found: {}", task_id);
            }
        }

        Some(Commands::Trace { task_id }) => {
            println!("Event Trace for Task {}:\n", task_id);
            if let Ok(events) = memory.get_events(&task_id) {
                if events.is_empty() {
                    println!("No events found for this task.");
                    return Ok(());
                }

                println!(
                    "{:<4} | {:<24} | {:<22} | {:<10} | {:<7} | Details",
                    "Seq", "Timestamp", "Event Type", "Actor", "Level"
                );
                println!(
                    "{:-<4}-+-{:-<24}-+-{:-<22}-+-{:-<10}-+-{:-<7}-+-{:-<40}",
                    "", "", "", "", "", ""
                );

                for ev in events {
                    let ts: String = ev.timestamp.chars().take(24).collect();
                    let payload = serde_json::to_string(&ev.payload_json).unwrap_or_default();
                    let display_payload = if payload.chars().count() > 40 {
                        let truncated: String = payload.chars().take(37).collect();
                        format!("{}...", truncated)
                    } else {
                        payload
                    };

                    println!(
                        "{:<4} | {:<24} | {:<22} | {:<10} | {:<7} | {}",
                        ev.sequence, ts, ev.event_type, ev.actor, ev.severity, display_payload
                    );
                }
            } else {
                eprintln!(
                    "[-] Task ID not found or error loading events for: {}",
                    task_id
                );
            }
        }

        Some(Commands::Top) => {
            println!("Kerna Top (Agent Observability)\n");
            println!(
                "{:<36} | {:<20} | {:<10} | {:<10}",
                "Task ID", "Goal", "Tokens", "Duration"
            );
            println!("{:-<36}-+-{:-<20}-+-{:-<10}-+-{:-<10}", "", "", "", "");

            if let Ok(running) = memory.get_running_tasks() {
                if running.is_empty() {
                    println!(
                        "{:<36} | {:<20} | {:<10} | {:<10}",
                        "No active agents", "", "", ""
                    );
                } else {
                    for (id, goal, dur, tokens) in running {
                        let g = if goal.chars().count() > 17 {
                            let truncated: String = goal.chars().take(17).collect();
                            format!("{}...", truncated)
                        } else {
                            goal
                        };
                        println!("{:<36} | {:<20} | {:<10} | {}s", id, g, tokens, dur);
                    }
                }
            }
        }

        Some(Commands::Mcp { action }) => {
            match action {
                Some(McpCommands::Add { name, command, args }) => {
                    if config.mcp_servers.iter().any(|s| s.name == name) {
                        eprintln!("[-] MCP server '{}' already exists.", name);
                        std::process::exit(1);
                    }
                    let server = config::McpServerConfig {
                        name: name.clone(),
                        command,
                        args,
                        enabled: true, // Auto-enable on add
                        capabilities: vec![],
                        allowed_paths: vec![],
                        approval_required: vec![],
                        allow_tools: vec![],
                        deny_tools: vec![],
                        runtime_mode: "native".to_string(),
                        docker_image: "ubuntu:latest".to_string(),
                    };
                    config.mcp_servers.push(server);
                    config.save();
                    println!("[+] Added and enabled MCP server '{}'", name);
                }
                Some(McpCommands::Enable { name }) => {
                    if let Some(server) = config.mcp_servers.iter_mut().find(|s| s.name == name) {
                        server.enabled = true;
                        config.save();
                        println!("[+] Enabled MCP server '{}'", name);
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::Disable { name }) => {
                    if let Some(server) = config.mcp_servers.iter_mut().find(|s| s.name == name) {
                        server.enabled = false;
                        config.save();
                        println!("[+] Disabled MCP server '{}'", name);
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::Filter { action: filter_action }) => match filter_action {
                    FilterCommands::Allow { server_name, tool_name } => {
                        if let Some(server) = config.mcp_servers.iter_mut().find(|s| s.name == server_name) {
                            if !server.allow_tools.contains(&tool_name) {
                                server.allow_tools.push(tool_name.clone());
                                config.save();
                                println!("[+] Added '{}' to allow_tools for '{}'", tool_name, server_name);
                            } else {
                                println!("[-] '{}' is already in allow_tools for '{}'", tool_name, server_name);
                            }
                        } else {
                            eprintln!("[-] MCP server '{}' not found.", server_name);
                        }
                    }
                    FilterCommands::Deny { server_name, tool_name } => {
                        if let Some(server) = config.mcp_servers.iter_mut().find(|s| s.name == server_name) {
                            if !server.deny_tools.contains(&tool_name) {
                                server.deny_tools.push(tool_name.clone());
                                config.save();
                                println!("[+] Added '{}' to deny_tools for '{}'", tool_name, server_name);
                            } else {
                                println!("[-] '{}' is already in deny_tools for '{}'", tool_name, server_name);
                            }
                        } else {
                            eprintln!("[-] MCP server '{}' not found.", server_name);
                        }
                    }
                },
                Some(McpCommands::Probe { name }) => {
                    if let Some(server) = config.mcp_servers.iter().find(|s| s.name == name) {
                        let _ = mcp_governance::probe(server).await;
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::Inspect { name }) => {
                    if let Some(server) = config.mcp_servers.iter().find(|s| s.name == name) {
                        let _ = mcp_governance::inspect(server).await;
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::Risk { name }) => {
                    if let Some(server) = config.mcp_servers.iter().find(|s| s.name == name) {
                        let _ = mcp_governance::generate_risk_card(server).await;
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::Doctor { name }) => {
                    if let Some(server) = config.mcp_servers.iter().find(|s| s.name == name) {
                        println!("Doctoring MCP Server: {}", server.name);
                        let cmd_exists = std::path::Path::new(&server.command).exists() || {
                            let checker = if cfg!(target_os = "windows") { "where" } else { "which" };
                            std::process::Command::new(checker)
                                .arg(&server.command)
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false)
                        };
                        println!("  Command exists: {}", if cmd_exists { "\x1b[32mOK\x1b[0m" } else { "\x1b[31mMISSING\x1b[0m" });
                        println!("  Capabilities defined: {}", server.capabilities.len());
                        println!("  Allowed paths defined: {}", server.allowed_paths.len());
                        println!("\n  To test transport and list tools, run `kerna mcp probe {}`", server.name);
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                None | Some(McpCommands::List) => {
                    println!("Kerna MCP Servers\n");
                    for p in &config.mcp_servers {
                        let status = if p.enabled {
                            "🟢 ENABLED"
                        } else {
                            "🔴 DISABLED"
                        };
                        println!("- {} [{}]", p.name, status);
                        println!("  Command: {} {:?}", p.command, p.args);
                        if !p.allow_tools.is_empty() {
                            println!("  Allow Tools: {:?}", p.allow_tools);
                        }
                        if !p.deny_tools.is_empty() {
                            println!("  Deny Tools: {:?}", p.deny_tools);
                        }
                        println!();
                    }
                    println!("Plugins: {} loaded", config.mcp_servers.len());
                }
            }
        }

        Some(Commands::Doctor) => {
            println!("Kerna Doctor:\n");

            match rusqlite::Connection::open(&config.db_path) {
                Ok(conn) => {
                    if conn.query_row("SELECT 1", [], |_| Ok(())).is_ok() {
                        println!("Database: OK ({})", config.db_path);
                    } else {
                        println!("Database: ERROR (Cannot query database)");
                    }
                }
                Err(e) => println!("Database: ERROR ({})", e),
            }

            println!(
                "LLM Key: {}",
                if config.llm_api_key.is_empty() {
                    "MISSING"
                } else {
                    "OK"
                }
            );

            let mut valid_plugins = 0;
            for server in &config.mcp_servers {
                let cmd_exists = if std::path::Path::new(&server.command).exists() {
                    true
                } else {
                    let checker = if cfg!(target_os = "windows") {
                        "where"
                    } else {
                        "which"
                    };
                    std::process::Command::new(checker)
                        .arg(&server.command)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };

                if cmd_exists {
                    valid_plugins += 1;
                } else {
                    println!(
                        "Plugin Warning: Command '{}' for '{}' not found in PATH",
                        server.command, server.name
                    );
                }
            }
            println!(
                "Plugins: {}/{} loaded and executable",
                valid_plugins,
                config.mcp_servers.len()
            );
        }

        Some(Commands::Policy { action }) => {
            match action {
                PolicyCommands::Simulate { tool, args } => {
                    let permissions = permissions::PermissionManager::new(config.clone());
                    let sandbox = sandbox::ProcessSandbox::new(
                        &config.sandbox_dir,
                        config.runtime_mode.clone(),
                        config.allow_dynamic_installs,
                        config.network_mode.clone(),
                        config.egress_proxy.clone(),
                    )?;
                    // Initialize registry to check MCP filters
                    let mut registry = crate::mcp_registry::McpRegistry::new();
                    let _ = registry.initialize(&config.mcp_servers).await;
                    
                    let mut is_allowed = true;
                    let mut reasons = vec![];
                    
                    // 1. Check MCP Fast-Path filters first
                    let mcp_err = if registry.has_tool(&tool) {
                        // Pass dummy args since we only care about the routing filters
                        let res = registry.call_tool(&tool, serde_json::Value::Null).await;
                        if let Err(e) = res {
                            let e_str = e.to_string();
                            if e_str.contains("Policy Violation") || e_str.contains("does not have capability") {
                                Some(e_str)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(e) = mcp_err {
                        is_allowed = false;
                        reasons.push(format!("\x1b[31mMCP Plugin Filter\x1b[0m: {}", e));
                    }

                    // 2. Check Sandbox / Global Policy
                    match sandbox.simulate_command(&tool, &args, &permissions) {
                        Ok(decision) => {
                            if !decision.is_allowed {
                                is_allowed = false;
                            }
                            for r in decision.reasons {
                                if r.contains("Deny") || r.contains("RequireConfirmation") || r.contains("deny") {
                                    reasons.push(format!("\x1b[33mGlobal Policy\x1b[0m: {}", r));
                                } else {
                                    reasons.push(format!("\x1b[32mGlobal Policy\x1b[0m: {}", r));
                                }
                            }
                        }
                        Err(e) => {
                            is_allowed = false;
                            reasons.push(format!("\x1b[31mSandbox Error\x1b[0m: {}", e));
                        }
                    }

                    println!("============================================================");
                    println!("  Policy Simulation: {}", tool);
                    if is_allowed {
                        println!("  Final Decision: \x1b[1;32mALLOW\x1b[0m");
                    } else {
                        println!("  Final Decision: \x1b[1;31mDENY\x1b[0m");
                    }
                    println!("============================================================\n");
                    
                    if !reasons.is_empty() {
                        println!("Evaluation Trace:");
                        for reason in reasons {
                            println!("  - {}", reason);
                        }
                        println!();
                    }
                }
            }
        }
        Some(Commands::Memory { action }) => {
            match action {
                Some(MemoryCommands::Staged) => {
                    println!("Staged Memory Proposals:\n");
                    if let Ok(memories) = memory.get_staged_memories() {
                        if memories.is_empty() {
                            println!("No staged memories pending approval.");
                        } else {
                            for (id, content, date) in memories {
                                println!("[ID: {}] [{}]", id, date);
                                println!("  {}\n", content);
                            }
                            println!("Use `kerna memory approve <id>` or `kerna memory reject <id>`");
                        }
                    } else {
                        eprintln!("[-] Failed to read staged memories.");
                    }
                }
                Some(MemoryCommands::Approve { id }) => {
                    if let Err(e) = memory.approve_memory(&id) {
                        eprintln!("[-] Failed to approve memory: {}", e);
                    } else {
                        println!("[+] Memory {} approved and committed.", id);
                    }
                }
                Some(MemoryCommands::Reject { id }) => {
                    if let Err(e) = memory.reject_memory(&id) {
                        eprintln!("[-] Failed to reject memory: {}", e);
                    } else {
                        println!("[+] Memory {} rejected and deleted.", id);
                    }
                }
                Some(MemoryCommands::Search { query }) => {
                    println!("Memory Search: {}\n", query);
                    if let Ok(results) = memory.search_memory_by_text(&query, 10) {
                        if results.is_empty() {
                            println!("No results found.");
                        } else {
                            for r in results {
                                println!("- {}", r);
                            }
                        }
                    }
                }
                None => {
                    if let Ok(memories) = memory.get_episodic_memories_by_time() {
                        if memories.is_empty() {
                            println!("Memory is empty.");
                        } else {
                            let mut current_date = String::new();
                            for (content, _ts, date) in memories {
                                let relative = if date == chrono::Utc::now().format("%Y-%m-%d").to_string()
                                {
                                    "Today"
                                } else {
                                    &date
                                };
                                if relative != current_date {
                                    println!("\n## {}", relative);
                                    current_date = relative.to_string();
                                }
                                println!("- {}", content);
                            }
                        }
                    }
                }
            }
        }

        Some(Commands::Watch { url, interval }) => {
            println!("[*] Watchdog mode: monitoring {}", url);
            println!("[*] Check interval: {}", interval);
            println!("[!] Watchdog requires the daemon to be running (kerna daemon).");

            let task_id = uuid::Uuid::new_v4();
            memory.create_task(task_id, None, &format!("Watch {} every {}", url, interval))?;
            memory.update_task_status(task_id, "watching")?;
            println!("[+] Watch registered as Task ID: {}", task_id);
        }

        Some(Commands::Provider { action }) => {
            match action {
                ProviderCommands::Add { name, provider_type, api_key_env, default_model, base_url } => {
                    let provider = config::ProviderConfig {
                        provider_type,
                        api_key_env,
                        default_model,
                        base_url,
                    };
                    config.providers.insert(name.clone(), provider);
                    config.save();
                    println!("[+] Provider '{}' added successfully.", name);
                }
                ProviderCommands::List => {
                    println!("Configured Providers:\n");
                    for (name, p) in &config.providers {
                        println!("- {} (type: {}, default_model: {})", name, p.provider_type, p.default_model);
                    }
                    if config.providers.is_empty() {
                        println!("No providers configured.");
                    }
                }
                ProviderCommands::Test { name } => {
                    if let Some(p) = config.providers.get(&name) {
                        println!("Testing provider '{}'...", name);
                        println!("  Type: {}", p.provider_type);
                        if let Some(env_var) = &p.api_key_env {
                            if std::env::var(env_var).is_ok() {
                                println!("  Key: Found in {}", env_var);
                            } else {
                                println!("  Key: \x1b[31mMISSING\x1b[0m ({})", env_var);
                            }
                        }
                        println!("[+] Simulation: Connection successful.");
                    } else {
                        eprintln!("[-] Provider '{}' not found.", name);
                    }
                }
                ProviderCommands::Route { action: route_action } => {
                    match route_action {
                        RouteCommands::List => {
                            println!("Model Routes:\n");
                            for (route, target) in &config.model_routes {
                                println!("- {}: {}", route, target);
                            }
                            if config.model_routes.is_empty() {
                                println!("No model routes configured.");
                            }
                        }
                        RouteCommands::Set { route_name, target } => {
                            config.model_routes.insert(route_name.clone(), target.clone());
                            config.save();
                            println!("[+] Route '{}' set to '{}'", route_name, target);
                        }
                    }
                }
            }
        }

        Some(Commands::Config { action }) => match action {
            Some(ConfigCommands::Path) => {
                let path = std::env::current_dir()?.join("kerna.toml");
                println!("{}", path.display());
            }
            _ => {
                println!("Usage: kerna config path");
            }
        },

        Some(Commands::Task { action }) => match action {
            TaskCommands::List => {
                let tasks = memory.get_tasks().unwrap_or_default();
                println!("\n  Task Registry");
                println!("  {:<36} │ {:<40} │ {:<10}", "Task ID", "Goal", "Status");
                println!("  {}┼{}┼{}", "─".repeat(37), "─".repeat(42), "─".repeat(12));
                if tasks.is_empty() {
                    println!("  No tasks recorded.");
                } else {
                    for (id, goal, status) in tasks.iter().take(15) {
                        let g = if goal.chars().count() > 37 {
                            let truncated: String = goal.chars().take(37).collect();
                            format!("{}...", truncated)
                        } else {
                            goal.clone()
                        };
                        let icon = match status.as_str() {
                            "completed" => "✅",
                            "running" => "🔄",
                            "failed" => "❌",
                            _ => "⏳",
                        };
                        println!("  {:<36} │ {:<40} │ {} {}", id, g, icon, status);
                    }
                }
                println!();
            }
            TaskCommands::Replay { task_id } => {
                println!("Replaying Task: {}\n", task_id);
                if let Ok(logs) = memory.get_task_logs(&task_id) {
                    if logs.is_empty() {
                        println!("No logs to replay.");
                    } else {
                        for (_ts, _lvl, msg) in logs {
                            let display = if msg.starts_with("Received goal") {
                                "Planning..."
                            } else if msg.starts_with("Tool [web") {
                                "Browser..."
                            } else if msg.starts_with("Tool [fs") {
                                "Filesystem..."
                            } else if msg.starts_with("Tool [run_command") {
                                "Terminal..."
                            } else {
                                "Reasoning..."
                            };
                            println!("{}", display);
                            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
                            println!("↓");
                        }
                        println!("Done");
                    }
                } else {
                    eprintln!("[-] Task ID not found.");
                }
            }
            TaskCommands::Export {
                task_id,
                format,
                out,
            } => {
                if let Ok(obs) = memory.get_task_observability(&task_id) {
                    let logs = memory.get_task_logs(&task_id).unwrap_or_default();
                    let mut output = String::new();

                    if format == "json" {
                        let mut tools = vec![];
                        let mut timeline = vec![];
                        for (ts, lvl, msg) in &logs {
                            if msg.starts_with("Tool [") {
                                let parts: Vec<&str> = msg.split("]:").collect();
                                if parts.len() > 1 {
                                    tools.push(parts[0].replace("Tool [", ""));
                                }
                            }
                            let action = if msg.starts_with("Tool") {
                                "Action"
                            } else if lvl == "ERROR" {
                                "Retry"
                            } else {
                                "Planning"
                            };
                            timeline.push(format!("{} {}", ts, action));
                        }

                        let json_dump = serde_json::json!({
                            "task_id": task_id,
                            "goal": obs.0,
                            "status": obs.1,
                            "started_at": obs.2,
                            "duration_ms": obs.3 * 1000,
                            "model": obs.4,
                            "tokens": { "input": 0, "output": 0, "total": obs.6 },
                            "estimated_cost_usd": obs.5,
                            "tools_used": tools,
                            "permission_decisions": [],
                            "retries": obs.7,
                            "memory_retrieved": [],
                            "timeline": timeline,
                            "final_output": "",
                            "artifacts": []
                        });
                        output = serde_json::to_string_pretty(&json_dump).unwrap();
                    } else {
                        output.push_str("# Kerna Task Export\n\n");
                        output.push_str(&format!("## Goal\n{}\n\n", obs.0));
                        output.push_str("## Summary\n");
                        output.push_str(&format!("- Status: {}\n", obs.1));
                        output.push_str(&format!("- Duration: {}s\n", obs.3));
                        output.push_str(&format!("- Model: {}\n", obs.4));
                        output.push_str(&format!("- Cost: ${:.4}\n", obs.5));
                        output.push_str(&format!("- Tokens: {}\n", obs.6));
                        output.push_str(&format!("- Retries: {}\n\n", obs.7));

                        output.push_str("## Timeline\n");
                        for (ts, lvl, msg) in &logs {
                            let time = ts
                                .split(' ')
                                .next_back()
                                .unwrap_or("")
                                .split('.')
                                .next()
                                .unwrap_or("");
                            let act = if msg.starts_with("Tool") {
                                "Action"
                            } else if lvl == "ERROR" {
                                "Retry"
                            } else {
                                "Planning"
                            };
                            output.push_str(&format!("- {} {}\n", time, act));
                        }

                        output.push_str("\n## Permission Decisions\nNone recorded.\n\n");
                        output.push_str("## Memory Retrieved\nNone recorded.\n\n");
                        output.push_str("## Final Output\n");
                        if let Some((_, _, final_msg)) = logs.last() {
                            output.push_str(&format!("{}\n\n", final_msg));
                        }
                        output.push_str("## Raw Logs\n```\n");
                        for (ts, lvl, msg) in &logs {
                            output.push_str(&format!("[{}] {} {}\n", ts, lvl, msg));
                        }
                        output.push_str("```\n");
                    }

                    if let Some(path) = out {
                        if let Err(e) = std::fs::write(&path, &output) {
                            eprintln!("[-] Failed to export task: {}", e);
                        } else {
                            println!("[+] Exported task to {}", path);
                        }
                    } else {
                        println!("{}", output);
                    }
                } else {
                    eprintln!("[-] Task ID not found.");
                }
            }
        },

        None => {
            use std::io::{self, Write};

            // Session Prompt
            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                  Kerna Developer Runtime                     ║");
            println!("╠══════════════════════════════════════════════════════════════╣");

            let recent = memory.get_recent_sessions().unwrap_or_default();
            println!("║  Recent Sessions:                                            ║");
            let mut session_map = std::collections::HashMap::new();

            for (i, (id, name)) in recent.iter().enumerate() {
                println!("║  {}) {:<55}║", i + 1, name);
                session_map.insert((i + 1).to_string(), (id.clone(), name.clone()));
            }
            let next_idx = recent.len() + 1;
            println!("║  {}) {:<55}║", next_idx, "New Session");
            println!("╚══════════════════════════════════════════════════════════════╝\n");

            print!("Choose session [{}]: ", next_idx);
            io::stdout().flush()?;
            let mut choice = String::new();
            io::stdin().read_line(&mut choice)?;
            let choice = choice.trim();

            let (active_session_id, session_name) =
                if choice.is_empty() || choice == next_idx.to_string() {
                    print!("Enter new session name: ");
                    io::stdout().flush()?;
                    let mut new_name = String::new();
                    io::stdin().read_line(&mut new_name)?;
                    let new_name = new_name.trim().to_string();
                    let name = if new_name.is_empty() {
                        "default".to_string()
                    } else {
                        new_name
                    };
                    let sid = memory.create_session(&name).unwrap_or_default();
                    (sid, name)
                } else if let Some((sid, name)) = session_map.get(choice) {
                    (sid.clone(), name.clone())
                } else {
                    let sid = memory.create_session("default").unwrap_or_default();
                    (sid, "default".to_string())
                };

            println!("\n[+] Resumed session: {}\n", session_name);

            loop {
                print!("> ");
                io::stdout().flush()?;

                let mut input = String::new();
                if io::stdin().read_line(&mut input).is_err() {
                    break;
                }

                let input = input.trim();
                if input.is_empty() {
                    continue;
                }

                if input.eq_ignore_ascii_case("/exit") || input.eq_ignore_ascii_case("/quit") {
                    println!("Goodbye!");
                    break;
                }

                if input.eq_ignore_ascii_case("/clear") {
                    print!("{esc}[2J{esc}[1;1H", esc = 27 as char);
                    continue;
                }

                if input.eq_ignore_ascii_case("/help") {
                    println!("\nKerna Commands:");
                    println!("  /help                 - Show this help message");
                    println!("  /status               - View running and completed tasks");
                    println!("  /memory <query>       - Search episodic memory");
                    println!("  /plugins              - List installed plugins");
                    println!("  /clear                - Clear the screen");
                    println!("  /exit, /quit          - Quit the terminal");
                    println!("  <any text>            - Dispatch as a goal for the agent");
                    println!();
                    continue;
                }

                if input.eq_ignore_ascii_case("/plugins") {
                    println!("\nInstalled Plugins:");
                    for srv in &config.mcp_servers {
                        println!("✓ {}", srv.name);
                    }
                    if config.mcp_servers.is_empty() {
                        println!("No plugins loaded.");
                    }
                    println!();
                    continue;
                }

                if input.eq_ignore_ascii_case("/status") {
                    let tasks = memory.get_tasks().unwrap_or_default();
                    println!("\n  Task Registry");
                    println!("  {:<36} │ {:<30} │ {:<10}", "Task ID", "Goal", "Status");
                    println!("  {}┬{}┬{}", "─".repeat(37), "─".repeat(32), "─".repeat(12));
                    for (id, goal, status) in tasks.iter().take(5) {
                        let g = if goal.chars().count() > 27 {
                            let truncated: String = goal.chars().take(27).collect();
                            format!("{}...", truncated)
                        } else {
                            goal.clone()
                        };
                        let icon = match status.as_str() {
                            "completed" => "✅",
                            "running" => "🔄",
                            "failed" => "❌",
                            _ => "⏳",
                        };
                        println!("  {:<36} │ {:<30} │ {} {}", id, g, icon, status);
                    }
                    println!();
                    continue;
                }

                if input.to_lowercase().starts_with("/memory") {
                    let parts: Vec<&str> = input.splitn(2, ' ').collect();
                    if parts.len() < 2 {
                        println!("Usage: /memory <search term>\n");
                        continue;
                    }
                    println!("\n[*] Searching memory for '{}'...\n", parts[1]);
                    println!("Today:");
                    let mock_query_embedding = vec![0.1, 0.2, 0.3];
                    if let Ok(results) = memory.search_episodic_memory(&mock_query_embedding, 3) {
                        for (content, _) in &results {
                            println!("  - {}", content);
                        }
                    }
                    println!();
                    continue;
                }

                // Execute goal
                let scheduler = match TaskScheduler::new(
                    config.clone(),
                    memory.clone(),
                    mcp_registry.clone(),
                    Some(active_session_id.clone()),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[-] Failed to initialize scheduler: {}", e);
                        continue;
                    }
                };
                match scheduler.run_goal(input).await {
                    Ok(task_id) => println!("\n[+] Goal achieved! Task ID: {}", task_id),
                    Err(e) => eprintln!("\n[-] Goal failed: {}", e),
                }
                println!();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod trust_layer_validation;
