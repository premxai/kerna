use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::timeout;
use wasmtime::{Engine, Module, Store, Linker};

pub struct ProcessSandbox {
    sandbox_dir: PathBuf,
}

impl ProcessSandbox {
    pub fn new<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let sandbox_dir = dir.as_ref().to_path_buf();
        if !sandbox_dir.exists() {
            fs::create_dir_all(&sandbox_dir)?;
        }
        Ok(ProcessSandbox { sandbox_dir })
    }

    pub async fn run_command(&self, cmd: &str, args: &[&str], timeout_sec: u64) -> Result<String> {
        let sandbox_dir = self.sandbox_dir.clone();
        let cmd_str = cmd.to_string();
        let args_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        // Run the process in a background blocking task
        let handle = tokio::task::spawn_blocking(move || -> Result<(i32, String, String)> {
            let child = Command::new(cmd_str)
                .args(&args_vec)
                .current_dir(&sandbox_dir)
                .env_clear() // Strip all sensitive parent env variables
                .env("PATH", std::env::var("PATH").unwrap_or_default()) // Only pass system PATH for basic execution
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let output = child.wait_with_output()?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            
            let code = output.status.code().unwrap_or(-1);
            Ok((code, stdout, stderr))
        });

        match timeout(Duration::from_secs(timeout_sec), handle).await {
            Ok(Ok(Ok((code, stdout, stderr)))) => {
                if code == 0 {
                    Ok(stdout)
                } else {
                    Err(anyhow!("Process exited with code {}: {}", code, stderr))
                }
            }
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(join_err)) => Err(anyhow!("Sandbox task joined with error: {}", join_err)),
            Err(_) => Err(anyhow!("Process execution timed out after {} seconds", timeout_sec)),
        }
    }
}

pub struct WasmSandbox {
    engine: Engine,
}

impl WasmSandbox {
    pub fn new() -> Result<Self> {
        let engine = Engine::default();
        Ok(WasmSandbox { engine })
    }

    pub fn run_wasm_module(&self, wasm_path: &Path) -> Result<String> {
        let wasm_bytes = fs::read(wasm_path)?;
        let module = Module::new(&self.engine, &wasm_bytes)?;
        
        let mut store = Store::new(&self.engine, ());
        let linker = Linker::new(&self.engine);
        
        // Instantiate the module (empty imports for core calculation modules)
        let instance = linker.instantiate(&mut store, &module)?;
        
        // Try calling default run or start functions if exported
        if let Some(func) = instance.get_typed_func::<(), ()>(&mut store, "run").ok() {
            func.call(&mut store, ())?;
            Ok("Wasm module execution completed successfully via run().".to_string())
        } else if let Some(func) = instance.get_typed_func::<(), ()>(&mut store, "_start").ok() {
            // Emulate WASI start call
            func.call(&mut store, ())?;
            Ok("Wasm module execution completed successfully via _start().".to_string())
        } else {
            Err(anyhow!("Wasm module has no exported run() or _start() entry point"))
        }
    }
}
