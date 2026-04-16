//! AutoAgents memory adapter backed by odyssey-rs-memory.

use autoagents_core::agent::memory::{MemoryProvider as AutoAgentsMemoryProvider, MemoryType};
use autoagents_llm::ToolCall;
use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
use autoagents_llm::error::LLMError;
use chrono::Utc;
use odyssey_rs_memory::{
    MemoryCapturePolicy, MemoryCompactionPolicy, MemoryProvider as OdysseyMemoryProvider,
    MemoryRecallOptions, MemoryRecord,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;
use uuid::Uuid;

const DEFAULT_WINDOW_SIZE: usize = 20;

/// AutoAgents memory adapter that persists to odyssey-rs-memory.
#[derive(Clone)]
pub struct OdysseyMemoryAdapter {
    session_id: Uuid,
    agent_id: String,
    provider: Arc<dyn OdysseyMemoryProvider>,
    capture_policy: MemoryCapturePolicy,
    compaction_policy: MemoryCompactionPolicy,
    recall_options: MemoryRecallOptions,
    recall_limit: Option<usize>,
    max_ephemeral: usize,
    ephemeral: VecDeque<MemoryRecord>,
}

impl OdysseyMemoryAdapter {
    /// Create a new memory adapter tied to a session and agent.
    pub fn new(
        session_id: Uuid,
        agent_id: String,
        provider: Arc<dyn OdysseyMemoryProvider>,
        capture_policy: MemoryCapturePolicy,
        compaction_policy: MemoryCompactionPolicy,
        recall_options: MemoryRecallOptions,
        recall_limit: Option<usize>,
    ) -> Self {
        let max_ephemeral = recall_limit
            .unwrap_or(DEFAULT_WINDOW_SIZE)
            .max(DEFAULT_WINDOW_SIZE);
        Self {
            session_id,
            agent_id,
            provider,
            capture_policy,
            compaction_policy,
            recall_options,
            recall_limit,
            max_ephemeral,
            ephemeral: VecDeque::new(),
        }
    }

    fn should_persist_message(&self, message: &ChatMessage) -> bool {
        if is_tool_result_message(message) {
            return self.capture_policy.capture_tool_output;
        }
        self.capture_policy.capture_messages
    }

    fn record_from_message(&self, message: &ChatMessage) -> Result<MemoryRecord, LLMError> {
        let content = content_for_message(message);
        let metadata = metadata_for_message(&self.agent_id, message);
        Ok(MemoryRecord {
            id: Uuid::new_v4(),
            session_id: self.session_id,
            role: message.role.to_string(),
            content,
            metadata,
            created_at: Utc::now(),
        })
    }

    fn record_for_persistence(
        &self,
        message: &ChatMessage,
        mut record: MemoryRecord,
    ) -> Option<MemoryRecord> {
        if self.should_persist_message(message) {
            return Some(record);
        }
        if !self.capture_policy.capture_messages {
            return None;
        }
        if let MessageType::ToolResult(calls) = &message.message_type {
            record.content = super::tool_messages::TOOL_RESULT_PLACEHOLDER.to_string();
            if let Some(map) = record.metadata.as_object_mut() {
                map.insert("kind".to_string(), json!("message"));
                let sanitized_calls = calls
                    .iter()
                    .map(super::tool_messages::placeholder_tool_result)
                    .collect::<Vec<_>>();
                if let Ok(value) = serde_json::to_value(sanitized_calls) {
                    map.insert("tool_calls".to_string(), value);
                }
            }
            return Some(record);
        }
        None
    }

    fn message_from_record(&self, record: &MemoryRecord) -> ChatMessage {
        let message_type = message_type_from_metadata(&record.metadata);
        let content = if matches!(message_type, MessageType::ToolResult(_)) {
            String::new()
        } else {
            record.content.clone()
        };
        ChatMessage {
            role: role_from_str(&record.role),
            message_type,
            content,
        }
    }

    fn push_ephemeral(&mut self, record: MemoryRecord) {
        if self.max_ephemeral > 0 && self.ephemeral.len() >= self.max_ephemeral {
            self.ephemeral.pop_front();
        }
        self.ephemeral.push_back(record);
    }

    async fn compact_if_needed(&self, message: &ChatMessage) {
        if !self.compaction_policy.enabled {
            return;
        }
        if matches!(message.role, ChatRole::Assistant)
            && matches!(message.message_type, MessageType::Text)
        {
            let _ = self
                .provider
                .compact(self.session_id, &self.compaction_policy)
                .await;
        }
    }
}

#[async_trait::async_trait]
impl AutoAgentsMemoryProvider for OdysseyMemoryAdapter {
    async fn remember(&mut self, message: &ChatMessage) -> Result<(), LLMError> {
        let record = self.record_from_message(message)?;
        self.push_ephemeral(record.clone());

        if let Some(record) = self.record_for_persistence(message, record)
            && let Err(err) = self
                .provider
                .store_with_policy(record, &self.capture_policy)
                .await
        {
            return Err(LLMError::ProviderError(err.to_string()));
        }
        self.compact_if_needed(message).await;
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ChatMessage>, LLMError> {
        let limit = limit.or(self.recall_limit).unwrap_or(0);
        let mut records = Vec::new();
        if limit > 0 {
            let fetch_limit = limit.saturating_add(self.ephemeral.len());
            let stored = self
                .provider
                .recall_with_options(self.session_id, None, fetch_limit, self.recall_options)
                .await
                .map_err(|err| LLMError::ProviderError(err.to_string()))?;
            let cutoff = self.ephemeral.front().map(|record| record.created_at);
            records = if let Some(cutoff) = cutoff {
                stored
                    .into_iter()
                    .filter(|record| record.created_at < cutoff)
                    .collect()
            } else {
                stored
            };
        }
        records.extend(self.ephemeral.iter().cloned());

        if limit > 0 && records.len() > limit {
            let start = records.len().saturating_sub(limit);
            records = records[start..].to_vec();
        }

        let messages = records
            .into_iter()
            .map(|record| self.message_from_record(&record))
            .collect();
        Ok(super::tool_messages::ensure_tool_results(messages))
    }

    async fn clear(&mut self) -> Result<(), LLMError> {
        self.ephemeral.clear();
        Ok(())
    }

    fn memory_type(&self) -> MemoryType {
        MemoryType::SlidingWindow
    }

    fn size(&self) -> usize {
        self.ephemeral.len()
    }

    fn clone_box(&self) -> Box<dyn AutoAgentsMemoryProvider> {
        Box::new(self.clone())
    }

    fn id(&self) -> Option<String> {
        Some(format!("odyssey:{}:{}", self.session_id, self.agent_id))
    }
}

fn is_tool_result_message(message: &ChatMessage) -> bool {
    matches!(message.message_type, MessageType::ToolResult(_))
        || matches!(message.role, ChatRole::Tool)
}

fn role_from_str(value: &str) -> ChatRole {
    match value {
        "system" => ChatRole::System,
        "assistant" => ChatRole::Assistant,
        "tool" => ChatRole::Tool,
        _ => ChatRole::User,
    }
}

fn record_kind(message: &ChatMessage) -> &'static str {
    if is_tool_result_message(message) {
        "tool_output"
    } else {
        "message"
    }
}

fn metadata_for_message(agent_id: &str, message: &ChatMessage) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("agent_id".to_string(), json!(agent_id));
    map.insert("kind".to_string(), json!(record_kind(message)));
    map.insert(
        "message_type".to_string(),
        json!(message_type_label(message)),
    );
    if let Some(tool_calls) = tool_calls_value(message) {
        map.insert("tool_calls".to_string(), tool_calls);
    }
    serde_json::Value::Object(map)
}

fn message_type_label(message: &ChatMessage) -> &'static str {
    match message.message_type {
        MessageType::Text => "text",
        MessageType::Image(_) => "image",
        MessageType::Pdf(_) => "pdf",
        MessageType::ImageURL(_) => "image_url",
        MessageType::ToolUse(_) => "tool_use",
        MessageType::ToolResult(_) => "tool_result",
    }
}

fn tool_calls_value(message: &ChatMessage) -> Option<serde_json::Value> {
    match &message.message_type {
        MessageType::ToolUse(calls) | MessageType::ToolResult(calls) => {
            serde_json::to_value(calls).ok()
        }
        _ => None,
    }
}

fn message_type_from_metadata(metadata: &serde_json::Value) -> MessageType {
    let Some(label) = metadata
        .get("message_type")
        .and_then(|value| value.as_str())
    else {
        return MessageType::Text;
    };
    match label {
        "tool_use" => {
            if let Some(calls) = tool_calls_from_metadata(metadata) {
                MessageType::ToolUse(calls)
            } else {
                MessageType::Text
            }
        }
        "tool_result" => {
            if let Some(calls) = tool_calls_from_metadata(metadata) {
                MessageType::ToolResult(calls)
            } else {
                MessageType::Text
            }
        }
        _ => MessageType::Text,
    }
}

fn tool_calls_from_metadata(metadata: &serde_json::Value) -> Option<Vec<ToolCall>> {
    metadata
        .get("tool_calls")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn content_for_message(message: &ChatMessage) -> String {
    if !message.content.trim().is_empty() {
        return message.content.clone();
    }
    match &message.message_type {
        MessageType::ToolUse(calls) => format!("tool_use: {}", tool_call_names(calls)),
        MessageType::ToolResult(calls) => tool_result_content(calls),
        _ => message.content.clone(),
    }
}

fn tool_call_names(calls: &[ToolCall]) -> String {
    let names = calls
        .iter()
        .map(|call| call.function.name.as_str())
        .collect::<Vec<_>>();
    names.join(", ")
}

fn tool_result_content(calls: &[ToolCall]) -> String {
    if calls.is_empty() {
        return String::new();
    }
    let mut parts = Vec::with_capacity(calls.len());
    for call in calls {
        let args = call.function.arguments.as_str();
        if args.trim().is_empty() {
            parts.push(format!("{}: <empty>", call.function.name));
        } else {
            parts.push(format!("{}: {}", call.function.name, args));
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        OdysseyMemoryAdapter, content_for_message, message_type_from_metadata, role_from_str,
        tool_call_names, tool_result_content,
    };
    use crate::agent::tool_messages::TOOL_RESULT_PLACEHOLDER;
    use autoagents_core::agent::memory::MemoryProvider as AutoAgentsMemoryProvider;
    use autoagents_llm::FunctionCall;
    use autoagents_llm::ToolCall;
    use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
    use odyssey_rs_memory::{
        MemoryCapturePolicy, MemoryCompactionPolicy, MemoryProvider, MemoryRecallOptions,
        MemoryRecord,
    };
    use parking_lot::Mutex;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Default)]
    struct RecordingProvider {
        records: Mutex<Vec<MemoryRecord>>,
    }

    #[async_trait::async_trait]
    impl MemoryProvider for RecordingProvider {
        async fn store(&self, record: MemoryRecord) -> Result<(), odyssey_rs_memory::MemoryError> {
            self.records.lock().push(record);
            Ok(())
        }

        async fn recall(
            &self,
            session_id: Uuid,
            _query: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<MemoryRecord>, odyssey_rs_memory::MemoryError> {
            Ok(self
                .records
                .lock()
                .iter()
                .filter(|record| record.session_id == session_id)
                .cloned()
                .collect())
        }
    }

    fn tool_call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: args.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn remember_persists_text_messages() {
        let session_id = Uuid::new_v4();
        let provider = Arc::new(RecordingProvider::default());
        let adapter = OdysseyMemoryAdapter::new(
            session_id,
            "agent".to_string(),
            provider.clone(),
            MemoryCapturePolicy::default(),
            MemoryCompactionPolicy::default(),
            MemoryRecallOptions::default(),
            Some(5),
        );
        let mut adapter = adapter;
        let message = ChatMessage {
            role: ChatRole::User,
            message_type: MessageType::Text,
            content: "hello".to_string(),
        };
        adapter.remember(&message).await.expect("remember");

        let stored = provider.records.lock();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].content, "hello");
        assert_eq!(stored[0].metadata["kind"], json!("message"));
    }

    #[tokio::test]
    async fn remember_masks_tool_results_when_disabled() {
        let session_id = Uuid::new_v4();
        let provider = Arc::new(RecordingProvider::default());
        let capture = MemoryCapturePolicy {
            capture_messages: true,
            capture_tool_output: false,
            ..MemoryCapturePolicy::default()
        };
        let mut adapter = OdysseyMemoryAdapter::new(
            session_id,
            "agent".to_string(),
            provider.clone(),
            capture,
            MemoryCompactionPolicy::default(),
            MemoryRecallOptions::default(),
            Some(5),
        );
        let call = tool_call("Read", "{}");
        let message = ChatMessage {
            role: ChatRole::Tool,
            message_type: MessageType::ToolResult(vec![call]),
            content: String::new(),
        };
        adapter.remember(&message).await.expect("remember");

        let stored = provider.records.lock();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].content, TOOL_RESULT_PLACEHOLDER.to_string());
        assert_eq!(stored[0].metadata["kind"], json!("message"));
    }

    #[test]
    fn helper_functions_cover_tool_call_paths() {
        let call = tool_call("Write", "");
        assert_eq!(tool_call_names(std::slice::from_ref(&call)), "Write");
        assert_eq!(
            tool_result_content(std::slice::from_ref(&call)),
            "Write: <empty>"
        );

        let message = ChatMessage {
            role: ChatRole::Assistant,
            message_type: MessageType::ToolUse(vec![call]),
            content: String::new(),
        };
        assert_eq!(content_for_message(&message), "tool_use: Write");

        let metadata = json!({
            "message_type": "tool_result",
            "tool_calls": [tool_call("Read", "{}")]
        });
        let message_type = message_type_from_metadata(&metadata);
        assert_eq!(
            message_type,
            MessageType::ToolResult(vec![tool_call("Read", "{}")])
        );

        assert_eq!(role_from_str("tool"), ChatRole::Tool);
    }
}
