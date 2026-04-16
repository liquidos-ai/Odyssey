//! Utilities for normalizing tool-use and tool-result messages.

use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
use autoagents_llm::{FunctionCall, ToolCall};
use std::collections::HashMap;

pub(crate) const TOOL_RESULT_PLACEHOLDER: &str = "[tool output omitted]";

pub(crate) fn ensure_tool_results(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut result_map = collect_tool_results(&messages);
    let mut output = Vec::with_capacity(messages.len());

    for mut message in messages {
        match &mut message.message_type {
            MessageType::ToolUse(calls) => {
                let mut resolved = Vec::with_capacity(calls.len());
                for call in calls.iter() {
                    let result = result_map
                        .remove(&call.id)
                        .unwrap_or_else(|| placeholder_tool_result(call));
                    resolved.push(result);
                }
                output.push(message);
                if !resolved.is_empty() {
                    output.push(ChatMessage {
                        role: ChatRole::Tool,
                        message_type: MessageType::ToolResult(resolved),
                        content: String::new(),
                    });
                }
            }
            MessageType::ToolResult(_) => {}
            _ => output.push(message),
        }
    }

    output
}

pub(crate) fn placeholder_tool_result(call: &ToolCall) -> ToolCall {
    ToolCall {
        id: call.id.clone(),
        call_type: call.call_type.clone(),
        function: FunctionCall {
            name: call.function.name.clone(),
            arguments: TOOL_RESULT_PLACEHOLDER.to_string(),
        },
    }
}

fn collect_tool_results(messages: &[ChatMessage]) -> HashMap<String, ToolCall> {
    let mut results = HashMap::new();
    for message in messages {
        if let MessageType::ToolResult(calls) = &message.message_type {
            for call in calls {
                results
                    .entry(call.id.clone())
                    .or_insert_with(|| call.clone());
            }
        }
    }
    results
}
