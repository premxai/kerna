use crate::config::Config;
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
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
