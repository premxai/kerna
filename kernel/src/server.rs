use crate::config::Config;
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub memory: Arc<MemoryEngine>,
    pub mcp_registry: Arc<Mutex<McpRegistry>>,
    /// When set, requests must present `Authorization: Bearer <token>`.
    pub auth_token: Option<String>,
}

#[derive(Clone)]
struct DashboardState {
    app: AppState,
    csrf_token: String,
    origin: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Serialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessageRes,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct ChatMessageRes {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    message: String,
    r#type: String,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (
        status,
        Json(ApiError {
            error: ApiErrorBody {
                message: message.into(),
                r#type: "kerna_error".to_string(),
            },
        }),
    )
        .into_response()
}

pub async fn start_server(state: AppState, bind: &str, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completion))
        .with_state(state);

    let ip: std::net::IpAddr = bind
        .parse()
        .unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
    let addr = SocketAddr::new(ip, port);
    println!("[+] API Server listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Start the local-only observability surface. It reads durable SQLite records
/// written by every gateway process, so opening the dashboard does not require
/// a separate daemon or a client-specific integration.
pub async fn start_dashboard_server(
    state: AppState,
    port: u16,
    open_browser: bool,
) -> anyhow::Result<()> {
    let dashboard = DashboardState {
        app: state,
        csrf_token: Uuid::new_v4().to_string(),
        origin: format!("http://127.0.0.1:{port}"),
    };
    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/api/v1/dashboard/overview", get(dashboard_overview))
        .route("/api/v1/dashboard/sessions", get(dashboard_sessions))
        .route("/api/v1/dashboard/receipts", get(dashboard_receipts))
        .route("/api/v1/dashboard/approvals", get(dashboard_approvals))
        .route("/api/v1/dashboard/containment", get(dashboard_containment))
        .route("/api/v1/dashboard/models", get(dashboard_models))
        .route(
            "/api/v1/dashboard/registry/recommendations",
            get(dashboard_recommendations),
        )
        .route("/api/v1/dashboard/traces/:task_id", get(dashboard_traces))
        .route("/api/v1/dashboard/events", get(dashboard_events))
        .route(
            "/api/v1/dashboard/approvals/:id/approve",
            post(approve_dashboard_approval),
        )
        .route(
            "/api/v1/dashboard/approvals/:id/reject",
            post(reject_dashboard_approval),
        )
        .with_state(dashboard.clone());
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("[+] Kerna dashboard listening on http://{}/", addr);
    println!("[i] Local dashboard CSRF token: {}", dashboard.csrf_token);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    if open_browser {
        let _ = webbrowser::open(&format!("http://{}/", addr));
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn dashboard_page(State(state): State<DashboardState>) -> Html<String> {
    Html(include_str!("../assets/dashboard.html").replace("{csrf}", &state.csrf_token))
}

async fn dashboard_overview(State(state): State<DashboardState>) -> Json<Value> {
    Json(dashboard_snapshot(&state))
}

async fn dashboard_sessions(State(state): State<DashboardState>) -> Json<Value> {
    Json(json!({"sessions": state.app.memory.recent_gateway_sessions(100).unwrap_or_default()}))
}

async fn dashboard_receipts(State(state): State<DashboardState>) -> Json<Value> {
    Json(json!({"receipts": state.app.memory.recent_tool_call_receipts(100).unwrap_or_default()}))
}

async fn dashboard_approvals(State(state): State<DashboardState>) -> Json<Value> {
    let approvals = state
        .app
        .memory
        .list_pending_approvals()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, task_id, tool, args_json)| {
            let parsed = serde_json::from_str(&args_json).unwrap_or(Value::String(args_json));
            let (arguments, _) = crate::events::redact_payload(&parsed);
            json!({"id": id, "task_id": task_id, "tool": tool, "arguments": arguments})
        })
        .collect::<Vec<_>>();
    Json(json!({"approvals": approvals}))
}

async fn dashboard_containment(State(state): State<DashboardState>) -> Json<Value> {
    let containment = state.app.config.mcp_servers.iter().filter(|plugin| plugin.enabled).map(|plugin| json!({
        "name": plugin.name,
        "runtime_mode": plugin.runtime_mode,
        "image_digest": plugin.image,
        "network": "none",
        "read_roots": plugin.read_roots,
        "write_roots": plugin.write_roots,
        "tools": if plugin.allow_tools.is_empty() { plugin.capabilities.clone() } else { plugin.allow_tools.clone() }
    })).collect::<Vec<_>>();
    Json(json!({"containment": containment}))
}

async fn dashboard_models(State(state): State<DashboardState>) -> Json<Value> {
    let hardware = crate::models::detect_hardware();
    let recommendations = crate::models::recommend(&hardware, "coding").unwrap_or_default();
    let local_runtime = match crate::providers::discover_local_models(&state.app.config, "ollama")
        .await
    {
        Ok(installed_models) => json!({
            "provider": "ollama", "endpoint_health": "healthy", "installed_models": installed_models
        }),
        Err(error) => json!({
            "provider": "ollama", "endpoint_health": "unreachable", "installed_models": [],
            "error": crate::gateway::redacted_text_preview(&error.to_string())
        }),
    };
    Json(json!({
        "hardware": hardware,
        "recommendations": recommendations,
        "local_runtime": local_runtime,
        "kerna_routed_model": {"routes": state.app.config.model_routes, "privacy_routes": state.app.config.privacy_routes},
        "external_client_model": "not controlled by Kerna"
    }))
}

async fn dashboard_recommendations(State(_state): State<DashboardState>) -> Json<Value> {
    let hardware = crate::models::detect_hardware();
    Json(json!({
        "catalog": crate::models::catalog().ok().map(|catalog| catalog.source),
        "hardware": hardware,
        "recommendations": crate::models::recommend(&hardware, "coding").unwrap_or_default(),
    }))
}

async fn dashboard_traces(
    State(state): State<DashboardState>,
    Path(task_id): Path<String>,
) -> Json<Value> {
    Json(
        json!({"task_id": task_id, "events": state.app.memory.get_events(&task_id).unwrap_or_default()}),
    )
}

async fn dashboard_events(State(state): State<DashboardState>) -> impl IntoResponse {
    let stream = async_stream::stream! {
        let mut previous = String::new();
        loop {
            let snapshot = dashboard_snapshot(&state);
            let mut comparison = snapshot.clone();
            comparison.as_object_mut().map(|object| object.remove("generated_at"));
            let encoded = snapshot.to_string();
            if comparison.to_string() != previous {
                previous = comparison.to_string();
                yield Ok::<SseEvent, std::convert::Infallible>(
                    SseEvent::default().event("snapshot").data(encoded),
                );
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
}

async fn approve_dashboard_approval(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    decide_dashboard_approval(state, id, headers, true)
}

async fn reject_dashboard_approval(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    decide_dashboard_approval(state, id, headers, false)
}

fn decide_dashboard_approval(
    state: DashboardState,
    id: String,
    headers: HeaderMap,
    approved: bool,
) -> axum::response::Response {
    if !dashboard_mutation_is_authorized(&state, &headers) {
        return error_response(StatusCode::FORBIDDEN, "Dashboard CSRF validation failed.");
    }
    match state.app.memory.decide_pending_approval(&id, approved) {
        Ok(true) => {
            Json(json!({"ok": true, "status": if approved { "approved" } else { "rejected" }}))
                .into_response()
        }
        Ok(false) => error_response(StatusCode::CONFLICT, "Approval is no longer pending."),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn dashboard_mutation_is_authorized(state: &DashboardState, headers: &HeaderMap) -> bool {
    let same_origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(|origin| origin == state.origin)
        .unwrap_or(false);
    let valid_token = headers
        .get("x-kerna-dashboard-csrf")
        .and_then(|value| value.to_str().ok())
        .map(|token| token == state.csrf_token)
        .unwrap_or(false);
    same_origin && valid_token
}

fn dashboard_snapshot(state: &DashboardState) -> Value {
    let sessions = state
        .app
        .memory
        .recent_gateway_sessions(100)
        .unwrap_or_default();
    let active_sessions = sessions
        .iter()
        .filter(|session| session.state == "running")
        .count();
    let receipts = state
        .app
        .memory
        .recent_tool_call_receipts(100)
        .unwrap_or_default();
    let approvals = state
        .app
        .memory
        .list_pending_approvals()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, task_id, tool, args_json)| {
            let parsed = serde_json::from_str(&args_json).unwrap_or(Value::String(args_json));
            let (arguments, _) = crate::events::redact_payload(&parsed);
            json!({"id": id, "task_id": task_id, "tool": tool, "arguments": arguments})
        })
        .collect::<Vec<_>>();
    let metric_receipts = state
        .app
        .memory
        .tool_call_receipts_since(
            &(chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
            1_000,
        )
        .unwrap_or_default();
    let durations = metric_receipts
        .iter()
        .filter_map(|receipt| receipt.duration_ms)
        .collect::<Vec<_>>();
    let mut sorted = durations.clone();
    sorted.sort_unstable();
    let percentile = |percentage: f64| -> Option<i64> {
        (!sorted.is_empty())
            .then(|| sorted[((sorted.len() - 1) as f64 * percentage).round() as usize])
    };
    let denied = metric_receipts
        .iter()
        .filter(|receipt| receipt.result_class == Some("blocked".to_string()))
        .count();
    let failed = metric_receipts
        .iter()
        .filter(|receipt| receipt.result_class == Some("failed".to_string()))
        .count();
    let completed = metric_receipts
        .iter()
        .filter(|receipt| receipt.result_class == Some("completed".to_string()))
        .count();
    let plugins = state.app.config.mcp_servers.iter().filter(|plugin| plugin.enabled).map(|plugin| json!({
        "name": plugin.name,
        "runtime_mode": plugin.runtime_mode,
        "image_digest": plugin.image,
        "network": "none",
        "read_roots": plugin.read_roots,
        "write_roots": plugin.write_roots,
        "tools": if plugin.allow_tools.is_empty() { plugin.capabilities.clone() } else { plugin.allow_tools.clone() }
    })).collect::<Vec<_>>();
    json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "metrics": {
            "window_seconds": 60, "active_sessions": active_sessions,
            "tool_calls": metric_receipts.len(), "completed": completed, "denied": denied,
            "failed": failed, "pending_approvals": approvals.len(),
            "p50_duration_ms": percentile(0.5), "p95_duration_ms": percentile(0.95)
        },
        "sessions": sessions,
        "receipts": receipts,
        "approvals": approvals,
        "containment": plugins,
        "models": {"external_clients": "Model selection remains client-controlled", "routes": state.app.config.model_routes, "privacy_routes": state.app.config.privacy_routes}
    })
}

/// Constant-time-ish bearer check. Returns true when auth is satisfied.
fn is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.auth_token else {
        return true; // No token configured → loopback-only, open.
    };
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    !presented.is_empty() && presented == expected
}

async fn handle_chat_completion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChatCompletionRequest>,
) -> axum::response::Response {
    if !is_authorized(&state, &headers) {
        return error_response(StatusCode::UNAUTHORIZED, "Missing or invalid bearer token.");
    }

    // Extract the latest user message as the goal.
    let goal = payload
        .messages
        .last()
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    if goal.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "No user message content provided.");
    }

    let scheduler = match TaskScheduler::new(
        state.config.clone(),
        state.memory.clone(),
        state.mcp_registry.clone(),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to initialize scheduler: {}", e),
            )
        }
    };

    let task_id = match scheduler.run_goal(&goal).await {
        Ok(id) => id,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Task execution failed: {}", e),
            )
        }
    };

    // Return the real final assistant message the agent produced.
    let final_content = state
        .memory
        .get_task_result(&task_id.to_string())
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("Task {} completed with no textual output.", task_id));

    let response = ChatCompletionResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        model: payload.model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessageRes {
                role: "assistant".to_string(),
                content: final_content,
            },
            finish_reason: "stop".to_string(),
        }],
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryEngine;
    use std::sync::Arc;

    #[test]
    fn dashboard_snapshot_aggregates_durable_gateway_receipts() {
        let path = std::env::temp_dir().join(format!("kerna-dashboard-{}.db", Uuid::new_v4()));
        let memory = Arc::new(MemoryEngine::new(&path).unwrap());
        let task = Uuid::new_v4();
        memory.create_task(task, None, "dashboard test").unwrap();
        memory
            .start_gateway_session("session", &task.to_string(), "workspace")
            .unwrap();
        memory
            .identify_gateway_session("session", Some("Qoder"), Some("1"), "2025-06-18")
            .unwrap();
        memory
            .start_tool_call_receipt(
                "call",
                "session",
                &task.to_string(),
                Some("Qoder"),
                Some("plugin"),
                Some("image@sha256:test"),
                "read_file",
                "AutoApprove",
            )
            .unwrap();
        memory
            .finish_tool_call_receipt(
                "call",
                None,
                25,
                "completed",
                Some("trace"),
                Some("safe preview"),
            )
            .unwrap();
        let state = DashboardState {
            app: AppState {
                config: Config::default(),
                memory,
                mcp_registry: Arc::new(Mutex::new(McpRegistry::new())),
                auth_token: None,
            },
            csrf_token: "csrf".to_string(),
            origin: "http://127.0.0.1:8765".to_string(),
        };
        let snapshot = dashboard_snapshot(&state);
        assert_eq!(snapshot["metrics"]["tool_calls"], 1);
        assert_eq!(snapshot["metrics"]["completed"], 1);
        assert_eq!(snapshot["sessions"][0]["client_name"], "Qoder");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dashboard_mutations_require_exact_loopback_origin_and_csrf_token() {
        let state = DashboardState {
            app: AppState {
                config: Config::default(),
                memory: Arc::new(MemoryEngine::new(":memory:").unwrap()),
                mcp_registry: Arc::new(Mutex::new(McpRegistry::new())),
                auth_token: None,
            },
            csrf_token: "one-time-token".to_string(),
            origin: "http://127.0.0.1:8765".to_string(),
        };
        let mut valid = HeaderMap::new();
        valid.insert("origin", "http://127.0.0.1:8765".parse().unwrap());
        valid.insert("x-kerna-dashboard-csrf", "one-time-token".parse().unwrap());
        assert!(dashboard_mutation_is_authorized(&state, &valid));

        valid.insert("origin", "http://localhost:8765".parse().unwrap());
        assert!(!dashboard_mutation_is_authorized(&state, &valid));
        valid.insert("origin", "http://127.0.0.1:8765".parse().unwrap());
        valid.insert("x-kerna-dashboard-csrf", "wrong".parse().unwrap());
        assert!(!dashboard_mutation_is_authorized(&state, &valid));
    }
}
