use crate::guard_routing::RouteMode;
use anyhow::{anyhow, Context, Result};
use dialoguer::{Confirm, Password, Select};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

pub const PINNED_CLAUDE_CODE_VERSION: &str = "2.1.270";
const BROKER_PORT: u16 = 8766;
const DASHBOARD_PORT: u16 = 8877;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
    pub required: bool,
}

pub fn storage_locations() -> serde_json::Value {
    let data_root = demo_data_root();
    serde_json::json!({
        "cargo_target": std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("kerna-target")),
        "sessions": session_root(),
        "ollama_models": std::env::var_os("OLLAMA_MODELS").map(PathBuf::from).unwrap_or_else(default_ollama_models_root),
        "demo_runtime": data_root,
    })
}

pub fn system_profile() -> serde_json::Value {
    let cpu = if cfg!(windows) {
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| std::env::consts::ARCH.to_string())
    } else if cfg!(target_os = "macos") {
        Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| std::env::consts::ARCH.to_string())
    } else {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    line.strip_prefix("model name").and_then(|value| {
                        value
                            .split_once(':')
                            .map(|(_, name)| name.trim().to_string())
                    })
                })
            })
            .unwrap_or_else(|| std::env::consts::ARCH.to_string())
    };
    serde_json::json!({
        "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "cpu": cpu,
    })
}

pub async fn print_doctor_brief(demo: bool, repo: Option<&Path>) -> bool {
    let checks = doctor_checks(demo, repo).await;
    let hardware = crate::models::detect_hardware();
    let local_route = crate::guard_routing::active_local_model()
        .map(|model| format!("local {model}"))
        .unwrap_or_else(|| "local disabled".to_string());
    let ready = checks
        .iter()
        .filter(|check| check.status == "ready")
        .count();
    let required = checks.iter().filter(|check| check.required).count();
    println!(
        "Kerna doctor · {} {} · {} · {} GB VRAM",
        std::env::consts::OS,
        std::env::consts::ARCH,
        hardware.name,
        hardware.memory_gb.unwrap_or(0)
    );
    println!("[i] Route: auto · {local_route} · cloud on demand");
    println!("[i] Readiness: {ready}/{required} required checks ready");
    checks
        .iter()
        .filter(|check| check.status != "ready")
        .for_each(|check| {
            println!(
                "[{}] {} · {}",
                if check.status == "optional" { "~" } else { "-" },
                check.name,
                check.detail
            );
        });
    checks
        .iter()
        .all(|check| !check.required || check.status == "ready")
}

pub async fn print_doctor(demo: bool, repo: Option<&Path>) -> bool {
    let checks = doctor_checks(demo, repo).await;
    let hardware = crate::models::detect_hardware();
    println!("Kerna doctor");
    println!(
        "[i] System             {} {} · {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        system_profile()["cpu"].as_str().unwrap_or("CPU unknown")
    );
    println!(
        "[i] Accelerator        {} · {} GB",
        hardware.name,
        hardware
            .memory_gb
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    let local_route = crate::guard_routing::active_local_model()
        .map(|model| format!("local {model}"))
        .unwrap_or_else(|| "local disabled".to_string());
    println!(
        "[i] Routing            auto · {local_route} for private/read-only · cloud for complex changes"
    );
    for check in &checks {
        let marker = match check.status.as_str() {
            "ready" => "+",
            "optional" => "~",
            _ => "-",
        };
        println!("[{marker}] {:<18} {}", check.name, check.detail);
    }
    let storage = storage_locations();
    println!("[i] Keys               Anthropic is requested at cloud launch and never persisted");
    println!(
        "[i] Runtime data       {}",
        storage["demo_runtime"]
            .as_str()
            .unwrap_or("configured locally")
    );
    println!(
        "[i] Session clones     {}",
        storage["sessions"].as_str().unwrap_or("system temp")
    );
    checks
        .iter()
        .all(|check| !check.required || check.status == "ready")
}

/// Run the same readiness checks without turning a normal launch into a doctor report.
pub async fn readiness_ok(demo: bool, repo: Option<&Path>) -> bool {
    doctor_checks(demo, repo)
        .await
        .iter()
        .all(|check| !check.required || check.status == "ready")
}

pub async fn doctor_checks(demo: bool, repo: Option<&Path>) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    checks.push(command_check("Git", "git", &["--version"], true));
    checks.push(command_check_with_candidates(
        "Docker",
        "docker",
        &["version", "--format", "{{.Server.Version}}"],
        &[r"C:\Program Files\Docker\Docker\resources\bin\docker.exe"],
        true,
    ));
    checks.push(command_check("Node.js", "node", &["--version"], true));
    checks.push(command_check_with_candidates(
        "Ollama",
        "ollama",
        &["--version"],
        &[r"C:\Users\ptula\AppData\Local\Programs\Ollama\ollama.exe"],
        demo,
    ));
    let selected_model = crate::guard_routing::active_local_model();
    let model_ready = match selected_model.as_deref() {
        Some(model) => ollama_model_ready_for(model).await,
        None => false,
    };
    checks.push(DoctorCheck {
        name: "Local model".to_string(),
        status: if selected_model.is_none() {
            "optional".to_string()
        } else if model_ready {
            "ready".to_string()
        } else {
            "missing".to_string()
        },
        detail: match selected_model.as_deref() {
            Some(model) if model_ready => model.to_string(),
            Some(model) => format!("{model} is not installed"),
            None => "not selected; local routing and shadow disabled".to_string(),
        },
        required: demo && selected_model.is_some(),
    });
    checks.push(DoctorCheck {
        name: "Claude Code".to_string(),
        status: if command_exists("npm") {
            "ready"
        } else {
            "missing"
        }
        .to_string(),
        detail: format!("pinned launcher {}", PINNED_CLAUDE_CODE_VERSION),
        required: true,
    });
    checks.push(DoctorCheck {
        name: "Cloud model".to_string(),
        status: "optional".to_string(),
        detail: format!(
            "{}; key requested in a hidden launch prompt",
            crate::guard_routing::DEFAULT_CLOUD_MODEL
        ),
        required: false,
    });
    checks.push(DoctorCheck {
        name: "Wasmer SDK".to_string(),
        status: if crate::sponsor_runtime::bridge_available() {
            "ready"
        } else {
            "missing"
        }
        .to_string(),
        detail: "Node SDK bridge; sandbox networking disabled".to_string(),
        required: demo,
    });
    checks.push(DoctorCheck {
        name: "Tenki".to_string(),
        status: if std::env::var("TENKI_API_KEY")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            "ready"
        } else {
            "optional"
        }
        .to_string(),
        detail: if std::env::var_os("TENKI_API_KEY").is_some() {
            "API key detected; remote backend optional".to_string()
        } else {
            "configured — authentication required".to_string()
        },
        required: false,
    });
    if let Some(repo) = repo {
        checks.push(DoctorCheck {
            name: "Repository".to_string(),
            status: if git_root(repo).is_ok() {
                "ready"
            } else {
                "missing"
            }
            .to_string(),
            detail: repo.display().to_string(),
            required: true,
        });
    }
    checks
}

pub async fn launch_claude(
    repo: &Path,
    route: RouteMode,
    shadow: bool,
    prompt: Option<&str>,
) -> Result<()> {
    let demo_profile = crate::guard_routing::load_demo_profile();
    if route != RouteMode::Local && !demo_profile.cloud_enabled {
        return Err(anyhow!(
            "cloud routing is disabled in the demo profile; rerun `kerna init --demo` or use --route local"
        ));
    }
    if !readiness_ok(false, Some(repo)).await {
        return Err(anyhow!("Kerna Guard readiness checks failed"));
    }
    let session_token = Uuid::new_v4().to_string();
    let session_dir = create_disposable_clone(repo, &session_token)?;
    let (contract_dir, mcp_config) = prepare_demo_contract(&session_dir)?;
    let broker_port = available_loopback_port(BROKER_PORT)?;
    let dashboard_port = available_loopback_port(DASHBOARD_PORT)?;
    let cloud_key = if route == RouteMode::Local {
        None
    } else {
        Some(
            Password::new()
                .with_prompt("Anthropic API key (kept only in trusted broker memory)")
                .interact()?,
        )
    };
    let executable = std::env::current_exe()?;
    let mut broker = Command::new(&executable)
        .args([
            "serve",
            "--port",
            &broker_port.to_string(),
            "--bind",
            "127.0.0.1",
            "--token",
            &session_token,
            "--route",
            route_name(route),
            "--provider-key-stdin",
        ])
        .args(shadow.then_some("--shadow"))
        .current_dir(&contract_dir)
        .env("KERNA_GUARD_AGENT_VERSION", PINNED_CLAUDE_CODE_VERSION)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("could not start the Kerna broker")?;
    if let Some(mut stdin) = broker.stdin.take() {
        stdin.write_all(cloud_key.as_deref().unwrap_or_default().as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    drop(cloud_key);

    let mut dashboard = Command::new(&executable)
        .args([
            "dashboard",
            "--workspace",
            contract_dir.to_string_lossy().as_ref(),
            "--port",
            &dashboard_port.to_string(),
            "--route",
            route_name(route),
        ])
        .args(shadow.then_some("--shadow"))
        .current_dir(&contract_dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("could not start the Kerna dashboard")?;

    std::thread::sleep(Duration::from_millis(900));
    println!("[+] Disposable session: {}", session_dir.display());
    println!("[+] Dashboard: http://127.0.0.1:{dashboard_port}/");
    println!("[i] Claude is model-seam governed; the host process is not fully containerized.");
    let result = run_claude(
        &session_dir,
        &mcp_config,
        &session_token,
        broker_port,
        prompt,
    );
    terminate_child(&mut dashboard);
    terminate_child(&mut broker);
    result
}

/// Guided setup for the hackathon surface. It stores only non-secret choices;
/// provider credentials are intentionally collected at the operation that uses them.
pub async fn run_demo_setup(brief: bool) -> Result<()> {
    let fast_path = crate::guard_routing::demo_profile_path().is_file();
    println!("  _  __                     ");
    println!(" | |/ /___ _ __ _ __   __ _ ");
    println!(" | ' // _ \\ '__| '_ \\ / _` |");
    println!(" | . \\  __/ |  | | | | (_| |");
    println!(" |_|\\_\\___|_|  |_| |_|\\__,_|");
    println!();
    println!("Welcome to Kerna — the runtime trust layer for autonomous agents.");
    println!("✓ routing  ✓ fail-closed permissions  ✓ approvals  ✓ receipts");
    println!();
    println!("Kerna demo setup\n");
    if fast_path {
        println!("[i] Existing demo profile found; starting the preflighted control room.");
    }
    let local_choices = [
        "qwen3.5:9b — recommended, strongest local demo",
        "qwen3:4b — faster fallback",
        "Skip local model — cloud only, no shadow",
    ];
    let mut profile = crate::guard_routing::load_demo_profile();
    if !fast_path {
        println!("Choose the local model used for private routing and cloud shadows.");
        let local_selection = Select::new()
            .with_prompt("Local model")
            .items(local_choices)
            .default(0)
            .interact()?;
        profile.local_model = match local_selection {
            0 => Some("qwen3.5:9b".to_string()),
            1 => Some("qwen3:4b".to_string()),
            _ => None,
        };
    }

    if let Some(model) = profile.local_model.as_deref() {
        if !ollama_model_ready_for(model).await {
            let should_pull = Confirm::new()
                .with_prompt(format!("{model} is not installed. Pull it now?"))
                .default(true)
                .interact()?;
            if should_pull {
                if let Some(ollama) = ollama_executable() {
                    println!("[i] Pulling {model}; this may take a few minutes...");
                    let status = Command::new(ollama).args(["pull", model]).status()?;
                    if !status.success() {
                        eprintln!(
                            "[!] Ollama could not pull {model}; local routing remains unavailable."
                        );
                    }
                } else {
                    eprintln!("[!] Ollama is unavailable. Run `kerna doctor` after installing it.");
                }
            }
        }
    }

    if !fast_path {
        let cloud_selection = Select::new()
            .with_prompt("Cloud route")
            .items([
                "Anthropic claude-sonnet-5 — key requested only at cloud launch",
                "Skip cloud for now",
            ])
            .default(0)
            .interact()?;
        profile.cloud_enabled = cloud_selection == 0;
        profile.wasmer_enabled = Confirm::new()
            .with_prompt("Enable Wasmer local sandbox? (required for the demo)")
            .default(true)
            .interact()?;
        profile.tenki_enabled = Confirm::new()
            .with_prompt("Enable Tenki remote sandbox? Its key is requested only at first use")
            .default(true)
            .interact()?;
    }
    crate::guard_routing::save_demo_profile(&profile)?;

    std::fs::create_dir_all(demo_data_root())?;
    std::fs::create_dir_all(session_root())?;
    std::fs::create_dir_all(demo_data_root().join("wasmer-cache"))?;
    std::fs::create_dir_all(demo_data_root().join("npm-cache"))?;
    println!(
        "\n[+] Non-secret demo profile saved at {}",
        crate::guard_routing::demo_profile_path().display()
    );

    let workspace = std::env::current_dir()?;
    let ready = if brief {
        print_doctor_brief(true, Some(&workspace)).await
    } else {
        print_doctor(true, Some(&workspace)).await
    };
    if !ready {
        eprintln!("[-] Demo setup is incomplete. Start Docker Desktop and rerun `kerna doctor`.");
        return Ok(());
    }

    let port = available_loopback_port(DASHBOARD_PORT)?;
    let executable = std::env::current_exe()?;
    let mut dashboard_command = if cfg!(windows) {
        let quote_powershell = |value: &str| format!("'{}'", value.replace('\'', "''"));
        let arguments = format!(
            "dashboard --workspace \"{}\" --port {} --route auto --shadow --no-open",
            workspace.display(),
            port
        );
        let dashboard_log = demo_data_root().join("dashboard.log");
        let dashboard_error_log = demo_data_root().join("dashboard.error.log");
        let script = format!(
            "Start-Process -WindowStyle Hidden -FilePath {} -ArgumentList {} -WorkingDirectory {} -RedirectStandardOutput {} -RedirectStandardError {}",
            quote_powershell(&executable.display().to_string()),
            quote_powershell(&arguments),
            quote_powershell(&workspace.display().to_string()),
            quote_powershell(&dashboard_log.display().to_string()),
            quote_powershell(&dashboard_error_log.display().to_string())
        );
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]);
        command
    } else {
        let mut command = Command::new(&executable);
        command
            .arg("dashboard")
            .arg("--workspace")
            .arg(&workspace)
            .arg("--port")
            .arg(port.to_string())
            .arg("--route")
            .arg("auto")
            .arg("--shadow")
            .arg("--no-open");
        command
    };
    dashboard_command
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    dashboard_command
        .spawn()
        .context("could not start the demo dashboard")?;
    let url = format!("http://127.0.0.1:{port}/");
    let ready = (0..20).any(|_| {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            true
        } else {
            std::thread::sleep(Duration::from_millis(100));
            false
        }
    });
    if !ready {
        return Err(anyhow!("demo dashboard did not become ready at {url}"));
    }
    println!("[+] Dashboard: {url}");
    let _ = webbrowser::open(&url);
    println!("\nYour next commands:");
    println!("  kerna claude --route local --prompt \"Summarize the security invariants. Do not modify files.\"");
    println!("  kerna");
    println!("  kerna sandbox");
    Ok(())
}

fn run_claude(
    session_dir: &Path,
    mcp_config: &Path,
    session_token: &str,
    broker_port: u16,
    prompt: Option<&str>,
) -> Result<()> {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let package = format!("@anthropic-ai/claude-code@{PINNED_CLAUDE_CODE_VERSION}");
    let mut command = Command::new(npm);
    command.args(["exec", "--yes", "--package", &package, "--", "claude"]);
    command
        .arg("--bare")
        .arg("--disable-slash-commands")
        .arg("--tools")
        .arg("")
        .arg("--allowedTools")
        .arg(
            "mcp__kerna-governed-tools__echo,mcp__kerna-governed-tools__kerna_session_status,mcp__kerna-governed-tools__kerna_sandbox_run,mcp__kerna-governed-tools__secret_probe,mcp__kerna-governed-tools__network_probe",
        )
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--strict-mcp-config");
    if let Some(prompt) = prompt {
        // Claude Code 2.1.270 accepts --no-session-persistence only with
        // --print/-p. Interactive sessions are still isolated by Kerna's
        // disposable clone, but must omit Claude's print-only flag.
        command.args(["--no-session-persistence", "-p", "--max-turns", "8", prompt]);
    }
    command
        .current_dir(session_dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "SYSTEMROOT",
            std::env::var("SYSTEMROOT").unwrap_or_default(),
        )
        .env("COMSPEC", std::env::var("COMSPEC").unwrap_or_default())
        .env("PATHEXT", std::env::var("PATHEXT").unwrap_or_default())
        .env("TEMP", std::env::var("TEMP").unwrap_or_default())
        .env("TMP", std::env::var("TMP").unwrap_or_default())
        .env("npm_config_cache", demo_data_root().join("npm-cache"))
        .env("ANTHROPIC_API_KEY", "")
        .env("ANTHROPIC_AUTH_TOKEN", session_token)
        .env(
            "ANTHROPIC_BASE_URL",
            format!("http://127.0.0.1:{broker_port}/anthropic"),
        );
    let status = command
        .status()
        .context("could not launch pinned Claude Code")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Claude Code exited with status {status}"))
    }
}

fn create_disposable_clone(repo: &Path, session_token: &str) -> Result<PathBuf> {
    let source = git_root(repo)?;
    let root = session_root();
    std::fs::create_dir_all(&root)?;
    let destination = root.join(format!("session-{}", &session_token[..8]));
    let output = Command::new("git")
        .args([
            "-c",
            "safe.directory=*",
            "clone",
            "--local",
            "--no-hardlinks",
            "--quiet",
        ])
        .arg(&source)
        .arg(&destination)
        .output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "could not create disposable clone: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(destination)
}

fn prepare_demo_contract(session_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let contract_dir = session_dir.join(".kerna-demo");
    std::fs::create_dir_all(&contract_dir)?;
    let evidence_db = contract_dir
        .join("kerna-demo.db")
        .to_string_lossy()
        .replace('\\', "/");
    std::fs::write(
        contract_dir.join("kerna.toml"),
        format!(
            r#"llm_provider = "mock"
llm_model = "mock"
db_path = '{evidence_db}'
sandbox_dir = "sandbox"
memory_backend = "sqlite"
runtime_mode = "docker"
network_mode = "none"
max_tool_calls = 20
max_llm_calls = 0
max_runtime_seconds = 900
max_output_bytes = 65536
max_memory_writes = 0
max_cost_usd = 0.0

[[mcp_servers]]
name = "kerna-demo-tools"
runtime_mode = "demo"
enabled = true

[[permissions]]
tool = "echo"
action = "auto_approve"

[[permissions]]
tool = "mcp__kerna-governed-tools__echo"
action = "auto_approve"

[[permissions]]
tool = "kerna_session_status"
action = "auto_approve"

[[permissions]]
tool = "mcp__kerna-governed-tools__kerna_session_status"
action = "auto_approve"

[[permissions]]
tool = "kerna_sandbox_run"
action = "auto_approve"

[[permissions]]
tool = "mcp__kerna-governed-tools__kerna_sandbox_run"
action = "auto_approve"

[[permissions]]
tool = "secret_probe"
action = "require_confirmation"

[[permissions]]
tool = "mcp__kerna-governed-tools__secret_probe"
action = "require_confirmation"

[[permissions]]
tool = "network_probe"
action = "deny"

[[permissions]]
tool = "mcp__kerna-governed-tools__network_probe"
action = "deny"

[[permissions]]
tool = "*"
action = "deny"
"#
        ),
    )?;

    let executable = std::env::current_exe()?;
    let mcp_config = session_dir.join(".mcp.json");
    let rendered = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "kerna-governed-tools": {
                "command": executable,
                "args": ["gateway", "--workspace", contract_dir]
            }
        }
    }))?;
    std::fs::write(&mcp_config, rendered)?;
    Ok((contract_dir, mcp_config))
}

fn git_root(repo: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["-c", "safe.directory=*", "rev-parse", "--show-toplevel"])
        .current_dir(repo)
        .output()?;
    if !output.status.success() {
        return Err(anyhow!("{} is not a Git repository", repo.display()));
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)?.trim().to_string(),
    ))
}

fn route_name(route: RouteMode) -> &'static str {
    match route {
        RouteMode::Auto => "auto",
        RouteMode::Local => "local",
        RouteMode::Cloud => "cloud",
    }
}

fn demo_data_root() -> PathBuf {
    if let Some(root) = std::env::var_os("KERNA_DEMO_DATA_DIR") {
        return PathBuf::from(root);
    }
    if cfg!(windows) {
        return PathBuf::from(r"C:\KernaData\kerna-demo");
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("kerna-demo")
}

fn session_root() -> PathBuf {
    std::env::var_os("KERNA_SESSION_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from(r"C:\Temp\kerna-sessions")
            } else {
                std::env::temp_dir().join("kerna-sessions")
            }
        })
}

fn default_ollama_models_root() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"))
            .join(".ollama/models")
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".ollama/models")
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn available_loopback_port(start: u16) -> Result<u16> {
    for port in start..start.saturating_add(100) {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            drop(listener);
            return Ok(port);
        }
    }
    Err(anyhow!("no free loopback port available near {start}"))
}

fn command_exists(program: &str) -> bool {
    let executable = if cfg!(windows) && program == "npm" {
        "npm.cmd"
    } else {
        program
    };
    Command::new(executable)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn command_check(name: &str, program: &str, args: &[&str], required: bool) -> DoctorCheck {
    command_check_with_candidates(name, program, args, &[], required)
}

fn command_check_with_candidates(
    name: &str,
    program: &str,
    args: &[&str],
    candidates: &[&str],
    required: bool,
) -> DoctorCheck {
    let candidate = std::iter::once(PathBuf::from(program))
        .chain(candidates.iter().map(PathBuf::from))
        .find(|candidate| {
            Command::new(candidate)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        });
    DoctorCheck {
        name: name.to_string(),
        status: if candidate.is_some() {
            "ready"
        } else {
            "missing"
        }
        .to_string(),
        detail: candidate
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not available".to_string()),
        required,
    }
}

async fn ollama_model_ready_for(model: &str) -> bool {
    let Ok(response) = reqwest::Client::new()
        .get(format!(
            "{}/api/tags",
            crate::guard_routing::DEFAULT_LOCAL_BASE_URL
        ))
        .timeout(Duration::from_secs(2))
        .send()
        .await
    else {
        return false;
    };
    let Ok(payload) = response.json::<serde_json::Value>().await else {
        return false;
    };
    payload
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
        .any(|name| name == model || name.strip_suffix(":latest") == model.strip_suffix(":latest"))
}

fn ollama_executable() -> Option<PathBuf> {
    let candidates = if cfg!(windows) {
        vec![
            PathBuf::from("ollama"),
            PathBuf::from(r"C:\Users\ptula\AppData\Local\Programs\Ollama\ollama.exe"),
        ]
    } else {
        vec![PathBuf::from("ollama")]
    };
    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_names_are_stable_cli_values() {
        assert_eq!(route_name(RouteMode::Auto), "auto");
        assert_eq!(route_name(RouteMode::Local), "local");
        assert_eq!(route_name(RouteMode::Cloud), "cloud");
    }

    #[test]
    fn demo_contract_binds_every_process_to_one_evidence_database() {
        let session_dir = std::env::temp_dir().join(format!("kerna-contract-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&session_dir).unwrap();
        let (contract_dir, _) = prepare_demo_contract(&session_dir).unwrap();
        let config: crate::config::Config =
            toml::from_str(&std::fs::read_to_string(contract_dir.join("kerna.toml")).unwrap())
                .unwrap();
        let expected = contract_dir.join("kerna-demo.db");
        assert!(Path::new(&config.db_path).is_absolute());
        assert_eq!(PathBuf::from(config.db_path), expected);
        std::fs::remove_dir_all(session_dir).unwrap();
    }
}
