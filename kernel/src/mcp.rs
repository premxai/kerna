use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

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
    stdin_writer: tokio::process::ChildStdin,
    request_id: u64,
}

impl McpClient {
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

        // Inject declared secrets into the child's environment (native mode).
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
            stdin_writer: stdin,
            request_id: 1,
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        let id = self.next_id();
        let request = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "Kerna",
                    "version": "0.1.0"
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
        self.stdin_writer.write_all(req_str.as_bytes()).await?;
        self.stdin_writer.flush().await?;

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

    async fn send_request(&mut self, request: serde_json::Value) -> Result<serde_json::Value> {
        let expected_id = request.get("id").cloned();

        let mut req_str = request.to_string();
        req_str.push('\n');

        self.stdin_writer.write_all(req_str.as_bytes()).await?;
        self.stdin_writer.flush().await?;

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

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.try_wait(); // Attempt to reap process handle
    }
}
