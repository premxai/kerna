use crate::guard_routing::RouteMode;
use anyhow::{anyhow, Context, Result};
use dialoguer::{Confirm, Password, Select};
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use uuid::Uuid;
use zeroize::Zeroizing;

pub const PINNED_CLAUDE_CODE_VERSION: &str = "2.1.270";
pub const CLAUDE_AGENT_IMAGE: &str = "kerna-claude-agent:0.2.9-claude-2.1.270";
const CLAUDE_AGENT_CONTRACT: &str = "claude-agent-v1";
const CLAUDE_PACKAGE_SPEC: &str = "@anthropic-ai/claude-code@2.1.270";
const CLAUDE_PACKAGE_INTEGRITY: &str = "sha512-0zMkfIWQu7/SG56VP8r780HZWvrNShzK28AbAnhKRK0ns+ToGXPT0W8UqyZmZCUKAkJDd5//TrwSOhk1+hysiw==";
const RUST_BASE_DIGEST: &str =
    "sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0";
const NODE_BASE_DIGEST: &str =
    "sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d";
const BROKER_PORT: u16 = 8766;
const DASHBOARD_PORT: u16 = 8877;
/// Docker label scope for every Kerna-managed container and network. The
/// crash sweep finds resources by this label, so a half-dead session can never
/// hide from it behind an unexpected name.
const MANAGED_LABEL: &str = "dev.kerna.managed=true";
/// `MANAGED_LABEL` in `--filter` form. Docker filters require the `label=` key,
/// and a bare value is rejected by the daemon as an invalid filter.
const MANAGED_LABEL_FILTER: &str = "label=dev.kerna.managed=true";
const MANAGED_SESSION_LABEL_PREFIX: &str = "dev.kerna.session=";

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
    let mut checks = vec![
        command_check("Git", "git", &["--version"], true),
        command_check_with_candidates(
            "Docker",
            "docker",
            &["version", "--format", "{{.Server.Version}}"],
            &[r"C:\Program Files\Docker\Docker\resources\bin\docker.exe"],
            true,
        ),
        command_check("Node.js", "node", &["--version"], true),
        command_check_with_candidates(
            "Ollama",
            "ollama",
            &["--version"],
            &[r"C:\Users\ptula\AppData\Local\Programs\Ollama\ollama.exe"],
            demo,
        ),
    ];
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
    let contained_image = (!demo).then(verified_agent_image_id);
    checks.push(DoctorCheck {
        name: "Claude Code".to_string(),
        status: if if demo {
            command_exists("npm")
        } else {
            matches!(contained_image, Some(Ok(_)))
        } {
            "ready"
        } else {
            "missing"
        }
        .to_string(),
        detail: if demo {
            format!("degraded host-demo launcher {}", PINNED_CLAUDE_CODE_VERSION)
        } else if let Some(Ok(image_id)) = contained_image {
            format!("contained image {}", &image_id[..image_id.len().min(19)])
        } else {
            format!(
                "build {} with scripts/build-claude-agent-image",
                CLAUDE_AGENT_IMAGE
            )
        },
        required: true,
    });
    checks.push(DoctorCheck {
        name: "Cloud model".to_string(),
        status: "optional".to_string(),
        detail: format!(
            "{}; key from {}",
            crate::guard_routing::DEFAULT_CLOUD_MODEL,
            if std::env::var_os("KERNA_ANTHROPIC_KEY_FILE").is_some() {
                "KERNA_ANTHROPIC_KEY_FILE"
            } else {
                "a hidden prompt at launch"
            }
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
    let stale_containers = list_labeled(&[
        "ps",
        "-a",
        "--filter",
        MANAGED_LABEL_FILTER,
        "--format",
        "{{.Names}}",
    ])
    .unwrap_or_default();
    let stale_networks = list_labeled(&[
        "network",
        "ls",
        "--filter",
        MANAGED_LABEL_FILTER,
        "--format",
        "{{.Name}}",
    ])
    .unwrap_or_default();
    let retained_worktrees = retained_session_worktrees().len();
    checks.push(DoctorCheck {
        name: "Managed resources".to_string(),
        status: if stale_containers.is_empty() && stale_networks.is_empty() {
            "ready"
        } else {
            "degraded"
        }
        .to_string(),
        detail: if stale_containers.is_empty() && stale_networks.is_empty() {
            format!("{retained_worktrees} disposable worktree(s) retained for review")
        } else {
            format!(
                "{} container(s) and {} network(s) left by a crashed session; run kerna guard cleanup",
                stale_containers.len(),
                stale_networks.len()
            )
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

/// Provider key source for a contained session.
///
/// Default is the hidden terminal prompt. `KERNA_ANTHROPIC_KEY_FILE` exists for
/// the unattended rehearsal harness only: the value still travels to the broker
/// over stdin, is held in a zeroizing buffer, and never reaches argv, Docker
/// metadata, logs, or the repository. Keep the file outside any project tree.
fn read_provider_key() -> Result<Zeroizing<String>> {
    read_provider_key_from(std::env::var_os("KERNA_ANTHROPIC_KEY_FILE").as_deref())
}

fn read_provider_key_from(key_file: Option<&std::ffi::OsStr>) -> Result<Zeroizing<String>> {
    if let Some(path) = key_file {
        let raw = std::fs::read_to_string(path).map_err(|error| {
            // The path is deliberately omitted: it is operator-controlled and a
            // diagnostic must not become a second disclosure channel for it.
            anyhow!(
                "KERNA_ANTHROPIC_KEY_FILE could not be read ({})",
                error.kind()
            )
        })?;
        return Ok(Zeroizing::new(raw.trim().to_string()));
    }
    Ok(Zeroizing::new(
        Password::new()
            .with_prompt(
                "Anthropic API key (sent to the broker over stdin; never stored in container metadata)",
            )
            .interact()?,
    ))
}

/// Production Claude launch. The agent and trusted broker run in separate
/// containers. Only the broker receives provider authority or outbound egress.
pub async fn launch_claude(
    repo: &Path,
    route: RouteMode,
    shadow: bool,
    prompt: Option<&str>,
) -> Result<()> {
    if route != RouteMode::Cloud || shadow {
        return Err(anyhow!(
            "production containment currently supports --route cloud without shadow; use --host-demo only for the explicitly degraded local demo"
        ));
    }
    if !git_root(repo)?.is_dir() {
        return Err(anyhow!("repository is unavailable"));
    }
    // Rehearsals and provider faults can leave a labeled broker holding the
    // loopback port this session is about to allocate.
    match sweep_stale_sessions() {
        Ok(report) if !report.is_empty() => {
            println!(
                "[i] swept {} stale managed container(s) and {} network(s); {} disposable worktree(s) retained for review",
                report.containers.len(),
                report.networks.len(),
                report.retained_worktrees.len()
            );
        }
        Ok(_) => {}
        Err(error) => println!("[!] could not sweep stale managed resources: {error}"),
    }
    let image_id = verified_agent_image_id()?;
    let session_token = Uuid::new_v4().to_string();
    let suffix = &session_token[..8];
    let session_dir = create_disposable_clone(repo, &session_token)?;
    let state_dir = prepare_broker_state(&session_token)?;
    let evidence_db = state_dir.join("evidence.db");
    let cloud_key = read_provider_key()?;
    if cloud_key.trim().is_empty() {
        return Err(anyhow!("Anthropic API key cannot be empty"));
    }

    let agent_network = format!("kerna-agent-{suffix}");
    let egress_network = format!("kerna-egress-{suffix}");
    let broker_name = format!("kerna-broker-{suffix}");
    create_network(&agent_network, true, &session_token)?;
    let mut cleanup = ContainerCleanup::new(
        broker_name.clone(),
        agent_network.clone(),
        egress_network.clone(),
    );
    create_network(&egress_network, false, &session_token)?;

    let mut broker = start_broker_container(
        &image_id,
        &session_dir,
        &state_dir,
        &agent_network,
        &broker_name,
        &session_token,
    )?;
    if let Some(mut stdin) = broker.stdin.take() {
        stdin.write_all(cloud_key.as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    drop(cloud_key);
    cleanup.broker = Some(broker);
    wait_for_container(&broker_name)?;
    docker_status(&["network", "connect", &egress_network, &broker_name])?;

    let executable = std::env::current_exe()?;
    let dashboard_port = available_loopback_port(DASHBOARD_PORT)?;
    let mut dashboard = Command::new(&executable)
        .args([
            "dashboard",
            "--workspace",
            session_dir.to_string_lossy().as_ref(),
            "--port",
            &dashboard_port.to_string(),
            "--route",
            "cloud",
        ])
        .current_dir(&session_dir)
        .env("KERNA_DB_PATH", &evidence_db)
        .env("KERNA_DB_SHARED", "1")
        .env("KERNA_DB_READER", "1")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("could not start the Kerna dashboard")?;

    println!("[+] Contained session: {}", session_dir.display());
    println!("[+] Trusted evidence: {}", evidence_db.display());
    println!("[+] Dashboard: http://127.0.0.1:{dashboard_port}/");
    println!("[+] Claude boundary: Docker agent network; broker-only connectivity");
    let result = run_claude_container(
        &image_id,
        &session_dir,
        &agent_network,
        &broker_name,
        &session_token,
        prompt,
    );
    terminate_child(&mut dashboard);
    drop(cleanup);
    result
}

pub struct NativeAskBroker {
    pub base_url: String,
    pub session_token: String,
    container_name: String,
    network_name: String,
    child: Option<Child>,
}

impl Drop for NativeAskBroker {
    fn drop(&mut self) {
        let _ = docker_command()
            .args(["rm", "--force", &self.container_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Some(child) = self.child.as_mut() {
            terminate_child(child);
        }
        let _ = docker_command()
            .args(["network", "rm", &self.network_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Start the short-lived trusted provider broker used by `kerna ask`.
/// The provider key crosses stdin once and is never placed in Docker metadata.
pub fn start_native_ask_broker(
    repo: &Path,
    provider: &str,
    provider_key: &str,
) -> Result<NativeAskBroker> {
    if !matches!(provider, "anthropic" | "openai") {
        return Err(anyhow!(
            "native broker currently supports anthropic and openai"
        ));
    }
    let image_id = verified_agent_image_id()?;
    let session_token = Uuid::new_v4().to_string();
    let suffix = &session_token[..8];
    let session_dir = create_disposable_clone(repo, &session_token)?;
    let state_dir = prepare_broker_state(&session_token)?;
    let network_name = format!("kerna-native-egress-{suffix}");
    let container_name = format!("kerna-native-broker-{suffix}");
    let host_port = available_loopback_port(BROKER_PORT)?;
    create_network(&network_name, false, &session_token)?;

    let args = native_broker_container_args(&NativeBrokerContainerSpec {
        image_id: &image_id,
        session_dir: &session_dir,
        state_dir: &state_dir,
        network_name: &network_name,
        container_name: &container_name,
        host_port,
        session_token: &session_token,
        provider,
    });
    let child = match docker_command()
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = docker_command()
                .args(["network", "rm", &network_name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            return Err(error).context("could not start the native Kerna broker");
        }
    };
    let provider_path = if provider == "anthropic" {
        "native/anthropic"
    } else {
        "native/openai/v1"
    };
    let mut broker = NativeAskBroker {
        base_url: format!("http://127.0.0.1:{host_port}/{provider_path}"),
        session_token,
        container_name,
        network_name,
        child: Some(child),
    };
    if let Some(mut stdin) = broker.child.as_mut().and_then(|child| child.stdin.take()) {
        stdin.write_all(provider_key.as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    wait_for_container(&broker.container_name)?;
    let address = format!("127.0.0.1:{host_port}");
    for attempt in 0..50 {
        if std::net::TcpStream::connect(&address).is_ok() {
            break;
        }
        if attempt == 49 {
            return Err(anyhow!("native Kerna broker did not become reachable"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(broker)
}

struct NativeBrokerContainerSpec<'a> {
    image_id: &'a str,
    session_dir: &'a Path,
    state_dir: &'a Path,
    network_name: &'a str,
    container_name: &'a str,
    host_port: u16,
    session_token: &'a str,
    provider: &'a str,
}

fn native_broker_container_args(spec: &NativeBrokerContainerSpec<'_>) -> Vec<String> {
    let mut args = common_container_args(
        Some(spec.container_name),
        spec.network_name,
        spec.session_dir,
        spec.session_token,
    );
    args.extend([
        "--mount".to_string(),
        format!(
            "type=bind,src={},dst=/kerna-state",
            spec.state_dir.to_string_lossy()
        ),
        "--publish".to_string(),
        format!("127.0.0.1:{}:{BROKER_PORT}", spec.host_port),
        "--env".to_string(),
        "KERNA_DB_PATH=/kerna-state/evidence.db".to_string(),
        "--env".to_string(),
        "KERNA_DB_SHARED=1".to_string(),
        spec.image_id.to_string(),
        "kerna".to_string(),
        "serve".to_string(),
        "--port".to_string(),
        BROKER_PORT.to_string(),
        "--bind".to_string(),
        "0.0.0.0".to_string(),
        "--token".to_string(),
        spec.session_token.to_string(),
        "--route".to_string(),
        "cloud".to_string(),
        "--provider-key-stdin".to_string(),
        "--provider-key-kind".to_string(),
        spec.provider.to_string(),
    ]);
    args
}

/// Legacy guided-demo path. This remains available only behind `--host-demo`
/// and must never be represented as production containment.
pub async fn launch_claude_host_demo(
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
        Some(Zeroizing::new(
            Password::new()
                .with_prompt("Anthropic API key (kept only in trusted broker memory)")
                .interact()?,
        ))
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
        stdin.write_all(cloud_key.as_deref().map_or(b"", String::as_bytes))?;
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
    println!("[!] DEGRADED HOST DEMO: Claude is model-seam governed, not structurally contained.");
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

pub(crate) fn run_claude(
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

fn verified_agent_image_id() -> Result<String> {
    let output = docker_command()
        .args([
            "image",
            "inspect",
            CLAUDE_AGENT_IMAGE,
            "--format",
            "{{.Id}}|{{index .Config.Labels \"dev.kerna.contract\"}}|{{index .Config.Labels \"dev.kerna.claude-code-version\"}}|{{index .Config.Labels \"dev.kerna.claude-package\"}}|{{index .Config.Labels \"dev.kerna.claude-package-integrity\"}}|{{index .Config.Labels \"dev.kerna.rust-base-digest\"}}|{{index .Config.Labels \"dev.kerna.node-base-digest\"}}",
        ])
        .output()
        .context("could not inspect the pinned Kerna Claude agent image")?;
    if !output.status.success() {
        return Err(anyhow!(
            "the contained Claude image is unavailable; run scripts/build-claude-agent-image before a production session"
        ));
    }
    validate_agent_image_inspection(&String::from_utf8(output.stdout)?)
}

fn validate_agent_image_inspection(rendered: &str) -> Result<String> {
    let mut fields = rendered.trim().split('|');
    let image_id = fields.next().unwrap_or_default();
    let contract = fields.next().unwrap_or_default();
    let version = fields.next().unwrap_or_default();
    let package = fields.next().unwrap_or_default();
    let package_integrity = fields.next().unwrap_or_default();
    let rust_base = fields.next().unwrap_or_default();
    let node_base = fields.next().unwrap_or_default();
    if !image_id.starts_with("sha256:")
        || image_id.len() != 71
        || !image_id[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || contract != CLAUDE_AGENT_CONTRACT
        || version != PINNED_CLAUDE_CODE_VERSION
        || package != CLAUDE_PACKAGE_SPEC
        || package_integrity != CLAUDE_PACKAGE_INTEGRITY
        || rust_base != RUST_BASE_DIGEST
        || node_base != NODE_BASE_DIGEST
        || fields.next().is_some()
    {
        return Err(anyhow!(
            "Claude agent image provenance does not match the reviewed Kerna contract"
        ));
    }
    Ok(image_id.to_string())
}

fn docker_command() -> Command {
    if cfg!(windows) {
        let installed = Path::new(r"C:\Program Files\Docker\Docker\resources\bin\docker.exe");
        if installed.is_file() {
            return Command::new(installed);
        }
    }
    Command::new("docker")
}

fn docker_status(args: &[&str]) -> Result<()> {
    let output = docker_command().args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "docker {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn create_network(name: &str, internal: bool, session_token: &str) -> Result<()> {
    let mut args = vec![
        "network".to_string(),
        "create".to_string(),
        "--label".to_string(),
        MANAGED_LABEL.to_string(),
        "--label".to_string(),
        format!("{MANAGED_SESSION_LABEL_PREFIX}{}", &session_token[..8]),
    ];
    if internal {
        args.push("--internal".to_string());
    }
    args.push(name.to_string());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    docker_status(&borrowed)
}

fn container_user() -> String {
    if cfg!(unix) {
        let uid = Command::new("id").arg("-u").output();
        let gid = Command::new("id").arg("-g").output();
        if let (Ok(uid), Ok(gid)) = (uid, gid) {
            if uid.status.success() && gid.status.success() {
                return format!(
                    "{}:{}",
                    String::from_utf8_lossy(&uid.stdout).trim(),
                    String::from_utf8_lossy(&gid.stdout).trim()
                );
            }
        }
    }
    "10001:10001".to_string()
}

fn common_container_args(
    name: Option<&str>,
    network: &str,
    session_dir: &Path,
    session_token: &str,
) -> Vec<String> {
    let mut args = vec!["run".to_string(), "--rm".to_string(), "-i".to_string()];
    if let Some(name) = name {
        args.extend(["--name".to_string(), name.to_string()]);
    }
    // Same label convention create_network already uses, so a crash-swept
    // session's containers are discoverable and removable by label alone.
    args.extend([
        "--label".to_string(),
        MANAGED_LABEL.to_string(),
        "--label".to_string(),
        format!("{MANAGED_SESSION_LABEL_PREFIX}{}", &session_token[..8]),
    ]);
    args.extend([
        "--network".to_string(),
        network.to_string(),
        "--read-only".to_string(),
        "--cap-drop=ALL".to_string(),
        "--security-opt=no-new-privileges:true".to_string(),
        "--pids-limit=256".to_string(),
        "--memory=2g".to_string(),
        "--user".to_string(),
        container_user(),
        "--tmpfs".to_string(),
        "/tmp:rw,noexec,nosuid,nodev,size=128m,mode=1777".to_string(),
        "--mount".to_string(),
        format!(
            "type=bind,src={},dst=/workspace",
            session_dir.to_string_lossy()
        ),
        "--workdir".to_string(),
        "/workspace".to_string(),
        "--env".to_string(),
        "HOME=/tmp/kerna".to_string(),
    ]);
    args
}

fn start_broker_container(
    image_id: &str,
    session_dir: &Path,
    state_dir: &Path,
    network: &str,
    name: &str,
    session_token: &str,
) -> Result<Child> {
    let args = broker_container_args(
        image_id,
        session_dir,
        state_dir,
        network,
        name,
        session_token,
    );
    docker_command()
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("could not start the contained Kerna broker")
}

fn broker_container_args(
    image_id: &str,
    session_dir: &Path,
    state_dir: &Path,
    network: &str,
    name: &str,
    session_token: &str,
) -> Vec<String> {
    let mut args = common_container_args(Some(name), network, session_dir, session_token);
    args.extend([
        "--mount".to_string(),
        format!(
            "type=bind,src={},dst=/kerna-state",
            state_dir.to_string_lossy()
        ),
        "--env".to_string(),
        "KERNA_DB_PATH=/kerna-state/evidence.db".to_string(),
        "--env".to_string(),
        "KERNA_DB_SHARED=1".to_string(),
    ]);
    args.extend([
        image_id.to_string(),
        "kerna".to_string(),
        "serve".to_string(),
        "--port".to_string(),
        BROKER_PORT.to_string(),
        "--bind".to_string(),
        "0.0.0.0".to_string(),
        "--token".to_string(),
        session_token.to_string(),
        "--route".to_string(),
        "cloud".to_string(),
        "--provider-key-stdin".to_string(),
    ]);
    args
}

fn wait_for_container(name: &str) -> Result<()> {
    for _ in 0..50 {
        let output = docker_command()
            .args(["inspect", "--format", "{{.State.Running}}", name])
            .output()?;
        if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
            std::thread::sleep(Duration::from_millis(500));
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!("contained Kerna broker did not become ready"))
}

fn run_claude_container(
    image_id: &str,
    session_dir: &Path,
    network: &str,
    broker_name: &str,
    session_token: &str,
    prompt: Option<&str>,
) -> Result<()> {
    let mut args = common_container_args(None, network, session_dir, session_token);
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        args.insert(3, "-t".to_string());
    }
    args.extend([
        "--env".to_string(),
        "ANTHROPIC_API_KEY=".to_string(),
        "--env".to_string(),
        format!("ANTHROPIC_AUTH_TOKEN={session_token}"),
        "--env".to_string(),
        format!("ANTHROPIC_BASE_URL=http://{broker_name}:{BROKER_PORT}/anthropic"),
        image_id.to_string(),
        "claude".to_string(),
        "--bare".to_string(),
        "--disable-slash-commands".to_string(),
        "--tools".to_string(),
        "Read,Write,Edit,Bash,Glob,Grep".to_string(),
        "--allowedTools".to_string(),
        "Read,Write,Edit,Bash,Glob,Grep".to_string(),
    ]);
    if let Some(prompt) = prompt {
        args.extend([
            "--no-session-persistence".to_string(),
            "-p".to_string(),
            "--max-turns".to_string(),
            "8".to_string(),
            prompt.to_string(),
        ]);
    }
    let status = docker_command()
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("contained Claude exited with status {status}"))
    }
}

struct ContainerCleanup {
    broker_name: String,
    agent_network: String,
    egress_network: String,
    broker: Option<Child>,
}

impl ContainerCleanup {
    fn new(broker_name: String, agent_network: String, egress_network: String) -> Self {
        Self {
            broker_name,
            agent_network,
            egress_network,
            broker: None,
        }
    }
}

impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        let _ = docker_command()
            .args(["rm", "--force", &self.broker_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Some(child) = self.broker.as_mut() {
            terminate_child(child);
        }
        for network in [&self.agent_network, &self.egress_network] {
            let _ = docker_command()
                .args(["network", "rm", network])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Label-scoped crash cleanup. A rehearsal or a provider fault can leave a
/// broker container and its two networks behind, and a half-dead broker then
/// steals the loopback port the next session allocates. Managed resources are
/// found by label, never by name prefix. Disposable worktrees are deliberately
/// NOT deleted: a crash must preserve reviewable work.
pub struct SweepReport {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub retained_worktrees: Vec<PathBuf>,
}

impl SweepReport {
    pub fn is_empty(&self) -> bool {
        self.containers.is_empty() && self.networks.is_empty() && self.retained_worktrees.is_empty()
    }
}

pub fn sweep_stale_sessions() -> Result<SweepReport> {
    let mut report = SweepReport {
        containers: Vec::new(),
        networks: Vec::new(),
        retained_worktrees: Vec::new(),
    };
    for name in list_labeled(&[
        "ps",
        "-a",
        "--filter",
        MANAGED_LABEL_FILTER,
        "--format",
        "{{.Names}}",
    ])? {
        if remove_managed(&["rm", "--force"], &name)? {
            report.containers.push(name);
        }
    }
    for name in list_labeled(&[
        "network",
        "ls",
        "--filter",
        MANAGED_LABEL_FILTER,
        "--format",
        "{{.Name}}",
    ])? {
        if remove_managed(&["network", "rm"], &name)? {
            report.networks.push(name);
        }
    }
    report.retained_worktrees = retained_session_worktrees();
    Ok(report)
}

/// Managed resources still present. A live session owns its containers, so a
/// sweep legitimately leaves them; `kerna guard cleanup` reports the number
/// rather than staying silent about what it did not remove.
pub fn leftover_managed_count() -> Result<usize> {
    Ok(list_labeled(&[
        "ps",
        "-a",
        "--filter",
        MANAGED_LABEL_FILTER,
        "--format",
        "{{.Names}}",
    ])?
    .len()
        + list_labeled(&[
            "network",
            "ls",
            "--filter",
            MANAGED_LABEL_FILTER,
            "--format",
            "{{.Name}}",
        ])?
        .len())
}

/// Best-effort removal of one labeled resource. A resource a running session
/// still owns refuses to disappear, so the sweep reports it as left in place
/// rather than claiming a removal that Docker rejected.
fn remove_managed(prefix: &[&str], name: &str) -> Result<bool> {
    let status = docker_command()
        .args(prefix)
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

/// Disposable session worktrees on disk. A crash preserves reviewable work, so
/// neither the sweep nor doctor ever deletes these.
fn retained_session_worktrees() -> Vec<PathBuf> {
    let mut worktrees = Vec::new();
    if let Ok(entries) = std::fs::read_dir(session_root()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("session-"))
            {
                worktrees.push(path);
            }
        }
    }
    worktrees.sort();
    worktrees
}

fn list_labeled(args: &[&str]) -> Result<Vec<String>> {
    let output = docker_command().args(args).output()?;
    if !output.status.success() {
        // Docker may simply not be running yet; that is not a sweep failure.
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn prepare_broker_state(session_token: &str) -> Result<PathBuf> {
    let state_dir = session_root().join(format!("state-{}", &session_token[..8]));
    std::fs::create_dir_all(&state_dir)?;
    Ok(state_dir)
}

pub(crate) fn create_disposable_clone(repo: &Path, session_token: &str) -> Result<PathBuf> {
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

pub(crate) fn prepare_demo_contract(session_dir: &Path) -> Result<(PathBuf, PathBuf)> {
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

    fn scratch_key_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "kerna-provider-key-{}-{label}.tmp",
            std::process::id()
        ))
    }

    #[test]
    fn key_file_source_trims_the_value_and_nothing_else() {
        let path = scratch_key_path("valid");
        std::fs::write(&path, "  sk-rehearsal-token\n\n").unwrap();
        let key = read_provider_key_from(Some(path.as_os_str())).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&*key, "sk-rehearsal-token");
    }

    #[test]
    fn unreadable_key_file_reports_only_the_io_kind() {
        let path = scratch_key_path("missing");
        let _ = std::fs::remove_file(&path);
        let error = match read_provider_key_from(Some(path.as_os_str())) {
            Ok(_) => panic!("a missing key file must fail closed"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains("KERNA_ANTHROPIC_KEY_FILE"));
        assert!(
            !message.contains(&path.display().to_string()),
            "the operator-controlled path must not leak into diagnostics"
        );
    }

    #[test]
    fn sweep_report_is_empty_only_with_no_managed_resources_or_worktrees() {
        let mut report = SweepReport {
            containers: Vec::new(),
            networks: Vec::new(),
            retained_worktrees: Vec::new(),
        };
        assert!(report.is_empty());
        report
            .retained_worktrees
            .push(PathBuf::from("session-deadbeef"));
        assert!(
            !report.is_empty(),
            "retained worktrees must still be reported to the operator"
        );
    }

    #[test]
    fn agent_image_provenance_rejects_relabelled_or_mutable_inputs() {
        let valid = format!(
            "sha256:{}|{}|{}|{}|{}|{}|{}",
            "1".repeat(64),
            CLAUDE_AGENT_CONTRACT,
            PINNED_CLAUDE_CODE_VERSION,
            CLAUDE_PACKAGE_SPEC,
            CLAUDE_PACKAGE_INTEGRITY,
            RUST_BASE_DIGEST,
            NODE_BASE_DIGEST
        );
        assert!(validate_agent_image_inspection(&valid).is_ok());
        assert!(validate_agent_image_inspection(
            &valid.replace(CLAUDE_PACKAGE_SPEC, "@anthropic-ai/claude-code@latest")
        )
        .is_err());
        assert!(validate_agent_image_inspection(
            &valid.replace(NODE_BASE_DIGEST, "sha256:tampered")
        )
        .is_err());
        assert!(
            validate_agent_image_inspection(&valid.replace("sha256:1111", "tag:1111")).is_err()
        );
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

    #[test]
    fn production_container_contract_exposes_only_the_disposable_workspace() {
        let session_dir = std::env::temp_dir().join(format!("kerna-agent-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&session_dir).unwrap();
        let args = common_container_args(None, "kerna-agent-test", &session_dir, "abcdef01-token");
        let rendered = args.join(" ");
        assert!(rendered.contains("--network kerna-agent-test"));
        assert!(rendered.contains("--read-only"));
        assert!(rendered.contains("--cap-drop=ALL"));
        assert!(rendered.contains("--security-opt=no-new-privileges:true"));
        assert!(rendered.contains("dst=/workspace"));
        // Crash-sweep discoverability: containers must carry the same label
        // scope that managed networks already use.
        assert!(rendered.contains("--label dev.kerna.managed=true"));
        assert!(rendered.contains("--label dev.kerna.session=abcdef01"));
        assert!(!rendered.contains("abcdef01-token"));
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--mount").count(),
            1
        );
        for forbidden in [
            "docker.sock",
            "USERPROFILE",
            "APPDATA",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "provider-key",
        ] {
            assert!(!rendered.contains(forbidden), "leaked {forbidden}");
        }
        std::fs::remove_dir_all(session_dir).unwrap();
    }

    #[test]
    fn production_evidence_state_is_outside_the_agent_workspace() {
        let token = Uuid::new_v4().to_string();
        let state_dir = prepare_broker_state(&token).unwrap();
        let workspace = std::env::temp_dir().join(format!("kerna-agent-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        assert!(!state_dir.starts_with(&workspace));
        assert!(!workspace.join(".kerna-demo").exists());
        let _ = std::fs::remove_dir_all(state_dir);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn evidence_mount_is_broker_only_and_never_part_of_agent_arguments() {
        let workspace = std::env::temp_dir().join(format!("kerna-work-{}", Uuid::new_v4()));
        let state = std::env::temp_dir().join(format!("kerna-state-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let broker = broker_container_args(
            &format!("sha256:{}", "1".repeat(64)),
            &workspace,
            &state,
            "agent-net",
            "broker",
            "12345678-session",
        )
        .join(" ");
        let agent =
            common_container_args(None, "agent-net", &workspace, "12345678-session").join(" ");
        assert!(broker.contains("dst=/kerna-state"));
        assert!(broker.contains("KERNA_DB_PATH=/kerna-state/evidence.db"));
        assert!(broker.contains("KERNA_DB_SHARED=1"));
        assert!(!agent.contains("kerna-state"));
        assert!(!agent.contains("evidence.db"));
        assert!(!agent.contains("KERNA_DB_SHARED"));
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(state);
    }

    #[test]
    fn native_broker_is_loopback_only_and_provider_key_never_enters_metadata() {
        let workspace = std::env::temp_dir().join(format!("kerna-native-work-{}", Uuid::new_v4()));
        let state = std::env::temp_dir().join(format!("kerna-native-state-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let secret = "provider-secret-must-not-appear";
        let image = format!("sha256:{}", "1".repeat(64));
        let args = native_broker_container_args(&NativeBrokerContainerSpec {
            image_id: &image,
            session_dir: &workspace,
            state_dir: &state,
            network_name: "native-egress",
            container_name: "native-broker",
            host_port: 28766,
            session_token: "scoped-session-token",
            provider: "anthropic",
        });
        let rendered = args.join(" ");
        assert!(rendered.contains("127.0.0.1:28766:8766"));
        assert!(rendered.contains("--provider-key-stdin"));
        assert!(rendered.contains("--provider-key-kind anthropic"));
        assert!(rendered.contains("dst=/kerna-state"));
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains("ANTHROPIC_API_KEY"));
        assert!(!rendered.contains("OPENAI_API_KEY"));
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(state);
    }

    #[tokio::test]
    async fn production_launch_refuses_routes_that_need_host_services() {
        let error = launch_claude(Path::new("."), RouteMode::Local, false, None)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("production containment currently supports"));
        let error = launch_claude(Path::new("."), RouteMode::Cloud, true, None)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("production containment currently supports"));
    }
}
