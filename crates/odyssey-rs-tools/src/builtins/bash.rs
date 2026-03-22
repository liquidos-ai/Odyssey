use crate::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use odyssey_rs_sandbox::CommandSpec;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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
        "Run a sandboxed shell command"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["command"],"properties":{"command":{"type":"string"},"cwd":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: BashArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        if input.command.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "command cannot be empty".to_string(),
            ));
        }
        let permission_targets = permission_targets(&input.command)?;
        ctx.authorize_tool_with_targets(self.name(), &permission_targets)
            .await?;
        let cwd = input
            .cwd
            .as_deref()
            .map(|path| ctx.resolve_workspace_path(path))
            .transpose()?
            .unwrap_or_else(|| ctx.working_dir.clone());
        let shell = resolve_shell_path()?;
        ctx.check_execute(&shell)?;
        let mut spec = CommandSpec::new(shell);
        spec.args = vec!["-lc".to_string(), input.command.clone()];
        spec.cwd = Some(cwd);
        let output = ctx.run_command(self.name(), spec).await?;
        if output.status_code.unwrap_or_default() != 0 {
            return Err(ToolError::ExecutionFailed(format_command_failure(
                &input.command,
                output.status_code,
                &output.stderr,
                &output.stdout,
            )));
        }
        Ok(json!({
            "status_code": output.status_code,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "stdout_truncated": output.stdout_truncated,
            "stderr_truncated": output.stderr_truncated
        }))
    }
}

fn permission_targets(command: &str) -> Result<Vec<String>, ToolError> {
    let tokens = shell_words::split(command)
        .map_err(|err| ToolError::InvalidArguments(format!("invalid shell command: {err}")))?;
    if tokens.is_empty() {
        return Err(ToolError::InvalidArguments(
            "command cannot be empty".to_string(),
        ));
    }

    Ok((1..=tokens.len())
        .map(|prefix_len| {
            let head = tokens[..prefix_len].join(" ");
            let tail = tokens[prefix_len..].join(" ");
            format!("{head}:{tail}")
        })
        .collect())
}

fn resolve_shell_path() -> Result<PathBuf, ToolError> {
    let shell = which::which("sh")
        .or_else(|_| which::which("bash"))
        .map_err(|_| ToolError::ExecutionFailed("no system shell found in PATH".to_string()))?;
    canonicalize_shell_path(&shell)
}

fn canonicalize_shell_path(path: &Path) -> Result<PathBuf, ToolError> {
    path.canonicalize().map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to resolve {}: {err}", path.display()))
    })
}

fn format_command_failure(
    command: &str,
    status_code: Option<i32>,
    stderr: &str,
    stdout: &str,
) -> String {
    let mut message = format!(
        "command `{command}` exited with status {}",
        status_code.unwrap_or(-1)
    );
    let stderr = stderr.trim();
    let stdout = stdout.trim();
    if !stderr.is_empty() {
        message.push_str(&format!(": {stderr}"));
    } else if !stdout.is_empty() {
        message.push_str(&format!(": {stdout}"));
    }
    message
}
