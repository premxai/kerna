use crate::{
    config::Config,
    guard_launcher,
    guard_routing::RouteMode,
    mcp_registry::McpRegistry,
    memory::MemoryEngine,
    server,
    sponsor_runtime::{self, SandboxOutcome, SandboxRequest},
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use uuid::Uuid;

static TENKI: OnceLock<Mutex<TenkiWorker>> = OnceLock::new();

pub fn tenki_ready() -> bool {
    TENKI.get().is_some_and(|worker| {
        worker
            .try_lock()
            .map(|worker| worker.prepared.elapsed() < Duration::from_secs(840))
            .unwrap_or(true)
    })
}

struct TenkiWorker {
    prepared: Instant,
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<String>,
}
impl TenkiWorker {
    fn start(key: String) -> Result<Self> {
        let runtime = sponsor_runtime::runtime_dir().context("Sponsor runtime is not installed")?;
        let mut child = Command::new("node")
            .arg(runtime.join("tenki-session.mjs"))
            .current_dir(runtime)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env(
                "SYSTEMROOT",
                std::env::var("SYSTEMROOT").unwrap_or_default(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().context("Worker stdin unavailable")?;
        let output = child.stdout.take().context("Worker stdout unavailable")?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut worker = Self {
            prepared: Instant::now(),
            child,
            input,
            replies,
        };
        let ready = match worker.exchange(json!({"auth_token":key}), 70000) {
            Ok(ready) => ready,
            Err(error) => {
                worker.close();
                return Err(error);
            }
        };
        if ready["ready"] != true {
            worker.close();
            anyhow::bail!("Tenki preparation failed; rerun setup before presenting");
        }
        Ok(worker)
    }
    fn exchange(&mut self, request: Value, timeout_ms: u64) -> Result<Value> {
        if self.child.try_wait()?.is_some() {
            anyhow::bail!("Tenki worker is closed; restart the demo session");
        }
        writeln!(self.input, "{}", request)?;
        self.input.flush()?;
        let line = self
            .replies
            .recv_timeout(Duration::from_millis(timeout_ms))
            .map_err(|_| anyhow!("Tenki worker timed out; restart the demo session"))?;
        Ok(serde_json::from_str(&line)?)
    }
    fn close(&mut self) {
        let _ = writeln!(self.input, "{{\"close\":true}}");
        let _ = self.input.flush();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().ok().flatten() {
                if status.success() {
                    println!("[+] Tenki worker closed and VM cleanup completed.");
                } else {
                    eprintln!("[!] Remote cleanup unconfirmed; the VM's 15-minute lifetime remains enforced.");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        eprintln!("[!] Remote cleanup unconfirmed; the VM's 15-minute lifetime remains enforced.");
    }
}

pub fn tenki_run(request: SandboxRequest) -> Result<SandboxOutcome> {
    let worker = TENKI
        .get()
        .context("Tenki is not prepared; start with kerna demo start")?;
    let mut worker = worker
        .lock()
        .map_err(|_| anyhow!("Tenki worker unavailable"))?;
    let started = Instant::now();
    let result = match worker.exchange(
        json!({"code": request.code, "timeout_ms":request.timeout_ms}),
        20000,
    ) {
        Ok(result) => result,
        Err(error) => {
            worker.close();
            return Err(error);
        }
    };
    let output = result["output"]
        .as_str()
        .unwrap_or("Remote execution failed")
        .to_string();
    if output.len() > sponsor_runtime::MAX_OUTPUT_BYTES {
        anyhow::bail!("Output limit exceeded");
    }
    Ok(SandboxOutcome {
        backend: request.backend,
        status: result["status"].as_str().unwrap_or("failed").to_string(),
        exit_code: result["exit_code"].as_i64().unwrap_or(1) as i32,
        duration_ms: started.elapsed().as_millis(),
        output_sha256: format!("{:x}", Sha256::digest(output.as_bytes())),
        output,
        package: result["package"]
            .as_str()
            .unwrap_or("tenki/sandbox-v1")
            .into(),
        network: "inbound=false,outbound=false".into(),
    })
}

pub async fn forward_sandbox(arguments: Value) -> Result<Value> {
    let port: u16 = std::env::var("KERNA_DEMO_BROKER_PORT")?.parse()?;
    let token = std::env::var("KERNA_DEMO_SESSION_TOKEN")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = client
        .post(format!("http://127.0.0.1:{port}/kerna/sandbox"))
        .bearer_auth(token)
        .json(&arguments)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Kerna broker refused sandbox execution (HTTP {})",
            response.status()
        );
    }
    Ok(response.json().await?)
}

pub async fn start(repo: &Path, port: u16) -> Result<()> {
    if !guard_launcher::readiness_ok(false, Some(repo)).await {
        anyhow::bail!("Run kerna doctor to resolve readiness");
    }
    let anthropic = Arc::new(
        dialoguer::Password::new()
            .with_prompt("Anthropic key — this demo session only")
            .interact()?,
    );
    let tenki = dialoguer::Password::new()
        .with_prompt("Tenki key — this demo session only")
        .interact()?;
    let demo_id = Uuid::new_v4().to_string();
    let clone = guard_launcher::create_disposable_clone(repo, &demo_id)?;
    let (contract, mcp) = guard_launcher::prepare_demo_contract(&clone)?;
    std::env::set_current_dir(&contract)?;
    let config = Config::load();
    let memory = Arc::new(MemoryEngine::new(&config.db_path)?);
    let baseline = server::capture_worktree_baseline()?;
    let state = server::AppState {
        config: config.clone(),
        guard_policy: Arc::new(crate::load_guard_policy(&config)?),
        memory: memory.clone(),
        mcp_registry: Arc::new(tokio::sync::Mutex::new(McpRegistry::new())),
        worktree_baseline: baseline,
        auth_token: None,
        route_mode: RouteMode::Auto,
        shadow_enabled: true,
        anthropic_api_key: Some(anthropic),
        route_decisions: Arc::new(tokio::sync::Mutex::new(Default::default())),
    };
    let dashboard_state = state.clone();
    let dashboard =
        tokio::spawn(
            async move { server::start_dashboard_server(dashboard_state, port, true).await },
        );
    tokio::time::sleep(Duration::from_millis(500)).await;
    if dashboard.is_finished() {
        dashboard.await??;
        anyhow::bail!("Dashboard could not start");
    }
    println!("[i] Preparing Tenki VM before the presentation (up to 70 seconds)…");
    let worker = tokio::task::spawn_blocking(move || TenkiWorker::start(tenki)).await?;
    match worker {
        Ok(worker) => {
            TENKI
                .set(Mutex::new(worker))
                .map_err(|_| anyhow!("Demo already running"))?;
        }
        Err(error) => {
            dashboard.abort();
            return Err(error);
        }
    }
    println!("[+] Ready · http://127.0.0.1:{port}/ · keys stay in memory · Tenki VM ready for 15 minutes");
    println!("Commands: cloud-allow, cloud-deny, local-allow, local-deny, exit");
    loop {
        let line = tokio::task::spawn_blocking(|| -> Result<String> {
            print!("kerna-demo> ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            Ok(line)
        })
        .await??;
        let name = line.trim();
        if name == "exit" || name.is_empty() {
            break;
        }
        let (route, backend, code) = match name {
            "cloud-allow" => (
                RouteMode::Cloud,
                "tenki",
                "print(sum(i*i for i in range(10)))",
            ),
            "cloud-deny" => (RouteMode::Cloud, "tenki", "import os; print(os.environ)"),
            "local-allow" => (
                RouteMode::Local,
                "wasmer",
                "print(sum(i*i for i in range(10)))",
            ),
            "local-deny" => (RouteMode::Local, "wasmer", "import os; print(os.environ)"),
            _ => {
                println!("Use cloud-allow, cloud-deny, local-allow, local-deny, or exit");
                continue;
            }
        };
        let token = Uuid::new_v4().to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let broker_port = listener.local_addr()?.port();
        drop(listener);
        let mut case_state = state.clone();
        case_state.auth_token = Some(token.clone());
        case_state.route_mode = route;
        let broker = tokio::spawn(async move {
            server::start_server(case_state, "127.0.0.1", broker_port).await
        });
        std::fs::write(
            &mcp,
            serde_json::to_vec_pretty(&json!({"mcpServers":{"kerna-governed-tools":{
                "command":std::env::current_exe()?,"args":["gateway","--workspace",contract],
                "env":{"KERNA_DEMO_BROKER_PORT":broker_port.to_string(),"KERNA_DEMO_SESSION_TOKEN":token}
            }}}))?,
        )?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let prompt = format!("Call kerna_sandbox_run exactly once with backend {backend}, language python, timeout_ms 10000, and this exact code: {code}. Do not call any other tool. Report its result or Kerna policy denial in one sentence.");
        let task_clone = clone.clone();
        let task_mcp = mcp.clone();
        let task_token = token.clone();
        let result = tokio::task::spawn_blocking(move || {
            guard_launcher::run_claude(
                &task_clone,
                &task_mcp,
                &task_token,
                broker_port,
                Some(&prompt),
            )
        })
        .await?;
        if let Err(error) = result {
            eprintln!("[-] {error}");
        }
        // Give the tool-less shadow its bounded request window to finish in the background.
        let session_id = format!("guard-{:x}", Sha256::digest(token.as_bytes()));
        memory.finish_gateway_session(&session_id)?;
        broker.abort();
        println!(
            "[i] Receipt report → View opens Audit detail. Dashboard: http://127.0.0.1:{port}/"
        );
    }
    if let Some(worker) = TENKI.get() {
        if let Ok(mut worker) = worker.lock() {
            worker.close();
        }
    }
    dashboard.abort();
    println!(
        "[+] Demo session closed. Evidence retained at {}",
        config.db_path
    );
    Ok(())
}
