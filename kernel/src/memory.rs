use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use crate::events::{Event, EventSink};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub struct MemoryEngine {
    conn: Mutex<Connection>,
}

impl MemoryEngine {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Kerna database appears corrupted or inaccessible ({}).\nRun: kerna db repair\nor move kerna.db to kerna.db.bak and re-run kerna init.",
                    e
                ));
            }
        };

        // Enable foreign keys and WAL mode for concurrency
        conn.execute("PRAGMA foreign_keys = ON;", [])?;
        let _ = conn.query_row("PRAGMA journal_mode = WAL;", [], |_row| Ok(()));

        let engine = MemoryEngine { conn: Mutex::new(conn) };
        engine.bootstrap()?;
        Ok(engine)
    }

    fn get_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn bootstrap(&self) -> Result<()> {
        let conn = self.get_conn();
        // Create sessions table
        conn.execute(
                "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_active_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
                [],
            )
            .context("Failed to create sessions table")?;

        // Create tasks table
        conn.execute(
                "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                goal TEXT NOT NULL,
                status TEXT NOT NULL,
                duration_secs INTEGER DEFAULT 0,
                llm_used TEXT DEFAULT '',
                cost_estimate REAL DEFAULT 0.0,
                tokens_used INTEGER DEFAULT 0,
                retries INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                completed_at DATETIME,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL
            );",
                [],
            )
            .context("Failed to create tasks table")?;

        // Create agent_logs table
        conn.execute(
                "CREATE TABLE IF NOT EXISTS agent_logs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                log_level TEXT NOT NULL,
                message TEXT NOT NULL,
                FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
            );",
                [],
            )
            .context("Failed to create agent_logs table")?;

        // Create episodic_memory table (semantic memory for past goals/results)
        conn.execute(
                "CREATE TABLE IF NOT EXISTS episodic_memory (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                embedding_json TEXT NOT NULL,
                tags TEXT DEFAULT '',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
                [],
            )
            .context("Failed to create episodic_memory table")?;

        // Create user_preferences table (key-value store for user memory)
        conn.execute(
                "CREATE TABLE IF NOT EXISTS user_preferences (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
                [],
            )
            .context("Failed to create user_preferences table")?;

        // Create events table (Phase 4)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                event_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                session_id TEXT,
                sequence INTEGER NOT NULL,
                timestamp TEXT NOT NULL,
                event_type TEXT NOT NULL,
                actor TEXT NOT NULL,
                severity TEXT NOT NULL,
                model TEXT,
                tool TEXT,
                policy_decision TEXT,
                risk_score REAL,
                parent_event_id TEXT,
                correlation_id TEXT,
                redaction_status TEXT,
                budget_snapshot_json TEXT,
                payload_json TEXT
            );",
            [],
        )
        .context("Failed to create events table")?;

        conn.execute("CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id);", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);", [])?;

        // Create facts table (knowledge graph nodes)
        conn.execute(
                "CREATE TABLE IF NOT EXISTS facts (
                id TEXT PRIMARY KEY,
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                confidence REAL DEFAULT 1.0,
                source_task_id TEXT,
                valid_from DATETIME DEFAULT CURRENT_TIMESTAMP,
                valid_until DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
                [],
            )
            .context("Failed to create facts table")?;

        // Create index for fact lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_subject ON facts(subject);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_predicate ON facts(predicate);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_logs_task_id ON agent_logs(task_id);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_episodic_memory_created_at ON episodic_memory(created_at);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_last_active_at ON sessions(last_active_at);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_facts_valid_until ON facts(valid_until);",
            [],
        )?;

        Ok(())
    }

    // ─── Session Management ──────────────────────────────────────

    pub fn create_session(&self, name: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.get_conn();
        // Ignore if exists, or insert new
        conn.execute(
            "INSERT INTO sessions (id, name) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET last_active_at = CURRENT_TIMESTAMP",
            params![id, name],
        )?;
        
        // Fetch the ID (in case it already existed and we just updated last_active_at)
        let mut stmt = conn.prepare("SELECT id FROM sessions WHERE name = ?1")?;
        let actual_id: String = stmt.query_row(params![name], |row| row.get(0))?;
        
        Ok(actual_id)
    }

    pub fn get_recent_sessions(&self) -> Result<Vec<(String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT id, name FROM sessions ORDER BY last_active_at DESC LIMIT 5")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            Ok((id, name))
        })?;

        let mut sessions = Vec::new();
        for r in rows {
            sessions.push(r?);
        }
        Ok(sessions)
    }

    // ─── Task Management ─────────────────────────────────────────

    pub fn create_task(&self, id: Uuid, session_id: Option<&str>, goal: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO tasks (id, session_id, goal, status) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), session_id, goal, "pending"],
        )?;
        Ok(())
    }

    pub fn update_task_status(&self, id: Uuid, status: &str) -> Result<()> {
        let conn = self.get_conn();
        if status == "completed" || status == "failed" {
            conn.execute(
                "UPDATE tasks SET status = ?1, completed_at = CURRENT_TIMESTAMP WHERE id = ?2",
                params![status, id.to_string()],
            )?;
        } else {
            conn.execute(
                "UPDATE tasks SET status = ?1 WHERE id = ?2",
                params![status, id.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn update_task_observability(&self, id: Uuid, duration_secs: i64, llm: &str, cost: f64, tokens: i64, retries: i64) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE tasks SET duration_secs = ?1, llm_used = ?2, cost_estimate = ?3, tokens_used = ?4, retries = ?5 WHERE id = ?6",
            params![duration_secs, llm, cost, tokens, retries, id.to_string()],
        )?;
        Ok(())
    }

    pub fn log_message(&self, task_id: Uuid, level: &str, message: &str) -> Result<()> {
        let log_id = Uuid::new_v4().to_string();
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO agent_logs (id, task_id, log_level, message) VALUES (?1, ?2, ?3, ?4)",
            params![log_id, task_id.to_string(), level, message],
        )?;
        Ok(())
    }

    pub fn get_tasks(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT id, goal, status FROM tasks ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let goal: String = row.get(1)?;
            let status: String = row.get(2)?;
            Ok((id, goal, status))
        })?;

        let mut tasks = Vec::new();
        for r in rows {
            tasks.push(r?);
        }
        Ok(tasks)
    }

    pub fn get_running_tasks(&self) -> Result<Vec<(String, String, i64, i64)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT id, goal, duration_secs, tokens_used FROM tasks WHERE status = 'running' ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let goal: String = row.get(1)?;
            let dur: i64 = row.get(2)?;
            let tokens: i64 = row.get(3)?;
            Ok((id, goal, dur, tokens))
        })?;

        let mut tasks = Vec::new();
        for r in rows {
            tasks.push(r?);
        }
        Ok(tasks)
    }

    pub fn get_task_observability(&self, task_id: &str) -> Result<(String, String, String, i64, String, f64, i64, i64)> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT goal, status, created_at, duration_secs, llm_used, cost_estimate, tokens_used, retries FROM tasks WHERE id = ?1")?;
        let result = stmt.query_row(params![task_id], |row| {
            let goal: String = row.get(0)?;
            let status: String = row.get(1)?;
            let created: String = row.get(2)?;
            let dur: i64 = row.get(3)?;
            let llm: String = row.get(4)?;
            let cost: f64 = row.get(5)?;
            let tokens: i64 = row.get(6)?;
            let retries: i64 = row.get(7)?;
            Ok((goal, status, created, dur, llm, cost, tokens, retries))
        })?;
        Ok(result)
    }

    pub fn get_task_logs(&self, task_id: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT timestamp, log_level, message FROM agent_logs WHERE task_id = ?1 ORDER BY timestamp ASC",
        )?;
        let rows = stmt.query_map(params![task_id], |row| {
            let ts: String = row.get(0)?;
            let level: String = row.get(1)?;
            let msg: String = row.get(2)?;
            Ok((ts, level, msg))
        })?;

        let mut logs = Vec::new();
        for r in rows {
            logs.push(r?);
        }
        Ok(logs)
    }

    // ─── Episodic Memory (Semantic) ──────────────────────────────

    pub fn add_episodic_memory(&self, content: &str, embedding: &[f32]) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let embedding_json = serde_json::to_string(embedding)?;
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO episodic_memory (id, content, embedding_json) VALUES (?1, ?2, ?3)",
            params![id, content, embedding_json],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_tagged_memory(
        &self,
        content: &str,
        embedding: &[f32],
        tags: &str,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let embedding_json = serde_json::to_string(embedding)?;
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO episodic_memory (id, content, embedding_json, tags) VALUES (?1, ?2, ?3, ?4)",
            params![id, content, embedding_json, tags],
        )?;
        Ok(())
    }

    pub fn search_episodic_memory(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT content, embedding_json FROM episodic_memory")?;
        let rows = stmt.query_map([], |row| {
            let content: String = row.get(0)?;
            let embedding_json: String = row.get(1)?;
            Ok((content, embedding_json))
        })?;

        let mut matched = Vec::new();

        for row in rows {
            let (content, embedding_json) = row?;
            if let Ok(vec) = serde_json::from_str::<Vec<f32>>(&embedding_json) {
                let similarity = cosine_similarity(query_embedding, &vec);
                matched.push((content, similarity));
            }
        }

        // Sort by similarity descending
        matched.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        matched.truncate(limit);

        Ok(matched)
    }

    pub fn search_memory_by_text(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let pattern = format!("%{}%", query);
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT content FROM episodic_memory WHERE content LIKE ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let content: String = row.get(0)?;
            Ok(content)
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    pub fn get_episodic_memories_by_time(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT content, created_at, date(created_at) as d FROM episodic_memory ORDER BY created_at DESC LIMIT 50")?;
        let rows = stmt.query_map([], |row| {
            let content: String = row.get(0)?;
            let created: String = row.get(1)?;
            let date: String = row.get(2)?;
            Ok((content, created, date))
        })?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    // ─── User Preferences (Key-Value Memory) ────────────────────

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO user_preferences (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_preference(&self, key: &str) -> Result<Option<String>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT value FROM user_preferences WHERE key = ?1")?;
        let result = stmt.query_row(params![key], |row| {
            let value: String = row.get(0)?;
            Ok(value)
        });

        match result {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_all_preferences(&self) -> Result<Vec<(String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT key, value FROM user_preferences ORDER BY key ASC")?;
        let rows = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok((key, value))
        })?;

        let mut prefs = Vec::new();
        for r in rows {
            prefs.push(r?);
        }
        Ok(prefs)
    }

    // ─── Facts / Knowledge Graph ─────────────────────────────────

    #[allow(dead_code)]
    pub fn add_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f32,
    ) -> Result<()> {
        let mut conn = self.get_conn();
        let tx = conn.transaction()?;

        // Delete any existing fact for this subject/predicate that is currently valid
        tx.execute(
            "UPDATE facts SET valid_until = CURRENT_TIMESTAMP 
             WHERE subject = ?1 AND predicate = ?2 AND valid_until IS NULL",
            params![subject, predicate],
        )?;

        tx.execute(
            "INSERT INTO facts (id, subject, predicate, object, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid::Uuid::new_v4().to_string(), subject, predicate, object, confidence],
        )?;

        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn query_facts(&self, subject: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT subject, predicate, object FROM facts WHERE subject = ?1 AND valid_until IS NULL ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![subject], |row| {
            let s: String = row.get(0)?;
            let p: String = row.get(1)?;
            let o: String = row.get(2)?;
            Ok((s, p, o))
        })?;

        let mut facts = Vec::new();
        for r in rows {
            facts.push(r?);
        }
        Ok(facts)
    }

    pub fn search_facts(&self, query: &str) -> Result<Vec<(String, String, String)>> {
        let pattern = format!("%{}%", query);
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT subject, predicate, object FROM facts WHERE valid_until IS NULL AND (subject LIKE ?1 OR predicate LIKE ?1 OR object LIKE ?1) ORDER BY created_at DESC LIMIT 20",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            let s: String = row.get(0)?;
            let p: String = row.get(1)?;
            let o: String = row.get(2)?;
            Ok((s, p, o))
        })?;

        let mut facts = Vec::new();
        for r in rows {
            facts.push(r?);
        }
        Ok(facts)
    }

    // ─── Context Injection Helper ────────────────────────────────

    pub fn gather_context(&self, goal: &str) -> Result<String> {
        let mut context = String::new();

        // 1. Relevant text memories
        let memories = self.search_memory_by_text(goal, 3)?;
        if !memories.is_empty() {
            context.push_str("## Relevant past memories:\n");
            for m in &memories {
                let display = if m.chars().count() > 200 { 
                    let truncated: String = m.chars().take(200).collect();
                    format!("{}...", truncated)
                } else { 
                    m.to_string() 
                };
                context.push_str(&format!("- {}\n", display));
            }
            context.push('\n');
        }

        // 2. User preferences
        let prefs = self.get_all_preferences()?;
        if !prefs.is_empty() {
            context.push_str("## User preferences:\n");
            for (k, v) in &prefs {
                context.push_str(&format!("- {}: {}\n", k, v));
            }
            context.push('\n');
        }

        // 3. Related facts
        let facts = self.search_facts(goal)?;
        if !facts.is_empty() {
            context.push_str("## Known facts:\n");
            for (s, p, o) in &facts {
                context.push_str(&format!("- {} {} {}\n", s, p, o));
            }
            context.push('\n');
        }

        Ok(context)
    }
}

impl EventSink for MemoryEngine {
    fn record(&self, event: Event) -> Result<()> {
        let conn = self.get_conn();
        
        let budget_snapshot_str = event.budget_snapshot_json
            .map(|v| v.to_string());
            
        let payload_str = event.payload_json.to_string();
        
        conn.execute(
            "INSERT INTO events (
                event_id, task_id, session_id, sequence, timestamp, event_type, actor, severity,
                model, tool, policy_decision, risk_score, parent_event_id, correlation_id, redaction_status, budget_snapshot_json, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                event.event_id,
                event.task_id,
                event.session_id,
                event.sequence,
                event.timestamp,
                event.event_type,
                event.actor,
                event.severity,
                event.model,
                event.tool,
                event.policy_decision,
                event.risk_score,
                event.parent_event_id,
                event.correlation_id,
                event.redaction_status,
                budget_snapshot_str,
                payload_str
            ],
        ).context("Failed to insert event")?;
        
        Ok(())
    }
}

impl MemoryEngine {
    pub fn get_events(&self, task_id: &str) -> Result<Vec<Event>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT event_id, task_id, session_id, sequence, timestamp, event_type, actor, severity, 
                    model, tool, policy_decision, risk_score, parent_event_id, correlation_id, redaction_status, budget_snapshot_json, payload_json
             FROM events WHERE task_id = ?1 ORDER BY sequence ASC"
        )?;
        
        let rows = stmt.query_map(params![task_id], |row| {
            let budget_str: Option<String> = row.get(15)?;
            let budget_json = budget_str.and_then(|s| serde_json::from_str(&s).ok());
            
            let payload_str: String = row.get(16)?;
            let payload_json = serde_json::from_str(&payload_str).unwrap_or(serde_json::json!({}));
            
            Ok(Event {
                event_id: row.get(0)?,
                task_id: row.get(1)?,
                session_id: row.get(2)?,
                sequence: row.get(3)?,
                timestamp: row.get(4)?,
                event_type: row.get(5)?,
                actor: row.get(6)?,
                severity: row.get(7)?,
                model: row.get(8)?,
                tool: row.get(9)?,
                policy_decision: row.get(10)?,
                risk_score: row.get(11)?,
                parent_event_id: row.get(12)?,
                correlation_id: row.get(13)?,
                redaction_status: row.get(14)?,
                budget_snapshot_json: budget_json,
                payload_json,
            })
        })?;
        
        let mut events = Vec::new();
        for r in rows {
            events.push(r?);
        }
        Ok(events)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|y| y * y).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot_product / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use uuid::Uuid;

    fn setup_test_db(name: &str) -> MemoryEngine {
        let db_path = format!("{}.db", name);
        let _ = fs::remove_file(&db_path);
        MemoryEngine::new(&db_path).expect("Failed to initialize test DB")
    }

    #[test]
    fn test_memory_engine_creates_and_queries_tasks() {
        let mem = setup_test_db("test_tasks");
        let task_id = Uuid::new_v4();
        
        mem.create_task(task_id, None, "Test goal").unwrap();
        mem.update_task_status(task_id, "completed").unwrap();
        mem.log_message(task_id, "INFO", "Testing task log").unwrap();
        
        let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].2, "Testing task log");
    }

    #[test]
    fn test_memory_stress_test() {
        let mem = setup_test_db("test_stress");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "Stress Test").unwrap();
        
        for i in 0..100 {
            mem.log_message(task_id, "INFO", &format!("Log message {}", i)).unwrap();
        }
        
        let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
        assert_eq!(logs.len(), 100);
    }

    #[test]
    fn test_sabotage_db_concurrency() {
        use std::sync::Arc;
        let mem = Arc::new(setup_test_db("sabotage_concurrency"));
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "Concurrency Test").unwrap();
        
        let mut handles = vec![];
        
        for i in 0..50 {
            let mem_clone = mem.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    let _ = mem_clone.log_message(task_id, "INFO", &format!("Thread {} Msg {}", i, j));
                }
            }));
        }
        
        for h in handles {
            let _ = h.join();
        }
        
        let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
        assert_eq!(logs.len(), 5000, "Database must survive extreme concurrency without locking");
    }
}
