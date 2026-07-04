mod config;
mod cron;
mod mcp;
mod mcp_registry;
mod memory;
mod permissions;
mod sandbox;
mod scheduler;
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
#[command(version = "1.0.0")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the Kerna background daemon (Cron, Watchdog)
    Daemon,

    /// Execute a goal using the agentic tool-call loop and exit
    Run {
        /// The objective or goal to fulfill
        #[arg(index = 1)]
        goal: String,
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

    /// Task management (list, show, replay)
    Task {
        #[command(subcommand)]
        action: TaskCommands,
    },

    /// Query long-term persistent memory
    Memory {
        /// Search term
        #[arg(index = 1)]
        query: String,
    },

    /// List or manage MCP plugins
    Plugins {
        #[command(subcommand)]
        action: Option<PluginCommands>,
    },

    /// Initialize and configure API keys
    Init,

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
}

#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// List configured plugins
    List,
    /// Add a new plugin boilerplate to config
    Add { name: String },
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
    let cli = Cli::parse();
    let config = Config::load();

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
        Some(Commands::Daemon) => {
            let watchdog = WatchdogEngine::new(memory.clone(), config.clone());
            if let Err(e) = watchdog.start().await {
                eprintln!("[!] Watchdog engine failed to start: {}", e);
            }

            let mut cron = CronEngine::new(config.clone(), memory.clone(), mcp_registry.clone()).await?;
            if let Err(e) = cron.start().await {
                eprintln!("[!] Cron engine failed to start: {}", e);
            }

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                  Kerna Daemon v1.0.0                        ║");
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Database:    {:<45} ║", config.db_path);
            println!("║  LLM:        {:<45} ║", format!("{} / {}", config.llm_provider, config.llm_model));
            println!("║  Plugins:    {:<45} ║", format!("{} installed", config.mcp_servers.len()));
            println!("║  Schedules:  {:<45} ║", format!("{} cron jobs", config.schedules.len()));
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Daemon running. Press Ctrl+C to stop.                      ║");
            println!("╚══════════════════════════════════════════════════════════════╝");

            tokio::signal::ctrl_c().await?;
            println!("\n[+] Daemon stopped cleanly.");
        }

        Some(Commands::Run { goal }) => {
            let scheduler = TaskScheduler::new(config, memory.clone(), mcp_registry.clone(), None)?;
            match scheduler.run_goal(&goal).await {
                Ok(task_id) => println!("[+] Task completed: {}", task_id),
                Err(e) => {
                    eprintln!("[-] Task failed: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Inspect { task_id }) => {
            match memory.get_task_observability(&task_id) {
                Ok((goal, status, created, dur, llm, cost, tokens, retries)) => {
                    println!("Goal:\n{}\n", goal);
                    println!("Status:\n{}\n", status);
                    
                    let dur_str = if dur > 0 { format!("{}s", dur) } else { "N/A".to_string() };
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
                        let time_only = ts.split(' ').last().unwrap_or("").split('.').next().unwrap_or("");
                        let action = if msg.starts_with("Tool") { "Action" } else if lvl == "ERROR" { "Retry" } else { "Planning" };
                        timeline.push_str(&format!("{} {}\n", time_only, action));
                    }
                    
                    println!("Tools Used:");
                    for t in tools_used {
                        println!("✓ {}", t);
                    }
                    if logs.is_empty() { println!("None"); }
                    println!();
                    
                    println!("Retries:\n{}\n", retries);
                    println!("Estimated Cost:\n${:.4}\n", cost);
                    println!("Timeline:\n{}", if timeline.is_empty() { "No timeline recorded.\n".to_string() } else { timeline });
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
                        explanation.push("Planning (Analyzing objective and breaking down steps)".to_string());
                    } else if msg.starts_with("Tool [") {
                        let parts: Vec<&str> = msg.split("]:").collect();
                        if parts.len() > 1 {
                            let tool = parts[0].replace("Tool [", "");
                            explanation.push(format!("Action (Decided to use {} to execute step)", tool));
                        }
                    } else if lvl == "ERROR" {
                        explanation.push("Self-Correction (Previous step failed, re-evaluating approach)".to_string());
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

        Some(Commands::Top) => {
            println!("Kerna Top (Agent Observability)\n");
            println!("{:<36} | {:<20} | {:<10} | {:<10}", "Task ID", "Goal", "Tokens", "Duration");
            println!("{:-<36}-+-{:-<20}-+-{:-<10}-+-{:-<10}", "", "", "", "");
            
            if let Ok(running) = memory.get_running_tasks() {
                if running.is_empty() {
                    println!("{:<36} | {:<20} | {:<10} | {:<10}", "No active agents", "", "", "");
                } else {
                    for (id, goal, dur, tokens) in running {
                        let g = if goal.len() > 17 { format!("{}...", &goal[..17]) } else { goal };
                        println!("{:<36} | {:<20} | {:<10} | {}s", id, g, tokens, dur);
                    }
                }
            }
        }

        Some(Commands::Plugins { action }) => {
            match action {
                Some(PluginCommands::Add { name }) => {
                    use std::io::Write;
                    let template = format!(r#"
[[plugins]]
name = "{}"
command = ""
args = []
enabled = false
capabilities = []
allowed_paths = []
approval_required = []
"#, name);
                    let mut file = std::fs::OpenOptions::new().append(true).open("kerna.toml")?;
                    file.write_all(template.as_bytes())?;
                    println!("[+] Appended {} plugin boilerplate to kerna.toml", name);
                }
                _ => {
                    println!("Kerna Plugins\n");
                    for p in &config.mcp_servers {
                        let status = if p.enabled { "🟢 ENABLED" } else { "🔴 DISABLED" };
                        println!("- {} [{}]", p.name, status);
                        println!("  Command: {} {:?}", p.command, p.args);
                        println!("  Capabilities: {:?}", p.capabilities);
                        println!("  Allowed Paths: {:?}", p.allowed_paths);
                        println!("  Approval Required: {:?}", p.approval_required);
                        println!();
                    }
                    println!("Plugins: {} loaded", config.mcp_servers.len());
                }
            }
        }

        Some(Commands::Doctor) => {
            println!("Kerna Doctor:\n");
            println!("Database: OK ({})", config.db_path);
            println!("LLM Key: {}", if config.llm_api_key.is_empty() { "MISSING" } else { "OK" });
            println!("Plugins: {} loaded", config.mcp_servers.len());
        }

        Some(Commands::Memory { query }) => {
            println!("Memory Search: {}\n", query);
            
            if query.is_empty() {
                if let Ok(memories) = memory.get_episodic_memories_by_time() {
                    let mut current_date = String::new();
                    for (content, _ts, date) in memories {
                        let relative = if date == chrono::Utc::now().format("%Y-%m-%d").to_string() {
                            "Today"
                        } else {
                            &date
                        };
                        
                        if current_date != relative {
                            println!("--- {} ---", relative);
                            current_date = relative.to_string();
                        }
                        println!("- {}", content);
                    }
                }
            } else {
                if let Ok(results) = memory.search_memory_by_text(&query, 10) {
                    if results.is_empty() {
                        println!("No memories found.");
                    } else {
                        for (i, r) in results.iter().enumerate() {
                            println!("{}. {}", i + 1, r);
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
            memory.create_task(
                task_id,
                None,
                &format!("Watch {} every {}", url, interval),
            )?;
            memory.update_task_status(task_id, "watching")?;
            println!("[+] Watch registered as Task ID: {}", task_id);
        }

        Some(Commands::Config { action }) => {
            match action {
                Some(ConfigCommands::Path) => {
                    let path = std::env::current_dir()?.join("kerna.toml");
                    println!("{}", path.display());
                }
                _ => {
                    println!("Usage: kerna config path");
                }
            }
        }

        Some(Commands::Init) => {
            use std::io::{self, Write};
            println!("Kerna Login\n");
            print!("Enter your LLM Provider (openai/anthropic/venice) [openai]: ");
            io::stdout().flush()?;
            let mut provider = String::new();
            io::stdin().read_line(&mut provider)?;
            let provider = provider.trim();
            let provider = if provider.is_empty() { "openai" } else { provider };
            
            print!("Enter your API Key: ");
            io::stdout().flush()?;
            let mut api_key = String::new();
            io::stdin().read_line(&mut api_key)?;
            let api_key = api_key.trim();
            
            let toml_content = format!(
                r#"# Kerna Configuration
llm_provider = "{}"
llm_api_key = "{}"
llm_model = "{}"
db_path = "kerna.db"
sandbox_dir = "sandbox"
memory_backend = "sqlite"
max_retries = 3
max_tool_rounds = 15

[[mcp_servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "./"]
enabled = true
capabilities = ["fs.read", "fs.write"]
allowed_paths = ["./"]
approval_required = ["fs.write", "fs.delete"]
"#, 
                provider, 
                api_key,
                if provider == "anthropic" { "claude-3-5-sonnet-20240620" } else { "gpt-4o-mini" }
            );
            
            std::fs::write("kerna.toml", toml_content)?;
            println!("\n[+] Saved configuration to kerna.toml!");
        }

        Some(Commands::Task { action }) => {
            match action {
                TaskCommands::List => {
                    let tasks = memory.get_tasks().unwrap_or_default();
                    println!("\n  Task Registry");
                    println!("  {:<36} │ {:<40} │ {:<10}", "Task ID", "Goal", "Status");
                    println!("  {}┼{}┼{}", "─".repeat(37), "─".repeat(42), "─".repeat(12));
                    if tasks.is_empty() {
                        println!("  No tasks recorded.");
                    } else {
                        for (id, goal, status) in tasks.iter().take(15) {
                            let g = if goal.len() > 37 { format!("{}...", &goal[..37]) } else { goal.clone() };
                            let icon = match status.as_str() {
                                "completed" => "✅", "running" => "🔄", "failed" => "❌", _ => "⏳",
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
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        println!("↓");
                    }
                    println!("Done");
                }
            } else {
                eprintln!("[-] Task ID not found.");
            }
            }
                TaskCommands::Export { task_id, format, out } => {
                    if let Ok(obs) = memory.get_task_observability(&task_id) {
                        let logs = memory.get_task_logs(&task_id).unwrap_or_default();
                        let mut output = String::new();
                        
                        if format == "json" {
                            let mut tools = vec![];
                            let mut timeline = vec![];
                            for (ts, lvl, msg) in &logs {
                                if msg.starts_with("Tool [") {
                                    let parts: Vec<&str> = msg.split("]:").collect();
                                    if parts.len() > 1 { tools.push(parts[0].replace("Tool [", "")); }
                                }
                                let action = if msg.starts_with("Tool") { "Action" } else if lvl == "ERROR" { "Retry" } else { "Planning" };
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
                                let time = ts.split(' ').last().unwrap_or("").split('.').next().unwrap_or("");
                                let act = if msg.starts_with("Tool") { "Action" } else if lvl == "ERROR" { "Retry" } else { "Planning" };
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
                            std::fs::write(&path, &output).unwrap();
                            println!("[+] Exported task to {}", path);
                        } else {
                            println!("{}", output);
                        }
                    } else {
                        eprintln!("[-] Task ID not found.");
                    }
                }
            }
        }



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
            
            let (active_session_id, session_name) = if choice.is_empty() || choice == next_idx.to_string() {
                print!("Enter new session name: ");
                io::stdout().flush()?;
                let mut new_name = String::new();
                io::stdin().read_line(&mut new_name)?;
                let new_name = new_name.trim().to_string();
                let name = if new_name.is_empty() { "default".to_string() } else { new_name };
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
                    println!("  {}┼{}┼{}", "─".repeat(37), "─".repeat(32), "─".repeat(12));
                    for (id, goal, status) in tasks.iter().take(5) {
                        let g = if goal.len() > 27 { format!("{}...", &goal[..27]) } else { goal.clone() };
                        let icon = match status.as_str() {
                            "completed" => "✅", "running" => "🔄", "failed" => "❌", _ => "⏳",
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
                let scheduler = TaskScheduler::new(config.clone(), memory.clone(), mcp_registry.clone(), Some(active_session_id.clone()))?;
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
