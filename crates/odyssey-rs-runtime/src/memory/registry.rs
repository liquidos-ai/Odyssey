use crate::RuntimeError;
use crate::session::{TurnChatMessageRecord, TurnRecord};
use autoagents_core::agent::memory::{MemoryProvider, SlidingWindowMemory};
use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
use odyssey_rs_manifest::BundleManifest;
use serde_json::Value;

const DEFAULT_SLIDING_WINDOW_SIZE: usize = 100;

pub fn build_memory(
    manifest: &BundleManifest,
    turns: &[TurnRecord],
) -> Result<Option<Box<dyn MemoryProvider>>, RuntimeError> {
    match manifest.memory.provider.id.as_str() {
        "sliding_window" => {
            let window =
                window_size(&manifest.memory.config).unwrap_or(DEFAULT_SLIDING_WINDOW_SIZE);
            let mut memory = SlidingWindowMemory::new(window);
            let history = session_history(turns);
            let _ = memory.preload(history);
            Ok(Some(Box::new(memory)))
        }
        other => Err(RuntimeError::Unsupported(format!(
            "unsupported prebuilt memory provider: {other}"
        ))),
    }
}

fn session_history(turns: &[TurnRecord]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for turn in turns {
        if !turn.chat_history.is_empty() {
            messages.extend(
                turn.chat_history
                    .iter()
                    .cloned()
                    .map(TurnChatMessageRecord::into_chat_message),
            );
        } else {
            messages.push(ChatMessage {
                role: ChatRole::User,
                message_type: MessageType::Text,
                content: turn.prompt.clone(),
            });
            messages.push(ChatMessage {
                role: ChatRole::Assistant,
                message_type: MessageType::Text,
                content: turn.response.clone(),
            });
        }
    }
    messages
}

fn window_size(value: &Value) -> Option<usize> {
    value
        .get("max_window")
        .or_else(|| value.get("window_size"))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

#[cfg(test)]
mod tests {
    use super::build_memory;
    use crate::session::{TurnChatMessageKind, TurnChatMessageRecord, TurnRecord};
    use autoagents_llm::chat::{ChatRole, MessageType};
    use autoagents_llm::{FunctionCall, ToolCall};
    use chrono::Utc;
    use odyssey_rs_manifest::{
        BundleExecutor, BundleManifest, BundleMemory, BundleSandbox, BundleServer,
    };
    use pretty_assertions::assert_eq;
    use serde_json::Value;

    #[tokio::test]
    async fn build_memory_preloads_recent_session_history() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: "prebuilt".to_string(),
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            resources: Vec::new(),
            skills: Vec::new(),
            tools: Vec::new(),
            server: BundleServer::default(),
            sandbox: BundleSandbox::default(),
        };
        let turns = vec![
            TurnRecord {
                turn_id: uuid::Uuid::new_v4(),
                prompt: "first".to_string(),
                response: "one".to_string(),
                chat_history: Vec::new(),
                created_at: Utc::now(),
            },
            TurnRecord {
                turn_id: uuid::Uuid::new_v4(),
                prompt: "second".to_string(),
                response: "two".to_string(),
                chat_history: Vec::new(),
                created_at: Utc::now(),
            },
        ];

        let memory = build_memory(&manifest, &turns)
            .expect("memory")
            .expect("provider");
        let recalled = memory.recall("", None).await.expect("recall");

        assert_eq!(recalled.len(), 4);
        assert_eq!(recalled[0].role, ChatRole::User);
        assert_eq!(recalled[0].content, "first");
        assert_eq!(recalled[3].role, ChatRole::Assistant);
        assert_eq!(recalled[3].content, "two");
    }

    #[tokio::test]
    async fn build_memory_replays_tool_calls_from_chat_history() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: "prebuilt".to_string(),
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            resources: Vec::new(),
            skills: Vec::new(),
            tools: Vec::new(),
            server: BundleServer::default(),
            sandbox: BundleSandbox::default(),
        };
        let turns = vec![TurnRecord {
            turn_id: uuid::Uuid::new_v4(),
            prompt: "create file".to_string(),
            response: "failed".to_string(),
            chat_history: vec![
                TurnChatMessageRecord::from_text(ChatRole::User, "create file"),
                TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Assistant,
                    TurnChatMessageKind::ToolUse,
                    vec![ToolCall {
                        id: "call_1".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "Write".to_string(),
                            arguments: "{\"path\":\"hello.py\"}".to_string(),
                        },
                    }],
                ),
                TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Tool,
                    TurnChatMessageKind::ToolResult,
                    vec![ToolCall {
                        id: "call_1".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "Write".to_string(),
                            arguments: "{\"error\":\"permission denied\"}".to_string(),
                        },
                    }],
                ),
                TurnChatMessageRecord::from_text(ChatRole::Assistant, "failed"),
            ],
            created_at: Utc::now(),
        }];

        let memory = build_memory(&manifest, &turns)
            .expect("memory")
            .expect("provider");
        let recalled = memory.recall("", None).await.expect("recall");

        assert_eq!(recalled.len(), 4);
        assert!(matches!(recalled[1].message_type, MessageType::ToolUse(_)));
        assert!(matches!(
            recalled[2].message_type,
            MessageType::ToolResult(_)
        ));
    }
}
