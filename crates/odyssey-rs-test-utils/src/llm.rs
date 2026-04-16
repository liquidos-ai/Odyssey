use async_trait::async_trait;
use autoagents_llm::chat::{
    ChatMessage, ChatProvider, ChatResponse, StreamChunk, StructuredOutputFormat, Tool,
};
use autoagents_llm::completion::{CompletionProvider, CompletionRequest, CompletionResponse};
use autoagents_llm::embedding::EmbeddingProvider;
use autoagents_llm::error::LLMError;
use autoagents_llm::models::ModelsProvider;
use autoagents_llm::{LLMProvider, ToolCall};
use futures_util::Stream;
use futures_util::stream;
use parking_lot::Mutex;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct FixedChatResponse {
    text: String,
    tool_calls: Option<Vec<ToolCall>>,
}

impl FixedChatResponse {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: None,
        }
    }

    pub fn with_tool_calls(text: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Some(tool_calls),
        }
    }
}

impl std::fmt::Display for FixedChatResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.text)
    }
}

impl ChatResponse for FixedChatResponse {
    fn text(&self) -> Option<String> {
        Some(self.text.clone())
    }

    fn tool_calls(&self) -> Option<Vec<ToolCall>> {
        self.tool_calls.clone()
    }
}

#[derive(Debug, Clone)]
pub struct FixedLLM {
    response: String,
    completion: String,
    embedding: Vec<f32>,
}

impl FixedLLM {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            completion: "mock completion".to_string(),
            embedding: vec![0.0, 0.0],
        }
    }

    pub fn with_completion(mut self, completion: impl Into<String>) -> Self {
        self.completion = completion.into();
        self
    }

    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = embedding;
        self
    }
}

#[async_trait]
impl ChatProvider for FixedLLM {
    async fn chat_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        Ok(Box::new(FixedChatResponse::new(self.response.clone())))
    }
}

#[async_trait]
impl CompletionProvider for FixedLLM {
    async fn complete(
        &self,
        _req: &CompletionRequest,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        Ok(CompletionResponse {
            text: self.completion.clone(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FixedLLM {
    async fn embed(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Ok(input.into_iter().map(|_| self.embedding.clone()).collect())
    }
}

#[async_trait]
impl ModelsProvider for FixedLLM {}

impl LLMProvider for FixedLLM {}

#[derive(Debug, Clone)]
pub struct RecordingLLM {
    response: String,
    seen_tools: Arc<Mutex<Vec<String>>>,
}

impl RecordingLLM {
    pub fn new(response: impl Into<String>) -> (Self, Arc<Mutex<Vec<String>>>) {
        let seen_tools = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                response: response.into(),
                seen_tools: seen_tools.clone(),
            },
            seen_tools,
        )
    }

    pub fn with_sink(response: impl Into<String>, seen_tools: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            response: response.into(),
            seen_tools,
        }
    }
}

#[async_trait]
impl ChatProvider for RecordingLLM {
    async fn chat_with_tools(
        &self,
        _messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        let names = tools
            .unwrap_or(&[])
            .iter()
            .map(|tool| tool.function.name.clone())
            .collect::<Vec<_>>();
        *self.seen_tools.lock() = names;
        Ok(Box::new(FixedChatResponse::new(self.response.clone())))
    }
}

#[async_trait]
impl CompletionProvider for RecordingLLM {
    async fn complete(
        &self,
        _req: &CompletionRequest,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        Ok(CompletionResponse {
            text: "mock completion".to_string(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for RecordingLLM {
    async fn embed(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Ok(input.into_iter().map(|_| vec![0.0, 0.0]).collect())
    }
}

#[async_trait]
impl ModelsProvider for RecordingLLM {}

impl LLMProvider for RecordingLLM {}

#[derive(Debug, Clone)]
pub struct StreamingLLM {
    chunks: Vec<String>,
    response: String,
}

impl StreamingLLM {
    pub fn new(chunks: Vec<String>) -> Self {
        let response = chunks.join("");
        Self { chunks, response }
    }
}

type LlmStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send>>;

#[async_trait]
impl ChatProvider for StreamingLLM {
    async fn chat_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        Ok(Box::new(FixedChatResponse::new(self.response.clone())))
    }

    async fn chat_stream_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<LlmStream, LLMError> {
        let chunks = self
            .chunks
            .iter()
            .cloned()
            .map(StreamChunk::Text)
            .map(Ok)
            .collect::<Vec<_>>();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

#[async_trait]
impl CompletionProvider for StreamingLLM {
    async fn complete(
        &self,
        _req: &CompletionRequest,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        Ok(CompletionResponse {
            text: "mock completion".to_string(),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for StreamingLLM {
    async fn embed(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Ok(input.into_iter().map(|_| vec![0.0, 0.0]).collect())
    }
}

#[async_trait]
impl ModelsProvider for StreamingLLM {}

impl LLMProvider for StreamingLLM {}

#[derive(Debug, Clone)]
pub struct RecordingChatLLM {
    response: String,
    pub last_messages: Arc<Mutex<Vec<ChatMessage>>>,
}

impl RecordingChatLLM {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            last_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ChatProvider for RecordingChatLLM {
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        *self.last_messages.lock() = messages.to_vec();
        Ok(Box::new(FixedChatResponse::new(self.response.clone())))
    }
}

#[async_trait]
impl CompletionProvider for RecordingChatLLM {
    async fn complete(
        &self,
        _req: &CompletionRequest,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        Err(LLMError::ProviderError("recording".to_string()))
    }
}

#[async_trait]
impl EmbeddingProvider for RecordingChatLLM {
    async fn embed(&self, _input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Err(LLMError::ProviderError("recording".to_string()))
    }
}

#[async_trait]
impl ModelsProvider for RecordingChatLLM {}

impl LLMProvider for RecordingChatLLM {}

#[derive(Debug, Clone)]
pub struct FailingLLM {
    message: String,
}

impl FailingLLM {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
impl ChatProvider for FailingLLM {
    async fn chat_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<Box<dyn ChatResponse>, LLMError> {
        Err(LLMError::ProviderError(self.message.clone()))
    }
}

#[async_trait]
impl CompletionProvider for FailingLLM {
    async fn complete(
        &self,
        _req: &CompletionRequest,
        _json_schema: Option<StructuredOutputFormat>,
    ) -> Result<CompletionResponse, LLMError> {
        Err(LLMError::ProviderError(self.message.clone()))
    }
}

#[async_trait]
impl EmbeddingProvider for FailingLLM {
    async fn embed(&self, _input: Vec<String>) -> Result<Vec<Vec<f32>>, LLMError> {
        Err(LLMError::ProviderError(self.message.clone()))
    }
}

#[async_trait]
impl ModelsProvider for FailingLLM {}

impl LLMProvider for FailingLLM {}
