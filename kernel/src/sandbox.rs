use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::time::timeout;
use wasmtime::{Engine, Linker, Module, Store};

#[derive(Debug, PartialEq)]
pub enum CommandClass {
    SafeReadOnly,
    WorkspaceMutating,
    DangerousGlobal,
}

pub fn classify_command(cmd: &str, args: &[&str]) -> CommandClass {
    let dangerous = ["rm", "sudo", "curl", "wget", "npm", "chmod", "chown", "su"];
    if dangerous.contains(&cmd) {
        if cmd == "rm" && args.contains(&"-rf") && (args.contains(&"/") || args.contains(&"~")) {
            return CommandClass::DangerousGlobal;
        }
        if cmd == "sudo" || cmd == "su" || cmd == "chmod" || cmd == "chown" {
            return CommandClass::DangerousGlobal;
        }
        if cmd == "npm" && args.contains(&"install") && args.contains(&"-g") {
            return CommandClass::DangerousGlobal;
        }
    }

    let safe_read = ["ls", "cat", "grep", "git", "pwd", "whoami", "echo"];
    if safe_read.contains(&cmd) {
        if cmd == "git"
            && !args.contains(&"status")
            && !args.contains(&"log")
            && !args.contains(&"diff")
        {
            return CommandClass::WorkspaceMutating;
        }
        if cmd == "echo" && args.contains(&">") {
            return CommandClass::WorkspaceMutating;
        }
        return CommandClass::SafeReadOnly;
    }

    // Default to workspace mutating
    CommandClass::WorkspaceMutating
}

pub struct ProcessSandbox {
    sandbox_dir: PathBuf,
    runtime_mode: String,
    allow_dynamic_installs: bool,
    network_mode: String,
    egress_proxy: Option<String>,
}

impl ProcessSandbox {
    pub fn new<P: AsRef<Path>>(
        dir: P,
        runtime_mode: String,
        allow_dynamic_installs: bool,
        network_mode: String,
        egress_proxy: Option<String>,
    ) -> Result<Self> {
        let sandbox_dir = dir.as_ref().to_path_buf();
        if !sandbox_dir.exists() {
            fs::create_dir_all(&sandbox_dir)?;
        }

        Ok(ProcessSandbox {
            sandbox_dir,
            runtime_mode,
            allow_dynamic_installs,
            network_mode,
            egress_proxy,
        })
    }

    pub fn snapshot(&self) -> Result<()> {
        let parent = self.sandbox_dir.parent().unwrap_or(&self.sandbox_dir);
        let name = self
            .sandbox_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".to_string());
        let snapshot_dir = parent.join(format!("{}_snapshot", name));

        if snapshot_dir.exists() {
            fs::remove_dir_all(&snapshot_dir)?;
        }
        fs::create_dir_all(&snapshot_dir)?;
        copy_dir_all(&self.sandbox_dir, &snapshot_dir)?;
        Ok(())
    }

    pub fn rollback(&self) -> Result<()> {
        let parent = self.sandbox_dir.parent().unwrap_or(&self.sandbox_dir);
        let name = self
            .sandbox_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".to_string());
        let snapshot_dir = parent.join(format!("{}_snapshot", name));

        if !snapshot_dir.exists() {
            return Err(anyhow!("No snapshot available for rollback."));
        }
        if self.sandbox_dir.exists() {
            fs::remove_dir_all(&self.sandbox_dir)?;
        }
        fs::create_dir_all(&self.sandbox_dir)?;
        copy_dir_all(&snapshot_dir, &self.sandbox_dir)?;
        Ok(())
    }

    pub fn is_path_in_workspace(&self, target_path: &Path) -> bool {
        let canonical_target = target_path
            .canonicalize()
            .unwrap_or_else(|_| target_path.to_path_buf());
        let canonical_workspace = self
            .sandbox_dir
            .canonicalize()
            .unwrap_or_else(|_| self.sandbox_dir.clone());
        canonical_target.starts_with(&canonical_workspace)
    }

    pub fn is_trusted_for_rollback(&self, cmd: &str, args: &[&str]) -> bool {
        let classification = classify_command(cmd, args);
        if classification == CommandClass::DangerousGlobal {
            return false; // Cannot trust rollback for global destructive commands
        }

        // Detect forbidden absolute paths
        for arg in args {
            if arg.starts_with('/') || arg.starts_with('~') || arg.contains(":\\") {
                let path = Path::new(arg);
                if !self.is_path_in_workspace(path) {
                    return false;
                }
            }
        }
        true
    }

    pub async fn run_command(&self, cmd: &str, args: &[&str], timeout_sec: u64) -> Result<String> {
        if !self.allow_dynamic_installs {
            let is_install = (cmd == "npm" && args.contains(&"install"))
                || (cmd == "pip" && args.contains(&"install"))
                || (cmd == "cargo" && args.contains(&"install"))
                || cmd == "apt-get";
            if is_install {
                return Err(anyhow!("Security policy violation: dynamic package installations are disabled via config."));
            }
        }

        let sandbox_dir = self.sandbox_dir.clone();
        let mut actual_cmd = cmd.to_string();
        let mut actual_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if self.runtime_mode == "docker" {
            let absolute_sandbox = std::env::current_dir()?.join(&sandbox_dir);
            let mut docker_args = vec![
                "run".to_string(),
                "-i".to_string(),
                "--rm".to_string(),
                "-v".to_string(),
                format!("{}:/workspace", absolute_sandbox.display()),
                "-w".to_string(),
                "/workspace".to_string(),
                "--cap-drop=ALL".to_string(),
                format!("--network={}", self.network_mode),
            ];

            if let Some(proxy) = &self.egress_proxy {
                docker_args.push("-e".to_string());
                docker_args.push(format!("http_proxy={}", proxy));
                docker_args.push("-e".to_string());
                docker_args.push(format!("https_proxy={}", proxy));
            }

            docker_args.push("ubuntu:latest".to_string()); // Default image for sandbox commands
            docker_args.push(actual_cmd);
            docker_args.append(&mut actual_args);
            actual_cmd = "docker".to_string();
            actual_args = docker_args;
        }

        let child = tokio::process::Command::new(actual_cmd)
            .args(&actual_args)
            .current_dir(&sandbox_dir)
            .env_clear() // Strip all sensitive parent env variables
            .env("PATH", std::env::var("PATH").unwrap_or_default()) // Only pass system PATH for basic execution
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        match timeout(Duration::from_secs(timeout_sec), child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if code == 0 {
                    Ok(stdout)
                } else {
                    Err(anyhow!("Process exited with code {}: {}", code, stderr))
                }
            }
            Ok(Err(e)) => Err(anyhow!("Failed to wait for process: {}", e)),
            Err(_) => Err(anyhow!(
                "Process execution timed out after {} seconds",
                timeout_sec
            )),
        }
    }
}

pub struct SimulationDecision {
    pub is_allowed: bool,
    pub reasons: Vec<String>,
}

impl ProcessSandbox {
    pub fn simulate_command(
        &self,
        tool: &str,
        args: &str,
        permissions: &crate::permissions::PermissionManager,
    ) -> Result<SimulationDecision> {
        let mut decision = SimulationDecision {
            is_allowed: true,
            reasons: Vec::new(),
        };

        // 1. Basic tool validation
        if tool != "run_command" && tool != "read_file" && tool != "write_file" && tool != "mcp" {
            decision.is_allowed = false;
            decision
                .reasons
                .push(format!("Tool '{}' is not recognized or not allowed.", tool));
            return Ok(decision);
        }

        // 2. Workspace bounds checking for run_command
        if tool == "run_command" {
            if let Ok(parsed_args) = serde_json::from_str::<serde_json::Value>(args) {
                if let Some(cmd) = parsed_args.get("command").and_then(|v| v.as_str()) {
                    if cmd == "rm" || cmd == "mv" || cmd == "cp" {
                        let is_global = parsed_args
                            .get("args")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter().any(|arg| {
                                    arg.as_str().unwrap_or("").starts_with("/")
                                        || arg.as_str().unwrap_or("").contains("..")
                                })
                            })
                            .unwrap_or(false);

                        if is_global {
                            decision.is_allowed = false;
                            decision.reasons.push(
                                "Command operates destructively outside the workspace boundary."
                                    .to_string(),
                            );
                        }
                    }
                }
            } else {
                decision.is_allowed = false;
                decision
                    .reasons
                    .push("Failed to parse tool arguments as JSON.".to_string());
            }
        }

        let parsed = serde_json::from_str::<serde_json::Value>(args).ok();
        let target_str = match &parsed {
            Some(p) => match tool {
                "run_command" => p.get("command").and_then(|v| v.as_str()).unwrap_or("*"),
                "read_file" | "write_file" => p.get("path").and_then(|v| v.as_str()).unwrap_or("*"),
                _ => "*",
            },
            None => "*",
        };

        let perm_level = permissions.check(tool, None);
        if perm_level == crate::permissions::PermissionLevel::Deny {
            decision.is_allowed = false;
            decision.reasons.push(format!(
                "Permission denied by Trust Layer for tool '{}' on target '{}'.",
                tool, target_str
            ));
        } else {
            decision.reasons.push(format!(
                "Trust layer allows tool '{}' on target '{}' (Level: {:?}).",
                tool, target_str, perm_level
            ));
        }

        if decision.is_allowed && decision.reasons.is_empty() {
            decision
                .reasons
                .push("Command passes all security policies and workspace bounds.".to_string());
        }

        Ok(decision)
    }
}

#[allow(dead_code)]
pub struct WasmSandbox {
    engine: Engine,
}

impl WasmSandbox {
    #[allow(dead_code)]
    pub fn new() -> Result<Self> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        Ok(WasmSandbox { engine })
    }

    #[allow(dead_code)]
    pub fn run_wasm_module(&self, wasm_path: &Path) -> Result<String> {
        let wasm_bytes = fs::read(wasm_path)?;
        let module = Module::new(&self.engine, &wasm_bytes)?;

        let mut store = Store::new(&self.engine, ());
        store.set_fuel(10_000_000)?; // 10 million instructions budget
        let linker = Linker::new(&self.engine);

        // Instantiate the module (empty imports for core calculation modules)
        let instance = linker.instantiate(&mut store, &module)?;

        // Try calling default run or start functions if exported
        if let Ok(func) = instance.get_typed_func::<(), ()>(&mut store, "run") {
            func.call(&mut store, ())?;
            Ok("Wasm module execution completed successfully via run().".to_string())
        } else if let Ok(func) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            // Emulate WASI start call
            func.call(&mut store, ())?;
            Ok("Wasm module execution completed successfully via _start().".to_string())
        } else {
            Err(anyhow!(
                "Wasm module has no exported run() or _start() entry point"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_package_manager_blocking() {
        let dir = env::temp_dir().join("kerna_test_sandbox");
        let sandbox =
            ProcessSandbox::new(&dir, "native".to_string(), false, "none".to_string(), None)
                .unwrap();

        let res = sandbox
            .run_command("npm", &["install", "evil-package"], 5)
            .await;
        assert!(res.is_err());
        let err_str = res.unwrap_err().to_string();
        assert!(err_str.contains("dynamic package installations are disabled"));

        let res2 = sandbox.run_command("whoami", &[], 5).await;
        // whoami should be allowed
        assert!(res2.is_ok(), "whoami failed: {:?}", res2.err());
    }
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
