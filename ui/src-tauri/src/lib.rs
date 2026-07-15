use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize)]
pub struct Task {
    id: String,
    goal: String,
    status: String,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Memory {
    id: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct KernaConfig {
    db_path: Option<String>,
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    schedules: Vec<ScheduleConfig>,
    #[serde(default)]
    permissions: Vec<PermissionRule>,
}

#[derive(Debug, Deserialize)]
struct McpServerConfig {
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    secrets: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ScheduleConfig {
    #[serde(default)]
    name: String,
    cron: String,
    goal: String,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct PermissionRule {
    tool: String,
    action: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectorStatus {
    name: String,
    enabled: bool,
    secrets_needed: usize,
    secrets_ready: bool,
    last_activity: Option<String>,
    last_result: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RoutineStatus {
    index: usize,
    name: String,
    cron: String,
    goal: String,
    enabled: bool,
    allowed_tools: Vec<String>,
    policy_ready: bool,
}

#[derive(Debug, Serialize)]
pub struct PendingApproval {
    id: String,
    task_id: String,
    tool: String,
    args_json: String,
    created_at: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct TaskEvent {
    timestamp: String,
    event_type: String,
    severity: String,
    tool: Option<String>,
    policy_decision: Option<String>,
    redaction_status: Option<String>,
}

fn find_workspace_dir() -> Result<PathBuf, String> {
    if let Ok(home) = std::env::var("KERNA_HOME") {
        let path = PathBuf::from(home);
        if path.join("kerna.toml").is_file() {
            return Ok(path);
        }
        return Err("KERNA_HOME must point to a folder containing kerna.toml".to_string());
    }

    let mut roots = vec![std::env::current_dir().map_err(|e| e.to_string())?];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    for root in roots {
        for candidate in root.ancestors() {
            if candidate.join("kerna.toml").is_file() {
                return Ok(candidate.to_path_buf());
            }
        }
    }

    Err("Could not find kerna.toml. Launch Kerna from its workspace or set KERNA_HOME.".to_string())
}

fn get_db_path() -> Result<PathBuf, String> {
    let workspace = find_workspace_dir()?;
    let config = load_config(&workspace)?;
    let db_path = config.db_path.unwrap_or_else(|| "kerna.db".to_string());
    let db_path = PathBuf::from(db_path);
    Ok(if db_path.is_absolute() {
        db_path
    } else {
        workspace.join(db_path)
    })
}

fn load_config(workspace: &Path) -> Result<KernaConfig, String> {
    let config_path = workspace.join("kerna.toml");
    let config_text = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    toml::from_str(&config_text).map_err(|e| e.to_string())
}

fn policy_action<'a>(permissions: &'a [PermissionRule], tool: &str) -> &'a str {
    permissions
        .iter()
        .find(|rule| rule.tool == tool)
        .or_else(|| permissions.iter().find(|rule| rule.tool == "*"))
        .map(|rule| rule.action.as_str())
        .unwrap_or("deny")
}

#[derive(Debug, Deserialize)]
struct ConnectorManifest {
    plugin: ConnectorManifestMetadata,
}

#[derive(Debug, Deserialize)]
struct ConnectorManifestMetadata {
    #[serde(default)]
    capabilities: Vec<String>,
}

fn connector_tools(server: &McpServerConfig) -> Vec<String> {
    let manifest = server.args.iter().find_map(|arg| {
        let entrypoint = PathBuf::from(arg);
        let path = entrypoint.parent()?.join("manifest.toml");
        let content = std::fs::read_to_string(path).ok()?;
        toml::from_str::<ConnectorManifest>(&content).ok()
    });
    manifest
        .map(|manifest| manifest.plugin.capabilities)
        .unwrap_or_default()
}

fn connector_last_result(
    db_path: &Path,
    tools: &[String],
) -> Result<Option<(String, String)>, String> {
    if tools.is_empty() || !db_path.is_file() {
        return Ok(None);
    }
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let mut latest: Option<(String, String)> = None;
    for tool in tools {
        let result = conn
            .query_row(
                "SELECT timestamp, event_type FROM events
                 WHERE tool = ?1 AND event_type IN ('tool.call.completed', 'tool.call.failed')
                 ORDER BY timestamp DESC LIMIT 1",
                [tool],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(candidate) = result {
            if latest
                .as_ref()
                .map(|current| candidate.0 > current.0)
                .unwrap_or(true)
            {
                latest = Some(candidate);
            }
        }
    }
    Ok(latest)
}

fn installed_kernel_path(home: &Path, binary: &str) -> PathBuf {
    home.join(".local").join("bin").join(binary)
}

fn get_kernel_path(workspace: &Path) -> PathBuf {
    if let Ok(path) = std::env::var("KERNA_BIN") {
        return PathBuf::from(path);
    }

    let binary = if cfg!(windows) { "kerna.exe" } else { "kerna" };
    let development_binary = workspace.join("target").join("debug").join(binary);
    if development_binary.is_file() {
        return development_binary;
    }

    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
        let installed_binary = installed_kernel_path(&home, binary);
        if installed_binary.is_file() {
            return installed_binary;
        }
    }

    // Fall back to the process PATH for package-manager or custom installs.
    PathBuf::from(binary)
}

#[tauri::command]
fn get_tasks() -> Result<Vec<Task>, String> {
    let conn = Connection::open(get_db_path()?).map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT id, goal, status, created_at FROM tasks ORDER BY created_at DESC LIMIT 50")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Task {
                id: row.get(0)?,
                goal: row.get(1)?,
                status: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut tasks = Vec::new();
    for task in rows.flatten() {
        tasks.push(task);
    }

    Ok(tasks)
}

#[tauri::command]
fn get_memories() -> Result<Vec<Memory>, String> {
    let conn = Connection::open(get_db_path()?).map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT id, content FROM episodic_memory ORDER BY created_at DESC LIMIT 20")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Memory {
                id: row.get(0)?,
                content: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut memories = Vec::new();
    for memory in rows.flatten() {
        memories.push(memory);
    }

    Ok(memories)
}

#[tauri::command]
fn get_task_events(task_id: String) -> Result<Vec<TaskEvent>, String> {
    let conn = Connection::open(get_db_path()?).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT timestamp, event_type, severity, tool, policy_decision, redaction_status
             FROM events WHERE task_id = ?1 ORDER BY sequence ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([task_id], |row| {
            Ok(TaskEvent {
                timestamp: row.get(0)?,
                event_type: row.get(1)?,
                severity: row.get(2)?,
                tool: row.get(3)?,
                policy_decision: row.get(4)?,
                redaction_status: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(|e| e.to_string())?);
    }
    Ok(events)
}

#[tauri::command]
fn get_connectors() -> Result<Vec<ConnectorStatus>, String> {
    let workspace = find_workspace_dir()?;
    let config = load_config(&workspace)?;
    let db_path = get_db_path()?;
    let mut last_results = HashMap::new();
    for server in &config.mcp_servers {
        last_results.insert(
            server.name.clone(),
            connector_last_result(&db_path, &connector_tools(server))?,
        );
    }
    Ok(config
        .mcp_servers
        .into_iter()
        .map(|server| {
            let last = last_results.remove(&server.name).flatten();
            ConnectorStatus {
                name: server.name,
                enabled: server.enabled,
                secrets_needed: server.secrets.len(),
                secrets_ready: server.secrets.iter().all(|secret| {
                    std::env::var(secret)
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
                }),
                last_activity: last.as_ref().map(|result| result.0.clone()),
                last_result: last.map(|result| {
                    if result.1 == "tool.call.completed" {
                        "success".to_string()
                    } else {
                        "failed".to_string()
                    }
                }),
            }
        })
        .collect())
}

#[tauri::command]
fn get_routines() -> Result<Vec<RoutineStatus>, String> {
    let workspace = find_workspace_dir()?;
    let config = load_config(&workspace)?;
    let permissions = &config.permissions;
    Ok(config
        .schedules
        .into_iter()
        .enumerate()
        .map(|(index, schedule)| {
            let policy_ready = !schedule.allowed_tools.is_empty()
                && schedule
                    .allowed_tools
                    .iter()
                    .all(|tool| policy_action(permissions, tool) == "auto_approve");
            RoutineStatus {
                index,
                name: if schedule.name.trim().is_empty() {
                    schedule.goal.clone()
                } else {
                    schedule.name
                },
                cron: schedule.cron,
                goal: schedule.goal,
                enabled: schedule.enabled,
                allowed_tools: schedule.allowed_tools,
                policy_ready,
            }
        })
        .collect())
}

#[tauri::command]
async fn run_goal(goal: String) -> Result<String, String> {
    let workspace = find_workspace_dir()?;
    let kernel_path = get_kernel_path(&workspace);

    // Spawn detached process
    let kernel_path_display = kernel_path.display().to_string();
    Command::new(&kernel_path)
        .arg("run")
        .arg("--approval-queue")
        .arg(&goal)
        .current_dir(workspace)
        .spawn()
        .map_err(|e| {
            format!(
                "Could not start Kerna at {kernel_path_display}: {e}. Install the Kerna CLI, or set KERNA_BIN to its full path."
            )
        })?;

    Ok(format!(
        "Started goal: {}. Approval-required actions appear in the local approval queue.",
        goal
    ))
}

#[tauri::command]
fn get_pending_approvals() -> Result<Vec<PendingApproval>, String> {
    let conn = Connection::open(get_db_path()?).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, task_id, tool, args_json, created_at
             FROM pending_approvals WHERE status = 'pending' ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PendingApproval {
                id: row.get(0)?,
                task_id: row.get(1)?,
                tool: row.get(2)?,
                args_json: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| row.map_err(|e| e.to_string())).collect()
}

#[tauri::command]
fn decide_approval(id: String, approved: bool) -> Result<(), String> {
    let conn = Connection::open(get_db_path()?).map_err(|e| e.to_string())?;
    let status = if approved { "approved" } else { "denied" };
    let changed = conn
        .execute(
            "UPDATE pending_approvals SET status = ?1, decided_at = CURRENT_TIMESTAMP
             WHERE id = ?2 AND status = 'pending'",
            rusqlite::params![status, id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("This approval is no longer pending.".to_string());
    }
    Ok(())
}

#[tauri::command]
async fn run_routine(index: usize) -> Result<String, String> {
    let workspace = find_workspace_dir()?;
    let kernel_path = get_kernel_path(&workspace);
    let kernel_path_display = kernel_path.display().to_string();
    Command::new(&kernel_path)
        .arg("routine")
        .arg("run")
        .arg(index.to_string())
        .current_dir(workspace)
        .spawn()
        .map_err(|e| {
            format!(
                "Could not start Kerna at {kernel_path_display}: {e}. Install the Kerna CLI, or set KERNA_BIN to its full path."
            )
        })?;

    Ok("Started a scoped routine run. Its receipt will appear in Recent runs.".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_tasks,
            get_memories,
            get_task_events,
            get_connectors,
            get_routines,
            get_pending_approvals,
            decide_approval,
            run_goal,
            run_routine
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_health_uses_the_latest_completed_or_failed_tool_event() {
        let db_path =
            std::env::temp_dir().join(format!("kerna-ui-health-{}.db", uuid::Uuid::new_v4()));
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (timestamp TEXT NOT NULL, event_type TEXT NOT NULL, tool TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (timestamp, event_type, tool) VALUES ('2026-07-15T09:00:00Z', 'tool.call.completed', 'google_list_events')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (timestamp, event_type, tool) VALUES ('2026-07-15T10:00:00Z', 'tool.call.failed', 'google_calendar_status')",
            [],
        )
        .unwrap();
        drop(conn);

        let status = connector_last_result(
            &db_path,
            &[
                "google_list_events".to_string(),
                "google_calendar_status".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            status,
            Some((
                "2026-07-15T10:00:00Z".to_string(),
                "tool.call.failed".to_string()
            ))
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn installed_kernel_path_matches_the_cli_installer_location() {
        assert_eq!(
            installed_kernel_path(Path::new("C:/Users/Ada"), "kerna.exe"),
            PathBuf::from("C:/Users/Ada/.local/bin/kerna.exe")
        );
    }
}
