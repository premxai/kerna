use crate::config::Config;
use crate::sandbox::ProcessSandbox;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Tool-schema fragment shared by every fs.* tool: an optional named root.
/// Omit it (or pass "workspace") for the sandboxed workspace; pass the name
/// of a folder granted via `kerna folders add` to reach a real directory.
fn root_property() -> Value {
    json!({
        "type": "string",
        "description": "Which granted folder to operate in. Omit for the sandboxed workspace, or the name of a folder from `kerna folders list`."
    })
}

pub fn get_tool_definitions() -> Vec<Value> {
    vec![
        // Filesystem Pack
        json!({
            "type": "function",
            "function": {
                "name": "fs.read",
                "description": "Read the contents of a file in the workspace, or in a granted real folder via `root`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "root": root_property()
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.write",
                "description": "Write contents to a file in the workspace, or in a granted real folder via `root` (folder must be granted read-write).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" },
                        "root": root_property()
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.list",
                "description": "List the contents of a directory in the workspace, or in a granted real folder via `root`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "root": root_property()
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "fs.delete",
                "description": "Delete a file or directory in the workspace, or in a granted real folder via `root` (folder must be granted read-write).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "root": root_property()
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

/// Resolve `args.root` (default: sandboxed workspace) and safely join
/// `args.path` onto it, returning the joined path and whether the root is
/// read-only. Rejects traversal/absolute paths and unknown root names.
fn resolve_arg_path(
    args: &Value,
    config: &Config,
    sandbox: &ProcessSandbox,
) -> Result<(std::path::PathBuf, bool)> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing path"))?;
    let root_name = args["root"]
        .as_str()
        .unwrap_or(crate::folders::WORKSPACE_ROOT);
    let (root, read_only) =
        crate::folders::resolve_root(config, sandbox.get_workspace_root(), root_name)?;
    let path = crate::folders::safe_join(&root, path_str)?;
    Ok((path, read_only))
}

pub async fn execute_tool(
    tool_name: &str,
    args: &Value,
    sandbox: &ProcessSandbox,
    config: &Config,
) -> Result<Value> {
    match tool_name {
        "fs.read" => {
            let (path, _) = resolve_arg_path(args, config, sandbox)?;
            let content = std::fs::read_to_string(path)?;
            Ok(json!({ "content": content }))
        }
        "fs.write" => {
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing content"))?;
            let (path, read_only) = resolve_arg_path(args, config, sandbox)?;
            if read_only {
                return Err(anyhow!(
                    "This folder is granted read-only. Re-grant it with --read-write to allow writes: kerna folders add <name> <path> --read-write"
                ));
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, content)?;
            Ok(json!({ "status": "success" }))
        }
        "fs.list" => {
            let (path, _) = resolve_arg_path(args, config, sandbox)?;
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
            let (path, read_only) = resolve_arg_path(args, config, sandbox)?;
            if read_only {
                return Err(anyhow!(
                    "This folder is granted read-only. Re-grant it with --read-write to allow deletes: kerna folders add <name> <path> --read-write"
                ));
            }
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else if path.exists() {
                std::fs::remove_file(path)?;
            }
            Ok(json!({ "status": "deleted" }))
        }
        "shell.exec" => {
            let cmd = args["command"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing command"))?;
            let args_arr: Vec<&str> = args["args"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let output = sandbox.run_command(cmd, &args_arr, 30).await?;
            Ok(json!({ "output": output }))
        }
        "artifact.write_markdown" => {
            let title = args["title"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing title"))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing content"))?;
            let safe_title = title.replace(" ", "_").to_lowercase() + ".md";
            let path = std::path::Path::new(&sandbox.get_workspace_root())
                .join("artifacts")
                .join(&safe_title);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(json!({ "status": "success", "path": path.display().to_string() }))
        }
        _ => Err(anyhow!("Unknown tool pack function: {}", tool_name)),
    }
}
