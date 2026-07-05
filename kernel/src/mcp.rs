use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
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
    pub fn spawn(cmd: &str, args: &[&str]) -> Result<Self> {
        // Create an isolated working directory (not an OS sandbox)
        let _ = std::fs::create_dir_all("sandbox");
        
        let mut command = Command::new(cmd);
        command.args(args).env_clear().current_dir("sandbox");
        
        let retain_vars = ["PATH", "SystemRoot", "SystemDrive", "USERPROFILE", "APPDATA", "TEMP", "TMP", "PATHEXT"];
        for var in retain_vars {
            if let Ok(val) = std::env::var(var) {
                command.env(var, val);
            }
        }
        
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to open child stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to open child stdout"))?;
        let stdout_reader = BufReader::new(stdout);

        Ok(McpClient {
            child,
            stdout_reader,
            stdin_writer: stdin,
            request_id: 1,
        })
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

    pub async fn call_tool(&mut self, name: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
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
        let mut req_str = request.to_string();
        req_str.push('\n');

        self.stdin_writer.write_all(req_str.as_bytes()).await?;
        self.stdin_writer.flush().await?;

        let mut line = String::new();
        match tokio::time::timeout(std::time::Duration::from_secs(30), self.stdout_reader.read_line(&mut line)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(anyhow!("Failed to read from MCP server: {}", e)),
            Err(_) => {
                let _ = self.child.start_kill();
                return Err(anyhow!("MCP server request timed out after 30 seconds"));
            }
        }

        if line.is_empty() {
            return Err(anyhow!("MCP server disconnected or returned empty response"));
        }

        let val: serde_json::Value = serde_json::from_str(&line)?;
        Ok(val)
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
