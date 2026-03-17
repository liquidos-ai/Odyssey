use crate::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug)]
pub struct SkillTool;

#[derive(Debug, Deserialize)]
struct SkillArgs {
    #[serde(default)]
    name: Option<String>,
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }
    fn description(&self) -> &str {
        "List or load bundled skills"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","properties":{"name":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: SkillArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let provider = ctx
            .skills
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("skills are not enabled".to_string()))?;
        if let Some(name) = input.name {
            let content = provider.load(&name)?;
            return Ok(json!({"name": name, "content": content}));
        }
        let skills = provider
            .list()
            .into_iter()
            .map(|skill| json!({"name": skill.name, "description": skill.description, "path": skill.path}))
            .collect::<Vec<_>>();
        Ok(json!({"skills": skills}))
    }
}
