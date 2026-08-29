pub mod budget;
mod client;
mod config;
mod contract;
mod cron;
pub mod embeddings;
pub mod events;
pub mod folders;
mod gateway;
mod gateways;
mod mcp;
mod mcp_governance;
mod mcp_registry;
mod memory;
mod mockmcp;
mod models;
mod onboarding;
mod packs;
mod permissions;
pub mod plugin_manifest;
pub mod providers;
mod registry;
mod sandbox;
mod scheduler;
mod security;
mod server;
mod tool_packs;
mod watchdog;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc};
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
#[command(version)]
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

    /// Create a deterministic, reviewable governed-MCP starter contract
    Contract {
        #[command(subcommand)]
        action: ContractCommands,
    },

    /// Print a client MCP configuration; never edits client settings itself
    Client {
        #[command(subcommand)]
        action: ClientCommands,
    },

    /// Start the Kerna background daemon (Cron, Watchdog)
    Daemon,

    /// Start the OpenAI-compatible API Server
    Serve {
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Address to bind. Defaults to loopback; use 0.0.0.0 to expose on the
        /// network (requires --token).
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Bearer token required on requests. Mandatory when binding a non-loopback address.
        #[arg(long)]
        token: Option<String>,
    },

    /// Open a local live dashboard for governed MCP sessions and model routing
    Dashboard {
        /// Contract workspace containing kerna.toml
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Loopback port for the dashboard
        #[arg(long, default_value = "8765")]
        port: u16,
        /// Do not open the local dashboard in a browser automatically
        #[arg(long)]
        no_open: bool,
    },

    /// Run as an MCP server that proxies configured MCP servers through Kerna's
    /// policy engine and event log (point Claude Code / Cursor / Cline at this).
    Gateway {
        /// Contract directory containing kerna.toml. Avoids shell wrappers in
        /// IDE MCP configuration and is required when the client has no cwd option.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },

    /// Run the MockMCP deterministic integration test server
    Mockmcp {
        #[arg(index = 1)]
        action: Option<String>,

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

        /// Rung 1: record every policy decision and enforce none of them. Nothing is
        /// denied and nothing prompts; each action the policy would have stopped is
        /// printed and written to the audit trail. Use this to size a policy before
        /// trusting it -- and never leave it on believing you are protected.
        #[arg(long, conflicts_with = "converse")]
        audit: bool,

        /// Refuse approval-required calls instead of prompting on stdin. Use
        /// this for desktop, daemon, or other detached invocations.
        #[arg(long)]
        non_interactive: bool,

        /// Queue approval-required actions in the local SQLite ledger for the
        /// desktop control surface instead of reading from stdin.
        #[arg(long, conflicts_with = "non_interactive")]
        approval_queue: bool,

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
    Doctor {
        /// Include readiness checks for `kerna gateway`.
        #[arg(long)]
        gateway: bool,
    },

    /// Show the active containment, policy, and approval state for this project
    Status,

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

    /// Inspect the pinned local-model registry and verify local runtimes
    Models {
        #[command(subcommand)]
        action: ModelCommands,
    },

    /// Manage LLM API keys (guided setup; keys live in environment variables)
    Keys {
        #[command(subcommand)]
        action: KeysCommands,
    },

    /// Manage plugin secrets (guided setup; secrets live in environment variables)
    Secrets {
        #[command(subcommand)]
        action: SecretsCommands,
    },

    /// Install curated tool packs (e.g. productivity, dev)
    Pack {
        #[command(subcommand)]
        action: PackCommands,
    },

    /// Schedule recurring agent routines (daily digest, etc.) run by the daemon
    Routine {
        #[command(subcommand)]
        action: RoutineCommands,
    },

    /// Inspect or decide pending local approval requests
    #[command(name = "approvals", visible_alias = "approval")]
    Approval {
        #[command(subcommand)]
        action: ApprovalCommands,
    },

    /// Browse and install plugins from the registry
    Plugins {
        #[command(subcommand)]
        action: PluginsCommands,
    },

    /// Grant, list, or revoke real-filesystem folder access (outside the sandbox)
    Folders {
        #[command(subcommand)]
        action: FoldersCommands,
    },

    /// Set, list, or remove your communication-style preferences (explicit only —
    /// nothing is inferred; injected into every task's context once set)
    Preferences {
        #[command(subcommand)]
        action: PreferencesCommands,
    },

    /// Connect a messaging channel (Telegram) so allowlisted people can trigger
    /// governed agent runs by messaging your bot. Runs while `kerna daemon` is up.
    Channel {
        #[command(subcommand)]
        action: ChannelCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ContractCommands {
    /// Write a contract template without overwriting existing files
    Init {
        /// Template name (currently: deployment-assistant)
        #[arg(long, default_value = "deployment-assistant")]
        template: String,
        /// Human-readable contract name
        #[arg(long)]
        name: String,
        /// Directory that will receive kerna.toml and agent-contract.md
        #[arg(long, default_value = ".")]
        output: std::path::PathBuf,
        /// Omit the bundled demo server, leaving a contract that exposes no tools
        #[arg(long)]
        no_demo_server: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ClientCommands {
    /// Print a workspace-scoped MCP configuration for an IDE/client
    Config {
        /// qoder, claude-code, codex, or generic
        #[arg(long)]
        client: String,
        /// Contract workspace containing kerna.toml
        #[arg(long, default_value = ".")]
        workspace: std::path::PathBuf,
    },
    /// Validate a generated adapter and perform an MCP initialize/tools-list handshake
    Doctor {
        /// codex, claude-code, qoder, or generic
        #[arg(long)]
        client: String,
        /// Contract workspace containing kerna.toml
        #[arg(long, default_value = ".")]
        workspace: std::path::PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum ChannelCommands {
    /// Add a channel. e.g. `kerna channel add telegram --token-env TELEGRAM_BOT_TOKEN --allow-id 12345`
    Add {
        /// Platform: telegram
        #[arg(index = 1)]
        platform: String,
        /// Env var name holding the bot token (never the token itself)
        #[arg(long, default_value = "TELEGRAM_BOT_TOKEN")]
        token_env: String,
        /// A sender/chat id allowed to trigger runs (repeatable)
        #[arg(long = "allow-id")]
        allow_id: Vec<String>,
        /// Friendly name for this channel
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// List configured channels
    List,
    /// Allow another sender/chat id on an existing channel
    Allow {
        #[arg(index = 1)]
        name: String,
        #[arg(index = 2)]
        id: String,
    },
    /// Remove a channel
    Remove {
        #[arg(index = 1)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PreferencesCommands {
    /// Set a preference, e.g. `kerna preferences set tone concise`
    Set {
        #[arg(index = 1)]
        key: String,
        #[arg(index = 2)]
        value: String,
    },
    /// List your current preferences
    List,
    /// Remove a preference
    Remove {
        #[arg(index = 1)]
        key: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum FoldersCommands {
    /// Grant a real folder (e.g. Documents) a name file tools can address via `root`.
    /// Read-only unless --read-write is passed.
    Add {
        #[arg(index = 1)]
        name: String,
        #[arg(index = 2)]
        path: String,
        #[arg(long)]
        read_write: bool,
    },
    /// List granted folders
    List,
    /// Revoke a folder grant
    Remove {
        #[arg(index = 1)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PluginsCommands {
    /// List all plugins in the registry
    List,
    /// Search the registry by name, description, or tag
    Search {
        #[arg(index = 1)]
        query: String,
    },
    /// Install a plugin from the registry (fail-closed; you still grant each tool)
    Install {
        #[arg(index = 1)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RoutineCommands {
    /// List scheduled routines
    List,
    /// Add a routine from a template, or a custom one with --cron and --goal
    Add {
        /// Template name (morning-brief, meeting-prep, research-brief, daily-digest, morning-news, weekly-review). Omit to use --cron/--goal.
        #[arg(index = 1)]
        template: Option<String>,
        #[arg(long)]
        cron: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        /// Reviewed tools this custom routine may use (repeat for each tool).
        /// Built-in templates supply their own allowlist.
        #[arg(long = "allow-tool")]
        allow_tool: Vec<String>,
    },
    /// Show a routine's scope and whether it can safely run unattended
    Preview {
        #[arg(index = 1)]
        index: usize,
    },
    /// Enable a reviewed routine only when every allowed tool is explicitly
    /// auto-approved for unattended use
    Enable {
        #[arg(index = 1)]
        index: usize,
    },
    /// Pause a routine without deleting its reviewed scope or schedule
    Disable {
        #[arg(index = 1)]
        index: usize,
    },
    /// Run a reviewed routine once now, using the same scoped non-interactive
    /// policy as the daemon
    Run {
        #[arg(index = 1)]
        index: usize,
    },
    /// Remove a routine by its list index
    Remove {
        #[arg(index = 1)]
        index: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum ApprovalCommands {
    List,
    Approve {
        #[arg(index = 1)]
        id: String,
    },
    Deny {
        #[arg(index = 1)]
        id: String,
    },
    /// Reject a pending approval (preferred spelling; `deny` remains supported)
    Reject {
        #[arg(index = 1)]
        id: String,
    },
}

/// Built-in routine templates → (cron, goal). Cron is 6-field (sec min hour
/// day month day-of-week), matching tokio-cron-scheduler.
struct RoutineTemplate {
    cron: &'static str,
    goal: &'static str,
    allowed_tools: &'static [&'static str],
}

fn routine_template(name: &str) -> Option<RoutineTemplate> {
    match name {
        "morning-brief" => Some(RoutineTemplate {
            cron: "0 0 8 * * Mon-Fri",
            goal: "Create a concise morning brief from the local calendar, notes, and weather when those reviewed tools are available. State which sources were unavailable. Prioritize the three most important actions. Do not modify or send anything.",
            allowed_tools: &["list_events", "list_notes", "search_notes", "get_weather"],
        }),
        "meeting-prep" => Some(RoutineTemplate {
            cron: "0 0 8 * * Mon-Fri",
            goal: "Prepare a concise briefing for today's upcoming meetings from the local calendar and relevant notes. For each meeting, identify the purpose, relevant context, and suggested agenda questions. Do not modify or send anything.",
            allowed_tools: &["list_events", "list_notes", "read_note", "search_notes"],
        }),
        "research-brief" => Some(RoutineTemplate {
            cron: "0 0 8 * * Mon",
            goal: "Research the user's chosen topic using the reviewed search and web-reading tools. Produce a concise, cited weekly brief with source links, key takeaways, and open questions. Do not modify or publish anything.",
            allowed_tools: &["web_search", "read_page_text"],
        }),
        "daily-digest" => Some(RoutineTemplate {
            cron: "0 0 8 * * *",
            goal: "Summarize today's local calendar and notes, then list the top three priorities. Do not modify or send anything.",
            allowed_tools: &["list_events", "list_notes", "search_notes"],
        }),
        "morning-news" => Some(RoutineTemplate {
            cron: "0 0 7 * * *",
            goal: "Search the web for today's most important AI news and summarize the top five items with source links. Do not modify or publish anything.",
            allowed_tools: &["web_search", "read_page_text"],
        }),
        "weekly-review" => Some(RoutineTemplate {
            cron: "0 0 17 * * Fri",
            goal: "Review the user's local notes from this week and summarize what was worked on and what comes next. Do not modify or send anything.",
            allowed_tools: &["list_notes", "read_note", "search_notes"],
        }),
        _ => None,
    }
}

fn routine_name(schedule: &config::ScheduleConfig) -> &str {
    if schedule.name.trim().is_empty() {
        &schedule.goal
    } else {
        &schedule.name
    }
}

/// Background execution has no interactive approval path. Every reviewed tool
/// therefore needs an explicit auto-approve policy before a routine can run.
fn routine_enablement_gaps(
    config: &config::Config,
    schedule: &config::ScheduleConfig,
) -> Vec<String> {
    if schedule.allowed_tools.is_empty() {
        return vec!["no reviewed tool allowlist".to_string()];
    }

    schedule
        .allowed_tools
        .iter()
        .filter(|tool| config.check_permission(tool) != "auto_approve")
        .cloned()
        .collect()
}

#[derive(Subcommand, Debug)]
pub enum PackCommands {
    /// List available tool packs
    List,
    /// Install a pack's plugins (fail-closed; you still grant each tool)
    Install {
        /// Pack name (e.g. productivity, dev)
        #[arg(index = 1)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecretsCommands {
    /// Show which environment variables a plugin needs and whether they are set
    Add {
        /// MCP plugin/server name (as configured in kerna.toml)
        #[arg(index = 1)]
        plugin: String,
    },
    /// List every plugin and the status of the secrets it declares
    List,
}

#[derive(Subcommand, Debug)]
pub enum KeysCommands {
    /// Show setup instructions for a provider's API key and optionally validate it
    Add {
        /// Provider name (built-in preset or a configured provider)
        #[arg(index = 1)]
        provider: String,
    },
    /// List every known provider and whether its API key is set
    List,
}

#[derive(Subcommand, Debug)]
pub enum ProviderCommands {
    /// Add a new provider to config
    Add {
        #[arg(index = 1)]
        name: String,

        #[arg(long)]
        provider_type: Option<String>,

        #[arg(long)]
        api_key_env: Option<String>,

        #[arg(long)]
        default_model: Option<String>,

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
    /// Discover models actually installed in a local provider (for example Ollama)
    Models {
        #[arg(index = 1, default_value = "ollama")]
        name: String,
    },
    /// Manage model routing
    Route {
        #[command(subcommand)]
        action: RouteCommands,
    },
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
    },
    /// Resolve and display the exact model selected by a privacy label
    Resolve {
        #[arg(index = 1)]
        privacy_mode: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ModelCommands {
    /// Detect the supported local hardware profile
    Detect,
    /// List the pinned curated model recipes and their provenance
    List,
    /// Recommend validated, evidence-backed recipes for detected hardware
    Recommend {
        #[arg(long, default_value = "coding")]
        purpose: String,
        /// Optional JSON hardware profile for an unsupported machine; it is never treated as evidence.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Verify models actually reported by a local provider; never launches or downloads anything
    Verify {
        #[arg(long, default_value = "ollama")]
        provider: String,
    },
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
    /// Add a contained OCI MCP plugin to the project contract
    Add {
        name: String,
        /// Digest-pinned OCI image whose entrypoint speaks MCP over stdio.
        #[arg(long)]
        image: String,
        /// Reviewed manifest.toml path.
        #[arg(long)]
        manifest: String,
        /// SHA-256 fingerprint of the reviewed manifest file.
        #[arg(long)]
        manifest_sha256: String,
        /// Base64 Ed25519 public key that verifies the manifest signature.
        #[arg(long)]
        signing_public_key: String,
        /// Project-relative directory mounted read-only (repeatable).
        #[arg(long = "read-root")]
        read_roots: Vec<String>,
        /// Project-relative directory mounted read-write (repeatable).
        #[arg(long = "write-root")]
        write_roots: Vec<String>,
        /// Arguments passed to the image entrypoint.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Sign a reviewed plugin manifest; the Ed25519 seed is read only from an environment variable.
    SignManifest {
        /// Project-relative path to manifest.toml.
        #[arg(long)]
        manifest: String,
        /// Environment variable containing a base64 32-byte Ed25519 signing seed.
        #[arg(long)]
        signing_key_env: String,
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
    /// Run the official core MCP client conformance scenarios through a
    /// benchmark-only streamable-HTTP-to-stdio bridge.
    #[command(hide = true)]
    ConformanceClient {
        /// URL supplied by the official MCP conformance scenario server.
        #[arg(index = 1)]
        server_url: String,
    },
    /// Measure the isolated stdio MCP client path against the built-in MockMCP.
    #[command(hide = true)]
    BenchmarkEcho {
        /// Number of echo calls made after a single initialization.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=10_000))]
        iterations: u32,
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

/// Lightweight live check that an API key reaches the provider. Uses a cheap
/// read-only endpoint (`GET /models` for OpenAI-compatible hosts, a 1-token
/// message for Anthropic). Returns the model name on success.
async fn validate_key(config: &Config, provider: &str, key: &str) -> Result<String> {
    use providers::WireProtocol;
    let resolved = providers::resolve(config, provider, None, key)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;

    match resolved.protocol {
        WireProtocol::Mock => Ok("mock".to_string()),
        WireProtocol::OpenAiCompat => {
            let url = format!("{}/models", resolved.base_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .bearer_auth(&resolved.api_key)
                .send()
                .await?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(anyhow::anyhow!("401 Unauthorized — key rejected"));
            }
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("HTTP {}", resp.status()));
            }
            Ok(resolved.model)
        }
        WireProtocol::Anthropic => {
            let url = format!("{}/v1/messages", resolved.base_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": resolved.model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            });
            let resp = client
                .post(&url)
                .header("x-api-key", &resolved.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(anyhow::anyhow!("401 Unauthorized — key rejected"));
            }
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("HTTP {}", resp.status()));
            }
            Ok(resolved.model)
        }
    }
}

/// Resolve a task-id argument, expanding the `last` alias to the most recently
/// created task. Exits with a friendly message if there is nothing to resolve.
fn resolve_task_id(memory: &MemoryEngine, arg: &str) -> String {
    if arg == "last" {
        match memory.get_last_task_id() {
            Ok(Some(id)) => id,
            _ => {
                eprintln!("[-] No tasks recorded yet. Run `kerna run <goal>` first.");
                std::process::exit(1);
            }
        }
    } else {
        arg.to_string()
    }
}

/// Whether a command needs live MCP plugins spawned into the shared registry.
/// `mcp probe/inspect/risk/doctor` spawn their own short-lived clients, so they
/// don't need the shared registry pre-initialized.
fn command_needs_mcp(command: &Option<Commands>) -> bool {
    matches!(
        command,
        Some(Commands::Run { .. })
            | Some(Commands::Routine {
                action: RoutineCommands::Run { .. }
            })
            | Some(Commands::Serve { .. })
            | Some(Commands::Daemon)
            | Some(Commands::Top)
            | Some(Commands::Watch { .. })
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    // We rely on the local ctrl_c wait in Daemon instead of global exit(0)

    let cli = Cli::parse();

    // Contracts must work in a brand-new directory. Do this before config and
    // memory initialization so the command neither requires nor creates a
    // runtime database in the caller's current directory.
    match &cli.command {
        Some(Commands::Contract { action }) => match action {
            ContractCommands::Init {
                template,
                name,
                output,
                no_demo_server,
            } => {
                let files = contract::init(template, name, output, !no_demo_server)?;
                println!("Created reviewed starter contract:");
                println!("  {}", files.config.display());
                println!("  {}", files.contract.display());
                if *no_demo_server {
                    println!(
                        "This contract enables no server, so the gateway will expose no tools."
                    );
                } else {
                    println!(
                        "It enables the bundled demo server, which runs from this binary and is not contained."
                    );
                }
                println!("Next: review the files, then run `kerna gateway` from that directory.");
                return Ok(());
            }
        },
        Some(Commands::Client { action }) => match action {
            ClientCommands::Config { client, workspace } => {
                print!("{}", client::config(client, workspace)?);
                return Ok(());
            }
            ClientCommands::Doctor { client, workspace } => {
                for line in client::doctor(client, workspace).await? {
                    println!("{}", line);
                }
                return Ok(());
            }
        },
        _ => {}
    }
    // IDE MCP launchers do not consistently support a per-server working
    // directory on every platform. Let `gateway --workspace` select the
    // contract before configuration and relative runtime paths are loaded.
    match &cli.command {
        Some(Commands::Gateway {
            workspace: Some(workspace),
        })
        | Some(Commands::Dashboard {
            workspace: Some(workspace),
            ..
        }) => {
            std::env::set_current_dir(workspace)?;
        }
        _ => {}
    }
    let mut config = Config::load();

    // Manifests narrow the effective runtime policy (tools, approval-required
    // operations, and secret passthrough). They are not persisted back to the
    // user's config; this keeps explicit user intent separate from derived
    // plugin declarations. Configuration-management commands intentionally use
    // the unmodified config so adding a plugin cannot rewrite existing grants.
    let needs_effective_manifest_policy =
        command_needs_mcp(&cli.command) || matches!(&cli.command, Some(Commands::Gateway { .. }));
    if needs_effective_manifest_policy {
        if let Err(e) = plugin_manifest::apply_to_config(&mut config) {
            eprintln!("[-] Refusing to load plugin manifests: {}", e);
            std::process::exit(1);
        }
    }

    // Initialize Memory Engine
    let memory = Arc::new(MemoryEngine::new(&config.db_path)?);

    // Initialize MCP Registry
    let mcp_registry = Arc::new(Mutex::new(McpRegistry::new()));

    // Only spawn MCP plugins for commands that actually invoke live tools.
    // Read-only/observability commands (trace, inspect, task, memory, config,
    // policy, provider, keys, doctor) operate purely on SQLite + config and must
    // not pay the cost — or print the banner — of booting every plugin.
    if !config.mcp_servers.is_empty() && command_needs_mcp(&cli.command) {
        let mut registry = mcp_registry.lock().await;
        let workspace = std::env::current_dir()?;
        registry
            .initialize_production(&config, &workspace)
            .await
            .map_err(|e| anyhow::anyhow!("refusing to start uncontained MCP plugins: {}", e))?;
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
        // Handled before runtime initialization above. Keeping this arm makes
        // the exhaustive command dispatch explicit if that early return is
        // ever refactored.
        Some(Commands::Contract { .. }) => {
            unreachable!("contract command returns before runtime setup")
        }
        Some(Commands::Client { .. }) => {
            unreachable!("client command returns before runtime setup")
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

            // Messaging-channel listeners (Telegram, …). Each allowlisted
            // inbound message becomes a governed, non-interactive agent run.
            gateways::start_channels(config.clone(), memory.clone(), mcp_registry.clone());

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!(
                "║                  Kerna Daemon v{}                        ║",
                env!("CARGO_PKG_VERSION")
            );
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
            println!(
                "║  Channels:   {:<45} ║",
                format!(
                    "{} messaging channel(s)",
                    config.channels.iter().filter(|c| c.enabled).count()
                )
            );
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Daemon running. Press Ctrl+C to stop.                      ║");
            println!("╚══════════════════════════════════════════════════════════════╝");

            tokio::signal::ctrl_c().await?;
            println!("\n[+] Daemon stopped cleanly.");
        }

        Some(Commands::Serve { port, bind, token }) => {
            let is_loopback = bind == "127.0.0.1" || bind == "localhost" || bind == "::1";
            if !is_loopback && token.is_none() {
                eprintln!(
                    "[-] Refusing to bind non-loopback address '{}' without authentication.\n    Pass --token <secret> to require a bearer token, or bind 127.0.0.1 for local-only use.",
                    bind
                );
                std::process::exit(1);
            }
            if token.is_none() {
                println!("[i] No --token set: this server is loopback-only and unauthenticated.");
            }
            let state = server::AppState {
                config: config.clone(),
                memory: memory.clone(),
                mcp_registry: mcp_registry.clone(),
                auth_token: token,
            };
            if let Err(e) = server::start_server(state, &bind, port).await {
                eprintln!("[-] Server failed: {}", e);
            }
        }

        Some(Commands::Dashboard { port, no_open, .. }) => {
            let state = server::AppState {
                config: config.clone(),
                memory: memory.clone(),
                mcp_registry: mcp_registry.clone(),
                auth_token: None,
            };
            if let Err(e) = server::start_dashboard_server(state, port, !no_open).await {
                eprintln!("[-] Dashboard failed: {}", e);
            }
        }

        Some(Commands::Mockmcp { action: _, mode }) => {
            let mut server = mockmcp::MockMcpServer::new(&mode);
            if let Err(e) = server.run().await {
                eprintln!("[-] MockMCP failed: {}", e);
            }
        }

        Some(Commands::Gateway { .. }) => {
            // stdout is the MCP JSON-RPC channel, so spawn downstream servers in
            // quiet mode (diagnostics → stderr) and never println! to stdout.
            {
                let mut registry = mcp_registry.lock().await;
                registry.set_quiet(true);
                let workspace = std::env::current_dir()?;
                registry
                    .initialize_production(&config, &workspace)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("[gateway] refusing to start uncontained plugins: {}", e)
                    })?;
            }
            let gw = gateway::Gateway::new(config.clone(), mcp_registry.clone(), memory.clone());
            if let Err(e) = gw.run().await {
                eprintln!("[gateway] fatal: {}", e);
            }
        }

        Some(Commands::Run {
            goal,
            converse,
            audit,
            non_interactive,
            approval_queue,
            privacy,
        }) => {
            if converse {
                config.converse = true;
            }

            if audit {
                config.audit_only = true;
                // Said once, loudly, before anything runs. The single failure mode of
                // rung 1 is that it goes unnoticed and someone believes a policy is
                // being enforced when it is only being recorded.
                println!("+--------------------------------------------------------------+");
                println!("|  AUDIT MODE - policy is RECORDED but NOT ENFORCED             |");
                println!("|  Nothing will be denied. Nothing will prompt.                 |");
                println!("|  Actions your policy would have stopped are marked [observe]. |");
                println!("+--------------------------------------------------------------+");
            }

            if let Some(priv_mode) = privacy {
                let (selected, _) =
                    providers::resolve_privacy_route(&config, &priv_mode, &config.llm_api_key)?;
                config.llm_provider = selected.provider;
                config.llm_model = selected.model;
                if priv_mode == "local-only" || priv_mode == "local_only" {
                    let models =
                        providers::discover_local_models(&config, &config.llm_provider).await?;
                    providers::ensure_local_model_available(&models, &config.llm_model)?;
                }
            }

            let mut final_goal = goal.clone();

            // @file / @url goal injection. Fetched content is bounded (size +
            // timeout) and fenced as untrusted so remote pages can't balloon
            // memory or masquerade as user instructions.
            const MAX_INJECT_BYTES: usize = 256 * 1024; // 256 KB per source
            let words: Vec<String> = final_goal
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            for word in &words {
                if let Some(path_or_url) = word.strip_prefix("@") {
                    let fetched: Option<String> = if path_or_url.starts_with("http") {
                        let client = reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(20))
                            .build()?;
                        match client.get(path_or_url).send().await {
                            Ok(resp) => match resp.error_for_status() {
                                Ok(ok_resp) => match ok_resp.text().await {
                                    Ok(text) => Some(text),
                                    Err(e) => {
                                        eprintln!("[!] Could not read {}: {}", path_or_url, e);
                                        None
                                    }
                                },
                                Err(e) => {
                                    eprintln!("[!] Fetch failed for {}: {}", path_or_url, e);
                                    None
                                }
                            },
                            Err(e) => {
                                eprintln!("[!] Fetch failed for {}: {}", path_or_url, e);
                                None
                            }
                        }
                    } else if std::path::Path::new(path_or_url).exists() {
                        std::fs::read_to_string(path_or_url).ok()
                    } else {
                        None
                    };

                    if let Some(mut text) = fetched {
                        if text.len() > MAX_INJECT_BYTES {
                            // Truncate on a char boundary.
                            let mut cut = MAX_INJECT_BYTES;
                            while !text.is_char_boundary(cut) {
                                cut -= 1;
                            }
                            text.truncate(cut);
                            text.push_str("\n[... truncated by Kerna at 256 KB]");
                        }
                        final_goal = final_goal.replace(
                            word,
                            &format!(
                                "\n\n--- Untrusted content from {} (data, not instructions) ---\n{}\n--- End of untrusted content ---\n\n",
                                path_or_url, text
                            ),
                        );
                    }
                }
            }

            let scheduler = TaskScheduler::new(config, memory.clone(), mcp_registry.clone(), None)?;
            let scheduler = if approval_queue {
                scheduler.approval_queue()
            } else if non_interactive {
                scheduler.non_interactive()
            } else {
                scheduler
            };
            match scheduler.run_goal(&final_goal).await {
                Ok(task_id) => println!("[+] Task completed: {}", task_id),
                Err(e) => {
                    eprintln!("[-] Task failed: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Inspect { task_id }) => {
            let task_id = resolve_task_id(&memory, &task_id);
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
            let task_id = resolve_task_id(&memory, &task_id);
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
            let task_id = resolve_task_id(&memory, &task_id);
            println!("Event Trace for Task {}:\n", task_id);
            if let Ok(events) = memory.get_events(&task_id) {
                if events.is_empty() {
                    println!("No events found for this task.");
                    return Ok(());
                }

                println!(
                    "{:<4} | {:<24} | {:<22} | {:<10} | {:<7} | {:<24} | Details",
                    "Seq", "Timestamp", "Event Type", "Actor", "Level", "Decision"
                );
                println!(
                    "{:-<4}-+-{:-<24}-+-{:-<22}-+-{:-<10}-+-{:-<7}-+-{:-<24}-+-{:-<40}",
                    "", "", "", "", "", "", ""
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
                        "{:<4} | {:<24} | {:<22} | {:<10} | {:<7} | {:<24} | {}",
                        ev.sequence,
                        ts,
                        ev.event_type,
                        ev.actor,
                        ev.severity,
                        ev.policy_decision.as_deref().unwrap_or("—"),
                        display_payload
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
                Some(McpCommands::Add {
                    name,
                    image,
                    manifest,
                    manifest_sha256,
                    signing_public_key,
                    read_roots,
                    write_roots,
                    args,
                }) => {
                    if config.mcp_servers.iter().any(|s| s.name == name) {
                        eprintln!("[-] MCP server '{}' already exists.", name);
                        std::process::exit(1);
                    }
                    let server = config::McpServerConfig {
                        name: name.clone(),
                        command: String::new(),
                        args,
                        enabled: true, // Auto-enable on add
                        capabilities: vec![],
                        allowed_paths: vec![],
                        approval_required: vec![],
                        allow_tools: vec![],
                        deny_tools: vec![],
                        secrets: vec![],
                        runtime_mode: "docker".to_string(),
                        docker_image: String::new(),
                        image,
                        manifest_path: manifest,
                        manifest_sha256,
                        signing_public_key,
                        read_roots,
                        write_roots,
                    };
                    let workspace = std::env::current_dir()?;
                    if let Err(error) =
                        plugin_manifest::verify_production_server(&server, &workspace)
                    {
                        eprintln!("[-] Refusing to add uncontained plugin: {}", error);
                        std::process::exit(1);
                    }
                    config.mcp_servers.push(server);
                    config.save();
                    println!("[+] Added and enabled MCP server '{}'", name);
                }
                Some(McpCommands::SignManifest {
                    manifest,
                    signing_key_env,
                }) => {
                    let secret = std::env::var(&signing_key_env).map_err(|_| {
                        anyhow::anyhow!(
                            "signing key environment variable '{}' is not set",
                            signing_key_env
                        )
                    })?;
                    let workspace = std::env::current_dir()?;
                    let relative = std::path::Path::new(&manifest);
                    if relative.is_absolute()
                        || relative.components().any(|part| {
                            matches!(
                                part,
                                std::path::Component::ParentDir
                                    | std::path::Component::RootDir
                                    | std::path::Component::Prefix(_)
                            )
                        })
                    {
                        return Err(anyhow::anyhow!("manifest must be a project-relative path"));
                    }
                    let root = workspace.canonicalize()?;
                    let path = workspace.join(relative).canonicalize()?;
                    if !path.starts_with(root) {
                        return Err(anyhow::anyhow!(
                            "manifest must remain inside the project workspace"
                        ));
                    }
                    let (fingerprint, public_key) = plugin_manifest::sign_manifest(&path, &secret)?;
                    println!("[+] Signed {}", manifest);
                    println!("manifest_sha256={}", fingerprint);
                    println!("signing_public_key={}", public_key);
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
                Some(McpCommands::Filter {
                    action: filter_action,
                }) => match filter_action {
                    FilterCommands::Allow {
                        server_name,
                        tool_name,
                    } => {
                        if let Some(server) = config
                            .mcp_servers
                            .iter_mut()
                            .find(|s| s.name == server_name)
                        {
                            if !server.allow_tools.contains(&tool_name) {
                                server.allow_tools.push(tool_name.clone());
                                config.save();
                                println!(
                                    "[+] Added '{}' to allow_tools for '{}'",
                                    tool_name, server_name
                                );
                            } else {
                                println!(
                                    "[-] '{}' is already in allow_tools for '{}'",
                                    tool_name, server_name
                                );
                            }
                        } else {
                            eprintln!("[-] MCP server '{}' not found.", server_name);
                        }
                    }
                    FilterCommands::Deny {
                        server_name,
                        tool_name,
                    } => {
                        if let Some(server) = config
                            .mcp_servers
                            .iter_mut()
                            .find(|s| s.name == server_name)
                        {
                            if !server.deny_tools.contains(&tool_name) {
                                server.deny_tools.push(tool_name.clone());
                                config.save();
                                println!(
                                    "[+] Added '{}' to deny_tools for '{}'",
                                    tool_name, server_name
                                );
                            } else {
                                println!(
                                    "[-] '{}' is already in deny_tools for '{}'",
                                    tool_name, server_name
                                );
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
                        let workspace = std::env::current_dir()?;
                        match plugin_manifest::verify_production_server(server, &workspace) {
                            Ok(manifest) => {
                                println!("  Signed manifest: \x1b[32mOK\x1b[0m");
                                println!("  OCI image: {}", server.image);
                                println!(
                                    "  Docker image present: {}",
                                    mcp_registry::image_available(&server.image)
                                );
                                println!(
                                    "  Declared tools: {}",
                                    manifest.plugin.capabilities.join(", ")
                                );
                                println!("  Read roots: {}", server.read_roots.join(", "));
                                println!("  Write roots: {}", server.write_roots.join(", "));
                            }
                            Err(error) => {
                                println!("  Production readiness: \x1b[31mERROR\x1b[0m — {}", error)
                            }
                        }
                        println!(
                            "\n  To test the governed transport, run `kerna gateway --workspace .` from this project",
                        );
                    } else {
                        eprintln!("[-] MCP server '{}' not found in config.", name);
                    }
                }
                Some(McpCommands::ConformanceClient { server_url }) => {
                    // Kerna intentionally owns a process-isolated stdio MCP
                    // boundary. The conformance framework provides a local HTTP
                    // test server, so the benchmark starts a pinned bridge as
                    // the untrusted child process rather than adding a remote
                    // transport to Kerna's production configuration surface.
                    let remote_args = ["--yes", "mcp-remote@0.1.38", server_url.as_str()];
                    let npx_command = if cfg!(windows) { "npx.cmd" } else { "npx" };
                    let mut client = mcp::McpClient::spawn(
                        npx_command,
                        &remote_args,
                        "native",
                        "",
                        "bridge",
                        None,
                        &[],
                    )?;
                    client.initialize().await?;
                    let tools = client.list_tools().await?;

                    match std::env::var("MCP_CONFORMANCE_SCENARIO")
                        .unwrap_or_else(|_| "initialize".to_string())
                        .as_str()
                    {
                        "initialize" => {}
                        "tools_call" => {
                            if !tools.iter().any(|tool| tool.name == "add_numbers") {
                                anyhow::bail!(
                                    "MCP conformance tools_call scenario did not expose add_numbers"
                                );
                            }
                            let result = client
                                .call_tool("add_numbers", serde_json::json!({ "a": 40, "b": 2 }))
                                .await?;
                            let expected = result
                                .pointer("/content/0/text")
                                .and_then(serde_json::Value::as_str)
                                .map(|text| text.contains("42"))
                                .unwrap_or(false);
                            if !expected {
                                anyhow::bail!(
                                    "MCP conformance tools_call scenario returned an unexpected result"
                                );
                            }
                        }
                        scenario => {
                            anyhow::bail!(
                                "Unsupported MCP conformance scenario '{}'; Kerna currently validates only the official core stdio-compatible scenarios",
                                scenario
                            );
                        }
                    }
                }
                Some(McpCommands::BenchmarkEcho { iterations }) => {
                    // This intentionally benchmarks only the process-isolated
                    // stdio client path. It is deterministic, has no provider
                    // call, and does not represent scheduler or model latency.
                    let executable = std::env::current_exe()?;
                    let executable = executable
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Kerna executable path is not UTF-8"))?;
                    let args = ["mockmcp"];
                    let started = std::time::Instant::now();
                    let mut client = mcp::McpClient::spawn(
                        executable,
                        &args,
                        "native",
                        "",
                        "bridge",
                        None,
                        &[],
                    )?;
                    client.initialize().await?;
                    let initialized_ms = started.elapsed().as_secs_f64() * 1000.0;

                    let discovery_started = std::time::Instant::now();
                    let tools = client.list_tools().await?;
                    let discovery_ms = discovery_started.elapsed().as_secs_f64() * 1000.0;
                    if !tools.iter().any(|tool| tool.name == "echo") {
                        anyhow::bail!("MockMCP benchmark fixture did not expose echo");
                    }

                    let mut calls_ms = Vec::with_capacity(iterations as usize);
                    for _ in 0..iterations {
                        let call_started = std::time::Instant::now();
                        let result = client
                            .call_tool("echo", serde_json::json!({ "message": "benchmark" }))
                            .await?;
                        if result.is_null() {
                            anyhow::bail!("MockMCP benchmark echo returned null");
                        }
                        calls_ms.push(call_started.elapsed().as_secs_f64() * 1000.0);
                    }

                    println!(
                        "{}",
                        serde_json::json!({
                            "benchmark": "kerna-mcp-stdio-echo",
                            "protocolVersion": mcp::MCP_PROTOCOL_VERSION,
                            "initializationMs": initialized_ms,
                            "toolDiscoveryMs": discovery_ms,
                            "toolCallMs": calls_ms,
                        })
                    );
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

        Some(Commands::Doctor { gateway }) => {
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

            // Active provider + per-provider key status.
            println!(
                "Active provider: {} (model: {})",
                config.llm_provider, config.llm_model
            );
            println!(
                "  Active key: {}",
                if config.llm_provider == "mock" {
                    "\x1b[32mnot required (Demo mode)\x1b[0m"
                } else if config.llm_api_key.is_empty() {
                    "\x1b[31mMISSING\x1b[0m"
                } else {
                    "\x1b[32mOK\x1b[0m"
                }
            );

            let mut key_names: Vec<String> = config.providers.keys().cloned().collect();
            if !key_names.contains(&config.llm_provider) && config.llm_provider != "mock" {
                key_names.push(config.llm_provider.clone());
            }
            if !key_names.is_empty() {
                println!("Configured provider keys:");
                for name in &key_names {
                    let env_var = providers::api_key_env_for(&config, name);
                    let local = providers::preset_info(name)
                        .map(|p| {
                            let l = p.base_url.to_lowercase();
                            l.contains("://localhost") || l.contains("://127.0.0.1")
                        })
                        .unwrap_or(false);
                    let status = if local {
                        "\x1b[32mlocal (no key needed)\x1b[0m".to_string()
                    } else if std::env::var(&env_var)
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false)
                    {
                        "\x1b[32mset\x1b[0m".to_string()
                    } else {
                        format!("\x1b[31mmissing\x1b[0m ({})", env_var)
                    };
                    println!("  - {:<12} {}", name, status);
                }
            }

            let mut valid_plugins = 0;
            for server in &config.mcp_servers {
                if server.runtime_mode == "docker" && !server.image.is_empty() {
                    valid_plugins += 1;
                    continue;
                }
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

            if !config.mcp_servers.is_empty() {
                println!("Connector setup:");
                for server in &config.mcp_servers {
                    let (declared_secrets, manifest_issue) =
                        match plugin_manifest::load_for_server(server) {
                            Ok(Some((_path, manifest))) => (manifest.plugin.secrets, None),
                            Ok(None) => (server.secrets.clone(), None),
                            Err(error) => (Vec::new(), Some(error.to_string())),
                        };
                    if let Some(issue) = manifest_issue {
                        println!(
                            "  - {:<16} \x1b[31mmanifest error\x1b[0m ({})",
                            server.name, issue
                        );
                        continue;
                    }

                    let not_configured: Vec<&String> = declared_secrets
                        .iter()
                        .filter(|secret| !server.secrets.contains(*secret))
                        .collect();
                    let effective_secrets: Vec<&String> = server
                        .secrets
                        .iter()
                        .filter(|secret| declared_secrets.contains(*secret))
                        .collect();
                    let ignored_config_secrets: Vec<&String> = server
                        .secrets
                        .iter()
                        .filter(|secret| !declared_secrets.contains(*secret))
                        .collect();
                    let missing_environment: Vec<&String> = effective_secrets
                        .iter()
                        .filter(|secret| {
                            std::env::var(secret.as_str())
                                .map(|value| value.trim().is_empty())
                                .unwrap_or(true)
                        })
                        .copied()
                        .collect();

                    if !not_configured.is_empty() {
                        println!(
                            "  - {:<16} \x1b[31mnot configured\x1b[0m (declare: {})",
                            server.name,
                            not_configured
                                .iter()
                                .map(|secret| secret.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    } else if !missing_environment.is_empty() {
                        println!(
                            "  - {:<16} \x1b[33mneeds setup\x1b[0m (missing: {})",
                            server.name,
                            missing_environment
                                .iter()
                                .map(|secret| secret.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    } else if effective_secrets.is_empty() {
                        println!("  - {:<16} ready (no secrets required)", server.name);
                    } else {
                        println!("  - {:<16} \x1b[32mready\x1b[0m", server.name);
                    }

                    if !ignored_config_secrets.is_empty() {
                        println!(
                            "      ignored by manifest: {}",
                            ignored_config_secrets
                                .iter()
                                .map(|secret| secret.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
            }

            if gateway {
                let enabled: Vec<_> = config
                    .mcp_servers
                    .iter()
                    .filter(|server| server.enabled)
                    .collect();
                let auto_approved = config
                    .permissions
                    .iter()
                    .filter(|rule| rule.action == "auto_approve")
                    .count();
                let confirmation_required = config
                    .permissions
                    .iter()
                    .filter(|rule| rule.action == "require_confirmation")
                    .count();
                let denied = config
                    .permissions
                    .iter()
                    .filter(|rule| rule.action == "deny")
                    .count();

                println!("Gateway readiness:");
                if enabled.is_empty() {
                    println!("  ERROR: no enabled downstream MCP server is configured.");
                } else {
                    let workspace = std::env::current_dir()?;
                    let mut production_errors = Vec::new();
                    // Doctor has to answer the question the gateway will answer,
                    // not a stricter one. Reporting a containment error for a
                    // server the gateway starts happily is a false alarm, and
                    // the moment it is read is right before a demo.
                    let (demo, contained): (Vec<&config::McpServerConfig>, Vec<&config::McpServerConfig>) = enabled
                        .iter()
                        .copied()
                        .partition(|server| server.is_bundled_demo());
                    if !contained.is_empty() && !sandbox::docker_available() {
                        production_errors.push("Docker is unavailable".to_string());
                    }
                    for server in &contained {
                        if let Err(error) =
                            crate::plugin_manifest::verify_production_server(server, &workspace)
                        {
                            production_errors.push(format!("{}: {}", server.name, error));
                        } else if !mcp_registry::image_available(&server.image) {
                            production_errors.push(format!(
                                "{}: image is not available locally ({})",
                                server.name, server.image
                            ));
                        }
                    }
                    if production_errors.is_empty() {
                        if !contained.is_empty() {
                            println!(
                                "  Ready: {} contained downstream MCP server(s); gateway uses stdio.",
                                contained.len()
                            );
                        }
                        for server in &demo {
                            println!(
                                "  Ready: '{}' is the bundled demo server. It runs from this binary and is NOT contained -- replace it before governing real work.",
                                server.name
                            );
                        }
                    } else {
                        println!("  ERROR: production containment is not ready:");
                        for error in production_errors {
                            println!("    - {}", error);
                        }
                    }
                }
                println!(
                    "  Policy: {} auto-approved, {} confirmation-required, {} denied rule(s).",
                    auto_approved, confirmation_required, denied
                );
                println!(
                    "  Note: denied tools are hidden. Confirmation-required calls queue a one-time local approval."
                );
            }
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
                        config.sandbox_image.clone(),
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
                            if e_str.contains("Policy Violation")
                                || e_str.contains("does not have capability")
                            {
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
                    let mut needs_confirmation = false;
                    match sandbox.simulate_command(&tool, &args, &permissions) {
                        Ok(decision) => {
                            if !decision.is_allowed {
                                is_allowed = false;
                            }
                            if decision.needs_confirmation {
                                needs_confirmation = true;
                            }
                            for r in decision.reasons {
                                if r.contains("Deny")
                                    || r.contains("RequireConfirmation")
                                    || r.contains("deny")
                                {
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
                    // Three outcomes, because the policy has three. This printed a green
                    // ALLOW for anything that was not denied, so a tool set to
                    // `require_confirmation` -- which stops and asks a human -- reported
                    // as allowed. For the one command that exists to let an operator test
                    // a policy before trusting it, that is the wrong way to round.
                    if !is_allowed {
                        println!("  Final Decision: \x1b[1;31mDENY\x1b[0m");
                    } else if needs_confirmation {
                        println!(
                            "  Final Decision: \x1b[1;33mASK\x1b[0m  (a human is prompted before this runs)"
                        );
                    } else {
                        println!("  Final Decision: \x1b[1;32mALLOW\x1b[0m");
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
        Some(Commands::Memory { action }) => match action {
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
                // Semantic search first (embedding cosine similarity), then fall
                // back to lexical LIKE for anything the embedder ranks low.
                let query_embedding = crate::embeddings::embed(&query);
                let mut shown = std::collections::HashSet::new();
                if let Ok(results) = memory.search_episodic_memory(&query_embedding, 10) {
                    for (content, score) in results {
                        if score > 0.10 && shown.insert(content.clone()) {
                            println!("- ({:.2}) {}", score, content);
                        }
                    }
                }
                if let Ok(results) = memory.search_memory_by_text(&query, 10) {
                    for r in results {
                        if shown.insert(r.clone()) {
                            println!("- {}", r);
                        }
                    }
                }
                if shown.is_empty() {
                    println!("No results found.");
                }
            }
            None => {
                if let Ok(memories) = memory.get_episodic_memories_by_time() {
                    if memories.is_empty() {
                        println!("Memory is empty.");
                    } else {
                        let mut current_date = String::new();
                        for (content, _ts, date) in memories {
                            let relative =
                                if date == chrono::Utc::now().format("%Y-%m-%d").to_string() {
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
        },

        Some(Commands::Watch { url, interval }) => {
            println!("[*] Watchdog mode: monitoring {}", url);
            println!("[*] Check interval: {}", interval);
            println!("[!] Watchdog requires the daemon to be running (kerna daemon).");

            let task_id = uuid::Uuid::new_v4();
            memory.create_task(task_id, None, &format!("Watch {} every {}", url, interval))?;
            memory.update_task_status(task_id, "watching")?;
            println!("[+] Watch registered as Task ID: {}", task_id);
        }

        Some(Commands::Provider { action }) => match action {
            ProviderCommands::Add {
                name,
                provider_type,
                api_key_env,
                default_model,
                base_url,
            } => {
                // Pre-fill from the built-in preset when flags are omitted, so
                // `kerna provider add ollama` works with zero extra arguments.
                let preset = providers::preset_info(&name);
                let provider = config::ProviderConfig {
                    provider_type: provider_type
                        .or_else(|| preset.as_ref().map(|p| p.provider_type.clone()))
                        .unwrap_or_else(|| "openai_compatible".to_string()),
                    api_key_env: api_key_env
                        .or_else(|| preset.as_ref().map(|p| p.api_key_env.clone())),
                    default_model: default_model
                        .or_else(|| preset.as_ref().map(|p| p.default_model.clone()))
                        .unwrap_or_default(),
                    base_url: base_url.or_else(|| preset.as_ref().map(|p| p.base_url.clone())),
                };
                let key_env = provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "KERNA_LLM_API_KEY".to_string());
                config.providers.insert(name.clone(), provider);
                config.save();
                println!("[+] Provider '{}' added.", name);
                println!("    Set the API key with:  kerna keys add {}", name);
                println!("    (reads environment variable {})", key_env);
            }
            ProviderCommands::List => {
                println!("Configured Providers:\n");
                for (name, p) in &config.providers {
                    let env = p.api_key_env.as_deref().unwrap_or("KERNA_LLM_API_KEY");
                    let is_local = p
                        .base_url
                        .as_deref()
                        .map(|u| {
                            let l = u.to_lowercase();
                            l.contains("://localhost") || l.contains("://127.0.0.1")
                        })
                        .unwrap_or(false);
                    let status = if is_local {
                        "\x1b[32mlocal (no key needed)\x1b[0m"
                    } else if std::env::var(env).is_ok() {
                        "\x1b[32mkey set\x1b[0m"
                    } else {
                        "\x1b[31mkey missing\x1b[0m"
                    };
                    println!(
                        "- {} (type: {}, model: {}, {})",
                        name, p.provider_type, p.default_model, status
                    );
                }
                if config.providers.is_empty() {
                    println!("No custom providers configured.");
                }
                println!(
                    "\nBuilt-in presets available: {}",
                    providers::builtin_names().join(", ")
                );
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
            ProviderCommands::Models { name } => {
                let models = providers::discover_local_models(&config, &name).await?;
                if models.is_empty() {
                    println!("No models reported by local provider '{}'.", name);
                } else {
                    println!("Local models reported by '{}':", name);
                    for model in models {
                        let size = model
                            .size_bytes
                            .map(|bytes| format!(" ({} bytes)", bytes))
                            .unwrap_or_default();
                        println!("- {}{}", model.id, size);
                    }
                }
            }
            ProviderCommands::Route {
                action: route_action,
            } => match route_action {
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
                    providers::parse_model_route_target(&config, &target)?;
                    config
                        .model_routes
                        .insert(route_name.clone(), target.clone());
                    config.save();
                    println!("[+] Route '{}' set to '{}'", route_name, target);
                }
                RouteCommands::Resolve { privacy_mode } => {
                    let (route, resolved) = providers::resolve_privacy_route(
                        &config,
                        &privacy_mode,
                        &config.llm_api_key,
                    )?;
                    println!("Privacy mode: {}", route.privacy_mode);
                    println!("Model route: {}", route.route_name);
                    println!("Target: {}/{}", route.provider, route.model);
                    println!("Endpoint: {}", resolved.base_url);
                    println!("Local endpoint: {}", resolved.is_local());
                }
            },
        },

        Some(Commands::Models { action }) => match action {
            ModelCommands::Detect => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&models::detect_hardware())?
                );
            }
            ModelCommands::List => {
                let catalog = models::catalog()?;
                println!(
                    "Catalog source: {} @ {}",
                    catalog.source.repository, catalog.source.revision
                );
                println!(
                    "License: {} · imported {}",
                    catalog.source.license, catalog.source.imported_at
                );
                for recipe in catalog.recipes {
                    println!(
                        "- {} · {} × {} · {} · {} · tools={} · {}",
                        recipe.model_instance_id,
                        recipe.hardware_count,
                        recipe.hardware_id,
                        recipe.engine,
                        recipe.status,
                        recipe.tools,
                        recipe.id
                    );
                }
            }
            ModelCommands::Recommend { purpose, profile } => {
                let profile = match profile {
                    Some(path) => models::load_manual_profile(&path)?,
                    None => models::detect_hardware(),
                };
                let recipes = models::recommend(&profile, &purpose)?;
                println!("Detected hardware: {}", serde_json::to_string(&profile)?);
                if recipes.is_empty() {
                    println!("No validated {} recipe matches this profile. Kerna will not make a launch claim.", purpose);
                } else {
                    for recipe in recipes {
                        println!(
                            "- {} ({}, {})",
                            recipe.model_instance_id, recipe.engine, recipe.id
                        );
                    }
                }
            }
            ModelCommands::Verify { provider } => {
                let installed = providers::discover_local_models(&config, &provider).await?;
                if installed.is_empty() {
                    println!("{} reported no installed models.", provider);
                } else {
                    println!("Models reported by local provider '{}':", provider);
                    for model in installed {
                        println!("- {}", model.id);
                    }
                }
            }
        },

        Some(Commands::Keys { action }) => {
            match action {
                KeysCommands::Add { provider } => {
                    let env_var = providers::api_key_env_for(&config, &provider);
                    let is_known = config.providers.contains_key(&provider)
                        || providers::preset_info(&provider).is_some()
                        || provider == "mock";

                    if !is_known {
                        eprintln!(
                            "[-] Unknown provider '{}'. Built-in presets: {}.",
                            provider,
                            providers::builtin_names().join(", ")
                        );
                        eprintln!(
                        "    Add a custom provider first: kerna provider add {} --base-url <url>",
                        provider
                    );
                        std::process::exit(1);
                    }

                    // Local runtimes (Ollama) need no key.
                    let local = providers::preset_info(&provider)
                        .map(|p| {
                            let l = p.base_url.to_lowercase();
                            l.contains("://localhost") || l.contains("://127.0.0.1")
                        })
                        .unwrap_or(false);
                    if local {
                        println!(
                            "Provider '{}' runs locally and needs no API key. You're ready to go:",
                            provider
                        );
                        println!("  kerna run \"Summarize README.md\" --privacy local-only");
                        return Ok(());
                    }

                    println!(
                        "Set your {} API key via the {} environment variable.\n",
                        provider, env_var
                    );
                    println!("  PowerShell (current session):");
                    println!("    $env:{} = \"<your-key>\"", env_var);
                    println!("  PowerShell (persist for future sessions):");
                    println!("    setx {} \"<your-key>\"", env_var);
                    println!("  bash / zsh:");
                    println!("    export {}=<your-key>\n", env_var);
                    println!("Kerna never writes your key to disk — it is read from the environment at run time.\n");

                    match std::env::var(&env_var) {
                        Ok(key) if !key.trim().is_empty() => {
                            println!(
                                "[✓] {} is currently set in this shell. Validating...",
                                env_var
                            );
                            match validate_key(&config, &provider, &key).await {
                                Ok(model) => {
                                    println!(
                                        "[✓] Key works. Reached provider '{}' (model: {}).",
                                        provider, model
                                    )
                                }
                                Err(e) => {
                                    println!("[!] Key is set but validation failed: {}", e);
                                    println!("    Double-check the key value and the provider's base URL.");
                                }
                            }
                        }
                        _ => {
                            println!(
                            "[i] {} is not set in this shell yet. Set it with a command above, then re-run:",
                            env_var
                        );
                            println!("    kerna keys add {}", provider);
                        }
                    }
                }
                KeysCommands::List => {
                    println!("API key status:\n");
                    // Union of configured providers and built-in presets.
                    let mut names: Vec<String> = providers::builtin_names()
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    for name in config.providers.keys() {
                        if !names.contains(name) {
                            names.push(name.clone());
                        }
                    }
                    for name in names {
                        let env_var = providers::api_key_env_for(&config, &name);
                        let local = providers::preset_info(&name)
                            .map(|p| {
                                let l = p.base_url.to_lowercase();
                                l.contains("://localhost") || l.contains("://127.0.0.1")
                            })
                            .unwrap_or(false);
                        let status = if local {
                            "\x1b[32mlocal (no key needed)\x1b[0m".to_string()
                        } else if std::env::var(&env_var)
                            .map(|v| !v.trim().is_empty())
                            .unwrap_or(false)
                        {
                            "\x1b[32mset\x1b[0m".to_string()
                        } else {
                            format!("\x1b[31mmissing\x1b[0m (set {})", env_var)
                        };
                        println!("  {:<12} {}", name, status);
                    }
                    println!("\nAdd a key with:  kerna keys add <provider>");
                }
            }
        }

        Some(Commands::Secrets { action }) => {
            // Secrets a plugin declares come from its manifest (plugins/<name>/manifest.toml)
            // unioned with anything already listed in kerna.toml. Only names are
            // shown here; values live in the environment and are never printed.
            let plugin_secrets = |name: &str| -> Vec<String> {
                let mut names: Vec<String> = config
                    .mcp_servers
                    .iter()
                    .find(|s| s.name == name)
                    .map(|s| s.secrets.clone())
                    .unwrap_or_default();
                if let Some(server) = config.mcp_servers.iter().find(|s| s.name == name) {
                    if let Ok(Some((_, m))) = plugin_manifest::load_for_server(server) {
                        for s in m.plugin.secrets {
                            if !names.contains(&s) {
                                names.push(s);
                            }
                        }
                    }
                }
                names
            };
            let is_set = |env_var: &str| {
                std::env::var(env_var)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
            };
            match action {
                SecretsCommands::Add { plugin } => {
                    let secrets = plugin_secrets(&plugin);
                    if config.mcp_servers.iter().all(|s| s.name != plugin)
                        && crate::plugin_manifest::find_for_server(&config::McpServerConfig {
                            name: plugin.clone(),
                            command: String::new(),
                            args: vec![],
                            enabled: true,
                            capabilities: vec![],
                            allowed_paths: vec![],
                            approval_required: vec![],
                            allow_tools: vec![],
                            deny_tools: vec![],
                            secrets: vec![],
                            runtime_mode: "native".to_string(),
                            docker_image: "ubuntu:latest".to_string(),
                            image: String::new(),
                            manifest_path: String::new(),
                            manifest_sha256: String::new(),
                            signing_public_key: String::new(),
                            read_roots: vec![],
                            write_roots: vec![],
                        })
                        .is_none()
                    {
                        eprintln!(
                            "[-] Plugin '{}' is not configured and has no manifest. Add it first: kerna mcp add {} <command> [args...]",
                            plugin, plugin
                        );
                    } else if secrets.is_empty() {
                        println!("Plugin '{}' declares no secrets — nothing to set.", plugin);
                    } else {
                        println!("Secrets for plugin '{}':\n", plugin);
                        for env_var in &secrets {
                            if is_set(env_var) {
                                println!("  \x1b[32m[set]\x1b[0m {}", env_var);
                            } else {
                                println!("  \x1b[31m[missing]\x1b[0m {}", env_var);
                                if cfg!(windows) {
                                    println!(
                                        "      setx {} \"your-value\"     (new terminals)",
                                        env_var
                                    );
                                    println!(
                                        "      $env:{} = \"your-value\"   (this session)",
                                        env_var
                                    );
                                } else {
                                    println!("      export {}=\"your-value\"", env_var);
                                }
                            }
                        }
                        println!(
                            "\nKerna reads these from your environment and injects them into the\nplugin's process only. They are never written to kerna.toml."
                        );
                    }
                }
                SecretsCommands::List => {
                    println!("Plugin secret status:\n");
                    if config.mcp_servers.is_empty() {
                        println!("  No plugins configured. Add one with: kerna mcp add <name> <command> [args...]");
                    }
                    for server in &config.mcp_servers {
                        let secrets = plugin_secrets(&server.name);
                        if secrets.is_empty() {
                            println!("  {:<14} (no secrets)", server.name);
                        } else {
                            for env_var in secrets {
                                let status = if is_set(&env_var) {
                                    "\x1b[32mset\x1b[0m".to_string()
                                } else {
                                    format!("\x1b[31mmissing\x1b[0m (set {})", env_var)
                                };
                                println!("  {:<14} {:<24} {}", server.name, env_var, status);
                            }
                        }
                    }
                    println!("\nConfigure with:  kerna secrets add <plugin>");
                }
            }
        }

        Some(Commands::Pack { action }) => match action {
            PackCommands::List => {
                println!("Available tool packs:\n");
                let packs = packs::list_packs();
                if packs.is_empty() {
                    println!(
                        "  (none found in {}). Set KERNA_PLUGINS_DIR if you installed the binary standalone.",
                        packs::plugins_dir().join("packs").display()
                    );
                }
                for (name, desc) in packs {
                    println!("  {:<14} {}", name, desc);
                }
                println!("\nInstall with:  kerna pack install <name>");
            }
            PackCommands::Install { name } => match packs::load_pack(&name) {
                Ok(pack) => {
                    let report = packs::install(&mut config, &pack);
                    config.save();
                    println!(
                        "[+] Installed pack '{}': {}",
                        pack.pack.name, pack.pack.description
                    );
                    if !report.added.is_empty() {
                        println!("    Added:   {}", report.added.join(", "));
                    }
                    if !report.skipped.is_empty() {
                        println!(
                            "    Skipped (already present): {}",
                            report.skipped.join(", ")
                        );
                    }
                    if !report.secrets_needed.is_empty() {
                        println!("\n  Set these secrets before use:");
                        for (plugin, env_var) in &report.secrets_needed {
                            let set = std::env::var(env_var)
                                .map(|v| !v.trim().is_empty())
                                .unwrap_or(false);
                            let status = if set {
                                "\x1b[32mset\x1b[0m"
                            } else {
                                "\x1b[31mmissing\x1b[0m"
                            };
                            println!(
                                "    {} → {} ({}); guide: kerna secrets add {}",
                                plugin, env_var, status, plugin
                            );
                        }
                    }
                    println!("\n  Suggested tools are set to require_confirmation (fail-closed).");
                    println!("  Review: kerna mcp list · kerna mcp risk <plugin>");
                }
                Err(e) => {
                    eprintln!("[-] {}", e);
                    std::process::exit(1);
                }
            },
        },

        Some(Commands::Routine { action }) => match action {
            RoutineCommands::List => {
                println!("Scheduled routines:\n");
                if config.schedules.is_empty() {
                    println!("  (none). Add one: kerna routine add morning-brief");
                }
                for (i, s) in config.schedules.iter().enumerate() {
                    let state = if s.enabled { "on" } else { "off" };
                    println!("  [{}] ({}) {}  →  {}", i, state, s.cron, s.goal);
                }
                if !config.schedules.is_empty() {
                    println!("\nRoutines run when the daemon is active:  kerna daemon");
                }
            }
            RoutineCommands::Add {
                template,
                cron,
                goal,
                allow_tool,
            } => {
                let (routine_name, cron_expr, goal_text, allowed_tools) = if let Some(t) =
                    template.as_deref()
                {
                    match routine_template(t) {
                        Some(template) => (
                            t.to_string(),
                            template.cron.to_string(),
                            template.goal.to_string(),
                            template
                                .allowed_tools
                                .iter()
                                .map(|tool| (*tool).to_string())
                                .collect(),
                        ),
                        None => {
                            eprintln!(
                                "[-] Unknown template '{}'. Available: morning-brief, meeting-prep, research-brief, daily-digest, morning-news, weekly-review.\n    Or add a custom routine: kerna routine add --cron \"0 0 8 * * *\" --goal \"...\"",
                                t
                            );
                            std::process::exit(1);
                        }
                    }
                } else if let (Some(c), Some(g)) = (cron.clone(), goal.clone()) {
                    ("custom".to_string(), c, g, allow_tool)
                } else {
                    eprintln!("[-] Provide a template name, or both --cron and --goal.");
                    std::process::exit(1);
                };
                config.schedules.push(config::ScheduleConfig {
                    name: routine_name.clone(),
                    cron: cron_expr.clone(),
                    goal: goal_text.clone(),
                    allowed_tools,
                    enabled: false,
                });
                config.save();
                let index = config.schedules.len() - 1;
                println!(
                    "[+] Added paused routine '{}' ({}): {}",
                    routine_name, cron_expr, goal_text
                );
                println!(
                    "    Reviewed tools: {}",
                    config.schedules[index].allowed_tools.join(", ")
                );
                println!("    Preview policy: kerna routine preview {}", index);
                println!("    Enable after review: kerna routine enable {}", index);
            }
            RoutineCommands::Preview { index } => {
                let Some(schedule) = config.schedules.get(index) else {
                    eprintln!(
                        "[-] No routine at index {} (see: kerna routine list).",
                        index
                    );
                    std::process::exit(1);
                };
                println!("Routine [{}]: {}", index, routine_name(schedule));
                println!(
                    "  State: {}",
                    if schedule.enabled {
                        "enabled"
                    } else {
                        "paused"
                    }
                );
                println!("  Schedule: {}", schedule.cron);
                println!("  Goal: {}", schedule.goal);
                println!(
                    "  Reviewed tools: {}",
                    if schedule.allowed_tools.is_empty() {
                        "(none)".to_string()
                    } else {
                        schedule.allowed_tools.join(", ")
                    }
                );
                let gaps = routine_enablement_gaps(&config, schedule);
                if gaps.is_empty() {
                    println!("  Policy: ready for unattended execution (all reviewed tools are explicitly auto-approved).");
                } else {
                    println!(
                        "  Policy: not ready. The following must be explicitly auto-approved: {}",
                        gaps.join(", ")
                    );
                    println!("  Keep write/send/delete tools out of routine allowlists. Review your kerna.toml before enabling.");
                }
            }
            RoutineCommands::Enable { index } => {
                let Some(schedule) = config.schedules.get(index) else {
                    eprintln!(
                        "[-] No routine at index {} (see: kerna routine list).",
                        index
                    );
                    std::process::exit(1);
                };
                let gaps = routine_enablement_gaps(&config, schedule);
                if !gaps.is_empty() {
                    eprintln!(
                        "[-] Routine '{}' remains paused. Explicit auto-approval is required for: {}.\n    Run `kerna routine preview {}` for the reviewed scope.",
                        routine_name(schedule),
                        gaps.join(", "),
                        index
                    );
                    std::process::exit(1);
                }
                let name = routine_name(schedule).to_string();
                config.schedules[index].enabled = true;
                config.save();
                println!("[+] Enabled routine '{}'. It will run only through its reviewed tool allowlist while `kerna daemon` is active.", name);
            }
            RoutineCommands::Disable { index } => {
                if index < config.schedules.len() {
                    let name = routine_name(&config.schedules[index]).to_string();
                    config.schedules[index].enabled = false;
                    config.save();
                    println!("[+] Paused routine '{}'.", name);
                } else {
                    eprintln!(
                        "[-] No routine at index {} (see: kerna routine list).",
                        index
                    );
                    std::process::exit(1);
                }
            }
            RoutineCommands::Run { index } => {
                let Some(schedule) = config.schedules.get(index) else {
                    eprintln!(
                        "[-] No routine at index {} (see: kerna routine list).",
                        index
                    );
                    std::process::exit(1);
                };
                let gaps = routine_enablement_gaps(&config, schedule);
                if !gaps.is_empty() {
                    eprintln!(
                            "[-] Routine '{}' cannot run unattended. Explicit auto-approval is required for: {}.\n    Run `kerna routine preview {}` for the reviewed scope.",
                            routine_name(schedule),
                            gaps.join(", "),
                            index
                        );
                    std::process::exit(1);
                }

                let name = routine_name(schedule).to_string();
                let goal = schedule.goal.clone();
                let allowed_tools = schedule.allowed_tools.clone();
                let scheduler =
                    TaskScheduler::new(config.clone(), memory.clone(), mcp_registry.clone(), None)?
                        .restrict_to_tools(allowed_tools)
                        .non_interactive();
                match scheduler.run_goal(&goal).await {
                    Ok(task_id) => {
                        println!("[+] Routine '{}' completed. Task ID: {}", name, task_id)
                    }
                    Err(e) => {
                        eprintln!("[-] Routine '{}' failed: {}", name, e);
                        std::process::exit(1);
                    }
                }
            }
            RoutineCommands::Remove { index } => {
                if index < config.schedules.len() {
                    let removed = config.schedules.remove(index);
                    config.save();
                    println!("[+] Removed routine: {}", removed.goal);
                } else {
                    eprintln!(
                        "[-] No routine at index {} (see: kerna routine list).",
                        index
                    );
                    std::process::exit(1);
                }
            }
        },

        Some(Commands::Approval { action }) => match action {
            ApprovalCommands::List => {
                let pending = memory.list_pending_approvals()?;
                if pending.is_empty() {
                    println!("No pending approvals.");
                }
                for (id, task_id, tool, args) in pending {
                    println!("{}  {}\n  task: {}\n  args: {}", id, tool, task_id, args);
                }
            }
            ApprovalCommands::Approve { id } => {
                if memory.decide_pending_approval(&id, true)? {
                    println!("[+] Approved approval {}.", id);
                } else {
                    eprintln!("[-] Approval {} is no longer pending.", id);
                    std::process::exit(1);
                }
            }
            ApprovalCommands::Deny { id } => {
                if memory.decide_pending_approval(&id, false)? {
                    println!("[+] Denied approval {}.", id);
                } else {
                    eprintln!("[-] Approval {} is no longer pending.", id);
                    std::process::exit(1);
                }
            }
            ApprovalCommands::Reject { id } => {
                if memory.decide_pending_approval(&id, false)? {
                    println!("[+] Rejected approval {}.", id);
                } else {
                    eprintln!("[-] Approval {} is no longer pending.", id);
                    std::process::exit(1);
                }
            }
        },

        Some(Commands::Status) => {
            let pending = memory.list_pending_approvals()?.len();
            let active = config
                .mcp_servers
                .iter()
                .filter(|server| server.enabled)
                .collect::<Vec<_>>();
            println!("Kerna project status\n");
            println!("Containment: Docker required for gateway plugins");
            println!("Network: disabled for production MCP containers");
            println!("Enabled plugins: {}", active.len());
            let workspace = std::env::current_dir()?;
            for server in active {
                let exposed =
                    match crate::plugin_manifest::verify_production_server(server, &workspace) {
                        Ok(manifest) => {
                            let configured = if !server.allow_tools.is_empty() {
                                server.allow_tools.clone()
                            } else if !server.capabilities.is_empty() {
                                server.capabilities.clone()
                            } else {
                                manifest.plugin.capabilities
                            };
                            if configured.is_empty() {
                                "none".to_string()
                            } else {
                                configured.join(", ")
                            }
                        }
                        Err(_) => "unverified (gateway will refuse startup)".to_string(),
                    };
                println!(
                    "  {}  image={}  ro=[{}]  rw=[{}]  exposed=[{}]",
                    server.name,
                    if server.image.is_empty() {
                        "<missing>"
                    } else {
                        &server.image
                    },
                    server.read_roots.join(","),
                    server.write_roots.join(","),
                    exposed
                );
            }
            println!("Pending approvals: {}", pending);
            println!("Audit store: {}", config.db_path);
        }

        Some(Commands::Plugins { action }) => {
            let reg = match registry::load() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "[-] {}\n    Set KERNA_PLUGINS_DIR if you installed the binary standalone.",
                        e
                    );
                    std::process::exit(1);
                }
            };
            let print_row = |p: &registry::RegistryPlugin| {
                let secrets = if p.secrets.is_empty() {
                    String::new()
                } else {
                    format!("  [needs: {}]", p.secrets.join(", "))
                };
                println!("  {:<10} {}{}", p.name, p.description, secrets);
            };
            match action {
                PluginsCommands::List => {
                    println!("Registry plugins:\n");
                    for p in &reg.plugins {
                        print_row(p);
                    }
                    println!("\nInstall with:  kerna plugins install <name>");
                }
                PluginsCommands::Search { query } => {
                    let hits = registry::search(&reg, &query);
                    println!("Plugins matching '{}':\n", query);
                    if hits.is_empty() {
                        println!("  (none)");
                    }
                    for p in &hits {
                        print_row(p);
                    }
                }
                PluginsCommands::Install { name } => match registry::find(&reg, &name) {
                    Some(plugin) => {
                        let report = registry::install(&mut config, plugin);
                        config.save();
                        if report.added.is_empty() {
                            println!("[i] Plugin '{}' is already installed.", name);
                        } else {
                            println!("[+] Installed plugin '{}': {}", name, plugin.description);
                        }
                        for (plug, env_var) in &report.secrets_needed {
                            let set = std::env::var(env_var)
                                .map(|v| !v.trim().is_empty())
                                .unwrap_or(false);
                            let status = if set {
                                "\x1b[32mset\x1b[0m"
                            } else {
                                "\x1b[31mmissing\x1b[0m"
                            };
                            println!(
                                "    Set {} ({}); guide: kerna secrets add {}",
                                env_var, status, plug
                            );
                        }
                        println!(
                            "  Suggested tools are require_confirmation (fail-closed). Review: kerna mcp risk {}",
                            name
                        );
                    }
                    None => {
                        eprintln!("[-] Plugin '{}' not found. Try: kerna plugins list", name);
                        std::process::exit(1);
                    }
                },
            }
        }

        Some(Commands::Folders { action }) => match action {
            FoldersCommands::Add {
                name,
                path,
                read_write,
            } => {
                if config.folders.iter().any(|g| g.name == name) {
                    eprintln!(
                        "[-] Folder '{}' is already granted. Remove it first: kerna folders remove {}",
                        name, name
                    );
                    std::process::exit(1);
                }
                let raw = std::path::Path::new(&path);
                if !raw.is_dir() {
                    eprintln!("[-] '{}' does not exist or is not a directory.", path);
                    std::process::exit(1);
                }
                let canonical = match raw.canonicalize() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("[-] Could not resolve '{}': {}", path, e);
                        std::process::exit(1);
                    }
                };
                // Windows canonicalize() emits a \\?\ extended-path prefix; strip it
                // for a path users recognize and can paste elsewhere.
                let display_path = canonical
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .to_string();
                config.folders.push(config::FolderGrant {
                    name: name.clone(),
                    path: display_path.clone(),
                    read_write,
                });
                config.save();
                let mode = if read_write {
                    "read-write"
                } else {
                    "read-only"
                };
                println!(
                    "[+] Granted {} access to '{}' as '{}'.",
                    mode, display_path, name
                );
                println!(
                    "    Agents can reach it with root: \"{}\" on file tools. Every {} still requires your confirmation, same as any other tool.",
                    name,
                    if read_write { "write" } else { "read" }
                );
                if !read_write {
                    println!("    To allow writes here: kerna folders remove {} && kerna folders add {} {} --read-write", name, name, path);
                }
            }
            FoldersCommands::List => {
                if config.folders.is_empty() {
                    println!("No folders granted. Add one with: kerna folders add <name> <path>");
                } else {
                    println!("Granted folders:\n");
                    for g in &config.folders {
                        let mode = if g.read_write {
                            "read-write"
                        } else {
                            "read-only "
                        };
                        println!("  {:<12} {}  {}", g.name, mode, g.path);
                    }
                }
            }
            FoldersCommands::Remove { name } => {
                let before = config.folders.len();
                config.folders.retain(|g| g.name != name);
                if config.folders.len() == before {
                    eprintln!("[-] No folder grant named '{}'.", name);
                    std::process::exit(1);
                }
                config.save();
                println!("[+] Revoked folder grant '{}'.", name);
            }
        },

        Some(Commands::Preferences { action }) => match action {
            PreferencesCommands::Set { key, value } => {
                if let Err(e) = memory.set_style_preference(&key, &value) {
                    eprintln!("[-] Could not save preference: {}", e);
                    std::process::exit(1);
                }
                println!("[+] Set preference '{}' = '{}'.", key, value);
                println!("    This is now included in every task's context.");
            }
            PreferencesCommands::List => match memory.get_style_preferences() {
                Ok(prefs) if prefs.is_empty() => {
                    println!(
                        "No preferences set. Add one with: kerna preferences set <key> <value>"
                    );
                }
                Ok(prefs) => {
                    println!("Your preferences:\n");
                    for (k, v) in prefs {
                        println!("  {:<20} {}", k, v);
                    }
                }
                Err(e) => {
                    eprintln!("[-] Could not read preferences: {}", e);
                    std::process::exit(1);
                }
            },
            PreferencesCommands::Remove { key } => match memory.remove_style_preference(&key) {
                Ok(true) => println!("[+] Removed preference '{}'.", key),
                Ok(false) => {
                    eprintln!("[-] No preference named '{}'.", key);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("[-] Could not remove preference: {}", e);
                    std::process::exit(1);
                }
            },
        },

        Some(Commands::Channel { action }) => match action {
            ChannelCommands::Add {
                platform,
                token_env,
                allow_id,
                name,
            } => {
                if platform != "telegram" {
                    eprintln!(
                        "[-] Unsupported platform '{}'. Supported: telegram.",
                        platform
                    );
                    std::process::exit(1);
                }
                if config.channels.iter().any(|c| c.name == name) {
                    eprintln!(
                        "[-] Channel '{}' already exists. Remove it first: kerna channel remove {}",
                        name, name
                    );
                    std::process::exit(1);
                }
                let has_token = std::env::var(&token_env)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                config.channels.push(config::ChatChannelConfig {
                    platform: platform.clone(),
                    name: name.clone(),
                    token_env: token_env.clone(),
                    allowed_ids: allow_id.clone(),
                    enabled: true,
                });
                config.save();
                println!("[+] Added {} channel '{}'.", platform, name);
                println!(
                    "    Bot token: reads {} ({}).",
                    token_env,
                    if has_token {
                        "set"
                    } else {
                        "MISSING — set it before starting the daemon"
                    }
                );
                if allow_id.is_empty() {
                    println!("    \x1b[33mNo allowlisted ids yet — nobody can trigger it.\x1b[0m Add one: kerna channel allow {} <id>", name);
                } else {
                    println!("    Allowed ids: {}", allow_id.join(", "));
                }
                println!("    Start listening with: kerna daemon");
            }
            ChannelCommands::List => {
                if config.channels.is_empty() {
                    println!(
                        "No channels. Add one with: kerna channel add telegram --allow-id <id>"
                    );
                } else {
                    println!("Configured channels:\n");
                    for c in &config.channels {
                        println!(
                            "  {:<12} {:<9} token_env={} allowed={:?}",
                            c.name, c.platform, c.token_env, c.allowed_ids
                        );
                    }
                }
            }
            ChannelCommands::Allow { name, id } => {
                match config.channels.iter_mut().find(|c| c.name == name) {
                    Some(c) => {
                        if c.allowed_ids.contains(&id) {
                            println!("[i] '{}' is already allowed on channel '{}'.", id, name);
                        } else {
                            c.allowed_ids.push(id.clone());
                            config.save();
                            println!("[+] Allowed '{}' on channel '{}'.", id, name);
                        }
                    }
                    None => {
                        eprintln!("[-] No channel named '{}'.", name);
                        std::process::exit(1);
                    }
                }
            }
            ChannelCommands::Remove { name } => {
                let before = config.channels.len();
                config.channels.retain(|c| c.name != name);
                if config.channels.len() == before {
                    eprintln!("[-] No channel named '{}'.", name);
                    std::process::exit(1);
                }
                config.save();
                println!("[+] Removed channel '{}'.", name);
            }
        },

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
                    println!("Most relevant:");
                    let query_embedding = crate::embeddings::embed(parts[1]);
                    if let Ok(results) = memory.search_episodic_memory(&query_embedding, 3) {
                        for (content, score) in &results {
                            println!("  - ({:.2}) {}", score, content);
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
mod routine_template_tests {
    use super::{routine_enablement_gaps, routine_template};
    use crate::config::{Config, PermissionRule, ScheduleConfig};

    #[test]
    fn daily_productivity_templates_are_available_and_read_only() {
        for name in ["morning-brief", "meeting-prep", "research-brief"] {
            let template = routine_template(name)
                .unwrap_or_else(|| panic!("daily productivity template '{name}' should exist"));
            assert_eq!(template.cron.split_whitespace().count(), 6);
            assert!(!template.allowed_tools.is_empty());
            assert!(
                template.goal.contains("Do not modify") || template.goal.contains("Do not publish")
            );
        }
    }

    #[test]
    fn routine_enablement_requires_an_explicit_read_allowlist_policy() {
        let schedule = ScheduleConfig {
            name: "brief".to_string(),
            cron: "0 0 8 * * Mon-Fri".to_string(),
            goal: "Read my calendar".to_string(),
            allowed_tools: vec!["list_events".to_string(), "list_notes".to_string()],
            enabled: false,
        };
        let mut config = Config::default();
        assert_eq!(
            routine_enablement_gaps(&config, &schedule),
            vec!["list_events".to_string(), "list_notes".to_string()]
        );

        config.permissions = vec![
            PermissionRule {
                tool: "list_events".to_string(),
                action: "auto_approve".to_string(),
            },
            PermissionRule {
                tool: "list_notes".to_string(),
                action: "auto_approve".to_string(),
            },
        ];
        assert!(routine_enablement_gaps(&config, &schedule).is_empty());
    }

    #[test]
    fn legacy_unscoped_routine_cannot_be_enabled() {
        let config = Config::default();
        let schedule = ScheduleConfig {
            name: String::new(),
            cron: "0 0 8 * * *".to_string(),
            goal: "Legacy task".to_string(),
            allowed_tools: vec![],
            enabled: true,
        };
        assert_eq!(
            routine_enablement_gaps(&config, &schedule),
            vec!["no reviewed tool allowlist".to_string()]
        );
    }
}

#[cfg(test)]
mod trust_layer_validation;
