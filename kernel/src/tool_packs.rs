use serde_json::{json, Value};
use anyhow::{Result, anyhow};
use crate::sandbox::ProcessSandbox;

pub fn get_tool_definitions() -> Vec<Value> {
    vec![
        // Filesystem Pack
        json!({
            "type": "function",
            "function": {
                "name": "fs.read",
                "description": "Read the contents of a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.write",
                "description": "Write contents to a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.list",
                "description": "List the contents of a directory in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.delete",
                "description": "Delete a file or directory in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }
        }),
        
        // Shell Pack
        json!({
            "type": "function",
            "function": {
                "name": "shell.exec",
                "description": "Execute a shell command in the sandbox.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "args": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["command"]
                }
            }
        }),
        
        // Artifact Pack
        json!({
            "type": "function",
            "function": {
                "name": "artifact.write_markdown",
                "description": "Write a markdown artifact to the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["title", "content"]
                }
            }
        }),
    ]
}

pub async fn execute_tool(
    tool_name: &str,
    args: &Value,
    sandbox: &ProcessSandbox,
) -> Result<Value> {
    match tool_name {
        "fs.read" => {
            let path_str = args["path"].as_str().ok_or_else(|| anyhow!("Missing path"))?;
            let path = std::path::Path::new(&sandbox.get_workspace_root()).join(path_str);
            let content = std::fs::read_to_string(path)?;
            Ok(json!({ "content": content }))
        }
        "fs.write" => {
            let path_str = args["path"].as_str().ok_or_else(|| anyhow!("Missing path"))?;
            let content = args["content"].as_str().ok_or_else(|| anyhow!("Missing content"))?;
            let path = std::path::Path::new(&sandbox.get_workspace_root()).join(path_str);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, content)?;
            Ok(json!({ "status": "success" }))
        }
        "fs.list" => {
            let path_str = args["path"].as_str().ok_or_else(|| anyhow!("Missing path"))?;
            let path = std::path::Path::new(&sandbox.get_workspace_root()).join(path_str);
            let mut entries = Vec::new();
            if path.is_dir() {
                for entry in std::fs::read_dir(path)? {
                    let entry = entry?;
                    entries.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            Ok(json!({ "entries": entries }))
        }
        "fs.delete" => {
            let path_str = args["path"].as_str().ok_or_else(|| anyhow!("Missing path"))?;
            let path = std::path::Path::new(&sandbox.get_workspace_root()).join(path_str);
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else if path.exists() {
                std::fs::remove_file(path)?;
            }
            Ok(json!({ "status": "deleted" }))
        }
        "shell.exec" => {
            let cmd = args["command"].as_str().ok_or_else(|| anyhow!("Missing command"))?;
            let args_arr: Vec<&str> = args["args"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let output = sandbox.run_command(cmd, &args_arr, 30).await?;
            Ok(json!({ "output": output }))
        }
        "artifact.write_markdown" => {
            let title = args["title"].as_str().ok_or_else(|| anyhow!("Missing title"))?;
            let content = args["content"].as_str().ok_or_else(|| anyhow!("Missing content"))?;
            let safe_title = title.replace(" ", "_").to_lowercase() + ".md";
            let path = std::path::Path::new(&sandbox.get_workspace_root()).join("artifacts").join(&safe_title);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(json!({ "status": "success", "path": path.display().to_string() }))
        }
        _ => Err(anyhow!("Unknown tool pack function: {}", tool_name)),
    }
}
