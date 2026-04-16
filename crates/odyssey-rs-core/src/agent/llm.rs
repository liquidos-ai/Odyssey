//! LLM provider construction for Odyssey agents.

use crate::agent::tool_messages::ensure_tool_results;
use autoagents_llm::LLMProvider;
use autoagents_llm::async_trait;
use autoagents_llm::chat::{
    ChatMessage, ChatProvider, ChatResponse, MessageType, StreamChunk, StreamResponse,
    StructuredOutputFormat, Tool,
};
use autoagents_llm::completion::{CompletionProvider, CompletionRequest, CompletionResponse};
use autoagents_llm::embedding::EmbeddingProvider;
use autoagents_llm::error::LLMError;
use autoagents_llm::models::ModelsProvider;
use futures_util::stream::Stream;
use std::pin::Pin;
use std::sync::Arc;

pub fn wrap_llm_provider(llm: Arc<dyn LLMProvider>) -> Arc<dyn LLMProvider> {
    Arc::new(GuardedLLMProvider::new(llm))
}

#[derive(Clone)]
struct GuardedLLMProvider {
    inner: Arc<dyn LLMProvider>,
}

impl GuardedLLMProvider {
    fn new(inner: Arc<dyn LLMProvider>) -> Self {
        Self { inner }
    }

    fn sanitize_messages(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        if messages.iter().all(|message| {
            !matches!(
                message.message_type,
                MessageType::ToolUse(_) | MessageType::ToolResult(_)
            )
        }) {
            return messages.to_vec();
        }
        ensure_tool_results(messages.to_vec())
    }
}

#[async_trait]
impl ChatProvider for GuardedLLMProvider {
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        let sanitized = self.sanitize_messages(messages);
        self.inner
            .chat_with_tools(&sanitized, tools, json_schema)
            .await
    }

    async fn chat_with_web_search(&self, input: String) -> Result<Box<dyn ChatResponse>, LLMError> {
        self.inner.chat_with_web_search(input).await
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, LLMError>> + Send>>, LLMError> {
        let sanitized = self.sanitize_messages(messages);
        self.inner.chat_stream(&sanitized, json_schema).await
    }

    async fn chat_stream_struct(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse, LLMError>> + Send>>, LLMError>
    {
        let sanitized = self.sanitize_messages(messages);
        self.inner
            .chat_stream_struct(&sanitized, tools, json_schema)
            .await
    }

    async fn chat_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send>>, LLMError> {
        let sanitized = self.sanitize_messages(messages);
        self.inner
            .chat_stream_with_tools(&sanitized, tools, json_schema)
            .await
    }
}

#[async_trait]
impl CompletionProvider for GuardedLLMProvider {
    async fn complete(
        &self,
        req: &CompletionRequest,
        json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        self.inner.complete(req, json_schema).await
    }
}

#[async_trait]
impl EmbeddingProvider for GuardedLLMProvider {
    async fn embed(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        self.inner.embed(input).await
    }
}

#[async_trait]
impl ModelsProvider for GuardedLLMProvider {}

impl LLMProvider for GuardedLLMProvider {}

#[cfg(test)]
mod tests {
    use super::wrap_llm_provider;
    use autoagents_llm::FunctionCall;
    use autoagents_llm::ToolCall;
    use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
    use odyssey_rs_test_utils::RecordingChatLLM;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[tokio::test]
    async fn wrapped_llm_passthrough_for_plain_messages() {
        let inner = Arc::new(RecordingChatLLM::new("ok"));
        let wrapped = wrap_llm_provider(inner.clone());
        let messages = vec![ChatMessage {
            role: ChatRole::User,
            message_type: MessageType::Text,
            content: "hi".to_string(),
        }];

        wrapped
            .chat_with_tools(&messages, None, None)
            .await
            .expect("chat");

        let captured = inner.last_messages.lock().clone();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].content, "hi");
        assert_eq!(captured[0].role, ChatRole::User);
    }

    #[tokio::test]
    async fn wrapped_llm_inserts_missing_tool_results() {
        let inner = Arc::new(RecordingChatLLM::new("ok"));
        let wrapped = wrap_llm_provider(inner.clone());

        let call = ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Read".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let tool_use = ChatMessage {
            role: ChatRole::Assistant,
            message_type: MessageType::ToolUse(vec![call.clone()]),
            content: String::new(),
        };

        wrapped
            .chat_with_tools(&[tool_use], None, None)
            .await
            .expect("chat");

        let captured = inner.last_messages.lock().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].role, ChatRole::Assistant);
        match &captured[1].message_type {
            MessageType::ToolResult(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].function.name, "Read".to_string());
                assert_eq!(
                    results[0].function.arguments,
                    super::super::tool_messages::TOOL_RESULT_PLACEHOLDER.to_string()
                );
            }
            other => panic!("unexpected message type: {other:?}"),
        }
    }

    #[tokio::test]
    async fn wrapped_llm_uses_existing_tool_results() {
        let inner = Arc::new(RecordingChatLLM::new("ok"));
        let wrapped = wrap_llm_provider(inner.clone());

        let call = ToolCall {
            id: "call_2".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Write".to_string(),
                arguments: "{\"ok\":true}".to_string(),
            },
        };
        let tool_use = ChatMessage {
            role: ChatRole::Assistant,
            message_type: MessageType::ToolUse(vec![call.clone()]),
            content: String::new(),
        };
        let tool_result = ChatMessage {
            role: ChatRole::Tool,
            message_type: MessageType::ToolResult(vec![call.clone()]),
            content: String::new(),
        };

        wrapped
            .chat_with_tools(&[tool_use, tool_result], None, None)
            .await
            .expect("chat");

        let captured = inner.last_messages.lock().clone();
        assert_eq!(captured.len(), 2);
        match &captured[1].message_type {
            MessageType::ToolResult(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].function.arguments, "{\"ok\":true}".to_string());
            }
            other => panic!("unexpected message type: {other:?}"),
        }
    }
}
