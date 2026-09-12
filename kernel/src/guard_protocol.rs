//! Streaming model-protocol parsing for Kerna Guard.
//!
//! This module deliberately makes no policy decision. It preserves every upstream SSE
//! byte while also emitting complete action candidates for the policy/approval layer.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        if self.pending.iter().all(u8::is_ascii_whitespace) {
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

#[derive(Default)]
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

#[cfg(test)]
mod tests {
    use super::*;

    const ANTHROPIC: &[u8] = include_bytes!("../tests/fixtures/anthropic/messages-tool-use.sse");
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
    fn openai_fixture_extracts_function_shell_and_patch_actions() {
        let actions = parse_every_boundary(Protocol::OpenAiResponses, OPENAI);
        assert_eq!(
            actions
                .iter()
                .map(|a| a.raw_tool_name.as_str())
                .collect::<Vec<_>>(),
            ["read_file", "shell", "apply_patch"]
        );
        assert_eq!(actions[0].arguments["path"], "src/lib.rs");
        assert!(actions[1]
            .arguments
            .as_str()
            .unwrap()
            .contains("cargo test"));
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
}
