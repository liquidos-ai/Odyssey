//! Built-in tool for running shell commands in the workspace.

use crate::builtins::utils::{ResolveMode, resolve_workspace_path};
use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::ToolInputT;
use autoagents_derive::ToolInput;
use chrono::Utc;
use log::{debug, info, warn};
use odyssey_rs_protocol::ToolError;
use odyssey_rs_protocol::{EventMsg, EventPayload, ExecStream};
use odyssey_rs_sandbox::{AccessMode, CommandOutputSink, CommandSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Serialize, Deserialize, ToolInput, Debug)]
#[serde(deny_unknown_fields)]
struct BashArgs {
    #[input(description = "The shell command to execute")]
    command: String,
    #[input(
        description = "Optional working directory for the command, If none the current directory will be used"
    )]
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Default)]
pub struct BashTool {}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Exeecute Bash commands using this tool"
    }

    fn args_schema(&self) -> Value {
        let params_str = BashArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool paramters")
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input = parse_bash_args(args)?;
        let (command_str, command_args) = parse_command_line(&input.command)?;
        info!(
            "executing command (args_len={}, has_cwd={})",
            command_args.len(),
            input.cwd.is_some(),
        );

        let cwd = match input.cwd.as_deref() {
            Some(cwd) => resolve_workspace_path(ctx, cwd, ResolveMode::Existing)?,
            None => ctx.services.cwd.clone(),
        };

        let raw_command = PathBuf::from(&command_str);
        let (command, check_execute) =
            if raw_command.components().count() > 1 || raw_command.is_absolute() {
                (
                    resolve_workspace_path(ctx, &command_str, ResolveMode::Existing)?,
                    true,
                )
            } else {
                (raw_command, false)
            };

        if check_execute {
            ctx.check_access(&command, AccessMode::Execute)?;
        }

        let mut argv = Vec::with_capacity(1 + command_args.len());
        argv.push(command_str.clone());
        argv.extend(command_args.iter().cloned());
        ctx.authorize_command(argv).await?;

        let mut spec = CommandSpec::new(command);
        spec.args = command_args;
        spec.cwd = Some(cwd);
        spec.env = BTreeMap::new(); //TODO: Replace with actual env later

        let sandbox = ctx.services.sandbox.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("sandbox provider not configured".to_string())
        })?;
        let result = if let (Some(turn_id), Some(sink)) = (
            ctx.turn_id,
            ctx.services.event_sink.as_ref().map(|sink| sink.as_ref()),
        ) {
            debug!("streaming command output");
            let exec_id = Uuid::new_v4();
            emit_exec_begin(ctx, sink, turn_id, exec_id, &command_str, &spec);
            let mut output_sink = ExecOutputSink {
                ctx,
                sink,
                turn_id,
                exec_id,
            };
            let result = sandbox
                .provider
                .run_command_streaming(&sandbox.handle, spec, &mut output_sink)
                .await
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
            emit_exec_end(ctx, sink, turn_id, exec_id, result.status_code);
            result
        } else {
            debug!("running command without streaming");
            sandbox
                .provider
                .run_command(&sandbox.handle, spec)
                .await
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
        };

        if result.status_code.unwrap_or(-1) != 0 {
            warn!("command finished with non-zero status");
        }
        Ok(json!({
            "status_code": result.status_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }))
    }
}

fn parse_bash_args(args: Value) -> Result<BashArgs, ToolError> {
    serde_json::from_value(args).map_err(|err| {
        let message = err.to_string();
        if message.contains("unknown field `args`") {
            return ToolError::InvalidArguments(
                "args is no longer supported; pass a single command string".to_string(),
            );
        }
        ToolError::InvalidArguments(message)
    })
}

fn parse_command_line(command: &str) -> Result<(String, Vec<String>), ToolError> {
    if command.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "command cannot be empty".to_string(),
        ));
    }
    let tokens =
        shell_words::split(command).map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
    let mut iter = tokens.into_iter();
    let Some(program) = iter.next() else {
        return Err(ToolError::InvalidArguments(
            "command cannot be empty".to_string(),
        ));
    };
    let args = iter.collect();
    Ok((program, args))
}

/// Output sink that streams command output via tool events.
struct ExecOutputSink<'a> {
    ctx: &'a ToolContext,
    sink: &'a dyn crate::EventSink,
    turn_id: Uuid,
    exec_id: Uuid,
}

impl CommandOutputSink for ExecOutputSink<'_> {
    /// Emit stdout chunks as streaming events.
    fn stdout(&mut self, chunk: &str) {
        emit_exec_delta(
            self.ctx,
            self.sink,
            self.turn_id,
            self.exec_id,
            ExecStream::Stdout,
            chunk,
        );
    }

    /// Emit stderr chunks as streaming events.
    fn stderr(&mut self, chunk: &str) {
        emit_exec_delta(
            self.ctx,
            self.sink,
            self.turn_id,
            self.exec_id,
            ExecStream::Stderr,
            chunk,
        );
    }
}

/// Emit a command begin event.
fn emit_exec_begin(
    ctx: &ToolContext,
    sink: &dyn crate::EventSink,
    turn_id: Uuid,
    exec_id: Uuid,
    command: &str,
    spec: &CommandSpec,
) {
    let mut argv = Vec::with_capacity(1 + spec.args.len());
    argv.push(command.to_string());
    argv.extend(spec.args.iter().cloned());
    let cwd = spec
        .cwd
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let event = EventMsg {
        id: Uuid::new_v4(),
        session_id: ctx.session_id,
        created_at: Utc::now(),
        payload: EventPayload::ExecCommandBegin {
            turn_id,
            exec_id,
            command: argv,
            cwd,
        },
    };
    sink.emit(event);
}

/// Emit a command output delta event.
fn emit_exec_delta(
    ctx: &ToolContext,
    sink: &dyn crate::EventSink,
    turn_id: Uuid,
    exec_id: Uuid,
    stream: ExecStream,
    delta: &str,
) {
    if delta.is_empty() {
        return;
    }
    let event = EventMsg {
        id: Uuid::new_v4(),
        session_id: ctx.session_id,
        created_at: Utc::now(),
        payload: EventPayload::ExecCommandOutputDelta {
            turn_id,
            exec_id,
            stream,
            delta: delta.to_string(),
        },
    };
    sink.emit(event);
}

/// Emit a command end event.
fn emit_exec_end(
    ctx: &ToolContext,
    sink: &dyn crate::EventSink,
    turn_id: Uuid,
    exec_id: Uuid,
    status_code: Option<i32>,
) {
    let event = EventMsg {
        id: Uuid::new_v4(),
        session_id: ctx.session_id,
        created_at: Utc::now(),
        payload: EventPayload::ExecCommandEnd {
            turn_id,
            exec_id,
            exit_code: status_code.unwrap_or(-1),
        },
    };
    sink.emit(event);
}

#[cfg(test)]
mod tests {
    use super::{BashTool, parse_bash_args, parse_command_line};
    use crate::{
        PermissionChecker, PermissionContext, PermissionOutcome, Tool, ToolContext, ToolSandbox,
        TurnServices,
    };
    use async_trait::async_trait;
    use odyssey_rs_protocol::{EventMsg, PermissionRequest, ToolError};
    use odyssey_rs_sandbox::{
        LocalSandboxProvider, SandboxContext, SandboxPolicy, SandboxProvider,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn parse_command_line_splits_args() {
        let (command, args) = parse_command_line("echo hello").expect("parse");
        assert_eq!(command, "echo");
        assert_eq!(args, vec!["hello".to_string()]);
    }

    #[test]
    fn parse_command_line_preserves_quotes() {
        let (command, args) = parse_command_line("echo \"hello world\"").expect("parse");
        assert_eq!(command, "echo");
        assert_eq!(args, vec!["hello world".to_string()]);
    }

    #[test]
    fn parse_bash_args_rejects_legacy_args() {
        let value = json!({
            "command": "echo hello",
            "args": ["world"]
        });
        let result = parse_bash_args(value);
        let ToolError::InvalidArguments(message) = result.expect_err("expected error") else {
            panic!("expected invalid arguments error");
        };
        assert_eq!(
            message,
            "args is no longer supported; pass a single command string"
        );
    }

    #[test]
    fn parse_command_line_rejects_empty() {
        let err = parse_command_line("   ").expect_err("empty");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments error");
        };
        assert_eq!(message, "command cannot be empty");
    }

    #[test]
    fn parse_bash_args_rejects_missing_command() {
        let value = json!({ "cwd": "src" });
        let err = parse_bash_args(value).expect_err("missing command");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments error");
        };
        assert_eq!(message.contains("command"), true);
    }

    struct AllowAllPermissions;

    #[async_trait]
    impl PermissionChecker for AllowAllPermissions {
        async fn authorize(
            &self,
            _ctx: &PermissionContext,
            _request: PermissionRequest,
        ) -> Result<PermissionOutcome, ToolError> {
            Ok(PermissionOutcome {
                allowed: true,
                reason: None,
            })
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: parking_lot::Mutex<VecDeque<EventMsg>>,
    }

    impl odyssey_rs_protocol::EventSink for RecordingSink {
        fn emit(&self, event: EventMsg) {
            self.events.lock().push_back(event);
        }
    }

    fn base_services(root: &std::path::Path) -> TurnServices {
        TurnServices {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            output_policy: None,
            sandbox: None,
            web: None,
            event_sink: None,
            skill_provider: None,
            question_handler: None,
            permission_checker: Some(Arc::new(AllowAllPermissions)),
            tool_result_handler: None,
        }
    }

    #[tokio::test]
    async fn bash_tool_errors_without_sandbox() {
        let workspace = tempdir().expect("workspace");
        let services = base_services(workspace.path());
        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };
        let tool = BashTool::default();
        let err = tool
            .call(&ctx, json!({ "command": "echo hello" }))
            .await
            .expect_err("missing sandbox");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "sandbox provider not configured");
    }

    #[tokio::test]
    async fn bash_tool_runs_with_streaming() {
        let workspace = tempdir().expect("workspace");
        let provider = LocalSandboxProvider::new();
        let sandbox_ctx = SandboxContext {
            workspace_root: workspace.path().to_path_buf(),
            mode: odyssey_rs_protocol::SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let handle = provider.prepare(&sandbox_ctx).await.expect("prepare");

        let sink = Arc::new(RecordingSink::default());
        let mut services = base_services(workspace.path());
        services.sandbox = Some(ToolSandbox {
            provider: Arc::new(provider),
            handle,
        });
        services.event_sink = Some(sink.clone());

        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: Some(Uuid::new_v4()),
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };

        let tool = BashTool::default();
        let result = tool
            .call(&ctx, json!({ "command": "printf hello", "cwd": "." }))
            .await
            .expect("call");
        assert_eq!(result["stdout"], "hello");

        let events = sink.events.lock();
        assert_eq!(events.is_empty(), false);
    }
}
