//! Streaming model-protocol parsing for Kerna Guard.
//!
//! This module deliberately makes no policy decision. It preserves every upstream SSE
//! byte while also emitting complete action candidates for the policy/approval layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    AnthropicMessages,
    OpenAiResponses,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionCandidate {
    pub protocol: Protocol,
    pub id: String,
    pub raw_tool_name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamItem {
    Bytes(Vec<u8>),
    Action(ActionCandidate),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidUtf8,
    InvalidJson,
    MalformedAction(&'static str),
    UnknownActionType(String),
    TruncatedFrame,
    TruncatedAction,
}

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
}

struct Frame {
    raw: Vec<u8>,
    event: Option<String>,
    data: Option<Value>,
}

impl SseDecoder {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, ProtocolError> {
        self.pending.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(end) = frame_end(&self.pending) {
            let raw = self.pending.drain(..end).collect::<Vec<_>>();
            frames.push(parse_frame(raw)?);
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        if self.pending.is_empty() {
            return Ok(None);
        }
        if self.pending.iter().all(u8::is_ascii_whitespace)
            || std::str::from_utf8(&self.pending)
                .map(|tail| {
                    tail.lines()
                        .all(|line| line.is_empty() || line.starts_with(':'))
                })
                .unwrap_or(false)
        {
            return Ok(Some(std::mem::take(&mut self.pending)));
        }
        Err(ProtocolError::TruncatedFrame)
    }
}

fn frame_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| index + 2)
        .or_else(|| {
            bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4)
        })
}

fn parse_frame(raw: Vec<u8>) -> Result<Frame, ProtocolError> {
    let text = std::str::from_utf8(&raw).map_err(|_| ProtocolError::InvalidUtf8)?;
    let mut event = None;
    let mut data_lines = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim_start().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        }
    }
    let data = if data_lines.is_empty() || data_lines == ["[DONE]"] {
        None
    } else {
        Some(serde_json::from_str(&data_lines.join("\n")).map_err(|_| ProtocolError::InvalidJson)?)
    };
    Ok(Frame { raw, event, data })
}

#[derive(Clone, Default)]
struct PendingAction {
    id: String,
    name: String,
    input: String,
    custom: bool,
}

pub struct ActionStreamParser {
    protocol: Protocol,
    decoder: SseDecoder,
    pending: HashMap<u64, PendingAction>,
}

impl ActionStreamParser {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            decoder: SseDecoder::default(),
            pending: HashMap::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<StreamItem>, ProtocolError> {
        let mut output = Vec::new();
        for frame in self.decoder.feed(bytes)? {
            let actions = match self.protocol {
                Protocol::AnthropicMessages => self.anthropic_actions(&frame)?,
                Protocol::OpenAiResponses => self.openai_actions(&frame)?,
            };
            output.push(StreamItem::Bytes(frame.raw));
            output.extend(actions.into_iter().map(StreamItem::Action));
        }
        Ok(output)
    }

    pub fn finish(&mut self) -> Result<Vec<StreamItem>, ProtocolError> {
        let mut output = Vec::new();
        if let Some(tail) = self.decoder.finish()? {
            output.push(StreamItem::Bytes(tail));
        }
        if self.pending.is_empty() {
            Ok(output)
        } else {
            Err(ProtocolError::TruncatedAction)
        }
    }

    fn anthropic_actions(&mut self, frame: &Frame) -> Result<Vec<ActionCandidate>, ProtocolError> {
        let Some(data) = &frame.data else {
            return Ok(Vec::new());
        };
        match frame.event.as_deref() {
            Some("content_block_start") => {
                let block = &data["content_block"];
                match block["type"].as_str() {
                    Some("tool_use") => {
                        let index = required_u64(data, "index")?;
                        let id = required_str(block, "id")?;
                        let name = required_str(block, "name")?;
                        self.pending.insert(
                            index,
                            PendingAction {
                                id: id.to_owned(),
                                name: name.to_owned(),
                                input: String::new(),
                                custom: false,
                            },
                        );
                    }
                    Some("server_tool_use") => {
                        return Err(ProtocolError::UnknownActionType(
                            "server_tool_use".to_owned(),
                        ));
                    }
                    _ => {}
                }
            }
            Some("content_block_delta") => {
                let index = required_u64(data, "index")?;
                if let Some(action) = self.pending.get_mut(&index) {
                    if data["delta"]["type"] != "input_json_delta" {
                        return Err(ProtocolError::MalformedAction(
                            "unexpected Anthropic tool delta",
                        ));
                    }
                    action
                        .input
                        .push_str(required_str(&data["delta"], "partial_json")?);
                }
            }
            Some("content_block_stop") => {
                let index = required_u64(data, "index")?;
                if let Some(action) = self.pending.remove(&index) {
                    return Ok(vec![complete_action(self.protocol, action)?]);
                }
            }
            _ => {}
        }
        Ok(Vec::new())
    }

    fn openai_actions(&mut self, frame: &Frame) -> Result<Vec<ActionCandidate>, ProtocolError> {
        let Some(data) = &frame.data else {
            return Ok(Vec::new());
        };
        match frame.event.as_deref() {
            Some("response.output_item.added") => {
                let item = &data["item"];
                let item_type = item["type"].as_str().unwrap_or_default();
                let custom = match item_type {
                    "function_call" => false,
                    "custom_tool_call" => true,
                    "message" | "reasoning" => return Ok(Vec::new()),
                    other
                        if other.contains("call")
                            || other.contains("tool")
                            || other.contains("action") =>
                    {
                        return Err(ProtocolError::UnknownActionType(other.to_owned()));
                    }
                    _ => return Ok(Vec::new()),
                };
                let index = required_u64(data, "output_index")?;
                self.pending.insert(
                    index,
                    PendingAction {
                        id: item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .ok_or(ProtocolError::MalformedAction("missing OpenAI call id"))?
                            .to_owned(),
                        name: required_str(item, "name")?.to_owned(),
                        input: item
                            .get(if custom { "input" } else { "arguments" })
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        custom,
                    },
                );
            }
            Some("response.function_call_arguments.delta")
            | Some("response.custom_tool_call_input.delta") => {
                let index = required_u64(data, "output_index")?;
                let action = self
                    .pending
                    .get_mut(&index)
                    .ok_or(ProtocolError::MalformedAction(
                        "OpenAI delta without action",
                    ))?;
                action.input.push_str(required_str(data, "delta")?);
            }
            Some("response.function_call_arguments.done")
            | Some("response.custom_tool_call_input.done") => {
                let index = required_u64(data, "output_index")?;
                let mut action =
                    self.pending
                        .remove(&index)
                        .ok_or(ProtocolError::MalformedAction(
                            "OpenAI completion without action",
                        ))?;
                if let Some(input) = data
                    .get(if action.custom { "input" } else { "arguments" })
                    .and_then(Value::as_str)
                {
                    action.input = input.to_owned();
                }
                return Ok(vec![complete_action(self.protocol, action)?]);
            }
            _ => {}
        }
        Ok(Vec::new())
    }
}

fn required_str<'a>(value: &'a Value, key: &'static str) -> Result<&'a str, ProtocolError> {
    value[key]
        .as_str()
        .ok_or(ProtocolError::MalformedAction(key))
}

fn required_u64(value: &Value, key: &'static str) -> Result<u64, ProtocolError> {
    value[key]
        .as_u64()
        .ok_or(ProtocolError::MalformedAction(key))
}

fn complete_action(
    protocol: Protocol,
    action: PendingAction,
) -> Result<ActionCandidate, ProtocolError> {
    let arguments = if action.custom {
        Value::String(action.input)
    } else {
        serde_json::from_str(&action.input).map_err(|_| ProtocolError::InvalidJson)?
    };
    Ok(ActionCandidate {
        protocol,
        id: action.id,
        raw_tool_name: action.name,
        arguments,
    })
}

/// The decision supplied by the broker after an action's complete arguments are known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Deny { reason: String },
    Hold,
}

struct HeldAnthropicAction {
    action: PendingAction,
    raw: Vec<Vec<u8>>,
}

/// A byte-preserving Anthropic Messages stream gate.
///
/// Ordinary frames leave immediately. `tool_use` frames are held only until their final
/// `content_block_stop`, when the caller can make a decision with complete arguments.
/// A denial is a valid text replacement and changes `stop_reason: tool_use` to `end_turn`.
pub struct AnthropicStreamGate<D> {
    decoder: SseDecoder,
    pending: HashMap<u64, HeldAnthropicAction>,
    approval: Option<(u64, HeldAnthropicAction, ActionCandidate)>,
    deferred: Vec<Frame>,
    decide: D,
    denied_this_turn: bool,
    released_this_turn: bool,
}

impl<D> AnthropicStreamGate<D>
where
    D: FnMut(&ActionCandidate) -> GateDecision,
{
    pub fn new(decide: D) -> Self {
        Self {
            decoder: SseDecoder::default(),
            pending: HashMap::new(),
            approval: None,
            deferred: Vec::new(),
            decide,
            denied_this_turn: false,
            released_this_turn: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let frames = self.decoder.feed(bytes)?;
        if self.approval.is_some() {
            self.deferred.extend(frames);
            return Ok(Vec::new());
        }
        self.process_frames(frames)
    }

    pub fn pending_approval(&self) -> Option<&ActionCandidate> {
        self.approval.as_ref().map(|(_, _, action)| action)
    }

    pub fn resolve_pending(
        &mut self,
        decision: GateDecision,
    ) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let (index, held, candidate) = self
            .approval
            .take()
            .ok_or(ProtocolError::MalformedAction("no approval is pending"))?;
        let mut output = match decision {
            GateDecision::Allow => {
                self.released_this_turn = true;
                held.raw
            }
            GateDecision::Deny { reason } => {
                self.denied_this_turn = true;
                anthropic_denial(index, &candidate.raw_tool_name, &reason)
            }
            GateDecision::Hold => {
                self.approval = Some((index, held, candidate));
                return Err(ProtocolError::MalformedAction(
                    "approval must resolve to allow or deny",
                ));
            }
        };
        let deferred = std::mem::take(&mut self.deferred);
        output.extend(self.process_frames(deferred)?);
        Ok(output)
    }

    fn process_frames(&mut self, frames: Vec<Frame>) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let mut output = Vec::new();
        let mut frames = frames.into_iter();
        while let Some(frame) = frames.next() {
            let Some(data) = &frame.data else {
                output.push(frame.raw);
                continue;
            };
            match frame.event.as_deref() {
                Some("content_block_start") if data["content_block"]["type"] == "tool_use" => {
                    let index = required_u64(data, "index")?;
                    let block = &data["content_block"];
                    self.pending.insert(
                        index,
                        HeldAnthropicAction {
                            action: PendingAction {
                                id: required_str(block, "id")?.to_owned(),
                                name: required_str(block, "name")?.to_owned(),
                                input: String::new(),
                                custom: false,
                            },
                            raw: vec![frame.raw],
                        },
                    );
                }
                Some("content_block_start")
                    if data["content_block"]["type"] == "server_tool_use" =>
                {
                    return Err(ProtocolError::UnknownActionType(
                        "server_tool_use".to_owned(),
                    ));
                }
                Some("content_block_delta") => {
                    let index = required_u64(data, "index")?;
                    if let Some(held) = self.pending.get_mut(&index) {
                        if data["delta"]["type"] != "input_json_delta" {
                            return Err(ProtocolError::MalformedAction(
                                "unexpected Anthropic tool delta",
                            ));
                        }
                        held.action
                            .input
                            .push_str(required_str(&data["delta"], "partial_json")?);
                        held.raw.push(frame.raw);
                    } else {
                        output.push(frame.raw);
                    }
                }
                Some("content_block_stop") => {
                    let index = required_u64(data, "index")?;
                    if let Some(mut held) = self.pending.remove(&index) {
                        held.raw.push(frame.raw);
                        let candidate =
                            complete_action(Protocol::AnthropicMessages, held.action.clone())?;
                        match (self.decide)(&candidate) {
                            GateDecision::Allow => {
                                self.released_this_turn = true;
                                output.extend(held.raw)
                            }
                            GateDecision::Deny { reason } => {
                                self.denied_this_turn = true;
                                output.extend(anthropic_denial(
                                    index,
                                    &candidate.raw_tool_name,
                                    &reason,
                                ));
                            }
                            GateDecision::Hold => {
                                self.approval = Some((index, held, candidate));
                                self.deferred.extend(frames);
                                break;
                            }
                        }
                    } else {
                        output.push(frame.raw);
                    }
                }
                Some("message_delta")
                    if self.denied_this_turn
                        && !self.released_this_turn
                        && data["delta"]["stop_reason"] == "tool_use" =>
                {
                    let mut rewritten = data.clone();
                    rewritten["delta"]["stop_reason"] = Value::String("end_turn".to_owned());
                    output.push(sse("message_delta", &rewritten));
                }
                Some("message_stop") => {
                    self.denied_this_turn = false;
                    self.released_this_turn = false;
                    output.push(frame.raw);
                }
                _ => output.push(frame.raw),
            }
        }
        Ok(output)
    }

    pub fn finish(&mut self) -> Result<Vec<Vec<u8>>, ProtocolError> {
        if self.approval.is_some() || !self.pending.is_empty() {
            return Err(ProtocolError::TruncatedAction);
        }
        self.decoder
            .finish()?
            .map_or_else(|| Ok(Vec::new()), |tail| Ok(vec![tail]))
    }
}

fn sse(event: &str, data: &Value) -> Vec<u8> {
    format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(data).expect("JSON value")
    )
    .into_bytes()
}

fn anthropic_denial(index: u64, tool: &str, reason: &str) -> Vec<Vec<u8>> {
    let message = format!("[blocked by Kerna policy] {tool} was not released: {reason}");
    vec![
        sse(
            "content_block_start",
            &serde_json::json!({
                "type": "content_block_start", "index": index,
                "content_block": {"type": "text", "text": ""}
            }),
        ),
        sse(
            "content_block_delta",
            &serde_json::json!({
                "type": "content_block_delta", "index": index,
                "delta": {"type": "text_delta", "text": message}
            }),
        ),
        sse(
            "content_block_stop",
            &serde_json::json!({"type": "content_block_stop", "index": index}),
        ),
    ]
}

struct HeldOpenAiAction {
    action: PendingAction,
    raw: Vec<Vec<u8>>,
}

/// A byte-preserving OpenAI Responses stream gate for function and custom tool calls.
///
/// Tool frames remain buffered until their complete arguments arrive. A denied call is
/// replaced with a normal assistant message item, so no executable call reaches Codex.
pub struct OpenAiResponsesStreamGate<D> {
    decoder: SseDecoder,
    pending: HashMap<u64, HeldOpenAiAction>,
    approval: Option<(u64, HeldOpenAiAction, ActionCandidate)>,
    deferred: Vec<Frame>,
    denied_indexes: HashSet<u64>,
    decide: D,
}

impl<D> OpenAiResponsesStreamGate<D>
where
    D: FnMut(&ActionCandidate) -> GateDecision,
{
    pub fn new(decide: D) -> Self {
        Self {
            decoder: SseDecoder::default(),
            pending: HashMap::new(),
            approval: None,
            deferred: Vec::new(),
            denied_indexes: HashSet::new(),
            decide,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let frames = self.decoder.feed(bytes)?;
        if self.approval.is_some() {
            self.deferred.extend(frames);
            return Ok(Vec::new());
        }
        self.process_frames(frames)
    }

    pub fn pending_approval(&self) -> Option<&ActionCandidate> {
        self.approval.as_ref().map(|(_, _, action)| action)
    }

    pub fn resolve_pending(
        &mut self,
        decision: GateDecision,
    ) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let (index, held, candidate) = self
            .approval
            .take()
            .ok_or(ProtocolError::MalformedAction("no approval is pending"))?;
        let mut output = match decision {
            GateDecision::Allow => held.raw,
            GateDecision::Deny { reason } => {
                self.denied_indexes.insert(index);
                openai_denial(index, &candidate.id, &candidate.raw_tool_name, &reason)
            }
            GateDecision::Hold => {
                self.approval = Some((index, held, candidate));
                return Err(ProtocolError::MalformedAction(
                    "approval must resolve to allow or deny",
                ));
            }
        };
        let deferred = std::mem::take(&mut self.deferred);
        output.extend(self.process_frames(deferred)?);
        Ok(output)
    }

    fn process_frames(&mut self, frames: Vec<Frame>) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let mut output = Vec::new();
        let mut frames = frames.into_iter();
        while let Some(frame) = frames.next() {
            let Some(data) = &frame.data else {
                output.push(frame.raw);
                continue;
            };
            match frame.event.as_deref() {
                Some("response.output_item.added") => {
                    let item = &data["item"];
                    let item_type = item["type"].as_str().unwrap_or_default();
                    let custom = match item_type {
                        "function_call" => false,
                        "custom_tool_call" => true,
                        "message" | "reasoning" => {
                            output.push(frame.raw);
                            continue;
                        }
                        other
                            if other.contains("call")
                                || other.contains("tool")
                                || other.contains("action") =>
                        {
                            return Err(ProtocolError::UnknownActionType(other.to_owned()));
                        }
                        _ => {
                            output.push(frame.raw);
                            continue;
                        }
                    };
                    let index = required_u64(data, "output_index")?;
                    self.pending.insert(
                        index,
                        HeldOpenAiAction {
                            action: PendingAction {
                                id: item
                                    .get("call_id")
                                    .or_else(|| item.get("id"))
                                    .and_then(Value::as_str)
                                    .ok_or(ProtocolError::MalformedAction(
                                        "missing OpenAI call id",
                                    ))?
                                    .to_owned(),
                                name: required_str(item, "name")?.to_owned(),
                                input: item
                                    .get(if custom { "input" } else { "arguments" })
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                custom,
                            },
                            raw: vec![frame.raw],
                        },
                    );
                }
                Some("response.function_call_arguments.delta")
                | Some("response.custom_tool_call_input.delta") => {
                    let index = required_u64(data, "output_index")?;
                    let held =
                        self.pending
                            .get_mut(&index)
                            .ok_or(ProtocolError::MalformedAction(
                                "OpenAI delta without action",
                            ))?;
                    held.action.input.push_str(required_str(data, "delta")?);
                    held.raw.push(frame.raw);
                }
                Some("response.function_call_arguments.done")
                | Some("response.custom_tool_call_input.done") => {
                    let index = required_u64(data, "output_index")?;
                    let mut held =
                        self.pending
                            .remove(&index)
                            .ok_or(ProtocolError::MalformedAction(
                                "OpenAI completion without action",
                            ))?;
                    held.raw.push(frame.raw);
                    if let Some(input) = data
                        .get(if held.action.custom {
                            "input"
                        } else {
                            "arguments"
                        })
                        .and_then(Value::as_str)
                    {
                        held.action.input = input.to_owned();
                    }
                    let candidate =
                        complete_action(Protocol::OpenAiResponses, held.action.clone())?;
                    match (self.decide)(&candidate) {
                        GateDecision::Allow => output.extend(held.raw),
                        GateDecision::Deny { reason } => {
                            self.denied_indexes.insert(index);
                            output.extend(openai_denial(
                                index,
                                &candidate.id,
                                &candidate.raw_tool_name,
                                &reason,
                            ));
                        }
                        GateDecision::Hold => {
                            self.approval = Some((index, held, candidate));
                            self.deferred.extend(frames);
                            break;
                        }
                    }
                }
                Some("response.output_item.done") => {
                    let index = required_u64(data, "output_index")?;
                    if !self.denied_indexes.remove(&index) {
                        output.push(frame.raw);
                    }
                }
                _ => output.push(frame.raw),
            }
        }
        Ok(output)
    }

    pub fn finish(&mut self) -> Result<Vec<Vec<u8>>, ProtocolError> {
        if self.approval.is_some() || !self.pending.is_empty() {
            return Err(ProtocolError::TruncatedAction);
        }
        self.decoder
            .finish()?
            .map_or_else(|| Ok(Vec::new()), |tail| Ok(vec![tail]))
    }
}

fn openai_denial(index: u64, call_id: &str, tool: &str, reason: &str) -> Vec<Vec<u8>> {
    let id = format!("kerna_denied_{call_id}");
    let message = format!("[blocked by Kerna policy] {tool} was not released: {reason}");
    let item = serde_json::json!({
        "id": id, "status": "completed", "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": message, "annotations": []}]
    });
    vec![
        sse(
            "response.output_item.added",
            &serde_json::json!({
                "type": "response.output_item.added", "output_index": index,
                "item": {"id": id, "status": "in_progress", "type": "message", "role": "assistant", "content": []}
            }),
        ),
        sse(
            "response.output_text.delta",
            &serde_json::json!({
                "type": "response.output_text.delta", "item_id": id, "output_index": index,
                "content_index": 0, "delta": message
            }),
        ),
        sse(
            "response.output_text.done",
            &serde_json::json!({
                "type": "response.output_text.done", "item_id": id, "output_index": index,
                "content_index": 0, "text": message
            }),
        ),
        sse(
            "response.output_item.done",
            &serde_json::json!({"type": "response.output_item.done", "output_index": index, "item": item}),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANTHROPIC: &[u8] = include_bytes!("../tests/fixtures/anthropic/messages-tool-use.sse");
    const ANTHROPIC_MULTI: &[u8] =
        include_bytes!("../tests/fixtures/anthropic/messages-two-tool-use.sse");
    const OPENAI: &[u8] = include_bytes!("../tests/fixtures/openai/responses-actions.sse");

    fn parse_every_boundary(protocol: Protocol, fixture: &[u8]) -> Vec<ActionCandidate> {
        let mut expected = None;
        for size in 1..=fixture.len() {
            let mut parser = ActionStreamParser::new(protocol);
            let mut bytes = Vec::new();
            let mut actions = Vec::new();
            for chunk in fixture.chunks(size) {
                for item in parser.feed(chunk).unwrap() {
                    match item {
                        StreamItem::Bytes(raw) => bytes.extend(raw),
                        StreamItem::Action(action) => actions.push(action),
                    }
                }
            }
            for item in parser.finish().unwrap() {
                if let StreamItem::Bytes(raw) = item {
                    bytes.extend(raw)
                }
            }
            assert_eq!(bytes, fixture, "bytes changed at chunk size {size}");
            if let Some(expected) = &expected {
                assert_eq!(&actions, expected, "actions changed at chunk size {size}");
            } else {
                expected = Some(actions);
            }
        }
        expected.unwrap()
    }

    #[test]
    fn anthropic_fixture_is_byte_exact_across_every_chunk_size() {
        let actions = parse_every_boundary(Protocol::AnthropicMessages, ANTHROPIC);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].raw_tool_name, "Bash");
        assert_eq!(actions[0].arguments["command"], "cargo test");
    }

    #[test]
    fn anthropic_multi_action_fixture_is_byte_exact_across_every_chunk_size() {
        let actions = parse_every_boundary(Protocol::AnthropicMessages, ANTHROPIC_MULTI);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].arguments["command"], "echo first");
        assert_eq!(actions[1].arguments["command"], "echo second");
    }

    #[test]
    fn openai_fixture_extracts_current_codex_function_and_custom_actions() {
        let actions = parse_every_boundary(Protocol::OpenAiResponses, OPENAI);
        assert_eq!(
            actions
                .iter()
                .map(|a| a.raw_tool_name.as_str())
                .collect::<Vec<_>>(),
            ["exec_command", "write_stdin", "apply_patch"]
        );
        assert_eq!(actions[0].arguments["cmd"], "echo KERNA_ALLOW_TEST");
        assert_eq!(actions[1].arguments["session_id"], 123);
        assert!(actions[2]
            .arguments
            .as_str()
            .unwrap()
            .contains("*** Begin Patch"));
    }

    #[test]
    fn malformed_arguments_fail_closed() {
        let stream = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"x\",\"name\":\"Bash\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{bad\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n";
        let mut parser = ActionStreamParser::new(Protocol::AnthropicMessages);
        assert_eq!(parser.feed(stream), Err(ProtocolError::InvalidJson));
    }

    #[test]
    fn unknown_action_types_fail_closed() {
        let stream = b"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"computer_action\"}}\n\n";
        let mut parser = ActionStreamParser::new(Protocol::OpenAiResponses);
        assert_eq!(
            parser.feed(stream),
            Err(ProtocolError::UnknownActionType(
                "computer_action".to_owned()
            ))
        );
    }

    #[test]
    fn disconnect_mid_action_fails_closed() {
        let stream = b"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"\"}}\n\n";
        let mut parser = ActionStreamParser::new(Protocol::OpenAiResponses);
        parser.feed(stream).unwrap();
        assert_eq!(parser.finish(), Err(ProtocolError::TruncatedAction));
    }

    fn gate_anthropic(
        fixture: &[u8],
        decision: GateDecision,
        chunk_size: usize,
    ) -> Result<Vec<u8>, ProtocolError> {
        let mut gate = AnthropicStreamGate::new(move |_| decision.clone());
        let mut output = Vec::new();
        for chunk in fixture.chunks(chunk_size) {
            output.extend(gate.feed(chunk)?.into_iter().flatten());
        }
        output.extend(gate.finish()?.into_iter().flatten());
        Ok(output)
    }

    #[test]
    fn anthropic_gate_releases_allowed_tool_bytes_unchanged() {
        for size in 1..=ANTHROPIC.len() {
            assert_eq!(
                gate_anthropic(ANTHROPIC, GateDecision::Allow, size).unwrap(),
                ANTHROPIC,
                "bytes changed at chunk size {size}"
            );
        }
    }

    #[test]
    fn anthropic_gate_denial_replaces_tool_and_ends_cleanly() {
        let output = gate_anthropic(
            ANTHROPIC,
            GateDecision::Deny {
                reason: "shell commands require approval".to_owned(),
            },
            1,
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("I will run the project tests."));
        assert!(text.contains("[blocked by Kerna policy] Bash was not released"));
        assert!(!text.contains("\"type\":\"tool_use\""));
        assert!(text.contains("\"stop_reason\":\"end_turn\""));
        assert!(!text.contains("\"stop_reason\":\"tool_use\""));
    }

    #[test]
    fn anthropic_gate_holds_tool_until_arguments_are_complete() {
        let tool_start = ANTHROPIC
            .windows(b"\"type\":\"tool_use\"".len())
            .position(|window| window == b"\"type\":\"tool_use\"")
            .unwrap();
        let tool_stop = tool_start
            + ANTHROPIC[tool_start..]
                .windows(b"event: content_block_stop".len())
                .position(|window| window == b"event: content_block_stop")
                .unwrap();
        let mut gate = AnthropicStreamGate::new(|_| GateDecision::Allow);
        let output = gate.feed(&ANTHROPIC[..tool_stop]).unwrap();
        let joined = output.into_iter().flatten().collect::<Vec<_>>();
        assert!(!joined
            .windows(b"tool_use".len())
            .any(|window| window == b"tool_use"));
        assert_eq!(gate.finish(), Err(ProtocolError::TruncatedAction));
    }

    #[test]
    fn anthropic_gate_fails_closed_on_disconnect_mid_tool() {
        let marker = b"event: content_block_stop";
        let tool_start = ANTHROPIC
            .windows(b"\"type\":\"tool_use\"".len())
            .position(|window| window == b"\"type\":\"tool_use\"")
            .unwrap();
        let end = tool_start
            + ANTHROPIC[tool_start..]
                .windows(marker.len())
                .position(|window| window == marker)
                .unwrap();
        let mut gate = AnthropicStreamGate::new(|_| GateDecision::Allow);
        gate.feed(&ANTHROPIC[..end]).unwrap();
        assert_eq!(gate.finish(), Err(ProtocolError::TruncatedAction));
    }

    #[test]
    fn anthropic_gate_holds_then_denies_without_leaking_a_tool_call() {
        let mut gate = AnthropicStreamGate::new(|_| GateDecision::Hold);
        let initial = gate
            .feed(ANTHROPIC)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert!(String::from_utf8_lossy(&initial).contains("I will run the project tests."));
        assert!(!String::from_utf8_lossy(&initial).contains("\"type\":\"tool_use\""));
        assert_eq!(gate.pending_approval().unwrap().raw_tool_name, "Bash");
        assert_eq!(gate.finish(), Err(ProtocolError::TruncatedAction));

        let output = gate
            .resolve_pending(GateDecision::Deny {
                reason: "shell commands require approval".to_owned(),
            })
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("[blocked by Kerna policy] Bash was not released"));
        assert!(text.contains("\"stop_reason\":\"end_turn\""));
        assert!(!text.contains("\"type\":\"tool_use\""));
    }

    #[test]
    fn anthropic_gate_holds_then_releases_the_original_tool_bytes() {
        let mut gate = AnthropicStreamGate::new(|_| GateDecision::Hold);
        let initial = gate
            .feed(ANTHROPIC)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let released = gate
            .resolve_pending(GateDecision::Allow)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut reconstructed = initial;
        reconstructed.extend(released);
        assert_eq!(reconstructed, ANTHROPIC);
    }

    #[test]
    fn anthropic_multi_action_keeps_tool_use_when_one_action_is_released() {
        let mut decisions = 0;
        let mut gate = AnthropicStreamGate::new(move |_| {
            decisions += 1;
            if decisions == 1 {
                GateDecision::Allow
            } else {
                GateDecision::Deny {
                    reason: "requires approval".to_owned(),
                }
            }
        });
        let output = gate
            .feed(ANTHROPIC_MULTI)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("toolu_redacted_a"));
        assert!(text.contains("[blocked by Kerna policy] Bash was not released"));
        assert!(text.contains("\"stop_reason\":\"tool_use\""));
        assert!(!text.contains("\"stop_reason\":\"end_turn\""));
    }

    fn gate_openai(
        fixture: &[u8],
        decision: GateDecision,
        chunk_size: usize,
    ) -> Result<Vec<u8>, ProtocolError> {
        let mut gate = OpenAiResponsesStreamGate::new(move |_| decision.clone());
        let mut output = Vec::new();
        for chunk in fixture.chunks(chunk_size) {
            output.extend(gate.feed(chunk)?.into_iter().flatten());
        }
        output.extend(gate.finish()?.into_iter().flatten());
        Ok(output)
    }

    #[test]
    fn openai_gate_releases_allowed_actions_unchanged() {
        for size in 1..=OPENAI.len() {
            assert_eq!(
                gate_openai(OPENAI, GateDecision::Allow, size).unwrap(),
                OPENAI,
                "bytes changed at chunk size {size}"
            );
        }
    }

    #[test]
    fn openai_gate_denial_replaces_function_and_custom_actions() {
        let output = gate_openai(
            OPENAI,
            GateDecision::Deny {
                reason: "requires approval".to_owned(),
            },
            1,
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("I will inspect and test the project."));
        assert!(text.contains("[blocked by Kerna policy] exec_command was not released"));
        assert!(text.contains("[blocked by Kerna policy] write_stdin was not released"));
        assert!(text.contains("[blocked by Kerna policy] apply_patch was not released"));
        assert!(!text.contains("\"type\":\"function_call\""));
        assert!(!text.contains("\"type\":\"custom_tool_call\""));
        assert!(text.contains("event: response.completed"));
    }

    #[test]
    fn openai_gate_fails_closed_on_disconnect_mid_tool() {
        let marker = b"event: response.function_call_arguments.done";
        let end = OPENAI
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        let mut gate = OpenAiResponsesStreamGate::new(|_| GateDecision::Allow);
        gate.feed(&OPENAI[..end]).unwrap();
        assert_eq!(gate.finish(), Err(ProtocolError::TruncatedAction));
    }

    #[test]
    fn openai_gate_holds_then_releases_original_function_call_bytes() {
        let done = b"event: response.function_call_arguments.done";
        let start = OPENAI
            .windows(done.len())
            .position(|window| window == done)
            .unwrap();
        let end = start + frame_end(&OPENAI[start..]).unwrap();
        let first_action = &OPENAI[..end];

        let mut gate = OpenAiResponsesStreamGate::new(|_| GateDecision::Hold);
        let initial = gate
            .feed(first_action)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(
            gate.pending_approval().unwrap().raw_tool_name,
            "exec_command"
        );
        assert!(!String::from_utf8_lossy(&initial).contains("\"type\":\"function_call\""));

        let released = gate
            .resolve_pending(GateDecision::Allow)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut reconstructed = initial;
        reconstructed.extend(released);
        assert_eq!(reconstructed, first_action);
    }

    #[test]
    fn openai_gate_holds_then_denies_all_actions_without_leaking_calls() {
        let mut decisions = 0;
        let mut gate = OpenAiResponsesStreamGate::new(move |_| {
            decisions += 1;
            if decisions == 1 {
                GateDecision::Hold
            } else {
                GateDecision::Deny {
                    reason: "requires approval".to_owned(),
                }
            }
        });
        let initial = gate
            .feed(OPENAI)
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(
            gate.pending_approval().unwrap().raw_tool_name,
            "exec_command"
        );
        let resumed = gate
            .resolve_pending(GateDecision::Deny {
                reason: "requires approval".to_owned(),
            })
            .unwrap()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut output = initial;
        output.extend(resumed);
        let text = String::from_utf8(output).unwrap();
        assert!(!text.contains("\"type\":\"function_call\""));
        assert!(!text.contains("\"type\":\"custom_tool_call\""));
        // Each denial appears in the text delta, text completion, and completed item.
        assert_eq!(text.matches("[blocked by Kerna policy]").count(), 9);
        assert!(text.contains("event: response.completed"));
    }
}
