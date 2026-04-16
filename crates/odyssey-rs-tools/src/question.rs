//! Interactive question prompts for tool execution.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use odyssey_rs_protocol::ToolError;

/// Option choice for a multiple-choice question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    /// Short label shown to the user.
    pub label: String,
    /// Optional machine-readable value.
    #[serde(default)]
    pub value: Option<String>,
    /// Optional description text.
    #[serde(default)]
    pub description: Option<String>,
}

/// Question prompt that can be presented to a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    /// Prompt text shown to the user.
    pub prompt: String,
    /// Optional choices for the question.
    pub options: Vec<QuestionOption>,
    /// Allow freeform text input when options are present.
    #[serde(default)]
    pub allow_freeform: bool,
}

/// Answer returned by a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionAnswer {
    /// Selected or provided value.
    pub value: String,
    /// Optional label of the selection.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional index of the selected option.
    #[serde(default)]
    pub index: Option<usize>,
}

/// Handler interface for interactive questions.
#[async_trait]
pub trait QuestionHandler: Send + Sync {
    /// Ask a question and return a user answer.
    async fn ask(&self, question: Question) -> Result<QuestionAnswer, ToolError>;
}
