use std::collections::HashMap;

use autoagents_llm::{FunctionCall, ToolCall, chat::ChatRole};
use odyssey_rs_protocol::{EventMsg, EventPayload, Task};
use uuid::Uuid;

use crate::session::{TurnChatMessageKind, TurnChatMessageRecord};

pub(crate) struct TurnHistoryCollector {
    turn_id: Uuid,
    messages: Vec<TurnChatMessageRecord>,
    assistant_text: String,
    pending_calls: HashMap<Uuid, ToolCall>,
}

impl TurnHistoryCollector {
    pub fn new(turn_id: Uuid, task: &Task) -> Self {
        Self {
            turn_id,
            messages: vec![TurnChatMessageRecord::from_text(
                ChatRole::User,
                task.prompt.clone(),
            )],
            assistant_text: String::default(),
            pending_calls: HashMap::new(),
        }
    }

    pub fn observe(&mut self, event: EventMsg) {
        match event.payload {
            EventPayload::AgentMessageDelta { turn_id, delta } if turn_id == self.turn_id => {
                self.assistant_text.push_str(&delta);
            }
            EventPayload::ToolCallStarted {
                turn_id,
                tool_call_id,
                tool_name,
                arguments,
            } if turn_id == self.turn_id => {
                self.flush_assistant_text();
                let call = ToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: tool_name,
                        arguments: arguments.to_string(),
                    },
                };
                self.pending_calls.insert(tool_call_id, call.clone());
                self.messages.push(TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Assistant,
                    TurnChatMessageKind::ToolUse,
                    vec![call],
                ));
            }
            EventPayload::ToolCallFinished {
                turn_id,
                tool_call_id,
                result,
                ..
            } if turn_id == self.turn_id => {
                self.flush_assistant_text();
                let started = self.pending_calls.remove(&tool_call_id);
                let call = ToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: started
                            .as_ref()
                            .map(|call| call.function.name.clone())
                            .unwrap_or_default(),
                        arguments: result.to_string(),
                    },
                };
                self.messages.push(TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Tool,
                    TurnChatMessageKind::ToolResult,
                    vec![call],
                ));
            }
            EventPayload::TurnCompleted { turn_id, message } if turn_id == self.turn_id => {
                self.assistant_text = message;
            }
            _ => {}
        }
    }

    pub fn finish(mut self, response: &str) -> Vec<TurnChatMessageRecord> {
        if self.assistant_text.is_empty() && !response.is_empty() {
            self.assistant_text = response.to_string();
        }
        self.flush_assistant_text();
        self.messages
    }

    fn flush_assistant_text(&mut self) {
        if self.assistant_text.is_empty() {
            return;
        }
        self.messages.push(TurnChatMessageRecord::from_text(
            ChatRole::Assistant,
            std::mem::take(&mut self.assistant_text),
        ));
    }
}
