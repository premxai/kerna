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
        json!({
            "type": "function",
            "function": {
                "name": "fs.move",
                "description": "Move or rename a file or directory (works on any file type, including PDFs and images). Both paths are within the same root — the workspace, or a granted real folder via `root` (must be read-write). Use this to organize/sort files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string", "description": "Current path, relative to the root." },
                        "to": { "type": "string", "description": "Destination path, relative to the root." },
                        "root": root_property()
                    },
                    "required": ["from", "to"]
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
        "fs.move" => {
            let from_str = args["from"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing 'from'"))?;
            let to_str = args["to"].as_str().ok_or_else(|| anyhow!("Missing 'to'"))?;
            let root_name = args["root"]
                .as_str()
                .unwrap_or(crate::folders::WORKSPACE_ROOT);
            // Both endpoints resolve within the same root; a move can't cross a
            // folder boundary, and a read-only grant refuses the operation.
            let (root, read_only) =
                crate::folders::resolve_root(config, sandbox.get_workspace_root(), root_name)?;
            if read_only {
                return Err(anyhow!(
                    "This folder is granted read-only. Re-grant it with --read-write to allow moves: kerna folders add <name> <path> --read-write"
                ));
            }
            let from = crate::folders::safe_join(&root, from_str)?;
            let to = crate::folders::safe_join(&root, to_str)?;
            if !from.exists() {
                return Err(anyhow!("Source '{}' does not exist.", from_str));
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&from, &to)?;
            Ok(json!({ "status": "moved", "from": from_str, "to": to_str }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FolderGrant;

    fn sandbox_in(dir: &std::path::Path) -> ProcessSandbox {
        ProcessSandbox::new(
            dir.to_path_buf(),
            "native".to_string(),
            false,
            "none".to_string(),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn fs_move_relocates_binary_file_within_a_readwrite_grant() {
        let base = std::env::temp_dir().join("kerna_fsmove_test");
        let _ = std::fs::remove_dir_all(&base);
        let sandbox_dir = base.join("sandbox");
        let real = base.join("documents");
        std::fs::create_dir_all(&sandbox_dir).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        // A genuinely binary (non-UTF8) file — the whole point vs fs.read/write.
        std::fs::write(real.join("scan.pdf"), [0xFFu8, 0xD8, 0x00, 0x01, 0xFE]).unwrap();

        let mut config = Config::default();
        config.folders.push(FolderGrant {
            name: "docs".to_string(),
            path: real.to_string_lossy().to_string(),
            read_write: true,
        });
        let sandbox = sandbox_in(&sandbox_dir);

        // Move it into a subfolder (sorting).
        let res = execute_tool(
            "fs.move",
            &json!({"root": "docs", "from": "scan.pdf", "to": "receipts/scan.pdf"}),
            &sandbox,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(res["status"], "moved");
        assert!(!real.join("scan.pdf").exists());
        assert_eq!(
            std::fs::read(real.join("receipts/scan.pdf")).unwrap(),
            vec![0xFFu8, 0xD8, 0x00, 0x01, 0xFE]
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn fs_move_refused_on_readonly_grant_and_on_traversal() {
        let base = std::env::temp_dir().join("kerna_fsmove_deny_test");
        let _ = std::fs::remove_dir_all(&base);
        let sandbox_dir = base.join("sandbox");
        let real = base.join("documents");
        std::fs::create_dir_all(&sandbox_dir).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("a.txt"), "x").unwrap();

        let mut config = Config::default();
        config.folders.push(FolderGrant {
            name: "docs".to_string(),
            path: real.to_string_lossy().to_string(),
            read_write: false, // read-only
        });
        let sandbox = sandbox_in(&sandbox_dir);

        // Read-only grant refuses the move.
        let ro = execute_tool(
            "fs.move",
            &json!({"root": "docs", "from": "a.txt", "to": "b.txt"}),
            &sandbox,
            &config,
        )
        .await;
        assert!(ro.is_err());
        assert!(ro.unwrap_err().to_string().contains("read-only"));
        assert!(real.join("a.txt").exists(), "file untouched after refusal");

        // Traversal out of the workspace is rejected even for the default root.
        let traversal = execute_tool(
            "fs.move",
            &json!({"from": "a.txt", "to": "../../escape.txt"}),
            &sandbox,
            &config,
        )
        .await;
        assert!(traversal.is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
