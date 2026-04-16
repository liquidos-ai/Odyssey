//! Built-in tool for asking users clarifying questions.

use crate::builtins::utils::parse_args;
use crate::question::{Question, QuestionOption};
use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::ToolInputT;
use autoagents_derive::ToolInput;
use log::info;
use odyssey_rs_protocol::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Tool that prompts the user for a multiple-choice answer.
#[derive(Debug, Default)]
pub struct AskUserQuestionTool;

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question with multiple choice options"
    }

    fn args_schema(&self) -> Value {
        let params_str = AskUserQuestionArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: AskUserQuestionArgs = parse_args(args)?;
        if input.prompt.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "prompt cannot be empty".to_string(),
            ));
        }
        if input.options.is_empty() && !input.allow_freeform {
            return Err(ToolError::InvalidArguments(
                "options cannot be empty unless allow_freeform is true".to_string(),
            ));
        }
        if input
            .options
            .iter()
            .any(|option| option.label.trim().is_empty())
        {
            return Err(ToolError::InvalidArguments(
                "option labels cannot be empty".to_string(),
            ));
        }

        info!(
            "asking user question (options={}, allow_freeform={})",
            input.options.len(),
            input.allow_freeform
        );
        let handler = ctx.services.question_handler.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("question handler not configured".to_string())
        })?;
        let question = Question {
            prompt: input.prompt,
            options: input.options,
            allow_freeform: input.allow_freeform,
        };
        let answer = handler.ask(question).await?;

        Ok(json!({
            "value": answer.value,
            "label": answer.label,
            "index": answer.index,
        }))
    }
}

/// Arguments for AskUserQuestionTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct AskUserQuestionArgs {
    #[input(description = "Prompt text shown to the user.")]
    prompt: String,
    #[input(description = "Multiple choice options for the question.")]
    options: Vec<QuestionOption>,
    #[input(description = "Allow a freeform answer when options are present.")]
    #[serde(default)]
    allow_freeform: bool,
}

#[cfg(test)]
mod tests {
    use super::AskUserQuestionTool;
    use crate::question::{Question, QuestionAnswer, QuestionHandler};
    use crate::{Tool, ToolContext, TurnServices};
    use async_trait::async_trait;
    use odyssey_rs_protocol::ToolError;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    struct DummyHandler;

    #[async_trait]
    impl QuestionHandler for DummyHandler {
        async fn ask(&self, question: Question) -> Result<QuestionAnswer, ToolError> {
            Ok(QuestionAnswer {
                value: question
                    .options
                    .first()
                    .and_then(|opt| opt.value.clone())
                    .unwrap_or_else(|| "ok".to_string()),
                label: Some("label".to_string()),
                index: Some(0),
            })
        }
    }

    fn base_context(root: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(TurnServices {
                cwd: root.to_path_buf(),
                workspace_root: root.to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: None,
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
        }
    }

    #[tokio::test]
    async fn ask_user_rejects_empty_prompt() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = AskUserQuestionTool;
        let err = tool
            .call(
                &ctx,
                json!({
                    "prompt": " ",
                    "options": []
                }),
            )
            .await
            .expect_err("empty prompt");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments");
        };
        assert_eq!(message, "prompt cannot be empty");
    }

    #[tokio::test]
    async fn ask_user_requires_options_or_freeform() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = AskUserQuestionTool;
        let err = tool
            .call(
                &ctx,
                json!({
                    "prompt": "Pick one",
                    "options": [],
                    "allow_freeform": false
                }),
            )
            .await
            .expect_err("missing options");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments");
        };
        assert_eq!(
            message,
            "options cannot be empty unless allow_freeform is true"
        );
    }

    #[tokio::test]
    async fn ask_user_rejects_empty_labels() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = AskUserQuestionTool;
        let err = tool
            .call(
                &ctx,
                json!({
                    "prompt": "Pick one",
                    "options": [{ "label": "" }]
                }),
            )
            .await
            .expect_err("empty label");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments");
        };
        assert_eq!(message, "option labels cannot be empty");
    }

    #[tokio::test]
    async fn ask_user_requires_handler() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = AskUserQuestionTool;
        let err = tool
            .call(
                &ctx,
                json!({
                    "prompt": "Pick one",
                    "options": [{ "label": "A" }]
                }),
            )
            .await
            .expect_err("missing handler");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "question handler not configured");
    }

    #[tokio::test]
    async fn ask_user_returns_answer() {
        let temp = tempdir().expect("tempdir");
        let ctx = ToolContext {
            services: Arc::new(TurnServices {
                cwd: temp.path().to_path_buf(),
                workspace_root: temp.path().to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: None,
                question_handler: Some(Arc::new(DummyHandler)),
                permission_checker: None,
                tool_result_handler: None,
            }),
            ..base_context(temp.path())
        };
        let tool = AskUserQuestionTool;
        let result = tool
            .call(
                &ctx,
                json!({
                    "prompt": "Pick one",
                    "options": [{ "label": "A", "value": "alpha" }]
                }),
            )
            .await
            .expect("answer");
        assert_eq!(result["value"], "alpha");
        assert_eq!(result["label"], "label");
        assert_eq!(result["index"], 0);
    }
}
