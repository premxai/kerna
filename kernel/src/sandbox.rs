use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug, PartialEq)]
pub enum CommandClass {
    SafeReadOnly,
    WorkspaceMutating,
    DangerousGlobal,
}

/// Returns true if `path` escapes the workspace: an absolute Unix path,
/// a parent-directory traversal, a Windows drive-letter path (e.g. `C:\`),
/// or a UNC path (`\\server\share`).
pub fn is_out_of_workspace_path(path: &str) -> bool {
    if path.starts_with('/') || path.contains("..") {
        return true;
    }
    // Home-directory reference (`~`, `~/foo`, `~\foo`) resolves outside the workspace.
    if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
        return true;
    }
    // UNC path, e.g. \\server\share
    if path.starts_with("\\\\") {
        return true;
    }
    // Windows drive-letter absolute path, e.g. C:\ or C:/
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    false
}

pub fn classify_command(cmd: &str, args: &[&str]) -> CommandClass {
    // Interpreter/shell wrappers can run arbitrary code that evades per-command
    // classification (e.g. `bash -c "rm -rf /"`). Treat the inline-code forms as
    // dangerous outright.
    let shell_wrappers = ["bash", "sh", "zsh", "dash", "powershell", "pwsh", "cmd"];
    if shell_wrappers.contains(&cmd)
        && (args.contains(&"-c")
            || args.contains(&"-Command")
            || args.contains(&"/C")
            || args.contains(&"/c"))
    {
        return CommandClass::DangerousGlobal;
    }
    if (cmd == "python" || cmd == "python3" || cmd == "node" || cmd == "ruby" || cmd == "perl")
        && args.contains(&"-c")
    {
        return CommandClass::DangerousGlobal;
    }

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

    pub fn get_workspace_root(&self) -> &Path {
        &self.sandbox_dir
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
            let is_install = ((cmd == "npm" || cmd == "pnpm" || cmd == "yarn")
                && args.contains(&"install"))
                || ((cmd == "pip" || cmd == "pip3") && args.contains(&"install"))
                || (cmd == "python" && args.contains(&"-m") && args.contains(&"pip"))
                || (cmd == "cargo" && args.contains(&"install"))
                || (cmd == "gem" && args.contains(&"install"))
                || (cmd == "go" && args.contains(&"install"))
                || cmd == "apt-get"
                || cmd == "apt"
                || cmd == "brew"
                || cmd == "winget"
                || cmd == "choco";
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
                    let arg_strings: Vec<String> = parsed_args
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .map(|a| a.as_str().unwrap_or("").to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    let arg_refs: Vec<&str> = arg_strings.iter().map(|s| s.as_str()).collect();

                    // Classify the command itself — this catches shell/interpreter
                    // wrappers (`bash -c "rm -rf /"`), sudo/chmod/chown, global
                    // installs, and `rm -rf /` or `rm -rf ~` regardless of the
                    // rm/mv/cp path heuristic below.
                    if classify_command(cmd, &arg_refs) == CommandClass::DangerousGlobal {
                        decision.is_allowed = false;
                        decision.reasons.push(format!(
                            "Command '{}' is classified as globally destructive/dangerous.",
                            cmd
                        ));
                    }

                    if cmd == "rm" || cmd == "mv" || cmd == "cp" {
                        let is_global = arg_refs.iter().any(|arg| is_out_of_workspace_path(arg));

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_shell_wrapper_classification() {
        // Inline-code shell wrappers can smuggle destructive commands.
        assert_eq!(
            classify_command("bash", &["-c", "rm -rf /"]),
            CommandClass::DangerousGlobal
        );
        assert_eq!(
            classify_command("powershell", &["-Command", "Remove-Item C:\\ -Recurse"]),
            CommandClass::DangerousGlobal
        );
        assert_eq!(
            classify_command("cmd", &["/C", "del /f /q C:\\"]),
            CommandClass::DangerousGlobal
        );
        assert_eq!(
            classify_command("python", &["-c", "import os; os.system('rm -rf /')"]),
            CommandClass::DangerousGlobal
        );
        // A plain read-only command is still safe.
        assert_eq!(classify_command("ls", &["-la"]), CommandClass::SafeReadOnly);
    }

    #[test]
    fn test_out_of_workspace_path_detection() {
        // Unix absolute + traversal (previously covered)
        assert!(is_out_of_workspace_path("/etc/passwd"));
        assert!(is_out_of_workspace_path("../secret"));
        // Windows drive-letter absolute paths (previously missed → fail-open)
        assert!(is_out_of_workspace_path("C:\\Windows\\System32"));
        assert!(is_out_of_workspace_path("C:/Windows/System32"));
        assert!(is_out_of_workspace_path("D:\\data"));
        // UNC paths
        assert!(is_out_of_workspace_path("\\\\server\\share"));
        // Home-directory references resolve outside the workspace
        assert!(is_out_of_workspace_path("~"));
        assert!(is_out_of_workspace_path("~/.ssh/id_rsa"));
        assert!(is_out_of_workspace_path("~\\Documents"));
        // In-workspace relative paths remain allowed
        assert!(!is_out_of_workspace_path("build/output.txt"));
        assert!(!is_out_of_workspace_path("./notes.md"));
        assert!(!is_out_of_workspace_path("file.txt"));
    }

    #[test]
    fn test_simulate_denies_wrappers_and_home() {
        let dir = env::temp_dir().join("kerna_sim_sandbox");
        let sandbox =
            ProcessSandbox::new(&dir, "native".to_string(), false, "none".to_string(), None)
                .unwrap();
        // Permissive policy so the *only* thing that can deny is the sandbox
        // boundary/classification logic under test.
        let config = crate::config::Config {
            permissions: vec![crate::config::PermissionRule {
                tool: "*".to_string(),
                action: "auto_approve".to_string(),
            }],
            ..Default::default()
        };
        let perms = crate::permissions::PermissionManager::new(config);

        let denied = |args: &str| {
            !sandbox
                .simulate_command("run_command", args, &perms)
                .unwrap()
                .is_allowed
        };
        // Shell wrapper smuggling a destructive command.
        assert!(denied(r#"{"command":"bash","args":["-c","rm -rf /"]}"#));
        // rm targeting the home directory.
        assert!(denied(r#"{"command":"rm","args":["-rf","~"]}"#));
        // Classic global rm.
        assert!(denied(r#"{"command":"rm","args":["-rf","/"]}"#));
        // A benign in-workspace command still passes.
        assert!(!denied(r#"{"command":"ls","args":["-la"]}"#));
    }

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
