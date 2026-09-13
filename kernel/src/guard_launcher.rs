use crate::guard_routing::RouteMode;
use anyhow::{anyhow, Context, Result};
use dialoguer::Password;
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

pub async fn print_doctor(demo: bool, repo: Option<&Path>) -> bool {
    let checks = doctor_checks(demo, repo).await;
    println!("Kerna Guard readiness");
    for check in &checks {
        let marker = match check.status.as_str() {
            "ready" => "+",
            "optional" => "~",
            _ => "-",
        };
        println!("[{marker}] {:<18} {}", check.name, check.detail);
    }
    println!("[i] Runtime data: C:\\KernaData\\kerna-demo");
    println!("[i] Session clones: C:\\Temp\\kerna-sessions");
    checks
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
    let model_ready = ollama_model_ready().await;
    checks.push(DoctorCheck {
        name: "Local model".to_string(),
        status: if model_ready { "ready" } else { "missing" }.to_string(),
        detail: if model_ready {
            crate::guard_routing::DEFAULT_LOCAL_MODEL.to_string()
        } else {
            format!(
                "{} is not installed",
                crate::guard_routing::DEFAULT_LOCAL_MODEL
            )
        },
        required: demo,
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
    if !print_doctor(false, Some(repo)).await {
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
        ])
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
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--strict-mcp-config");
    if let Some(prompt) = prompt {
        command.args(["-p", "--max-turns", "8", prompt]);
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
        .env("npm_config_cache", r"C:\KernaData\kerna-demo\npm-cache")
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
    let root = std::env::var_os("KERNA_SESSION_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                PathBuf::from(r"C:\Temp\kerna-sessions")
            } else {
                std::env::temp_dir().join("kerna-sessions")
            }
        });
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
    std::fs::write(
        contract_dir.join("kerna.toml"),
        r#"llm_provider = "mock"
llm_model = "mock"
db_path = "kerna-demo.db"
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
tool = "kerna_sandbox_run"
action = "auto_approve"

[[permissions]]
tool = "secret_probe"
action = "require_confirmation"

[[permissions]]
tool = "network_probe"
action = "deny"

[[permissions]]
tool = "*"
action = "deny"
"#,
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

async fn ollama_model_ready() -> bool {
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
        .any(|name| name == crate::guard_routing::DEFAULT_LOCAL_MODEL)
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
}
