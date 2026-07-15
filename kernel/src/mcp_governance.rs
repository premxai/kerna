use crate::config::McpServerConfig;
use crate::mcp::McpClient;
use anyhow::Result;
use std::time::Duration;
use tokio::time::timeout;

pub async fn probe(server_config: &McpServerConfig) -> Result<()> {
    println!("[*] Probing MCP Server: {}", server_config.name);

    // Convert Vec<String> to Vec<&str>
    let args: Vec<&str> = server_config.args.iter().map(|s| s.as_str()).collect();

    // We will use local runtime mode for probing, but sandbox is preferred.
    // For now we'll just run it locally since we're just inspecting.
    let mut client = McpClient::spawn(
        &server_config.command,
        &args,
        "local",
        "",
        "none",
        None,
        &server_config.secrets,
    )?;

    timeout(Duration::from_secs(5), client.initialize()).await??;
    println!("[+] Connection successful");
    println!("    Transport: stdio");
    println!(
        "    Command: {} {:?}",
        server_config.command, server_config.args
    );

    let tools = timeout(Duration::from_secs(5), client.list_tools()).await??;
    println!("[+] Discovered {} tools", tools.len());

    Ok(())
}

pub async fn inspect(server_config: &McpServerConfig) -> Result<()> {
    println!("[*] Inspecting MCP Server: {}", server_config.name);

    let args: Vec<&str> = server_config.args.iter().map(|s| s.as_str()).collect();
    let mut client = McpClient::spawn(
        &server_config.command,
        &args,
        "local",
        "",
        "none",
        None,
        &server_config.secrets,
    )?;

    timeout(Duration::from_secs(5), client.initialize()).await??;
    let tools = timeout(Duration::from_secs(5), client.list_tools()).await??;

    println!("[+] Raw Tools Extracted:");
    for tool in tools {
        println!(
            "  - {}: {:?}",
            tool.name,
            tool.description.unwrap_or_default()
        );
    }

    Ok(())
}

pub async fn generate_risk_card(server_config: &McpServerConfig) -> Result<()> {
    // Render the card for the effective policy, not merely the raw config. A
    // manifest can only narrow the callable tool set and add approval gates.
    let mut effective_server_config = server_config.clone();
    let manifest_path = crate::plugin_manifest::apply_to_server(&mut effective_server_config)?;
    let server_config = &effective_server_config;
    let args: Vec<&str> = server_config.args.iter().map(|s| s.as_str()).collect();
    let mut client = match McpClient::spawn(
        &server_config.command,
        &args,
        "local",
        "",
        "none",
        None,
        &server_config.secrets,
    ) {
        Ok(c) => c,
        Err(e) => {
            println!("\n[-] Failed to spawn plugin: {}", e);
            return Err(e);
        }
    };

    if let Err(e) = timeout(Duration::from_secs(5), client.initialize()).await {
        println!("\n[-] Failed to initialize plugin: {}", e);
        return Err(anyhow::anyhow!("Init timeout"));
    }

    let tools = match timeout(Duration::from_secs(5), client.list_tools()).await {
        Ok(Ok(t)) => t,
        _ => {
            println!("\n[-] Failed to list tools");
            return Err(anyhow::anyhow!("List tools timeout"));
        }
    };

    let mut auto_allow = Vec::new();
    let mut require_approval = Vec::new();
    let mut deny = Vec::new();

    for tool in &tools {
        let name_lower = tool.name.to_lowercase();

        // 1. Explicit Config Overrides
        if server_config.deny_tools.contains(&tool.name)
            || server_config.deny_tools.contains(&"*".to_string())
        {
            deny.push(format!("{} (explicitly blocked by deny_tools)", tool.name));
            continue;
        }
        if !server_config.allow_tools.is_empty()
            && !server_config.allow_tools.contains(&tool.name)
            && !server_config.allow_tools.contains(&"*".to_string())
        {
            deny.push(format!(
                "{} (implicitly blocked by allow_tools whitelist)",
                tool.name
            ));
            continue;
        }
        if server_config.approval_required.contains(&tool.name)
            || server_config.approval_required.contains(&"*".to_string())
        {
            require_approval.push(format!("{} (explicitly requires approval)", tool.name));
            continue;
        }

        // 2. Heuristic risk scoring. Fail-closed: a tool is auto-allowed ONLY
        //    when its name clearly denotes a read-only operation. Anything
        //    dangerous, sensitive, mutating, OR unrecognized requires review.
        //    Heuristics may only raise risk, never lower it.
        let has = |needles: &[&str]| needles.iter().any(|n| name_lower.contains(n));

        let dangerous = has(&[
            "delete", "drop", "remove", "destroy", "kill", "shell", "exec", "sudo", "format",
        ]);
        let sensitive = has(&[
            "secret",
            "credential",
            "password",
            "token",
            "apikey",
            "api_key",
            "network",
            "upload",
            "exfil",
            "probe",
            "env",
        ]);
        let mutating = has(&[
            "create", "write", "update", "insert", "post", "put", "modify", "send", "move",
            "rename", "install", "email",
        ]);
        let read_only = has(&[
            "get",
            "list",
            "read",
            "search",
            "find",
            "describe",
            "view",
            "show",
            "status",
            "ping",
            "echo",
            "query",
            "count",
            "fetch_metadata",
        ]);

        if dangerous {
            require_approval.push(format!("{} (heuristic: dangerous action)", tool.name));
        } else if sensitive {
            require_approval.push(format!(
                "{} (heuristic: touches secrets/network — review)",
                tool.name
            ));
        } else if mutating {
            require_approval.push(format!("{} (heuristic: mutating action)", tool.name));
        } else if read_only {
            auto_allow.push(tool.name.clone());
        } else {
            // Unknown capability — do not trust by default.
            require_approval.push(format!(
                "{} (unrecognized capability — requires review)",
                tool.name
            ));
        }
    }

    let (risk_level_text, risk_color) = if !deny.is_empty() {
        ("High", "\x1b[31m") // Red
    } else if !require_approval.is_empty() {
        ("Medium", "\x1b[33m") // Yellow
    } else {
        ("Low", "\x1b[32m") // Green
    };
    let reset = "\x1b[0m";

    println!("============================================================");
    println!("  Risk Card: {} MCP", server_config.name);
    println!("  Risk Level: {}{}{}", risk_color, risk_level_text, reset);
    println!("============================================================\n");

    println!("Security Scan Summary:");
    println!("  - Tools discovered: {}", tools.len());
    println!(
        "  - Explicit Deny rules: {}",
        server_config.deny_tools.len()
    );
    println!(
        "  - Explicit Allow rules: {}\n",
        server_config.allow_tools.len()
    );

    // Manifest disclosure: what the plugin *declares* it needs. Config secrets
    // and any plugins/<name>/manifest.toml are unioned so the user sees the
    // full picture (network reach + which secrets) before granting anything.
    let mut declared_secrets = server_config.secrets.clone();
    let mut declared_network: Vec<String> = Vec::new();
    if let Some(path) = manifest_path {
        if let Ok(m) = crate::plugin_manifest::PluginManifest::load(&path) {
            for s in m.plugin.secrets {
                if !declared_secrets.contains(&s) {
                    declared_secrets.push(s);
                }
            }
            declared_network = m.plugin.network_allowlist;
        }
    }
    if !declared_secrets.is_empty() {
        println!(
            "🔒 Secrets requested (set via `kerna secrets add {}`):",
            server_config.name
        );
        for s in &declared_secrets {
            let status = if std::env::var(s)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            {
                "\x1b[32mset\x1b[0m"
            } else {
                "\x1b[31mmissing\x1b[0m"
            };
            println!("    - {} ({})", s, status);
        }
        println!();
    }
    if !declared_network.is_empty() {
        println!("🌐 Network access (reaches the internet — not filesystem-sandboxed):");
        for n in &declared_network {
            println!("    - {}", n);
        }
        println!();
    }

    if !auto_allow.is_empty() {
        println!("🟢 Auto-allow (Read-only / Safe):");
        for t in auto_allow {
            println!("    - {}", t);
        }
        println!();
    }

    if !require_approval.is_empty() {
        println!("🟡 Require Approval (Mutating / Dangerous):");
        for t in require_approval {
            println!("    - {}", t);
        }
        println!();
    }

    if !deny.is_empty() {
        println!("🔴 Deny by Default (Blocked):");
        for t in deny {
            println!("    - {}", t);
        }
        println!();
    }

    Ok(())
}
