use crate::events::{Event, EventSink};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub struct MemoryEngine {
    conn: Mutex<Connection>,
}

/// Durable lifecycle row for one stdio MCP connection. This is separate from
/// generic events so the dashboard can answer "which client is connected?"
/// without inferring it from tool payloads.
#[derive(Debug, Clone, Serialize)]
pub struct GatewaySessionRecord {
    pub session_id: String,
    pub task_id: String,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub protocol_version: Option<String>,
    pub workspace: String,
    pub state: String,
    pub started_at: String,
    pub last_activity_at: String,
    pub ended_at: Option<String>,
}

/// One governed call, normalized for live metrics. The trace remains the
/// detailed receipt; this table makes aggregation deterministic and cheap.
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallReceipt {
    pub call_id: String,
    pub session_id: String,
    pub task_id: String,
    pub client_name: Option<String>,
    pub plugin_name: Option<String>,
    pub image_digest: Option<String>,
    pub tool: String,
    pub policy_decision: String,
    pub approval_id: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub result_class: Option<String>,
    pub trace_id: Option<String>,
    pub output_preview: Option<String>,
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

        let engine = MemoryEngine {
            conn: Mutex::new(conn),
        };
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

        // Idempotent migration: add result_text to pre-existing databases.
        // (CREATE TABLE IF NOT EXISTS won't add columns to an existing table.)
        let _ = conn.execute(
            "ALTER TABLE tasks ADD COLUMN result_text TEXT DEFAULT ''",
            [],
        );

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
                status TEXT DEFAULT 'STAGED',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
            [],
        )
        .context("Failed to create episodic_memory table")?;

        // Migration: add status column if it doesn't exist
        let _ = conn.execute(
            "ALTER TABLE episodic_memory ADD COLUMN status TEXT DEFAULT 'STAGED';",
            [],
        );

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

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);",
            [],
        )?;

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

        conn.execute(
            "CREATE TABLE IF NOT EXISTS pending_approvals (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                tool TEXT NOT NULL,
                args_json TEXT NOT NULL,
                binding_hash TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                decided_at DATETIME,
                expires_at DATETIME,
                used_at DATETIME,
                FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
            );",
            [],
        )?;
        // Existing local ledgers predate gateway approvals. SQLite has no
        // conditional ADD COLUMN, so tolerate the duplicate-column result.
        for migration in [
            "ALTER TABLE pending_approvals ADD COLUMN binding_hash TEXT",
            "ALTER TABLE pending_approvals ADD COLUMN expires_at DATETIME",
            "ALTER TABLE pending_approvals ADD COLUMN used_at DATETIME",
        ] {
            let _ = conn.execute(migration, []);
        }
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_approvals_status ON pending_approvals(status, created_at);",
            [],
        )?;

        // Gateway-specific summaries are intentionally normalized alongside
        // the generic append-only event log. They power a local dashboard
        // without weakening the richer trace/audit record.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS gateway_sessions (
                session_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                client_name TEXT,
                client_version TEXT,
                protocol_version TEXT,
                workspace TEXT NOT NULL,
                state TEXT NOT NULL,
                started_at TEXT NOT NULL,
                last_activity_at TEXT NOT NULL,
                ended_at TEXT
            );",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_gateway_sessions_state_activity
             ON gateway_sessions(state, last_activity_at DESC);",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tool_call_receipts (
                call_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                task_id TEXT NOT NULL,
                client_name TEXT,
                plugin_name TEXT,
                image_digest TEXT,
                tool TEXT NOT NULL,
                policy_decision TEXT NOT NULL,
                approval_id TEXT,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                duration_ms INTEGER,
                result_class TEXT,
                trace_id TEXT,
                output_preview TEXT
            );",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_call_receipts_started
             ON tool_call_receipts(started_at DESC);",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_call_receipts_session
             ON tool_call_receipts(session_id, started_at DESC);",
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
        let mut stmt =
            conn.prepare("SELECT id, name FROM sessions ORDER BY last_active_at DESC LIMIT 5")?;
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

    /// Persist the task's final assistant answer for later retrieval (e.g. the API server).
    pub fn set_task_result(&self, id: Uuid, result_text: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE tasks SET result_text = ?1 WHERE id = ?2",
            params![result_text, id.to_string()],
        )?;
        Ok(())
    }

    /// Retrieve the persisted final answer for a task, if any.
    pub fn get_task_result(&self, task_id: &str) -> Result<Option<String>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT result_text FROM tasks WHERE id = ?1")?;
        let result: rusqlite::Result<String> = stmt.query_row(params![task_id], |row| row.get(0));
        match result {
            Ok(s) if !s.is_empty() => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    pub fn update_task_observability(
        &self,
        id: Uuid,
        duration_secs: i64,
        llm: &str,
        cost: f64,
        tokens: i64,
        retries: i64,
    ) -> Result<()> {
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
        let mut stmt =
            conn.prepare("SELECT id, goal, status FROM tasks ORDER BY created_at DESC")?;
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

    /// Create a durable approval request. Arguments are still subject to event
    /// redaction when recorded separately; this row is shown only in the local
    /// approval surface so the user can judge the actual proposed action.
    pub fn create_pending_approval(
        &self,
        task_id: Uuid,
        tool: &str,
        args_json: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO pending_approvals (id, task_id, tool, args_json) VALUES (?1, ?2, ?3, ?4)",
            params![id, task_id.to_string(), tool, args_json],
        )?;
        Ok(id)
    }

    pub fn pending_approval_decision(&self, id: &str) -> Result<Option<bool>> {
        let conn = self.get_conn();
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM pending_approvals WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match status.as_deref() {
            Some("approved") => Some(true),
            Some("denied") | Some("expired") => Some(false),
            _ => None,
        })
    }

    pub fn expire_pending_approval(&self, id: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE pending_approvals SET status = 'expired', decided_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND status = 'pending'",
            params![id],
        )?;
        Ok(())
    }

    pub fn list_pending_approvals(&self) -> Result<Vec<(String, String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, tool, args_json FROM pending_approvals
             WHERE status = 'pending' AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn decide_pending_approval(&self, id: &str, approved: bool) -> Result<bool> {
        let conn = self.get_conn();
        let status = if approved { "approved" } else { "denied" };
        let changed = conn.execute(
            "UPDATE pending_approvals SET status = ?1, decided_at = CURRENT_TIMESTAMP
             WHERE id = ?2 AND status = 'pending'
               AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)",
            params![status, id],
        )?;
        if changed == 0 {
            conn.execute(
                "UPDATE pending_approvals SET status = 'expired', decided_at = CURRENT_TIMESTAMP
                 WHERE id = ?1 AND status = 'pending'
                   AND expires_at IS NOT NULL AND expires_at <= CURRENT_TIMESTAMP",
                params![id],
            )?;
        }
        Ok(changed == 1)
    }

    /// Create a one-time gateway approval. `binding_hash` covers the workspace,
    /// image digest, tool, and canonical arguments; approving a request can
    /// therefore never authorize a changed retry.
    pub fn create_gateway_approval(
        &self,
        task_id: Uuid,
        tool: &str,
        args_json: &str,
        binding_hash: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO pending_approvals (id, task_id, tool, args_json, binding_hash, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now', '+10 minutes'))",
            params![id, task_id.to_string(), tool, args_json, binding_hash],
        )?;
        Ok(id)
    }

    /// Consume exactly one approved, unexpired gateway approval. The UPDATE is
    /// conditional so parallel retries cannot replay the same grant.
    pub fn consume_gateway_approval(&self, binding_hash: &str) -> Result<bool> {
        let conn = self.get_conn();
        let changed = conn.execute(
            "UPDATE pending_approvals SET status = 'used', used_at = CURRENT_TIMESTAMP
             WHERE id = (
                 SELECT id FROM pending_approvals
                 WHERE binding_hash = ?1 AND status = 'approved'
                   AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)
                 ORDER BY decided_at ASC LIMIT 1
             ) AND status = 'approved'",
            params![binding_hash],
        )?;
        Ok(changed == 1)
    }

    pub fn start_gateway_session(
        &self,
        session_id: &str,
        task_id: &str,
        workspace: &str,
    ) -> Result<()> {
        let conn = self.get_conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO gateway_sessions
             (session_id, task_id, workspace, state, started_at, last_activity_at)
             VALUES (?1, ?2, ?3, 'running', ?4, ?4)",
            params![session_id, task_id, workspace, now],
        )?;
        Ok(())
    }

    pub fn identify_gateway_session(
        &self,
        session_id: &str,
        client_name: Option<&str>,
        client_version: Option<&str>,
        protocol_version: &str,
    ) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE gateway_sessions
             SET client_name = ?1, client_version = ?2, protocol_version = ?3,
                 last_activity_at = ?4
             WHERE session_id = ?5",
            params![
                client_name,
                client_version,
                protocol_version,
                chrono::Utc::now().to_rfc3339(),
                session_id
            ],
        )?;
        Ok(())
    }

    pub fn finish_gateway_session(&self, session_id: &str) -> Result<()> {
        let conn = self.get_conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE gateway_sessions
             SET state = 'completed', last_activity_at = ?1, ended_at = ?1
             WHERE session_id = ?2 AND state = 'running'",
            params![now, session_id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_tool_call_receipt(
        &self,
        call_id: &str,
        session_id: &str,
        task_id: &str,
        client_name: Option<&str>,
        plugin_name: Option<&str>,
        image_digest: Option<&str>,
        tool: &str,
        policy_decision: &str,
    ) -> Result<()> {
        let conn = self.get_conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO tool_call_receipts
             (call_id, session_id, task_id, client_name, plugin_name, image_digest,
              tool, policy_decision, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                call_id,
                session_id,
                task_id,
                client_name,
                plugin_name,
                image_digest,
                tool,
                policy_decision,
                now
            ],
        )?;
        conn.execute(
            "UPDATE gateway_sessions SET last_activity_at = ?1 WHERE session_id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    pub fn finish_tool_call_receipt(
        &self,
        call_id: &str,
        approval_id: Option<&str>,
        duration_ms: i64,
        result_class: &str,
        trace_id: Option<&str>,
        output_preview: Option<&str>,
    ) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE tool_call_receipts
             SET approval_id = ?1, completed_at = ?2, duration_ms = ?3,
                 result_class = ?4, trace_id = ?5, output_preview = ?6
             WHERE call_id = ?7",
            params![
                approval_id,
                chrono::Utc::now().to_rfc3339(),
                duration_ms,
                result_class,
                trace_id,
                output_preview,
                call_id
            ],
        )?;
        Ok(())
    }

    pub fn recent_gateway_sessions(&self, limit: usize) -> Result<Vec<GatewaySessionRecord>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT session_id, task_id, client_name, client_version, protocol_version,
                    workspace, state, started_at, last_activity_at, ended_at
             FROM gateway_sessions ORDER BY last_activity_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(GatewaySessionRecord {
                session_id: row.get(0)?,
                task_id: row.get(1)?,
                client_name: row.get(2)?,
                client_version: row.get(3)?,
                protocol_version: row.get(4)?,
                workspace: row.get(5)?,
                state: row.get(6)?,
                started_at: row.get(7)?,
                last_activity_at: row.get(8)?,
                ended_at: row.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn recent_tool_call_receipts(&self, limit: usize) -> Result<Vec<ToolCallReceipt>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT call_id, session_id, task_id, client_name, plugin_name, image_digest,
                    tool, policy_decision, approval_id, started_at, completed_at, duration_ms,
                    result_class, trace_id, output_preview
             FROM tool_call_receipts ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(ToolCallReceipt {
                call_id: row.get(0)?,
                session_id: row.get(1)?,
                task_id: row.get(2)?,
                client_name: row.get(3)?,
                plugin_name: row.get(4)?,
                image_digest: row.get(5)?,
                tool: row.get(6)?,
                policy_decision: row.get(7)?,
                approval_id: row.get(8)?,
                started_at: row.get(9)?,
                completed_at: row.get(10)?,
                duration_ms: row.get(11)?,
                result_class: row.get(12)?,
                trace_id: row.get(13)?,
                output_preview: row.get(14)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Receipts created at or after `started_after`, newest first.  Timestamps
    /// are RFC 3339 UTC strings, so SQLite's lexical comparison is stable.
    pub fn tool_call_receipts_since(
        &self,
        started_after: &str,
        limit: usize,
    ) -> Result<Vec<ToolCallReceipt>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT call_id, session_id, task_id, client_name, plugin_name, image_digest,
                    tool, policy_decision, approval_id, started_at, completed_at, duration_ms,
                    result_class, trace_id, output_preview
             FROM tool_call_receipts
             WHERE started_at >= ?1
             ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![started_after, limit as i64], |row| {
            Ok(ToolCallReceipt {
                call_id: row.get(0)?,
                session_id: row.get(1)?,
                task_id: row.get(2)?,
                client_name: row.get(3)?,
                plugin_name: row.get(4)?,
                image_digest: row.get(5)?,
                tool: row.get(6)?,
                policy_decision: row.get(7)?,
                approval_id: row.get(8)?,
                started_at: row.get(9)?,
                completed_at: row.get(10)?,
                duration_ms: row.get(11)?,
                result_class: row.get(12)?,
                trace_id: row.get(13)?,
                output_preview: row.get(14)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The most recently created task's id, if any. Powers the `last` alias.
    pub fn get_last_task_id(&self) -> Result<Option<String>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT id FROM tasks ORDER BY created_at DESC LIMIT 1")?;
        let result: rusqlite::Result<String> = stmt.query_row([], |row| row.get(0));
        match result {
            Ok(id) => Ok(Some(id)),
            Err(_) => Ok(None),
        }
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

    #[allow(clippy::type_complexity)]
    pub fn get_task_observability(
        &self,
        task_id: &str,
    ) -> Result<(String, String, String, i64, String, f64, i64, i64)> {
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

    /// Store a memory. The embedding is computed from the content with the
    /// built-in local embedder — callers no longer pass a vector.
    pub fn add_episodic_memory(&self, content: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let embedding = crate::embeddings::embed(content);
        let embedding_json = serde_json::to_string(&embedding)?;
        let conn = self.get_conn();
        conn.execute(
            "INSERT INTO episodic_memory (id, content, embedding_json, status) VALUES (?1, ?2, ?3, 'STAGED')",
            params![id, content, embedding_json],
        )?;
        Ok(id)
    }

    #[allow(dead_code)]
    pub fn add_tagged_memory(&self, content: &str, tags: &str) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let embedding = crate::embeddings::embed(content);
        let embedding_json = serde_json::to_string(&embedding)?;
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
        // Only return APPROVED memories
        let mut stmt = conn.prepare(
            "SELECT content, embedding_json FROM episodic_memory WHERE status = 'APPROVED'",
        )?;
        let rows = stmt.query_map([], |row| {
            let content: String = row.get(0)?;
            let embedding_json: String = row.get(1)?;
            Ok((content, embedding_json))
        })?;

        let mut matched = Vec::new();

        for row in rows {
            let (content, embedding_json) = row?;
            if let Ok(vec) = serde_json::from_str::<Vec<f32>>(&embedding_json) {
                let similarity = crate::embeddings::cosine_similarity(query_embedding, &vec);
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
            "SELECT content FROM episodic_memory WHERE status = 'APPROVED' AND content LIKE ?1 ORDER BY created_at DESC LIMIT ?2",
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
        let mut stmt = conn.prepare("SELECT content, created_at, date(created_at) as d FROM episodic_memory WHERE status = 'APPROVED' ORDER BY created_at DESC LIMIT 50")?;
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

    pub fn approve_memory(&self, id: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute(
            "UPDATE episodic_memory SET status = 'APPROVED' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn reject_memory(&self, id: &str) -> Result<()> {
        let conn = self.get_conn();
        conn.execute("DELETE FROM episodic_memory WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_staged_memories(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare("SELECT id, content, created_at FROM episodic_memory WHERE status = 'STAGED' ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            let created: String = row.get(2)?;
            Ok((id, content, created))
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

    /// Prefix used to namespace genuine, user-facing style/communication
    /// preferences (set via `kerna preferences`) within the same
    /// `user_preferences` table other subsystems also use for internal
    /// bookkeeping (e.g. `watchdog.rs` stores content-change hashes there).
    /// Namespacing means `gather_context` can safely surface only real
    /// preferences to the LLM without leaking internal state into the prompt.
    const STYLE_PREFIX: &'static str = "style.";

    /// Set a user-facing style/communication preference (e.g. "tone" ->
    /// "concise"). Explicit only — nothing here is inferred automatically.
    pub fn set_style_preference(&self, key: &str, value: &str) -> Result<()> {
        self.set_preference(&format!("{}{}", Self::STYLE_PREFIX, key), value)
    }

    /// All user-facing style preferences, with the internal `style.` prefix
    /// stripped. Excludes any other key stored in `user_preferences` (e.g.
    /// watchdog bookkeeping), so this is what should be injected into prompts.
    pub fn get_style_preferences(&self) -> Result<Vec<(String, String)>> {
        let conn = self.get_conn();
        let mut stmt = conn.prepare(
            "SELECT key, value FROM user_preferences WHERE key LIKE ?1 ORDER BY key ASC",
        )?;
        let pattern = format!("{}%", Self::STYLE_PREFIX);
        let rows = stmt.query_map(params![pattern], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok((key, value))
        })?;

        let mut prefs = Vec::new();
        for r in rows {
            let (key, value) = r?;
            let display_key = key
                .strip_prefix(Self::STYLE_PREFIX)
                .unwrap_or(&key)
                .to_string();
            prefs.push((display_key, value));
        }
        Ok(prefs)
    }

    /// Remove a previously-set style preference. Returns true if a row was deleted.
    pub fn remove_style_preference(&self, key: &str) -> Result<bool> {
        let conn = self.get_conn();
        let affected = conn.execute(
            "DELETE FROM user_preferences WHERE key = ?1",
            params![format!("{}{}", Self::STYLE_PREFIX, key)],
        )?;
        Ok(affected > 0)
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
            params![
                uuid::Uuid::new_v4().to_string(),
                subject,
                predicate,
                object,
                confidence
            ],
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

        // 1. Relevant past memories — semantic search first (embedding cosine
        //    similarity), then top up with lexical LIKE matches for anything the
        //    embedder missed. Deduplicated, capped at 3.
        let mut memories: Vec<String> = Vec::new();
        let query_embedding = crate::embeddings::embed(goal);
        if let Ok(semantic) = self.search_episodic_memory(&query_embedding, 3) {
            // Only keep clearly-relevant hits so unrelated memories aren't
            // injected. In this hashing-embedding space, related items land
            // around 0.14+ while unrelated ones sit below ~0.07.
            for (content, score) in semantic {
                if score > 0.10 && !memories.contains(&content) {
                    memories.push(content);
                }
            }
        }
        if memories.len() < 3 {
            if let Ok(text_hits) = self.search_memory_by_text(goal, 3) {
                for m in text_hits {
                    if memories.len() >= 3 {
                        break;
                    }
                    if !memories.contains(&m) {
                        memories.push(m);
                    }
                }
            }
        }
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

        // 2. User preferences (only genuine style/communication preferences —
        // NOT other subsystems' internal bookkeeping stored in the same table,
        // e.g. watchdog.rs's content-change hashes; see get_style_preferences).
        let prefs = self.get_style_preferences()?;
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

        let budget_snapshot_str = event.budget_snapshot_json.map(|v| v.to_string());
        let (safe_payload, payload_was_redacted) =
            crate::events::redact_payload(&event.payload_json);
        let payload_str = safe_payload.to_string();
        let redaction_status = match (event.redaction_status, payload_was_redacted) {
            (Some(status), true) => Some(format!("{}; payload_redacted", status)),
            (Some(status), false) => Some(status),
            (None, true) => Some("payload_redacted".to_string()),
            (None, false) => None,
        };

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
                redaction_status,
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
        mem.log_message(task_id, "INFO", "Testing task log")
            .unwrap();

        let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].2, "Testing task log");
    }

    #[test]
    fn test_pending_approval_has_one_terminal_decision() {
        let mem = setup_test_db("test_pending_approvals");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "approval test").unwrap();

        let approval_id = mem
            .create_pending_approval(task_id, "list_events", r#"{\"date\":\"today\"}"#)
            .unwrap();
        assert_eq!(mem.pending_approval_decision(&approval_id).unwrap(), None);

        let pending = mem.list_pending_approvals().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, approval_id);
        assert_eq!(pending[0].2, "list_events");

        assert!(mem.decide_pending_approval(&approval_id, true).unwrap());
        assert_eq!(
            mem.pending_approval_decision(&approval_id).unwrap(),
            Some(true)
        );
        assert!(mem.list_pending_approvals().unwrap().is_empty());
        assert!(
            !mem.decide_pending_approval(&approval_id, false).unwrap(),
            "a decision must not be overwritten"
        );
    }

    #[test]
    fn gateway_approval_is_single_use_and_bound_to_its_hash() {
        let mem = setup_test_db("test_gateway_approval_binding");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "gateway approval test")
            .unwrap();
        let approval = mem
            .create_gateway_approval(
                task_id,
                "write_file",
                r#"{\"path\":\"output/a.txt\"}"#,
                "hash-a",
            )
            .unwrap();
        assert!(mem.decide_pending_approval(&approval, true).unwrap());
        assert!(mem.consume_gateway_approval("hash-a").unwrap());
        assert!(!mem.consume_gateway_approval("hash-a").unwrap());
        assert!(!mem.consume_gateway_approval("hash-b").unwrap());
    }

    #[test]
    fn expired_gateway_approval_cannot_be_listed_or_approved() {
        let mem = setup_test_db("test_gateway_approval_expiry");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "expiry test").unwrap();
        let approval = mem
            .create_gateway_approval(task_id, "write_file", "{}", "expiry-hash")
            .unwrap();
        assert_eq!(mem.list_pending_approvals().unwrap().len(), 1);
        mem.expire_pending_approval(&approval).unwrap();
        assert!(mem.list_pending_approvals().unwrap().is_empty());
        assert!(!mem.decide_pending_approval(&approval, true).unwrap());
    }

    #[test]
    fn gateway_session_and_receipt_are_queryable_for_dashboard_metrics() {
        let mem = setup_test_db("test_gateway_dashboard_receipts");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "gateway session").unwrap();
        mem.start_gateway_session("gateway-test", &task_id.to_string(), "C:/workspace")
            .unwrap();
        mem.identify_gateway_session("gateway-test", Some("Codex"), Some("1.0"), "2025-06-18")
            .unwrap();
        mem.start_tool_call_receipt(
            "call-test",
            "gateway-test",
            &task_id.to_string(),
            Some("Codex"),
            Some("filesystem"),
            Some("python@sha256:test"),
            "read_file",
            "AutoApprove",
        )
        .unwrap();
        mem.finish_tool_call_receipt(
            "call-test",
            None,
            12,
            "completed",
            Some("trace-test"),
            Some("read content"),
        )
        .unwrap();

        let sessions = mem.recent_gateway_sessions(5).unwrap();
        assert_eq!(sessions[0].client_name.as_deref(), Some("Codex"));
        let receipts = mem.recent_tool_call_receipts(5).unwrap();
        assert_eq!(receipts[0].tool, "read_file");
        assert_eq!(receipts[0].duration_ms, Some(12));
        assert_eq!(receipts[0].result_class.as_deref(), Some("completed"));
    }

    #[test]
    fn test_event_payload_credentials_are_redacted_before_persistence() {
        let mem = setup_test_db("test_event_redaction");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "redaction test").unwrap();
        mem.record(Event {
            event_id: Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            session_id: None,
            sequence: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: "tool.call.requested".to_string(),
            actor: "llm".to_string(),
            severity: "info".to_string(),
            model: None,
            tool: Some("example".to_string()),
            policy_decision: None,
            risk_score: None,
            parent_event_id: None,
            correlation_id: None,
            redaction_status: None,
            budget_snapshot_json: None,
            payload_json: serde_json::json!({
                "args": r#"{"token":"must-not-persist","message":"safe"}"#
            }),
        })
        .unwrap();

        let events = mem.get_events(&task_id.to_string()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].redaction_status.as_deref(),
            Some("payload_redacted")
        );
        assert!(!events[0]
            .payload_json
            .to_string()
            .contains("must-not-persist"));
        assert!(events[0].payload_json.to_string().contains("[REDACTED]"));
    }

    #[test]
    fn test_style_preferences_are_namespaced_and_dont_leak_watchdog_bookkeeping() {
        let mem = setup_test_db("test_style_prefs");

        // A genuine user-facing preference.
        mem.set_style_preference("tone", "concise").unwrap();
        // Simulate watchdog.rs's internal bookkeeping, which writes to the same
        // underlying table via the generic set_preference (see watchdog.rs).
        mem.set_preference("watchdog_some-task-id", "9f8e7d6c5b4a")
            .unwrap();

        let prefs = mem.get_style_preferences().unwrap();
        assert_eq!(prefs.len(), 1, "only the real preference should surface");
        assert_eq!(prefs[0], ("tone".to_string(), "concise".to_string()));

        // The bug this guards against: gather_context must never show
        // watchdog's internal key in the prompt.
        let ctx = mem.gather_context("anything").unwrap();
        assert!(
            ctx.contains("tone: concise"),
            "real preference should be in context: {}",
            ctx
        );
        assert!(
            !ctx.contains("watchdog_"),
            "internal bookkeeping must never leak into the prompt: {}",
            ctx
        );

        // Removal only affects the namespaced key.
        assert!(mem.remove_style_preference("tone").unwrap());
        assert!(!mem.remove_style_preference("tone").unwrap()); // already gone
        assert!(mem.get_style_preferences().unwrap().is_empty());
        // watchdog's key is untouched by preference removal.
        assert_eq!(
            mem.get_preference("watchdog_some-task-id").unwrap(),
            Some("9f8e7d6c5b4a".to_string())
        );
    }

    #[test]
    fn test_semantic_memory_recall_ranks_relevant_first() {
        let mem = setup_test_db("test_semantic");

        // Store three unrelated memories and approve them so they're searchable.
        let ids = [
            mem.add_episodic_memory("Deleting files requires explicit confirmation for safety")
                .unwrap(),
            mem.add_episodic_memory("The user prefers dark mode in the terminal UI")
                .unwrap(),
            mem.add_episodic_memory("Kerna enforces execution budgets to stop runaway loops")
                .unwrap(),
        ];
        for id in &ids {
            mem.approve_memory(id).unwrap();
        }

        // A query about deleting files should surface the file-deletion memory first.
        let query = crate::embeddings::embed("how do I safely delete a file");
        let results = mem.search_episodic_memory(&query, 3).unwrap();
        assert!(!results.is_empty());
        assert!(
            results[0].0.contains("Deleting files"),
            "expected file-deletion memory ranked first, got: {}",
            results[0].0
        );

        // And gather_context should inject it for a related goal.
        let ctx = mem.gather_context("delete an old file").unwrap();
        assert!(
            ctx.contains("Deleting files"),
            "semantic context missing the relevant memory: {}",
            ctx
        );
    }

    #[test]
    fn test_memory_stress_test() {
        let mem = setup_test_db("test_stress");
        let task_id = Uuid::new_v4();
        mem.create_task(task_id, None, "Stress Test").unwrap();

        for i in 0..100 {
            mem.log_message(task_id, "INFO", &format!("Log message {}", i))
                .unwrap();
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
                    let _ =
                        mem_clone.log_message(task_id, "INFO", &format!("Thread {} Msg {}", i, j));
                }
            }));
        }

        for h in handles {
            let _ = h.join();
        }

        let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
        assert_eq!(
            logs.len(),
            5000,
            "Database must survive extreme concurrency without locking"
        );
    }
}
