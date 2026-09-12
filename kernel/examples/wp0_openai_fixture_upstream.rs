use axum::{
    body::Bytes,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone)]
struct State {
    tool: String,
    command: String,
    requests: Arc<AtomicUsize>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("KERNA_WP0_FIXTURE_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(18081);
    let command = std::env::var("KERNA_WP0_FIXTURE_COMMAND")
        .unwrap_or_else(|_| "echo KERNA_DENY_TEST".to_owned());
    let state = State {
        tool: std::env::var("KERNA_WP0_FIXTURE_TOOL").unwrap_or_else(|_| "exec_command".to_owned()),
        command,
        requests: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/responses", post(responses))
        .with_state(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("WP0 fixture upstream listening on http://{addr}/responses");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn responses(
    axum::extract::State(state): axum::extract::State<State>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let request_number = state.requests.fetch_add(1, Ordering::SeqCst) + 1;
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid JSON").into_response(),
    };
    let input_types = payload
        .get("input")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tool_contracts = payload
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .map(|tool| {
                    let kind = tool.get("type").and_then(Value::as_str).unwrap_or("?");
                    let name = tool.get("name").and_then(Value::as_str).unwrap_or("-");
                    let properties = tool
                        .pointer("/parameters/properties")
                        .and_then(Value::as_object)
                        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    format!("{kind}:{name}:{properties:?}")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let broker_auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some("Bearer wp0-broker-key");
    eprintln!(
        "request={request_number} broker_auth={broker_auth} input_types={input_types:?} tools={tool_contracts:?}"
    );

    let has_tool_output = input_types
        .iter()
        .any(|kind| matches!(*kind, "custom_tool_call_output" | "function_call_output"));
    let stream = if has_tool_output || state.tool == "text" {
        final_text_stream()
    } else if state.tool == "apply_patch" {
        apply_patch_call_stream()
    } else {
        exec_command_call_stream(&state.command)
    };
    let mut response = stream.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
}

fn apply_patch_call_stream() -> String {
    custom_call_stream(
        "apply_patch",
        "*** Begin Patch\n*** Add File: KERNA_WP0_PATCH_TEST.txt\n+KERNA_PATCH_TEST\n*** End Patch",
    )
}

fn custom_call_stream(name: &str, input: &str) -> String {
    let item = json!({
        "id": "ctc_wp0_action",
        "type": "custom_tool_call",
        "call_id": "call_wp0_action",
        "name": name,
        "input": input,
        "status": "completed"
    });
    let mut stream = String::new();
    event(
        &mut stream,
        "response.created",
        json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": response("resp_wp0_action", "in_progress", Vec::new())
        }),
    );
    event(
        &mut stream,
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "id": "ctc_wp0_action",
                "type": "custom_tool_call",
                "call_id": "call_wp0_action",
                "name": name,
                "input": "",
                "status": "in_progress"
            }
        }),
    );
    event(
        &mut stream,
        "response.custom_tool_call_input.delta",
        json!({
            "type": "response.custom_tool_call_input.delta",
            "sequence_number": 2,
            "output_index": 0,
            "item_id": "ctc_wp0_action",
            "delta": input
        }),
    );
    event(
        &mut stream,
        "response.custom_tool_call_input.done",
        json!({
            "type": "response.custom_tool_call_input.done",
            "sequence_number": 3,
            "output_index": 0,
            "item_id": "ctc_wp0_action",
            "input": input
        }),
    );
    event(
        &mut stream,
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "sequence_number": 4,
            "output_index": 0,
            "item": item.clone()
        }),
    );
    event(
        &mut stream,
        "response.completed",
        json!({
            "type": "response.completed",
            "sequence_number": 5,
            "response": response("resp_wp0_action", "completed", vec![item])
        }),
    );
    stream
}

fn exec_command_call_stream(command: &str) -> String {
    let arguments = json!({"cmd": command}).to_string();
    let item = json!({
        "id": "fc_wp0_exec_command",
        "type": "function_call",
        "call_id": "call_wp0_exec_command",
        "name": "exec_command",
        "arguments": arguments,
        "status": "completed"
    });
    let mut stream = String::new();
    event(
        &mut stream,
        "response.created",
        json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": response("resp_wp0_exec_command", "in_progress", Vec::new())
        }),
    );
    event(
        &mut stream,
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "id": "fc_wp0_exec_command",
                "type": "function_call",
                "call_id": "call_wp0_exec_command",
                "name": "exec_command",
                "arguments": "",
                "status": "in_progress"
            }
        }),
    );
    event(
        &mut stream,
        "response.function_call_arguments.delta",
        json!({
            "type": "response.function_call_arguments.delta",
            "sequence_number": 2,
            "output_index": 0,
            "item_id": "fc_wp0_exec_command",
            "delta": arguments
        }),
    );
    event(
        &mut stream,
        "response.function_call_arguments.done",
        json!({
            "type": "response.function_call_arguments.done",
            "sequence_number": 3,
            "output_index": 0,
            "item_id": "fc_wp0_exec_command",
            "arguments": arguments
        }),
    );
    event(
        &mut stream,
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "sequence_number": 4,
            "output_index": 0,
            "item": item.clone()
        }),
    );
    event(
        &mut stream,
        "response.completed",
        json!({
            "type": "response.completed",
            "sequence_number": 5,
            "response": response("resp_wp0_exec_command", "completed", vec![item])
        }),
    );
    stream
}

fn final_text_stream() -> String {
    let item = json!({
        "id": "msg_wp0_done",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": "KERNA_CODEX_TOOL_RESULT_OBSERVED",
            "annotations": []
        }]
    });
    let mut stream = String::new();
    event(
        &mut stream,
        "response.created",
        json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": response("resp_wp0_done", "in_progress", Vec::new())
        }),
    );
    event(
        &mut stream,
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "id": "msg_wp0_done",
                "type": "message",
                "status": "in_progress",
                "role": "assistant",
                "content": []
            }
        }),
    );
    event(
        &mut stream,
        "response.output_text.delta",
        json!({
            "type": "response.output_text.delta",
            "sequence_number": 2,
            "output_index": 0,
            "item_id": "msg_wp0_done",
            "content_index": 0,
            "delta": "KERNA_CODEX_TOOL_RESULT_OBSERVED"
        }),
    );
    event(
        &mut stream,
        "response.output_text.done",
        json!({
            "type": "response.output_text.done",
            "sequence_number": 3,
            "output_index": 0,
            "item_id": "msg_wp0_done",
            "content_index": 0,
            "text": "KERNA_CODEX_TOOL_RESULT_OBSERVED"
        }),
    );
    event(
        &mut stream,
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "sequence_number": 4,
            "output_index": 0,
            "item": item.clone()
        }),
    );
    event(
        &mut stream,
        "response.completed",
        json!({
            "type": "response.completed",
            "sequence_number": 5,
            "response": response("resp_wp0_done", "completed", vec![item])
        }),
    );
    stream
}

fn response(id: &str, status: &str, output: Vec<Value>) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": 0,
        "status": status,
        "error": null,
        "incomplete_details": null,
        "instructions": null,
        "model": "gpt-5.4",
        "output": output,
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "tools": [],
        "usage": {
            "input_tokens": 1,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": 1,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": 2
        }
    })
}

fn event(stream: &mut String, name: &str, data: Value) {
    stream.push_str("event: ");
    stream.push_str(name);
    stream.push('\n');
    stream.push_str("data: ");
    stream.push_str(&data.to_string());
    stream.push_str("\n\n");
}
