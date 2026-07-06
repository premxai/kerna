use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub task_id: String,
    pub session_id: Option<String>,
    pub sequence: i64,
    pub timestamp: String,
    pub event_type: String,
    pub actor: String,
    pub severity: String,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub policy_decision: Option<String>,
    pub risk_score: Option<f64>,
    pub parent_event_id: Option<String>,
    pub correlation_id: Option<String>,
    pub redaction_status: Option<String>,
    pub budget_snapshot_json: Option<serde_json::Value>,
    pub payload_json: serde_json::Value,
}

pub trait EventSink: Send + Sync {
    fn record(&self, event: Event) -> Result<()>;
}
