use crate::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use odyssey_rs_sandbox::CommandSpec;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug)]
pub struct BashTool;

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }
    fn description(&self) -> &str {
        "Run a shell command"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["command"],"properties":{"command":{"type":"string"},"cwd":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: BashArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let tokens = shell_words::split(&input.command)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let (program, args) = tokens
            .split_first()
            .ok_or_else(|| ToolError::InvalidArguments("command cannot be empty".to_string()))?;
        let cwd = input
            .cwd
            .as_deref()
            .map(|path| ctx.resolve_workspace_path(path))
            .transpose()?
            .unwrap_or_else(|| ctx.working_dir.clone());
        let raw_command = PathBuf::from(program);
        let command = if raw_command.is_absolute() || raw_command.components().count() > 1 {
            let resolved = ctx.resolve_workspace_path(program)?;
            ctx.check_execute(&resolved)?;
            resolved
        } else {
            raw_command
        };
        let mut spec = CommandSpec::new(command);
        spec.args = args.to_vec();
        spec.cwd = Some(cwd);
        let output = ctx.run_command(self.name(), spec).await?;
        Ok(json!({
            "status_code": output.status_code,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "stdout_truncated": output.stdout_truncated,
            "stderr_truncated": output.stderr_truncated
        }))
    }
}
