use crate::config::Config;
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use axum::{extract::State, routing::post, Json, Router};
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

pub async fn start_server(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/v1/chat/completions", post(handle_chat_completion))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("[+] API Server listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_chat_completion(
    State(state): State<AppState>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Json<ChatCompletionResponse> {
    // Extract the latest user message as the goal
    let goal = payload
        .messages
        .last()
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("Empty goal");

    // Spawn a scheduler and run
    let scheduler = TaskScheduler::new(
        state.config.clone(),
        state.memory.clone(),
        state.mcp_registry.clone(),
        None,
    )
    .unwrap();

    let task_id = scheduler
        .run_goal(goal)
        .await
        .unwrap_or_else(|_| Uuid::nil());

    // Grab the final response from memory logs (mocked for simplicity here, normally we'd return the final assistant message)
    let _logs = state
        .memory
        .get_task_logs(&task_id.to_string())
        .unwrap_or_default();
    let final_content = format!("Task {} completed successfully.", task_id);

    let response = ChatCompletionResponse {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
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

    Json(response)
}
