use crate::config::Config;
use crate::guard_policy::{ActionIntent, AgentKind, GuardPolicy, PolicyEffect};
use crate::mcp_registry::McpRegistry;
use crate::memory::GuardActionBinding;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::net::SocketAddr;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub guard_policy: Arc<GuardPolicy>,
    pub memory: Arc<MemoryEngine>,
    pub mcp_registry: Arc<Mutex<McpRegistry>>,
    /// Hashed repository/worktree state captured by the trusted broker at startup.
    /// Agent-controlled environment variables must not be able to redefine it.
    pub worktree_baseline: String,
    /// When set, requests must present `Authorization: Bearer <token>`.
    pub auth_token: Option<String>,
}

#[derive(Clone)]
struct GuardStreamContext {
    session_id: String,
    task_id: String,
    agent: AgentKind,
    agent_version: String,
    worktree_baseline: String,
}

#[derive(Clone)]
struct DashboardState {
    app: AppState,
    csrf_token: String,
    origin: String,
    signing_key: Arc<SigningKey>,
}

#[derive(Debug, Deserialize)]
struct ApplyWorkspaceRequest {
    target: String,
    selection: ApplyWorkspaceSelection,
    confirm: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ApplyWorkspaceSelection {
    Uncommitted,
    Commits { hashes: Vec<String> },
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
        .route("/anthropic/v1/messages", post(handle_guard_anthropic))
        .route("/openai/v1/responses", post(handle_guard_openai))
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

/// Relay an Anthropic Messages stream through the protocol gate. The session token
/// authenticates only to Kerna; the provider key is read in this trusted broker process.
async fn handle_guard_anthropic(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if !is_authorized(&state, &headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Missing or invalid session token.",
        );
    }
    let key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Anthropic broker key is unavailable.",
            )
        }
    };
    let context = match start_guard_stream(&state, AgentKind::ClaudeCode, "anthropic_messages") {
        Ok(context) => context,
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "Kerna could not persist the Claude session receipt.",
            )
        }
    };
    if observe_anthropic_results(&state.memory, &context, &body).is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Kerna could not persist the Claude tool result receipt.",
        );
    }
    let base = std::env::var("KERNA_ANTHROPIC_UPSTREAM")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let mut request = reqwest::Client::new()
        .post(provider_url(&base, "v1/messages"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .body(body);
    for name in ["anthropic-version", "anthropic-beta"] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    match request.send().await {
        Ok(upstream) => relay_anthropic(upstream, state.guard_policy, state.memory, context),
        Err(_) => error_response(
            StatusCode::BAD_GATEWAY,
            "Anthropic upstream is unavailable.",
        ),
    }
}

/// Relay an OpenAI Responses stream through the protocol gate. The caller's Authorization
/// header is deliberately not forwarded: it is a Kerna session credential, not a provider key.
async fn handle_guard_openai(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if !is_authorized(&state, &headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Missing or invalid session token.",
        );
    }
    let key = match std::env::var("OPENAI_API_KEY") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "OpenAI broker key is unavailable.",
            )
        }
    };
    let base = std::env::var("KERNA_OPENAI_UPSTREAM")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let request = reqwest::Client::new()
        .post(provider_url(&base, "responses"))
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(key)
        .body(body);
    match request.send().await {
        Ok(upstream) => relay_openai(upstream, state.guard_policy, state.memory),
        Err(_) => error_response(StatusCode::BAD_GATEWAY, "OpenAI upstream is unavailable."),
    }
}

fn provider_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}

/// Capture only a digest of the broker's starting Git/worktree state. The raw Git
/// output is transient and is never written to the database or returned to the
/// client. A missing Git repository is a startup error for the guarded protocol
/// server because approvals must not carry an unbound baseline.
pub fn capture_worktree_baseline() -> anyhow::Result<String> {
    let workspace = std::env::current_dir()?.canonicalize()?;
    let repo_root = git_output(&workspace, &["rev-parse", "--show-toplevel"])?;
    let head = git_output(&workspace, &["rev-parse", "HEAD"])?;
    let index_diff = git_output(
        &workspace,
        &["diff", "--binary", "--no-ext-diff", "--cached", "--", "."],
    )?;
    let worktree_diff = git_output(
        &workspace,
        &["diff", "--binary", "--no-ext-diff", "--", "."],
    )?;
    let status = git_output(
        &workspace,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    Ok(worktree_baseline_digest(
        &workspace,
        &repo_root,
        &head,
        &index_diff,
        &worktree_diff,
        &status,
    ))
}

fn worktree_baseline_digest(
    workspace: &std::path::Path,
    repo_root: &str,
    head: &str,
    index_diff: &str,
    worktree_diff: &str,
    status: &str,
) -> String {
    let material = json!({
        "workspace": workspace.to_string_lossy(),
        "repo_root": repo_root,
        "head": head,
        "index_diff": index_diff,
        "worktree_diff": worktree_diff,
        "status": status,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&material).expect("baseline material is serializable"))
    )
}

fn git_output(workspace: &std::path::Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git {} failed with status {}",
            args.join(" "),
            output.status
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn start_guard_stream(
    state: &AppState,
    agent: AgentKind,
    protocol: &str,
) -> Result<GuardStreamContext, String> {
    let session_id = state
        .auth_token
        .as_deref()
        .map(|token| format!("guard-{:x}", Sha256::digest(token.as_bytes())))
        .unwrap_or_else(|| format!("guard-{}", Uuid::new_v4()));
    let task_id = Uuid::new_v4();
    let agent_name = match agent {
        AgentKind::ClaudeCode => "claude_code",
        AgentKind::Codex => "codex",
    };
    let agent_version =
        std::env::var("KERNA_GUARD_AGENT_VERSION").unwrap_or_else(|_| "unknown".to_owned());
    let worktree_baseline = state.worktree_baseline.clone();
    state
        .memory
        .create_task(task_id, Some(&session_id), "Kerna Guard protocol session")
        .map_err(|error| error.to_string())?;
    state
        .memory
        .start_gateway_session(&session_id, &task_id.to_string(), &worktree_baseline)
        .map_err(|error| error.to_string())?;
    state
        .memory
        .identify_gateway_session(
            &session_id,
            Some(agent_name),
            Some(&agent_version),
            protocol,
        )
        .map_err(|error| error.to_string())?;
    Ok(GuardStreamContext {
        session_id,
        task_id: task_id.to_string(),
        agent,
        agent_version,
        worktree_baseline,
    })
}

fn observe_anthropic_results(
    memory: &MemoryEngine,
    context: &GuardStreamContext,
    body: &[u8],
) -> Result<(), String> {
    let Ok(request) = serde_json::from_slice::<Value>(body) else {
        return Ok(());
    };
    let Some(messages) = request.get("messages").and_then(Value::as_array) else {
        return Ok(());
    };
    for message in messages {
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(call_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            memory
                .observe_guard_result(&context.session_id, call_id)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn upstream_response(
    status: reqwest::StatusCode,
    stream: impl futures_core::Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + 'static,
) -> axum::response::Response {
    let mut response = axum::response::Response::new(Body::from_stream(stream));
    *response.status_mut() =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn relay_anthropic(
    mut upstream: reqwest::Response,
    policy: Arc<GuardPolicy>,
    memory: Arc<MemoryEngine>,
    context: GuardStreamContext,
) -> axum::response::Response {
    let status = upstream.status();
    let stream = async_stream::stream! {
        let stream_context = context.clone();
        let stream_memory = memory.clone();
        let decision_policy = policy.clone();
        let mut gate = crate::guard_protocol::AnthropicStreamGate::new(move |action| {
            stream_policy_decision_with_receipt(&decision_policy, &stream_memory, &stream_context, action)
        });
        'relay: loop {
            if matches!(
                memory.gateway_session_state(&context.session_id),
                Ok(Some(state)) if state == "stopped"
            ) {
                break 'relay;
            }
            match upstream.chunk().await {
                Ok(Some(chunk)) => match gate.feed(&chunk) {
                    Ok(frames) => {
                        for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); }
                        while let Some(action) = gate.pending_approval().cloned() {
                            let decision = wait_for_stream_approval(&memory, &policy, &context, &action).await;
                            match gate.resolve_pending(decision) {
                                Ok(frames) => for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); },
                                Err(_) => break 'relay,
                            }
                        }
                    },
                    Err(_) => break,
                },
                Ok(None) => {
                    if let Ok(frames) = gate.finish() {
                        for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); }
                    }
                    break;
                }
                Err(_) => break,
            }
        }
        let _ = memory.finish_gateway_session(&context.session_id);
    };
    upstream_response(status, stream)
}

fn relay_openai(
    mut upstream: reqwest::Response,
    policy: Arc<GuardPolicy>,
    memory: Arc<MemoryEngine>,
) -> axum::response::Response {
    let status = upstream.status();
    let stream = async_stream::stream! {
        let mut gate = crate::guard_protocol::OpenAiResponsesStreamGate::new(move |action| {
            stream_policy_decision(&policy, action)
        });
        'relay: loop {
            match upstream.chunk().await {
                Ok(Some(chunk)) => match gate.feed(&chunk) {
                    Ok(frames) => {
                        for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); }
                        while let Some(action) = gate.pending_approval().cloned() {
                            let decision = wait_for_stream_approval_legacy(&memory, &action).await;
                            match gate.resolve_pending(decision) {
                                Ok(frames) => for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); },
                                Err(_) => break 'relay,
                            }
                        }
                    },
                    Err(_) => break,
                },
                Ok(None) => {
                    if let Ok(frames) = gate.finish() {
                        for frame in frames { yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(frame)); }
                    }
                    break;
                }
                Err(_) => break,
            }
        }
    };
    upstream_response(status, stream)
}

fn stream_policy_decision(
    policy: &GuardPolicy,
    action: &crate::guard_protocol::ActionCandidate,
) -> crate::guard_protocol::GateDecision {
    let smoke_allow = std::env::var("KERNA_WP0_ALLOW_SMOKE_ECHO").ok().as_deref() == Some("1");
    let smoke_hold = std::env::var("KERNA_WP0_HOLD_SMOKE_ECHO").ok().as_deref() == Some("1");
    stream_policy_decision_with_smoke(policy, action, smoke_allow, smoke_hold)
}

fn stream_policy_decision_with_smoke(
    policy: &GuardPolicy,
    action: &crate::guard_protocol::ActionCandidate,
    smoke_allow: bool,
    smoke_hold: bool,
) -> crate::guard_protocol::GateDecision {
    let smoke_command = wp0_smoke_command(action);
    // WP0 can prove a real client release without granting a broadly dangerous
    // name-only rule. The opt-in is broker-local and matches one inert command
    // in either pinned client's wire representation.
    if smoke_allow && smoke_command == Some("echo KERNA_ALLOW_TEST") {
        return crate::guard_protocol::GateDecision::Allow;
    }
    // This equally narrow opt-in exercises a live dashboard approval without
    // turning a tool-name rule into permission for arbitrary shell commands.
    if smoke_hold && smoke_command == Some("echo KERNA_ASK_TEST") {
        return crate::guard_protocol::GateDecision::Hold;
    }
    let agent = match action.protocol {
        crate::guard_protocol::Protocol::AnthropicMessages => AgentKind::ClaudeCode,
        crate::guard_protocol::Protocol::OpenAiResponses => AgentKind::Codex,
    };
    let intent = ActionIntent::from_candidate(action, "wp0-stream", agent, "unbound");
    match policy.evaluate(&intent).effect {
        PolicyEffect::Allow => crate::guard_protocol::GateDecision::Allow,
        PolicyEffect::Ask => crate::guard_protocol::GateDecision::Hold,
        PolicyEffect::Deny => crate::guard_protocol::GateDecision::Deny {
            reason: "denied by Kerna policy".to_owned(),
        },
    }
}

fn guard_binding(
    policy: &GuardPolicy,
    context: &GuardStreamContext,
    action: &crate::guard_protocol::ActionCandidate,
) -> (ActionIntent, GuardActionBinding) {
    let intent = ActionIntent::from_candidate(
        action,
        context.session_id.clone(),
        context.agent,
        context.agent_version.clone(),
    );
    let protocol = match action.protocol {
        crate::guard_protocol::Protocol::AnthropicMessages => "anthropic_messages",
        crate::guard_protocol::Protocol::OpenAiResponses => "openai_responses",
    };
    let agent = match context.agent {
        AgentKind::ClaudeCode => "claude_code",
        AgentKind::Codex => "codex",
    };
    let policy_digest = policy.digest();
    let canonical_action_digest = intent.canonical_digest();
    let binding_material = json!({
        "session_id": context.session_id,
        "task_id": context.task_id,
        "agent": agent,
        "agent_version": context.agent_version,
        "protocol": protocol,
        "canonical_action_digest": canonical_action_digest,
        "policy_digest": policy_digest,
        "worktree_baseline": context.worktree_baseline,
    });
    let binding_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&binding_material).expect("binding always serializes"))
    );
    (
        intent,
        GuardActionBinding {
            call_id: action.id.clone(),
            session_id: context.session_id.clone(),
            task_id: context.task_id.clone(),
            agent: agent.to_owned(),
            agent_version: context.agent_version.clone(),
            protocol: protocol.to_owned(),
            tool: action.raw_tool_name.clone(),
            canonical_action_digest,
            policy_digest,
            worktree_baseline: context.worktree_baseline.clone(),
            binding_hash,
        },
    )
}

fn stream_policy_decision_with_receipt(
    policy: &GuardPolicy,
    memory: &MemoryEngine,
    context: &GuardStreamContext,
    action: &crate::guard_protocol::ActionCandidate,
) -> crate::guard_protocol::GateDecision {
    let (intent, binding) = guard_binding(policy, context, action);
    let mut decision = policy.evaluate(&intent);
    let smoke_command = wp0_smoke_command(action);
    if std::env::var("KERNA_WP0_ALLOW_SMOKE_ECHO").ok().as_deref() == Some("1")
        && smoke_command == Some("echo KERNA_ALLOW_TEST")
    {
        decision.effect = PolicyEffect::Allow;
        decision.reason = "WP0 inert smoke allow".to_owned();
    } else if std::env::var("KERNA_WP0_HOLD_SMOKE_ECHO").ok().as_deref() == Some("1")
        && smoke_command == Some("echo KERNA_ASK_TEST")
    {
        decision.effect = PolicyEffect::Ask;
        decision.reason = "WP0 inert smoke approval".to_owned();
    }
    let effect = match decision.effect {
        PolicyEffect::Allow => "allow",
        PolicyEffect::Ask => "ask",
        PolicyEffect::Deny => "deny",
    };
    let summary = json!({
        "protocol": binding.protocol,
        "agent": binding.agent,
        "agent_version": binding.agent_version,
        "tool": binding.tool,
        "kind": intent.kind,
        "canonical_resource": intent.canonical_resource,
        "redacted_display": intent.redacted_display,
        "risk_tags": intent.risk_tags,
        "arguments_sha256": intent.arguments_digest,
        "canonical_action_digest": binding.canonical_action_digest,
        "policy_digest": binding.policy_digest,
        "worktree_baseline": binding.worktree_baseline,
        "policy_effect": effect,
        "policy_reason": decision.reason,
    })
    .to_string();
    match memory.create_guard_action(
        &binding,
        effect,
        &summary,
        decision.effect == PolicyEffect::Ask,
    ) {
        Ok(Some(_)) if decision.effect == PolicyEffect::Ask => {
            crate::guard_protocol::GateDecision::Hold
        }
        Ok(None) if decision.effect == PolicyEffect::Allow => {
            crate::guard_protocol::GateDecision::Allow
        }
        Ok(None) if decision.effect == PolicyEffect::Deny => {
            crate::guard_protocol::GateDecision::Deny {
                reason: "denied by Kerna policy".to_owned(),
            }
        }
        Ok(_) => crate::guard_protocol::GateDecision::Deny {
            reason: "approval persistence is unavailable".to_owned(),
        },
        Err(_) => crate::guard_protocol::GateDecision::Deny {
            reason: "approval persistence is unavailable".to_owned(),
        },
    }
}

fn wp0_smoke_command(action: &crate::guard_protocol::ActionCandidate) -> Option<&str> {
    match (action.protocol, action.raw_tool_name.as_str()) {
        (crate::guard_protocol::Protocol::AnthropicMessages, "Bash") => {
            action.arguments.get("command")?.as_str()
        }
        (crate::guard_protocol::Protocol::OpenAiResponses, "exec_command") => {
            action.arguments.get("cmd")?.as_str()
        }
        (crate::guard_protocol::Protocol::OpenAiResponses, "shell") => action.arguments.as_str(),
        _ => None,
    }
}

async fn wait_for_stream_approval(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    context: &GuardStreamContext,
    action: &crate::guard_protocol::ActionCandidate,
) -> crate::guard_protocol::GateDecision {
    let (_, binding) = guard_binding(policy, context, action);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        if matches!(
            memory.gateway_session_state(&binding.session_id),
            Ok(Some(state)) if state == "stopped"
        ) {
            let _ = memory.deny_guard_action(&binding);
            return crate::guard_protocol::GateDecision::Deny {
                reason: "session stopped from the local dashboard".to_owned(),
            };
        }
        let approval_id =
            match memory.guard_approval_for_call(&binding.session_id, &binding.call_id) {
                Ok(Some(id)) => id,
                Ok(None) if tokio::time::Instant::now() >= deadline => {
                    let _ = memory.expire_guard_approval(&binding);
                    let _ = memory.deny_guard_action(&binding);
                    return crate::guard_protocol::GateDecision::Deny {
                        reason: "approval expired".to_owned(),
                    };
                }
                Ok(None) => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                Err(_) => {
                    return crate::guard_protocol::GateDecision::Deny {
                        reason: "approval persistence is unavailable".to_owned(),
                    }
                }
            };
        match memory.pending_approval_decision(&approval_id) {
            Ok(Some(true)) => {
                return match memory.release_guard_action(&binding) {
                    Ok(true) => crate::guard_protocol::GateDecision::Allow,
                    Ok(false) | Err(_) => crate::guard_protocol::GateDecision::Deny {
                        reason: "approval was not valid for this Claude action".to_owned(),
                    },
                }
            }
            Ok(Some(false)) => {
                let receipt_ok = memory.deny_guard_action(&binding).unwrap_or(false);
                return crate::guard_protocol::GateDecision::Deny {
                    reason: if receipt_ok {
                        "denied by local approval".to_owned()
                    } else {
                        "approval receipt persistence is unavailable".to_owned()
                    },
                };
            }
            Ok(None) if tokio::time::Instant::now() >= deadline => {
                let _ = memory.expire_guard_approval(&binding);
                let _ = memory.deny_guard_action(&binding);
                return crate::guard_protocol::GateDecision::Deny {
                    reason: "approval expired".to_owned(),
                };
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(250)).await,
            Err(_) => {
                return crate::guard_protocol::GateDecision::Deny {
                    reason: "approval persistence is unavailable".to_owned(),
                }
            }
        }
    }
}

/// Preserved only for the uncertified OpenAI compatibility adapter. Claude uses the bound
/// receipt path above; Codex receives no WP3 support claim until its provider-backed gate.
async fn wait_for_stream_approval_legacy(
    memory: &MemoryEngine,
    action: &crate::guard_protocol::ActionCandidate,
) -> crate::guard_protocol::GateDecision {
    let task_id = Uuid::new_v4();
    if memory
        .create_task(task_id, None, "Kerna protocol approval")
        .is_err()
    {
        return crate::guard_protocol::GateDecision::Deny {
            reason: "approval persistence is unavailable".to_owned(),
        };
    }
    let approval_id = match memory.create_pending_approval(
        task_id,
        &action.raw_tool_name,
        &approval_summary(action),
    ) {
        Ok(id) => id,
        Err(_) => {
            return crate::guard_protocol::GateDecision::Deny {
                reason: "approval persistence is unavailable".to_owned(),
            }
        }
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        match memory.pending_approval_decision(&approval_id) {
            Ok(Some(true)) => return crate::guard_protocol::GateDecision::Allow,
            Ok(Some(false)) => {
                return crate::guard_protocol::GateDecision::Deny {
                    reason: "denied by local approval".to_owned(),
                }
            }
            Ok(None) if tokio::time::Instant::now() >= deadline => {
                let _ = memory.expire_pending_approval(&approval_id);
                return crate::guard_protocol::GateDecision::Deny {
                    reason: "approval expired".to_owned(),
                };
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(250)).await,
            Err(_) => {
                return crate::guard_protocol::GateDecision::Deny {
                    reason: "approval persistence is unavailable".to_owned(),
                }
            }
        }
    }
}

fn approval_summary(action: &crate::guard_protocol::ActionCandidate) -> String {
    let encoded = serde_json::to_vec(&action.arguments).unwrap_or_default();
    let digest = Sha256::digest(encoded);
    json!({
        "protocol": format!("{:?}", action.protocol),
        "tool": action.raw_tool_name,
        "arguments_sha256": format!("{digest:x}"),
    })
    .to_string()
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
        signing_key: Arc::new(new_dashboard_signing_key()),
    };
    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/api/v1/dashboard/overview", get(dashboard_overview))
        .route("/api/v1/dashboard/sessions", get(dashboard_sessions))
        .route(
            "/api/v1/dashboard/sessions/:id/stop",
            post(stop_dashboard_session),
        )
        .route("/api/v1/dashboard/workspace", get(dashboard_workspace))
        .route(
            "/api/v1/dashboard/workspace/apply",
            post(apply_dashboard_workspace),
        )
        .route("/api/v1/dashboard/evidence", get(dashboard_evidence))
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

async fn stop_dashboard_session(
    State(state): State<DashboardState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    if !dashboard_mutation_is_authorized(&state, &headers) {
        return error_response(StatusCode::FORBIDDEN, "Dashboard CSRF validation failed.");
    }
    match state.app.memory.stop_gateway_session(&id) {
        Ok(true) => Json(json!({"ok": true, "status": "stopped"})).into_response(),
        Ok(false) => error_response(StatusCode::CONFLICT, "Session is no longer running."),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn dashboard_workspace(State(state): State<DashboardState>) -> axum::response::Response {
    match workspace_review(&state.app.worktree_baseline) {
        Ok(review) => Json(review).into_response(),
        Err(error) => error_response(StatusCode::SERVICE_UNAVAILABLE, error),
    }
}

async fn apply_dashboard_workspace(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    Json(request): Json<ApplyWorkspaceRequest>,
) -> axum::response::Response {
    if !dashboard_mutation_is_authorized(&state, &headers) {
        return error_response(StatusCode::FORBIDDEN, "Dashboard CSRF validation failed.");
    }
    if !request.confirm {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Explicit apply confirmation is required.",
        );
    }
    match apply_workspace_patch(&request.target, &request.selection) {
        Ok(applied) => Json(json!({"ok": true, "applied": applied})).into_response(),
        Err(error) => error_response(StatusCode::CONFLICT, error),
    }
}

async fn dashboard_evidence(State(state): State<DashboardState>) -> axum::response::Response {
    let payload = dashboard_snapshot(&state);
    let payload_bytes = match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let signature = state.signing_key.sign(&payload_bytes);
    let public_key = base64::engine::general_purpose::STANDARD
        .encode(state.signing_key.verifying_key().to_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    Json(json!({
        "algorithm": "Ed25519",
        "public_key": public_key,
        "signature": signature,
        "payload": payload,
    }))
    .into_response()
}

fn new_dashboard_signing_key() -> SigningKey {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut seed = [0_u8; 32];
    seed[..16].copy_from_slice(first.as_bytes());
    seed[16..].copy_from_slice(second.as_bytes());
    SigningKey::from_bytes(&seed)
}

fn apply_workspace_patch(
    target: &str,
    selection: &ApplyWorkspaceSelection,
) -> Result<Value, String> {
    let source = std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| format!("source workspace is unavailable: {error}"))?;
    let target = std::path::PathBuf::from(target)
        .canonicalize()
        .map_err(|error| format!("target workspace is unavailable: {error}"))?;
    if !target.is_dir() {
        return Err("target workspace is not a directory".to_owned());
    }
    let source_root = git_text_output(&source, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned();
    let target_root = git_text_output(&target, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned();
    if source_root == target_root {
        return Err("refusing to apply into the source worktree".to_owned());
    }
    let target_status = git_text_output(
        &target,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    if !target_status.trim().is_empty() {
        return Err("target worktree must be clean before apply".to_owned());
    }

    let patch = match selection {
        ApplyWorkspaceSelection::Uncommitted => {
            git_bytes_output(&source, &["diff", "--binary", "HEAD", "--", "."])?
        }
        ApplyWorkspaceSelection::Commits { hashes } => {
            if hashes.is_empty() || hashes.len() > 20 {
                return Err("select between one and twenty commits".to_owned());
            }
            let mut combined = Vec::new();
            for hash in hashes {
                if !(7..=64).contains(&hash.len()) || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("commit selection contains an invalid hash".to_owned());
                }
                git_text_output(&source, &["cat-file", "-e", &format!("{hash}^{{commit}}")])?;
                let commit_patch = git_bytes_output(
                    &source,
                    &["diff", "--binary", &format!("{hash}^"), hash, "--", "."],
                )?;
                combined.extend_from_slice(&commit_patch);
            }
            combined
        }
    };
    if patch.is_empty() {
        return Err("there are no selected changes to apply".to_owned());
    }

    let mut child = Command::new("git")
        .args(["apply", "--whitespace=nowarn", "-"])
        .current_dir(&target)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start git apply: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "could not open git apply input".to_owned())?
        .write_all(&patch)
        .map_err(|error| format!("could not send patch to git apply: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("git apply did not finish: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git apply rejected the patch: {}",
            detail.trim().chars().take(500).collect::<String>()
        ));
    }
    Ok(json!({
        "target": target_root,
        "source": source_root,
        "selection": match selection {
            ApplyWorkspaceSelection::Uncommitted => "uncommitted",
            ApplyWorkspaceSelection::Commits { .. } => "commits",
        },
    }))
}

fn workspace_review(worktree_baseline: &str) -> Result<Value, String> {
    let workspace = std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| format!("workspace is unavailable: {error}"))?;
    let repo_root = git_text_output(&workspace, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned();
    let head = git_text_output(&workspace, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let status = git_text_output(
        &workspace,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    let unstaged_diff = git_text_output(
        &workspace,
        &["diff", "--no-ext-diff", "--binary", "--", "."],
    )?;
    let staged_diff = git_text_output(
        &workspace,
        &["diff", "--cached", "--no-ext-diff", "--binary", "--", "."],
    )?;
    let log = git_text_output(
        &workspace,
        &["log", "-20", "--format=%H%x09%h%x09%aI%x09%s"],
    )?;
    let commits = log
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            Some(json!({
                "hash": fields.next()?,
                "short": fields.next()?,
                "authored_at": fields.next()?,
                "subject": fields.next()?,
            }))
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "degraded": false,
        "workspace": workspace.to_string_lossy(),
        "repo_root": repo_root,
        "head": head,
        "baseline": worktree_baseline,
        "status": status,
        "unstaged_diff": unstaged_diff,
        "staged_diff": staged_diff,
        "commits": commits,
    }))
}

fn git_text_output(workspace: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|error| format!("git is unavailable: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed with status {}",
            args.join(" "),
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_bytes_output(workspace: &std::path::Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .map_err(|error| format!("git is unavailable: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed with status {}",
            args.join(" "),
            output.status
        ));
    }
    Ok(output.stdout)
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
        // Kept as a Value, not its rendering. Clippy's suggestion for the old
        // string compare was to drop `.to_string()` and compare a Value against
        // a String -- which is never equal, so the stream would have pushed a
        // snapshot every second forever.
        let mut previous: Option<Value> = None;
        loop {
            let snapshot = dashboard_snapshot(&state);
            let mut comparison = snapshot.clone();
            if let Some(object) = comparison.as_object_mut() {
                // The timestamp changes every tick; comparing with it in would
                // make every snapshot look new.
                object.remove("generated_at");
            }
            let encoded = snapshot.to_string();
            if previous.as_ref() != Some(&comparison) {
                previous = Some(comparison);
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
    let decision = match state.app.memory.decide_guard_approval(&id, approved) {
        Ok(true) => Ok(true),
        Ok(false) => state.app.memory.decide_pending_approval(&id, approved),
        Err(error) => Err(error),
    };
    match decision {
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
            "p50_duration_ms": percentile(0.5), "p95_duration_ms": percentile(0.95),
            "degraded": state.app.memory.health_check().is_err()
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
                guard_policy: Arc::new(GuardPolicy::balanced()),
                memory,
                mcp_registry: Arc::new(Mutex::new(McpRegistry::new())),
                worktree_baseline: "sha256:test-baseline".to_owned(),
                auth_token: None,
            },
            csrf_token: "csrf".to_string(),
            origin: "http://127.0.0.1:8765".to_string(),
            signing_key: Arc::new(new_dashboard_signing_key()),
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
                guard_policy: Arc::new(GuardPolicy::balanced()),
                memory: Arc::new(MemoryEngine::new(":memory:").unwrap()),
                mcp_registry: Arc::new(Mutex::new(McpRegistry::new())),
                worktree_baseline: "sha256:test-baseline".to_owned(),
                auth_token: None,
            },
            csrf_token: "one-time-token".to_string(),
            origin: "http://127.0.0.1:8765".to_string(),
            signing_key: Arc::new(new_dashboard_signing_key()),
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

    #[test]
    fn dashboard_evidence_signing_key_verifies_its_payload() {
        use ed25519_dalek::Verifier;

        let key = new_dashboard_signing_key();
        let payload = br#"{"evidence":"redacted"}"#;
        let signature = key.sign(payload);
        assert!(key.verifying_key().verify(payload, &signature).is_ok());
    }

    #[test]
    fn provider_urls_append_exactly_one_path_separator() {
        assert_eq!(
            provider_url("https://api.anthropic.com", "v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            provider_url("http://127.0.0.1:8081/v1/", "responses"),
            "http://127.0.0.1:8081/v1/responses"
        );
    }

    #[test]
    fn worktree_baseline_digest_is_nonempty_and_state_bound() {
        let root = std::path::Path::new("C:/workspace");
        let baseline = worktree_baseline_digest(root, "C:/workspace", "abc123", "", "", "");
        let changed =
            worktree_baseline_digest(root, "C:/workspace", "abc123", "", "", " M src/main.rs");
        assert!(baseline.starts_with("sha256:"));
        assert_eq!(baseline.len(), "sha256:".len() + 64);
        assert_ne!(baseline, "sha256:unbound");
        assert_ne!(baseline, changed);
    }

    #[test]
    fn stream_policy_is_fail_closed_and_releases_only_an_explicit_exact_allow() {
        let action = crate::guard_protocol::ActionCandidate {
            protocol: crate::guard_protocol::Protocol::AnthropicMessages,
            id: "toolu_test".to_owned(),
            raw_tool_name: "Bash".to_owned(),
            arguments: serde_json::json!({"command": "echo KERNA_ALLOW_TEST"}),
        };
        let mut policy = GuardPolicy::from_legacy_permissions(&[], PolicyEffect::Deny);
        assert!(matches!(
            stream_policy_decision(&policy, &action),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));

        let mut config = Config::default();
        config.permissions.push(crate::config::PermissionRule {
            tool: "Bash".to_owned(),
            action: "auto_approve".to_owned(),
        });
        policy = GuardPolicy::from_legacy_permissions(&config.permissions, PolicyEffect::Deny);
        assert_eq!(
            stream_policy_decision(&policy, &action),
            crate::guard_protocol::GateDecision::Allow
        );
        config.permissions[0].action = "require_confirmation".to_owned();
        policy = GuardPolicy::from_legacy_permissions(&config.permissions, PolicyEffect::Deny);
        assert_eq!(
            stream_policy_decision(&policy, &action),
            crate::guard_protocol::GateDecision::Hold
        );
    }

    #[test]
    fn wp0_smoke_allow_releases_only_the_inert_exact_command() {
        let policy = GuardPolicy::from_legacy_permissions(&[], PolicyEffect::Deny);
        let mut action = crate::guard_protocol::ActionCandidate {
            protocol: crate::guard_protocol::Protocol::AnthropicMessages,
            id: "toolu_test".to_owned(),
            raw_tool_name: "Bash".to_owned(),
            arguments: serde_json::json!({"command": "echo KERNA_ALLOW_TEST"}),
        };
        assert_eq!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Allow
        );
        action.arguments = serde_json::json!({"command": "echo KERNA_DENY_TEST"});
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));
        action.raw_tool_name = "Write".to_owned();
        action.arguments = serde_json::json!({"command": "echo KERNA_ALLOW_TEST"});
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));

        action.protocol = crate::guard_protocol::Protocol::OpenAiResponses;
        action.raw_tool_name = "exec_command".to_owned();
        action.arguments = serde_json::json!({"cmd": "echo KERNA_ALLOW_TEST"});
        assert_eq!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Allow
        );
        action.arguments = serde_json::json!({"cmd": "echo KERNA_DENY_TEST"});
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));
        action.raw_tool_name = "apply_patch".to_owned();
        action.arguments = serde_json::Value::String("echo KERNA_ALLOW_TEST".to_owned());
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, true, false),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));
    }

    #[test]
    fn wp0_smoke_hold_matches_only_the_inert_approval_command() {
        let policy = GuardPolicy::from_legacy_permissions(&[], PolicyEffect::Deny);
        let mut action = crate::guard_protocol::ActionCandidate {
            protocol: crate::guard_protocol::Protocol::AnthropicMessages,
            id: "toolu_test".to_owned(),
            raw_tool_name: "Bash".to_owned(),
            arguments: serde_json::json!({"command": "echo KERNA_ASK_TEST"}),
        };
        assert_eq!(
            stream_policy_decision_with_smoke(&policy, &action, false, true),
            crate::guard_protocol::GateDecision::Hold
        );
        action.arguments = serde_json::json!({"command": "echo KERNA_ALLOW_TEST"});
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, false, true),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));

        action.protocol = crate::guard_protocol::Protocol::OpenAiResponses;
        action.raw_tool_name = "exec_command".to_owned();
        action.arguments = serde_json::json!({"cmd": "echo KERNA_ASK_TEST"});
        assert_eq!(
            stream_policy_decision_with_smoke(&policy, &action, false, true),
            crate::guard_protocol::GateDecision::Hold
        );
        action.arguments = serde_json::json!({"cmd": "echo KERNA_ALLOW_TEST"});
        assert!(matches!(
            stream_policy_decision_with_smoke(&policy, &action, false, true),
            crate::guard_protocol::GateDecision::Deny { .. }
        ));
    }

    #[test]
    fn approval_summary_redacts_arguments_but_is_deterministic() {
        let action = crate::guard_protocol::ActionCandidate {
            protocol: crate::guard_protocol::Protocol::AnthropicMessages,
            id: "toolu_test".to_owned(),
            raw_tool_name: "Bash".to_owned(),
            arguments: serde_json::json!({"command": "echo definitely-not-persisted"}),
        };
        let summary = approval_summary(&action);
        assert!(!summary.contains("definitely-not-persisted"));
        assert_eq!(summary, approval_summary(&action));
        assert!(summary.contains("arguments_sha256"));
    }
}
