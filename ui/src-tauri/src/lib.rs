use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Task {
    id: String,
    goal: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Memory {
    id: String,
    content: String,
}

fn get_db_path() -> PathBuf {
    // In a real app this would use a reliable config dir.
    // For now we use the known path.
    PathBuf::from("C:\\Users\\kanap\\.gemini\\antigravity\\scratch\\agentos\\agentos.db")
}

fn get_kernel_path() -> PathBuf {
    PathBuf::from("C:\\Users\\kanap\\.gemini\\antigravity\\scratch\\agentos\\target\\debug\\kerna.exe")
}

#[tauri::command]
fn get_tasks() -> Result<Vec<Task>, String> {
    let conn = Connection::open(get_db_path()).map_err(|e| e.to_string())?;
    
    let mut stmt = conn.prepare("SELECT id, goal, status FROM tasks ORDER BY created_at DESC LIMIT 50").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(Task {
            id: row.get(0)?,
            goal: row.get(1)?,
            status: row.get(2)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut tasks = Vec::new();
    for r in rows {
        if let Ok(t) = r {
            tasks.push(t);
        }
    }
    
    Ok(tasks)
}

#[tauri::command]
fn get_memories() -> Result<Vec<Memory>, String> {
    let conn = Connection::open(get_db_path()).map_err(|e| e.to_string())?;
    
    let mut stmt = conn.prepare("SELECT id, content FROM episodic_memory ORDER BY created_at DESC LIMIT 20").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(Memory {
            id: row.get(0)?,
            content: row.get(1)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut memories = Vec::new();
    for r in rows {
        if let Ok(m) = r {
            memories.push(m);
        }
    }
    
    Ok(memories)
}

#[tauri::command]
async fn run_goal(goal: String) -> Result<String, String> {
    // We spawn the kernel CLI in the background. It will update the database.
    let kernel_path = get_kernel_path();
    
    // Spawn detached process
    Command::new(kernel_path)
        .arg("run")
        .arg(&goal)
        .current_dir("C:\\Users\\kanap\\.gemini\\antigravity\\scratch\\agentos")
        .spawn()
        .map_err(|e| e.to_string())?;
        
    Ok(format!("Started goal: {}", goal))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_tasks, get_memories, run_goal])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
