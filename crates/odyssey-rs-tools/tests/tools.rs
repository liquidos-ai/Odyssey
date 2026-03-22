use async_trait::async_trait;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxError, SandboxHandle, SandboxProvider,
};
use odyssey_rs_tools::{
    PermissionAction, SkillEntry, SkillProvider, ToolApprovalHandler, ToolContext, ToolError,
    ToolEvent, ToolEventSink, ToolPermissionMatcher, ToolPermissionRule, ToolSandbox, ToolSpec,
    WorkspaceMount, builtin_registry,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use uuid::Uuid;

#[derive(Clone)]
struct FakeProvider {
    calls: Arc<Mutex<Vec<CommandSpec>>>,
    deny_paths: Arc<Mutex<Vec<String>>>,
    result_status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            deny_paths: Arc::new(Mutex::new(Vec::new())),
            result_status_code: Some(0),
            stdout: "line one".to_string(),
            stderr: "line two".to_string(),
        }
    }
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
            status_code: self.result_status_code,
            stdout: if self.stdout.is_empty() {
                format!("ran:{}:{}", spec.command.display(), spec.args.join(" "))
            } else {
                self.stdout.clone()
            },
            stderr: self.stderr.clone(),
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
        if !self.stdout.is_empty() {
            sink.stdout(&self.stdout);
        }
        if !self.stderr.is_empty() {
            sink.stderr(&self.stderr);
        }
        Ok(CommandResult {
            status_code: self.result_status_code,
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
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
    permission_rules: Vec<ToolPermissionRule>,
) -> ToolContext {
    test_context_with_mounts(bundle_root, provider, permission_rules, Vec::new())
}

fn test_context_with_mounts(
    bundle_root: &Path,
    provider: FakeProvider,
    permission_rules: Vec<ToolPermissionRule>,
    workspace_mounts: Vec<WorkspaceMount>,
) -> ToolContext {
    ToolContext {
        session_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        bundle_root: bundle_root.to_path_buf(),
        working_dir: bundle_root.to_path_buf(),
        workspace_mounts,
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
            "LS".to_string(),
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
    let rules = vec![
        ToolPermissionRule {
            action: PermissionAction::Ask,
            matcher: ToolPermissionMatcher::parse("Skill").expect("skill matcher"),
        },
        ToolPermissionRule {
            action: PermissionAction::Deny,
            matcher: ToolPermissionMatcher::parse("Denied").expect("deny matcher"),
        },
        ToolPermissionRule {
            action: PermissionAction::Ask,
            matcher: ToolPermissionMatcher::parse("Bash(cargo test:*)").expect("bash matcher"),
        },
    ];

    let mut ctx = test_context(temp.path(), provider.clone(), rules);
    ctx.event_sink = Some(events.clone());
    ctx.approval_handler = Some(approvals.clone());

    ctx.authorize_tool("Read").await.expect("default allow");
    ctx.authorize_tool("Skill").await.expect("approved");
    ctx.authorize_tool_with_targets("Bash", &["cargo test:-p odyssey-rs-tools".to_string()])
        .await
        .expect("approved granular bash");
    assert_eq!(
        approvals.requested.lock().expect("lock approvals").clone(),
        vec!["Skill".to_string(), "Bash(cargo test:*)".to_string()]
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
                "command": "echo hello world",
                "cwd": "."
            }),
        )
        .await
        .expect("bash call");

    assert_eq!(result["status_code"], json!(0));
    assert_eq!(result["stdout"], json!("line one"));
    assert_eq!(result["stderr"], json!("line two"));
    assert_eq!(provider.calls.lock().expect("lock calls").len(), 1);
    let recorded_call = provider.calls.lock().expect("lock calls")[0].clone();
    assert_eq!(
        recorded_call.args,
        vec!["-lc".to_string(), "echo hello world".to_string()]
    );
    assert!(
        matches!(
            recorded_call
                .command
                .file_name()
                .and_then(|name| name.to_str()),
            Some("sh" | "bash" | "dash")
        ),
        "expected shell command path, got {}",
        recorded_call.command.display()
    );

    let recorded = events.events.lock().expect("lock events");
    assert_eq!(recorded.len(), 4);
    assert_eq!(
        matches!(
            &recorded[0],
            ToolEvent::CommandStarted { tool, command, .. } if tool == "Bash"
                && matches!(
                    command
                        .first()
                        .and_then(|program| std::path::Path::new(program).file_name())
                        .and_then(|name| name.to_str()),
                    Some("sh" | "bash" | "dash")
                )
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
async fn bash_tool_returns_error_for_non_zero_shell_exit() {
    let temp = tempdir().expect("tempdir");
    let provider = FakeProvider {
        result_status_code: Some(1),
        stdout: String::default(),
        stderr: "Permission denied".to_string(),
        ..FakeProvider::default()
    };
    let ctx = test_context(temp.path(), provider, Vec::new());

    let bash = builtin_registry().get("Bash").expect("bash tool");
    let error = bash
        .call(
            &ctx,
            json!({
                "command": "echo hello > blocked.txt",
                "cwd": "."
            }),
        )
        .await
        .expect_err("non-zero shell exit should fail");

    assert_eq!(
        error.to_string(),
        "execution failed: command `echo hello > blocked.txt` exited with status 1: Permission denied"
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

    let ctx = test_context(temp.path(), FakeProvider::default(), Vec::new());
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

    let ls = call_tool(&registry, "LS", &ctx, json!({ "path": "docs" })).await;
    let glob = call_tool(&registry, "Glob", &ctx, json!({ "pattern": "docs/*.txt" })).await;
    let grep = call_tool(&registry, "Grep", &ctx, json!({ "pattern": "hello" })).await;

    assert_eq!(
        ls["entries"],
        json!([
            { "name": "new.txt", "path": "docs/new.txt", "type": "file" },
            { "name": "notes.txt", "path": "docs/notes.txt", "type": "file" }
        ])
    );
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
async fn filesystem_tools_use_mounted_filesystem_view() {
    let temp = tempdir().expect("tempdir");
    let bundle_root = temp.path().join("bundle");
    let host_mount = temp.path().join("host-project");
    let visible_mount = bundle_root.join("mount").join("read").join("current");

    fs::create_dir_all(&bundle_root).expect("create bundle root");
    fs::create_dir_all(host_mount.join("docs")).expect("create host docs");
    fs::write(
        host_mount.join("docs").join("notes.txt"),
        "hello from mount\n",
    )
    .expect("write mounted file");

    let ctx = test_context_with_mounts(
        &bundle_root,
        FakeProvider::default(),
        Vec::new(),
        vec![WorkspaceMount {
            visible_root: visible_mount,
            host_root: host_mount.clone(),
            writable: false,
        }],
    );
    let registry = builtin_registry();

    let read = call_tool(
        &registry,
        "Read",
        &ctx,
        json!({ "path": "mount/read/current/docs/notes.txt" }),
    )
    .await;
    let ls = call_tool(
        &registry,
        "LS",
        &ctx,
        json!({ "path": "mount/read/current/docs" }),
    )
    .await;
    let glob = call_tool(
        &registry,
        "Glob",
        &ctx,
        json!({ "pattern": "mount/read/current/**/*.txt" }),
    )
    .await;
    let grep = call_tool(&registry, "Grep", &ctx, json!({ "pattern": "hello" })).await;

    assert_eq!(read["content"], json!("hello from mount\n"));
    assert_eq!(
        ls["entries"],
        json!([{ "name": "notes.txt", "path": "mount/read/current/docs/notes.txt", "type": "file" }])
    );
    assert_eq!(
        glob["matches"],
        json!(["mount/read/current/docs/notes.txt"])
    );
    assert_eq!(
        grep["matches"],
        json!([{
            "path": "mount/read/current/docs/notes.txt",
            "line": 1,
            "text": "hello from mount"
        }])
    );
}

#[tokio::test]
async fn write_and_edit_tools_update_writable_mount_sources() {
    let temp = tempdir().expect("tempdir");
    let bundle_root = temp.path().join("bundle");
    let host_mount = temp.path().join("host-project");
    let visible_mount = bundle_root.join("mount").join("write").join("current");

    fs::create_dir_all(&bundle_root).expect("create bundle root");
    fs::create_dir_all(&host_mount).expect("create host mount");
    fs::write(host_mount.join("draft.txt"), "version one\n").expect("write host file");

    let ctx = test_context_with_mounts(
        &bundle_root,
        FakeProvider::default(),
        Vec::new(),
        vec![WorkspaceMount {
            visible_root: visible_mount,
            host_root: host_mount.clone(),
            writable: true,
        }],
    );
    let registry = builtin_registry();

    call_tool(
        &registry,
        "Write",
        &ctx,
        json!({
            "path": "mount/write/current/new.txt",
            "content": "mounted output"
        }),
    )
    .await;
    call_tool(
        &registry,
        "Edit",
        &ctx,
        json!({
            "path": "mount/write/current/draft.txt",
            "old_text": "one",
            "new_text": "two"
        }),
    )
    .await;

    assert_eq!(
        fs::read_to_string(host_mount.join("new.txt")).expect("read written mount file"),
        "mounted output"
    );
    assert_eq!(
        fs::read_to_string(host_mount.join("draft.txt")).expect("read edited mount file"),
        "version two\n"
    );
}

#[tokio::test]
async fn write_and_edit_tools_reject_read_only_mounts() {
    let temp = tempdir().expect("tempdir");
    let bundle_root = temp.path().join("bundle");
    let host_mount = temp.path().join("host-project");
    let visible_mount = bundle_root.join("mount").join("read").join("current");

    fs::create_dir_all(&bundle_root).expect("create bundle root");
    fs::create_dir_all(&host_mount).expect("create host mount");
    fs::write(host_mount.join("draft.txt"), "version one\n").expect("write host file");

    let ctx = test_context_with_mounts(
        &bundle_root,
        FakeProvider::default(),
        Vec::new(),
        vec![WorkspaceMount {
            visible_root: visible_mount,
            host_root: host_mount.clone(),
            writable: false,
        }],
    );
    let registry = builtin_registry();

    let write_error = registry
        .get("Write")
        .expect("write tool")
        .call(
            &ctx,
            json!({
                "path": "mount/read/current/new.txt",
                "content": "mounted output"
            }),
        )
        .await
        .expect_err("read-only mount should reject writes");
    let edit_error = registry
        .get("Edit")
        .expect("edit tool")
        .call(
            &ctx,
            json!({
                "path": "mount/read/current/draft.txt",
                "old_text": "one",
                "new_text": "two"
            }),
        )
        .await
        .expect_err("read-only mount should reject edits");

    assert_eq!(
        write_error.to_string(),
        format!(
            "permission denied: sandbox policy blocks Write access to {}",
            bundle_root
                .join("mount")
                .join("read")
                .join("current")
                .join("new.txt")
                .display()
        )
    );
    assert_eq!(
        edit_error.to_string(),
        format!(
            "permission denied: sandbox policy blocks Write access to {}",
            bundle_root
                .join("mount")
                .join("read")
                .join("current")
                .join("draft.txt")
                .display()
        )
    );
    assert!(!host_mount.join("new.txt").exists());
    assert_eq!(
        fs::read_to_string(host_mount.join("draft.txt")).expect("read unedited mount file"),
        "version one\n"
    );
}

#[tokio::test]
async fn filesystem_tools_respect_gitignore_in_workspace() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join("docs")).expect("create docs");
    fs::create_dir_all(temp.path().join("node_modules")).expect("create ignored dir");
    fs::write(
        temp.path().join(".gitignore"),
        "ignored.txt\nnode_modules/\n",
    )
    .expect("write gitignore");
    fs::write(
        temp.path().join("docs").join("notes.txt"),
        "hello visible\n",
    )
    .expect("write visible file");
    fs::write(temp.path().join("ignored.txt"), "secret\n").expect("write ignored file");
    fs::write(
        temp.path().join("node_modules").join("package.json"),
        "{\"name\":\"ignored\"}\n",
    )
    .expect("write ignored package");

    let ctx = test_context(temp.path(), FakeProvider::default(), Vec::new());
    let registry = builtin_registry();

    let ls = call_tool(&registry, "LS", &ctx, json!({})).await;
    let glob = call_tool(&registry, "Glob", &ctx, json!({ "pattern": "*.txt" })).await;
    let grep = call_tool(
        &registry,
        "Grep",
        &ctx,
        json!({ "pattern": "visible|secret" }),
    )
    .await;
    let read_visible =
        call_tool(&registry, "Read", &ctx, json!({ "path": "docs/notes.txt" })).await;

    assert_eq!(read_visible["content"], json!("hello visible\n"));
    assert_eq!(
        ls["entries"],
        json!([
            { "name": "docs", "path": "docs", "type": "dir" },
            { "name": ".gitignore", "path": ".gitignore", "type": "file" }
        ])
    );
    assert_eq!(glob["matches"], json!(["docs/notes.txt"]));
    assert_eq!(
        grep["matches"],
        json!([{
            "path": "docs/notes.txt",
            "line": 1,
            "text": "hello visible"
        }])
    );

    let read_ignored = registry
        .get("Read")
        .expect("read tool")
        .call(&ctx, json!({ "path": "ignored.txt" }))
        .await
        .expect_err("ignored file should be hidden");
    let ls_ignored = registry
        .get("LS")
        .expect("ls tool")
        .call(&ctx, json!({ "path": "node_modules" }))
        .await
        .expect_err("ignored directory should be hidden");

    assert_eq!(
        read_ignored.to_string(),
        "permission denied: path `ignored.txt` is ignored by .gitignore"
    );
    assert_eq!(
        ls_ignored.to_string(),
        "permission denied: path `node_modules` is ignored by .gitignore"
    );
}

#[tokio::test]
async fn filesystem_tools_respect_gitignore_in_mounts() {
    let temp = tempdir().expect("tempdir");
    let bundle_root = temp.path().join("bundle");
    let host_mount = temp.path().join("host-project");
    let visible_mount = bundle_root.join("mount").join("write").join("current");

    fs::create_dir_all(&bundle_root).expect("create bundle root");
    fs::create_dir_all(host_mount.join("src")).expect("create visible dir");
    fs::create_dir_all(host_mount.join("target")).expect("create ignored dir");
    fs::write(host_mount.join(".gitignore"), "ignored.txt\ntarget/\n").expect("write gitignore");
    fs::write(host_mount.join("src").join("main.rs"), "fn main() {}\n").expect("write source");
    fs::write(host_mount.join("ignored.txt"), "skip me\n").expect("write ignored file");
    fs::write(
        host_mount.join("target").join("output.txt"),
        "skip me too\n",
    )
    .expect("write ignored target file");

    let ctx = test_context_with_mounts(
        &bundle_root,
        FakeProvider::default(),
        Vec::new(),
        vec![WorkspaceMount {
            visible_root: visible_mount,
            host_root: host_mount.clone(),
            writable: true,
        }],
    );
    let registry = builtin_registry();

    let ls = call_tool(
        &registry,
        "LS",
        &ctx,
        json!({ "path": "mount/write/current" }),
    )
    .await;
    let ls_root = call_tool(&registry, "LS", &ctx, json!({})).await;
    let ls_mount = call_tool(&registry, "LS", &ctx, json!({ "path": "mount" })).await;
    let ls_mount_write = call_tool(&registry, "LS", &ctx, json!({ "path": "mount/write" })).await;
    let glob = call_tool(
        &registry,
        "Glob",
        &ctx,
        json!({ "pattern": "mount/write/current/**/*.txt" }),
    )
    .await;

    assert_eq!(
        ls["entries"],
        json!([
            { "name": "src", "path": "mount/write/current/src", "type": "dir" },
            { "name": ".gitignore", "path": "mount/write/current/.gitignore", "type": "file" }
        ])
    );
    assert_eq!(
        ls_root["entries"],
        json!([{ "name": "mount", "path": "mount", "type": "dir" }])
    );
    assert_eq!(
        ls_mount["entries"],
        json!([{ "name": "write", "path": "mount/write", "type": "dir" }])
    );
    assert_eq!(
        ls_mount_write["entries"],
        json!([{ "name": "current", "path": "mount/write/current", "type": "dir" }])
    );
    assert_eq!(glob["matches"], json!([]));

    let read_ignored = registry
        .get("Read")
        .expect("read tool")
        .call(&ctx, json!({ "path": "mount/write/current/ignored.txt" }))
        .await
        .expect_err("ignored mount file should be hidden");
    assert_eq!(
        read_ignored.to_string(),
        "permission denied: path `mount/write/current/ignored.txt` is ignored by .gitignore"
    );
}

#[tokio::test]
async fn skill_tool_and_invalid_grep_are_handled() {
    let temp = tempdir().expect("tempdir");
    let rules = vec![ToolPermissionRule {
        action: PermissionAction::Allow,
        matcher: ToolPermissionMatcher::parse("Skill").expect("skill matcher"),
    }];
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

#[tokio::test]
async fn granular_bash_permissions_match_prefixes_and_wildcards() {
    let temp = tempdir().expect("tempdir");
    let approvals = Arc::new(RecordingApproval::default());
    let rules = vec![
        ToolPermissionRule {
            action: PermissionAction::Ask,
            matcher: ToolPermissionMatcher::parse("Bash(find:*)").expect("find matcher"),
        },
        ToolPermissionRule {
            action: PermissionAction::Deny,
            matcher: ToolPermissionMatcher::parse("Bash(cargo build:*)")
                .expect("cargo build matcher"),
        },
    ];
    let mut ctx = test_context(temp.path(), FakeProvider::default(), rules);
    ctx.approval_handler = Some(approvals.clone());

    ctx.authorize_tool_with_targets("Bash", &["find:./src -name *.rs".to_string()])
        .await
        .expect("find should request and pass");
    assert_eq!(
        approvals.requested.lock().expect("lock approvals").clone(),
        vec!["Bash(find:*)".to_string()]
    );

    let error = ctx
        .authorize_tool_with_targets("Bash", &["cargo build:--workspace".to_string()])
        .await
        .expect_err("cargo build denied");
    assert_eq!(
        error.to_string(),
        "permission denied: tool Bash(cargo build:*) is denied"
    );

    ctx.authorize_tool_with_targets("Bash", &["cargo test:-p odyssey-rs-tools".to_string()])
        .await
        .expect("cargo test allowed by default");
}

#[tokio::test]
async fn granular_bash_allow_overrides_generic_bash_ask() {
    let temp = tempdir().expect("tempdir");
    let approvals = Arc::new(RecordingApproval::default());
    let rules = vec![
        ToolPermissionRule {
            action: PermissionAction::Allow,
            matcher: ToolPermissionMatcher::parse("Bash(curl:*)").expect("curl matcher"),
        },
        ToolPermissionRule {
            action: PermissionAction::Ask,
            matcher: ToolPermissionMatcher::parse("Bash").expect("bash matcher"),
        },
    ];
    let mut ctx = test_context(temp.path(), FakeProvider::default(), rules);
    ctx.approval_handler = Some(approvals.clone());

    ctx.authorize_tool_with_targets("Bash", &["curl:-s asdfa".to_string()])
        .await
        .expect("specific curl allow should bypass generic ask");

    assert!(
        approvals
            .requested
            .lock()
            .expect("lock approvals")
            .is_empty()
    );
}

#[tokio::test]
async fn granular_bash_deny_overrides_generic_bash_allow() {
    let temp = tempdir().expect("tempdir");
    let rules = vec![
        ToolPermissionRule {
            action: PermissionAction::Allow,
            matcher: ToolPermissionMatcher::parse("Bash").expect("bash matcher"),
        },
        ToolPermissionRule {
            action: PermissionAction::Deny,
            matcher: ToolPermissionMatcher::parse("Bash(curl:*)").expect("curl matcher"),
        },
    ];
    let ctx = test_context(temp.path(), FakeProvider::default(), rules);

    let error = ctx
        .authorize_tool_with_targets("Bash", &["curl:-s asdfa".to_string()])
        .await
        .expect_err("specific curl deny should override generic allow");

    assert_eq!(
        error.to_string(),
        "permission denied: tool Bash(curl:*) is denied"
    );
}

#[tokio::test]
async fn bash_tool_applies_granular_permission_targets() {
    let temp = tempdir().expect("tempdir");
    let provider = FakeProvider::default();
    let rules = vec![ToolPermissionRule {
        action: PermissionAction::Deny,
        matcher: ToolPermissionMatcher::parse("Bash(cargo build:*)").expect("cargo build matcher"),
    }];
    let ctx = test_context(temp.path(), provider.clone(), rules);

    let bash = builtin_registry().get("Bash").expect("bash tool");
    let error = bash
        .call(
            &ctx,
            json!({
                "command": "cargo build --workspace",
                "cwd": "."
            }),
        )
        .await
        .expect_err("granular permission should deny bash command");

    assert_eq!(
        error.to_string(),
        "permission denied: tool Bash(cargo build:*) is denied"
    );
    assert!(provider.calls.lock().expect("lock calls").is_empty());
}

#[test]
fn tool_permission_matcher_parses_exact_and_granular_values() {
    assert_eq!(
        ToolPermissionMatcher::parse("Read").expect("exact matcher"),
        ToolPermissionMatcher {
            tool: "Read".to_string(),
            target: None,
        }
    );
    assert_eq!(
        ToolPermissionMatcher::parse("Bash(cargo test:*)").expect("granular matcher"),
        ToolPermissionMatcher {
            tool: "Bash".to_string(),
            target: Some("cargo test:*".to_string()),
        }
    );
}
