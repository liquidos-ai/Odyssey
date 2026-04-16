//! Built-in tool for listing and loading skills.

use crate::builtins::utils::parse_args;
use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::ToolInputT;
use autoagents_derive::ToolInput;
use log::info;
use odyssey_rs_protocol::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Tool that lists and loads skill content.
#[derive(Debug, Default)]
pub struct SkillTool;

/// Arguments for SkillTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct SkillArgs {
    #[input(description = "Name of the skill to load. Omit to list available skills.")]
    #[serde(default)]
    name: Option<String>,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "Load or list available skills by name"
    }

    fn args_schema(&self) -> Value {
        let params_str = SkillArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let Some(provider) = ctx.services.skill_provider.as_ref() else {
            return Err(ToolError::ExecutionFailed(
                "skills are not enabled".to_string(),
            ));
        };
        let input: SkillArgs = parse_args(args)?;
        let name = input.name.as_deref();
        if let Some(name) = name {
            info!("loading skill (name={})", name);
            let content = provider.load(name).await?;
            return Ok(json!({
                "name": name,
                "content": content
            }));
        }
        info!("listing skills");
        let skills = provider
            .list()
            .into_iter()
            .map(|skill| {
                json!({
                    "name": skill.name,
                    "description": skill.description,
                    "path": skill.path.to_string_lossy().to_string()
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "skills": skills }))
    }
}

#[cfg(test)]
mod tests {
    use super::SkillTool;
    use crate::{Tool, ToolContext, TurnServices};
    use async_trait::async_trait;
    use odyssey_rs_protocol::{SkillProvider, SkillSummary, ToolError};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[derive(Default)]
    struct DummySkillProvider {
        skills: Vec<SkillSummary>,
    }

    #[async_trait]
    impl SkillProvider for DummySkillProvider {
        fn list(&self) -> Vec<SkillSummary> {
            self.skills.clone()
        }

        async fn load(&self, name: &str) -> Result<String, ToolError> {
            Ok(format!("content:{name}"))
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
    async fn skill_tool_requires_provider() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = SkillTool;
        let err = tool.call(&ctx, json!({})).await.expect_err("no provider");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "skills are not enabled");
    }

    #[tokio::test]
    async fn skill_tool_lists_skills() {
        let temp = tempdir().expect("tempdir");
        let provider = DummySkillProvider {
            skills: vec![SkillSummary {
                name: "alpha".to_string(),
                description: "desc".to_string(),
                path: PathBuf::from("/tmp/alpha.md"),
            }],
        };
        let ctx = ToolContext {
            services: Arc::new(TurnServices {
                cwd: temp.path().to_path_buf(),
                workspace_root: temp.path().to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: Some(Arc::new(provider)),
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
            ..base_context(temp.path())
        };
        let tool = SkillTool;
        let result = tool.call(&ctx, json!({})).await.expect("list");
        let skills = result["skills"].as_array().expect("skills");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["name"], "alpha");
    }

    #[tokio::test]
    async fn skill_tool_loads_content() {
        let temp = tempdir().expect("tempdir");
        let provider = DummySkillProvider::default();
        let ctx = ToolContext {
            services: Arc::new(TurnServices {
                cwd: temp.path().to_path_buf(),
                workspace_root: temp.path().to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: Some(Arc::new(provider)),
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
            ..base_context(temp.path())
        };
        let tool = SkillTool;
        let result = tool
            .call(&ctx, json!({ "name": "alpha" }))
            .await
            .expect("load");
        assert_eq!(result["name"], "alpha");
        assert_eq!(result["content"], "content:alpha");
    }
}
