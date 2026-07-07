use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::time::timeout;
use wasmtime::{Engine, Linker, Module, Store};

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
        let snapshot_dir = self.sandbox_dir.parent().unwrap().join(format!(
            "{}_snapshot",
            self.sandbox_dir.file_name().unwrap().to_string_lossy()
        ));
        if snapshot_dir.exists() {
            fs::remove_dir_all(&snapshot_dir)?;
        }
        let mut options = fs_extra::dir::CopyOptions::new();
        options.copy_inside = true;
        fs_extra::dir::copy(&self.sandbox_dir, &snapshot_dir, &options)?;
        Ok(())
    }

    pub fn rollback(&self) -> Result<()> {
        let snapshot_dir = self.sandbox_dir.parent().unwrap().join(format!(
            "{}_snapshot",
            self.sandbox_dir.file_name().unwrap().to_string_lossy()
        ));
        if !snapshot_dir.exists() {
            return Err(anyhow!("No snapshot available for rollback."));
        }
        if self.sandbox_dir.exists() {
            fs::remove_dir_all(&self.sandbox_dir)?;
        }
        let mut options = fs_extra::dir::CopyOptions::new();
        options.copy_inside = true;
        let original_name_dir = snapshot_dir.join(self.sandbox_dir.file_name().unwrap());
        if original_name_dir.exists() {
            fs_extra::dir::copy(
                &original_name_dir,
                self.sandbox_dir.parent().unwrap(),
                &options,
            )?;
        } else {
            // Fallback if structure differs slightly based on fs_extra version
            fs::create_dir_all(&self.sandbox_dir)?;
            fs_extra::dir::copy(&snapshot_dir, self.sandbox_dir.parent().unwrap(), &options)?;
        }
        Ok(())
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
