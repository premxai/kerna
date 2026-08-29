use crate::config::McpServerConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

/// Current MCP protocol revision advertised by Kerna's stdio client.
///
/// The client continues to accept an older revision returned by a compatible
/// server; negotiation is completed by the server's initialize response.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct McpListToolsResponse {
    result: McpListToolsResult,
}

#[derive(Debug, Deserialize)]
struct McpListToolsResult {
    tools: Vec<McpTool>,
}

#[derive(Debug, Deserialize)]
struct McpCallToolResponse {
    result: serde_json::Value,
}

pub struct McpClient {
    child: Child,
    stdout_reader: BufReader<tokio::process::ChildStdout>,
    stdin_writer: Option<tokio::process::ChildStdin>,
    request_id: u64,
}

impl McpClient {
    /// Start a reviewed production plugin in a hermetic Docker container. The
    /// image entrypoint is the MCP server; configuration arguments are passed
    /// only as image arguments and never through a host shell.
    pub fn spawn_container(server: &McpServerConfig, workspace: &Path) -> Result<Self> {
        let workspace = workspace.canonicalize()?;
        let mut docker_args = vec![
            "run".to_string(),
            "-i".to_string(),
            "--rm".to_string(),
            "--read-only".to_string(),
            "--cap-drop=ALL".to_string(),
            "--security-opt=no-new-privileges:true".to_string(),
            "--network=none".to_string(),
            "--tmpfs".to_string(),
            "/tmp:rw,noexec,nosuid,nodev,size=64m".to_string(),
            "--pids-limit=128".to_string(),
            "--memory=512m".to_string(),
            "--workdir=/workspace".to_string(),
        ];
        for root in &server.read_roots {
            docker_args.push("--mount".to_string());
            docker_args.push(mount_spec(&workspace, root, true)?);
        }
        for root in &server.write_roots {
            docker_args.push("--mount".to_string());
            docker_args.push(mount_spec(&workspace, root, false)?);
        }
        for name in &server.secrets {
            let value = std::env::var(name)
                .map_err(|_| anyhow!("declared secret '{}' is not set in the environment", name))?;
            docker_args.push("-e".to_string());
            docker_args.push(format!("{}={}", name, value));
        }
        docker_args.push(server.image.clone());
        docker_args.extend(server.args.clone());

        let mut command = Command::new("docker");
        command
            .args(&docker_args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for var in [
            "PATH",
            "SystemRoot",
            "SystemDrive",
            "TEMP",
            "TMP",
            "PATHEXT",
        ] {
            if let Ok(value) = std::env::var(var) {
                command.env(var, value);
            }
        }
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open container stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open container stdout"))?;
        Ok(McpClient {
            child,
            stdout_reader: BufReader::new(stdout),
            stdin_writer: Some(stdin),
            request_id: 1,
        })
    }

    pub fn spawn(
        cmd: &str,
        args: &[&str],
        runtime_mode: &str,
        docker_image: &str,
        network_mode: &str,
        egress_proxy: Option<&str>,
        secrets: &[String],
    ) -> Result<Self> {
        // Create an isolated working directory (not an OS sandbox)
        let _ = std::fs::create_dir_all("sandbox");

        // Resolve declared secret names to (name, value) from the environment.
        // Only names came from config; values live only in the environment and
        // are injected into the child process — never persisted.
        let resolved_secrets: Vec<(String, String)> = secrets
            .iter()
            .filter_map(|name| match std::env::var(name) {
                Ok(val) if !val.is_empty() => Some((name.clone(), val)),
                _ => {
                    eprintln!(
                        "[MCP] Warning: declared secret '{}' is not set in the environment; the plugin may not authenticate.",
                        name
                    );
                    None
                }
            })
            .collect();

        let mut actual_cmd = cmd.to_string();
        let mut actual_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if runtime_mode == "docker" {
            let absolute_sandbox = std::env::current_dir()?.join("sandbox");
            let mut docker_args = vec![
                "run".to_string(),
                "-i".to_string(),
                "--rm".to_string(),
                "-v".to_string(),
                format!("{}:/workspace", absolute_sandbox.display()),
                "-w".to_string(),
                "/workspace".to_string(),
                "--cap-drop=ALL".to_string(),
                format!("--network={}", network_mode),
            ];

            if let Some(proxy) = egress_proxy {
                docker_args.push("-e".to_string());
                docker_args.push(format!("http_proxy={}", proxy));
                docker_args.push("-e".to_string());
                docker_args.push(format!("https_proxy={}", proxy));
            }

            // Pass declared secrets into the container explicitly.
            for (name, val) in &resolved_secrets {
                docker_args.push("-e".to_string());
                docker_args.push(format!("{}={}", name, val));
            }

            docker_args.push(docker_image.to_string());
            docker_args.push(actual_cmd);
            docker_args.append(&mut actual_args);
            actual_cmd = "docker".to_string();
            actual_args = docker_args;
        }

        let mut command = Command::new(actual_cmd);
        command
            .args(&actual_args)
            .env_clear()
            .current_dir("sandbox");

        let retain_vars = [
            "PATH",
            "SystemRoot",
            "SystemDrive",
            "USERPROFILE",
            "APPDATA",
            "TEMP",
            "TMP",
            "PATHEXT",
        ];
        for var in retain_vars {
            if let Ok(val) = std::env::var(var) {
                command.env(var, val);
            }
        }

        // Inject declared secrets into the legacy developer child's environment.
        // In docker mode they were already passed via `-e` above, so skip to
        // avoid leaking them to the `docker` CLI process env.
        if runtime_mode != "docker" {
            for (name, val) in &resolved_secrets {
                command.env(name, val);
            }
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open child stdout"))?;
        let stdout_reader = BufReader::new(stdout);

        Ok(McpClient {
            child,
            stdout_reader,
            stdin_writer: Some(stdin),
            request_id: 1,
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        let id = self.next_id();
        let request = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "Kerna",
                    "version": env!("CARGO_PKG_VERSION")
                }
            },
            "id": id
        });

        // Some servers might not return a result for initialize, or might return capabilities.
        // We just ensure it doesn't fail.
        let _ = self.send_request(request).await?;

        // Also send initialized notification
        let notify = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let mut req_str = notify.to_string();
        req_str.push('\n');
        let writer = self
            .stdin_writer
            .as_mut()
            .ok_or_else(|| anyhow!("MCP client stdin is already closed"))?;
        writer.write_all(req_str.as_bytes()).await?;
        writer.flush().await?;

        Ok(())
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let id = self.next_id();
        let request = json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "params": {},
            "id": id
        });

        let response_val = self.send_request(request).await?;
        let list_resp: McpListToolsResponse = serde_json::from_value(response_val)
            .map_err(|e| anyhow!("Invalid MCP list tools response: {}", e))?;

        Ok(list_resp.result.tools)
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id();
        let request = json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            },
            "id": id
        });

        let response_val = self.send_request(request).await?;
        let call_resp: McpCallToolResponse = serde_json::from_value(response_val)
            .map_err(|e| anyhow!("Invalid MCP call tool response: {}", e))?;

        Ok(call_resp.result)
    }

    /// Close stdin first so an MCP server can finish its protocol lifecycle
    /// and flush durable session state. Fall back to termination only when a
    /// child ignores EOF; this is especially important for `client doctor`.
    pub async fn close(mut self) -> Result<()> {
        // `AsyncWriteExt::shutdown` alone is not enough to deliver EOF while
        // the pipe handle remains owned by this process on Windows.
        let mut writer = self
            .stdin_writer
            .take()
            .ok_or_else(|| anyhow!("MCP client stdin is already closed"))?;
        writer.shutdown().await?;
        drop(writer);
        match tokio::time::timeout(std::time::Duration::from_secs(3), self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => {
                let _ = self.child.start_kill();
                Ok(())
            }
        }
    }

    async fn send_request(&mut self, request: serde_json::Value) -> Result<serde_json::Value> {
        let expected_id = request.get("id").cloned();

        let mut req_str = request.to_string();
        req_str.push('\n');

        let writer = self
            .stdin_writer
            .as_mut()
            .ok_or_else(|| anyhow!("MCP client stdin is already closed"))?;
        writer.write_all(req_str.as_bytes()).await?;
        writer.flush().await?;

        // Read lines until we get the JSON-RPC response whose `id` matches our
        // request. Servers may interleave notifications (no `id`) or log lines;
        // skip those. Bounded by both a line cap and an overall timeout so a
        // chatty or hung server can't wedge us.
        const MAX_LINES: usize = 100;
        for _ in 0..MAX_LINES {
            let mut line = String::new();
            // Limit response size to 5MB to prevent OOM.
            let mut handle = (&mut self.stdout_reader).take(5 * 1024 * 1024);
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                handle.read_line(&mut line),
            )
            .await
            {
                Ok(Ok(0)) => {
                    return Err(anyhow!(
                        "MCP server disconnected or returned empty response"
                    ));
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(anyhow!("Failed to read from MCP server: {}", e)),
                Err(_) => {
                    let _ = self.child.start_kill();
                    return Err(anyhow!("MCP server request timed out after 30 seconds"));
                }
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Ignore any non-JSON stdout noise the server may print.
            let val: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // A response has an `id`; notifications do not. Match ours.
            match (val.get("id"), &expected_id) {
                (Some(got), Some(want)) if got == want => return Ok(val),
                (Some(_), Some(_)) => continue, // response to a different request
                (None, _) => continue,          // notification — skip
                (Some(_), None) => return Ok(val),
            }
        }

        Err(anyhow!(
            "MCP server sent {} messages without a matching response id",
            MAX_LINES
        ))
    }

    fn next_id(&mut self) -> u64 {
        let id = self.request_id;
        self.request_id += 1;
        id
    }
}

fn mount_spec(workspace: &Path, relative: &str, readonly: bool) -> Result<String> {
    let source = workspace.join(relative).canonicalize()?;
    if !source.starts_with(workspace) {
        return Err(anyhow!("mount root '{}' escapes workspace", relative));
    }
    let destination = format!("/workspace/{}", relative.replace('\\', "/"));
    let access = if readonly { ",readonly" } else { "" };
    // Docker Desktop's Windows CLI does not accept Rust's extended-length
    // `\\?\\` spelling for bind sources, even though Win32 APIs do.
    let source_display = source.to_string_lossy();
    let source_display = source_display
        .strip_prefix(r"\\?\")
        .unwrap_or(&source_display);
    Ok(format!(
        "type=bind,source={},target={}{}",
        source_display, destination, access
    ))
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.try_wait(); // Attempt to reap process handle
    }
}

#[cfg(test)]
mod containment_tests {
    use super::mount_spec;
    use std::fs;

    #[test]
    fn mount_spec_canonicalizes_inside_workspace_and_rejects_escape() {
        let root = std::env::temp_dir().join(format!("kerna-mount-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("read")).unwrap();
        let spec = mount_spec(&root.canonicalize().unwrap(), "read", true).unwrap();
        assert!(spec.contains("target=/workspace/read,readonly"));
        assert!(mount_spec(&root, "..", true).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
