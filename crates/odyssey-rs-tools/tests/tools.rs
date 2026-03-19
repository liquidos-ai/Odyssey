use async_trait::async_trait;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxError, SandboxHandle, SandboxProvider,
};
use odyssey_rs_tools::{
    PermissionAction, SkillEntry, SkillProvider, ToolApprovalHandler, ToolContext, ToolError,
    ToolEvent, ToolEventSink, ToolSandbox, ToolSpec, builtin_registry,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use uuid::Uuid;

#[derive(Clone, Default)]
struct FakeProvider {
    calls: Arc<Mutex<Vec<CommandSpec>>>,
    deny_paths: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SandboxProvider for FakeProvider {
    async fn prepare(&self, _ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        Ok(SandboxHandle { id: Uuid::new_v4() })
    }

    async fn run_command(
        &self,
        _handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError> {
        self.calls.lock().expect("lock calls").push(spec.clone());
        Ok(CommandResult {
            status_code: Some(0),
            stdout: format!("ran:{}:{}", spec.command.display(), spec.args.join(" ")),
            stderr: String::default(),
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }

    async fn run_command_streaming(
        &self,
        _handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        self.calls.lock().expect("lock calls").push(spec.clone());
        sink.stdout("line one");
        sink.stderr("line two");
        Ok(CommandResult {
            status_code: Some(0),
            stdout: "line one".to_string(),
            stderr: "line two".to_string(),
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }

    fn check_access(
        &self,
        _handle: &SandboxHandle,
        path: &Path,
        _mode: AccessMode,
    ) -> AccessDecision {
        let denied = self.deny_paths.lock().expect("lock deny paths");
        if denied
            .iter()
            .any(|fragment| path.to_string_lossy().contains(fragment))
        {
            AccessDecision::Deny(format!("blocked {}", path.display()))
        } else {
            AccessDecision::Allow
        }
    }

    async fn shutdown(&self, _handle: SandboxHandle) {}
}

#[derive(Default)]
struct RecordingEvents {
    events: Mutex<Vec<ToolEvent>>,
}

impl ToolEventSink for RecordingEvents {
    fn emit(&self, event: ToolEvent) {
        self.events.lock().expect("lock events").push(event);
    }
}

#[derive(Default)]
struct RecordingApproval {
    requested: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolApprovalHandler for RecordingApproval {
    async fn request_tool_approval(&self, tool: &str) -> Result<(), ToolError> {
        self.requested
            .lock()
            .expect("lock approvals")
            .push(tool.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct FakeSkills;

impl SkillProvider for FakeSkills {
    fn list(&self) -> Vec<SkillEntry> {
        vec![SkillEntry {
            name: "repo-hygiene".to_string(),
            description: "Keep repositories clean".to_string(),
            path: PathBuf::from("/skills/repo-hygiene/SKILL.md"),
        }]
    }

    fn load(&self, name: &str) -> Result<String, ToolError> {
        Ok(format!("# {name}\n"))
    }
}

fn test_context(
    bundle_root: &Path,
    provider: FakeProvider,
    permission_rules: HashMap<String, PermissionAction>,
) -> ToolContext {
    ToolContext {
        session_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        bundle_root: bundle_root.to_path_buf(),
        working_dir: bundle_root.to_path_buf(),
        sandbox: ToolSandbox {
            provider: Arc::new(provider),
            handle: SandboxHandle { id: Uuid::new_v4() },
            lease: None,
        },
        permission_rules,
        event_sink: None,
        approval_handler: None,
        skills: None,
    }
}

async fn call_tool(
    registry: &odyssey_rs_tools::ToolRegistry,
    name: &str,
    ctx: &ToolContext,
    args: serde_json::Value,
) -> serde_json::Value {
    registry
        .get(name)
        .expect("tool exists")
        .call(ctx, args)
        .await
        .expect("tool call succeeds")
}

#[test]
fn builtin_registry_exposes_expected_tools() {
    let registry = builtin_registry();
    let mut names = registry.names();
    names.sort();

    assert_eq!(
        names,
        vec![
            "Bash".to_string(),
            "Edit".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "Read".to_string(),
            "Skill".to_string(),
            "Write".to_string(),
        ]
    );

    let mut specs = registry
        .specs()
        .into_iter()
        .map(|spec: ToolSpec| spec.name)
        .collect::<Vec<_>>();
    specs.sort();
    assert_eq!(specs, names);
}

#[tokio::test]
async fn authorization_and_command_events_are_recorded() {
    let temp = tempdir().expect("tempdir");
    let provider = FakeProvider::default();
    let events = Arc::new(RecordingEvents::default());
    let approvals = Arc::new(RecordingApproval::default());
    let mut rules = HashMap::new();
    rules.insert("Skill".to_string(), PermissionAction::Ask);
    rules.insert("Denied".to_string(), PermissionAction::Deny);

    let mut ctx = test_context(temp.path(), provider.clone(), rules);
    ctx.event_sink = Some(events.clone());
    ctx.approval_handler = Some(approvals.clone());

    ctx.authorize_tool("Read").await.expect("default allow");
    ctx.authorize_tool("Skill").await.expect("approved");
    assert_eq!(
        approvals.requested.lock().expect("lock approvals").clone(),
        vec!["Skill".to_string()]
    );
    assert_eq!(
        ctx.authorize_tool("Denied")
            .await
            .expect_err("denied tool should error")
            .to_string(),
        "permission denied: tool Denied is denied"
    );

    let bash = builtin_registry().get("Bash").expect("bash tool");
    let result = bash
        .call(
            &ctx,
            json!({
                "command": "/bin/echo hello world",
                "cwd": "."
            }),
        )
        .await
        .expect("bash call");

    assert_eq!(result["status_code"], json!(0));
    assert_eq!(result["stdout"], json!("line one"));
    assert_eq!(result["stderr"], json!("line two"));
    assert_eq!(provider.calls.lock().expect("lock calls").len(), 1);

    let recorded = events.events.lock().expect("lock events");
    assert_eq!(recorded.len(), 4);
    assert_eq!(
        matches!(
            &recorded[0],
            ToolEvent::CommandStarted { tool, command, .. }
                if tool == "Bash" && command[0] == "/bin/echo"
        ),
        true
    );
    assert_eq!(
        matches!(
            &recorded[1],
            ToolEvent::CommandStdout { tool, line, .. } if tool == "Bash" && line == "line one"
        ),
        true
    );
    assert_eq!(
        matches!(
            &recorded[2],
            ToolEvent::CommandStderr { tool, line, .. } if tool == "Bash" && line == "line two"
        ),
        true
    );
    assert_eq!(
        matches!(
            &recorded[3],
            ToolEvent::CommandFinished { tool, status, .. } if tool == "Bash" && *status == 0
        ),
        true
    );
}

#[tokio::test]
async fn filesystem_tools_round_trip_and_search() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join("docs")).expect("create docs");
    fs::write(
        temp.path().join("docs").join("notes.txt"),
        "hello world\nhello odyssey\n",
    )
    .expect("write file");

    let ctx = test_context(temp.path(), FakeProvider::default(), HashMap::new());
    let registry = builtin_registry();

    let read = call_tool(&registry, "Read", &ctx, json!({ "path": "docs/notes.txt" })).await;
    assert_eq!(read["content"], json!("hello world\nhello odyssey\n"));

    let write = call_tool(
        &registry,
        "Write",
        &ctx,
        json!({
            "path": "docs/new.txt",
            "content": "new file"
        }),
    )
    .await;
    assert_eq!(write["bytes"], json!(8));

    let edit = call_tool(
        &registry,
        "Edit",
        &ctx,
        json!({
            "path": "docs/new.txt",
            "old_text": "new",
            "new_text": "updated"
        }),
    )
    .await;
    assert_eq!(edit["edited"], json!(true));
    assert_eq!(
        fs::read_to_string(temp.path().join("docs").join("new.txt")).expect("read edited file"),
        "updated file"
    );

    let glob = call_tool(&registry, "Glob", &ctx, json!({ "pattern": "docs/*.txt" })).await;
    let grep = call_tool(&registry, "Grep", &ctx, json!({ "pattern": "hello" })).await;

    assert_eq!(glob["matches"], json!(["docs/new.txt", "docs/notes.txt"]));
    assert_eq!(grep["matches"].as_array().expect("grep matches").len(), 2);

    let edit_error = registry
        .get("Edit")
        .expect("edit tool")
        .call(
            &ctx,
            json!({
                "path": "docs/new.txt",
                "old_text": "missing",
                "new_text": "value"
            }),
        )
        .await
        .expect_err("edit should fail without old text");
    assert_eq!(
        edit_error.to_string(),
        "execution failed: old_text not found"
    );
}

#[tokio::test]
async fn skill_tool_and_invalid_grep_are_handled() {
    let temp = tempdir().expect("tempdir");
    let mut rules = HashMap::new();
    rules.insert("Skill".to_string(), PermissionAction::Allow);
    let mut ctx = test_context(temp.path(), FakeProvider::default(), rules);
    ctx.skills = Some(Arc::new(FakeSkills));

    let registry = builtin_registry();
    let listed = call_tool(&registry, "Skill", &ctx, json!({})).await;
    let loaded = call_tool(&registry, "Skill", &ctx, json!({ "name": "repo-hygiene" })).await;

    assert_eq!(listed["skills"][0]["name"], json!("repo-hygiene"));
    assert_eq!(loaded["content"], json!("# repo-hygiene\n"));

    let grep_error = registry
        .get("Grep")
        .expect("grep tool")
        .call(&ctx, json!({ "pattern": "[" }))
        .await
        .expect_err("invalid regex should fail");
    assert!(matches!(grep_error, ToolError::InvalidArguments(_)));
}
