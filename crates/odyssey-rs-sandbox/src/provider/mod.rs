//! Sandbox provider traits and shared helpers.

use async_trait::async_trait;
use log::{debug, info, warn};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::SandboxError;
use crate::types::{
    AccessDecision, AccessMode, CommandResult, CommandSpec, SandboxContext, SandboxHandle,
    SandboxLimits, SandboxNetworkMode, SandboxPolicy,
};
use odyssey_rs_protocol::SandboxMode;

#[cfg(target_os = "linux")]
pub mod linux;
// pub mod noop;
pub mod local;

/// Report of missing dependencies for a sandbox provider.
#[derive(Debug, Default)]
pub struct DependencyReport {
    /// Hard errors preventing provider use.
    pub errors: Vec<String>,
    /// Warnings that may degrade functionality.
    pub warnings: Vec<String>,
}

/// Sandbox provider interface.
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    /// Prepare a sandbox for execution.
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError>;

    /// Run a command in the sandbox, capturing output.
    async fn run_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError>;

    /// Run a command in the sandbox with streaming output.
    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError>;

    /// Check access to a path within the sandbox.
    fn check_access(&self, handle: &SandboxHandle, path: &Path, mode: AccessMode)
    -> AccessDecision;

    /// Return a dependency report for the provider.
    fn dependency_report(&self) -> DependencyReport {
        DependencyReport::default()
    }

    /// Shutdown and release sandbox resources.
    async fn shutdown(&self, handle: SandboxHandle);
}

/// Streaming output sink for sandboxed commands.
pub trait CommandOutputSink: Send {
    /// Handle stdout chunk.
    fn stdout(&mut self, chunk: &str);
    /// Handle stderr chunk.
    fn stderr(&mut self, chunk: &str);
}

/// Mount specification for sandbox environments.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Mount {
    /// Source path on the host.
    source: PathBuf,
    /// Target path inside the sandbox.
    target: PathBuf,
    /// Whether the mount is writable.
    writable: bool,
}

/// Fully prepared sandbox execution plan.
#[derive(Debug, Clone)]
pub struct PreparedSandbox {
    /// Access policy derived from config.
    access: AccessPolicy,
    /// Environment variables to inject.
    env: BTreeMap<String, String>,
    /// Resource limits.
    #[allow(dead_code)]
    limits: SandboxLimits,
    /// Network policy.
    #[allow(dead_code)]
    network: SandboxNetworkMode,
    /// Default working directory.
    working_dir: PathBuf,
    /// Mount list for the sandbox.
    #[allow(dead_code)]
    mounts: Vec<Mount>,
}

/// Default access scope for paths without explicit rules.
#[derive(Debug, Clone, Copy)]
enum DefaultScope {
    None,
    WorkspaceOnly,
    All,
}

/// Allow/deny rules for a specific access mode.
#[derive(Debug, Clone)]
struct AccessRules {
    allow: Vec<PathBuf>,
    deny: Vec<PathBuf>,
    default_scope: DefaultScope,
}

/// Aggregated access policy for read/write/exec.
#[derive(Debug, Clone)]
struct AccessPolicy {
    workspace_root: PathBuf,
    read: AccessRules,
    write: AccessRules,
    exec: AccessRules,
}

impl AccessPolicy {
    /// Build access policy from sandbox mode and config.
    fn new(
        mode: SandboxMode,
        policy: &SandboxPolicy,
        workspace_root: &Path,
    ) -> Result<Self, SandboxError> {
        let workspace_root = normalize_path(workspace_root);
        let default_read = match mode {
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => DefaultScope::WorkspaceOnly,
            SandboxMode::DangerFullAccess => DefaultScope::All,
        };
        let default_write = match mode {
            SandboxMode::ReadOnly => DefaultScope::None,
            SandboxMode::WorkspaceWrite => DefaultScope::WorkspaceOnly,
            SandboxMode::DangerFullAccess => DefaultScope::All,
        };
        let default_exec = match mode {
            SandboxMode::ReadOnly => DefaultScope::None,
            SandboxMode::WorkspaceWrite => DefaultScope::WorkspaceOnly,
            SandboxMode::DangerFullAccess => DefaultScope::All,
        };
        let read = AccessRules {
            allow: normalize_patterns(&workspace_root, &policy.filesystem.allow_read)?,
            deny: normalize_patterns(&workspace_root, &policy.filesystem.deny_read)?,
            default_scope: default_read,
        };
        let write = AccessRules {
            allow: normalize_patterns(&workspace_root, &policy.filesystem.allow_write)?,
            deny: normalize_patterns(&workspace_root, &policy.filesystem.deny_write)?,
            default_scope: default_write,
        };
        let exec = AccessRules {
            allow: normalize_patterns(&workspace_root, &policy.filesystem.allow_exec)?,
            deny: normalize_patterns(&workspace_root, &policy.filesystem.deny_exec)?,
            default_scope: default_exec,
        };
        Ok(Self {
            workspace_root,
            read,
            write,
            exec,
        })
    }

    /// Check access against allow/deny rules.
    fn check(&self, path: &Path, mode: AccessMode) -> AccessDecision {
        let path = if path.is_absolute() {
            normalize_path(path)
        } else {
            normalize_path(&self.workspace_root.join(path))
        };
        let rules = match mode {
            AccessMode::Read => &self.read,
            AccessMode::Write => &self.write,
            AccessMode::Execute => &self.exec,
        };
        if matches_any(&path, &rules.deny) {
            return AccessDecision::Deny(format!(
                "access denied by sandbox policy: {}",
                path.display()
            ));
        }
        if !rules.allow.is_empty() {
            if matches_any(&path, &rules.allow) {
                return AccessDecision::Allow;
            }
            return AccessDecision::Deny(format!(
                "access not permitted by sandbox allowlist: {}",
                path.display()
            ));
        }
        match rules.default_scope {
            DefaultScope::All => AccessDecision::Allow,
            DefaultScope::WorkspaceOnly => {
                if path.starts_with(&self.workspace_root) {
                    AccessDecision::Allow
                } else {
                    AccessDecision::Deny(format!("path outside workspace root: {}", path.display()))
                }
            }
            DefaultScope::None => AccessDecision::Deny(format!(
                "sandbox mode blocks this access: {}",
                path.display()
            )),
        }
    }
}

/// Normalize path patterns into absolute paths.
fn normalize_patterns(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>, SandboxError> {
    let mut resolved = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            return Err(SandboxError::InvalidConfig(format!(
                "glob patterns are not supported in sandbox paths: {pattern}"
            )));
        }
        let path = PathBuf::from(pattern);
        let joined = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        resolved.push(normalize_path(&joined));
    }
    Ok(resolved)
}

/// Check whether a path matches any prefix pattern.
fn matches_any(path: &Path, patterns: &[PathBuf]) -> bool {
    patterns.iter().any(|pattern| path.starts_with(pattern))
}

/// Normalize a path by resolving components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{
        AccessPolicy, bind_if_exists, build_env, build_mounts, build_prepared_sandbox,
        command_display, matches_any, network_mode, normalize_path, normalize_patterns,
        run_local_process,
    };
    use crate::{AccessDecision, AccessMode, CommandSpec, SandboxNetworkMode, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn read_only_mode_allows_read_but_denies_write_exec() {
        let temp = tempdir().expect("tempdir");
        let policy = SandboxPolicy::default();
        let access =
            AccessPolicy::new(SandboxMode::ReadOnly, &policy, temp.path()).expect("access policy");
        let path = temp.path().join("file.txt");

        assert_eq!(access.check(&path, AccessMode::Read), AccessDecision::Allow);
        assert!(matches!(
            access.check(&path, AccessMode::Write),
            AccessDecision::Deny(_)
        ));
        assert!(matches!(
            access.check(&path, AccessMode::Execute),
            AccessDecision::Deny(_)
        ));
    }

    #[test]
    fn workspace_write_allows_within_workspace() {
        let temp = tempdir().expect("tempdir");
        let policy = SandboxPolicy::default();
        let access = AccessPolicy::new(SandboxMode::WorkspaceWrite, &policy, temp.path())
            .expect("access policy");
        let path = temp.path().join("bin");

        assert_eq!(access.check(&path, AccessMode::Read), AccessDecision::Allow);
        assert_eq!(
            access.check(&path, AccessMode::Write),
            AccessDecision::Allow
        );
        assert_eq!(
            access.check(&path, AccessMode::Execute),
            AccessDecision::Allow
        );
    }

    #[test]
    fn deny_rules_override_allow_rules() {
        let temp = tempdir().expect("tempdir");
        let mut policy = SandboxPolicy::default();
        let denied = temp.path().join("blocked");
        policy
            .filesystem
            .allow_read
            .push(denied.to_string_lossy().to_string());
        policy
            .filesystem
            .deny_read
            .push(denied.to_string_lossy().to_string());

        let access = AccessPolicy::new(SandboxMode::WorkspaceWrite, &policy, temp.path())
            .expect("access policy");
        assert!(matches!(
            access.check(&denied, AccessMode::Read),
            AccessDecision::Deny(_)
        ));
    }

    #[test]
    fn normalize_patterns_rejects_globs() {
        let temp = tempdir().expect("tempdir");
        let err = normalize_patterns(temp.path(), &["/tmp/*.txt".to_string()])
            .expect_err("glob rejected");
        match err {
            crate::SandboxError::InvalidConfig(message) => {
                assert!(message.contains("glob patterns"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn normalize_path_resolves_components() {
        let path = Path::new("/tmp/dir/../file.txt");
        let normalized = normalize_path(path);
        assert_eq!(normalized, PathBuf::from("/tmp/file.txt"));
    }

    #[test]
    fn matches_any_checks_prefixes() {
        let path = PathBuf::from("/tmp/data/file.txt");
        let patterns = vec![PathBuf::from("/tmp/data")];
        assert_eq!(matches_any(&path, &patterns), true);
    }

    #[test]
    fn command_display_resolves_relative_paths() {
        let display = command_display(Path::new("bin/run"), Path::new("/tmp")).expect("display");
        assert_eq!(display, "/tmp/bin/run".to_string());
    }

    #[test]
    fn bind_if_exists_adds_flag_when_present() {
        let temp = tempdir().expect("tempdir");
        let mut args = Vec::new();
        bind_if_exists(&mut args, "--ro-bind", temp.path(), temp.path());
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--ro-bind");
    }

    #[test]
    fn build_env_includes_set_values() {
        let mut policy = SandboxPolicy::default();
        policy
            .env
            .set
            .insert("ODYSSEY_TEST".to_string(), "1".to_string());
        let env = build_env(&policy);
        assert_eq!(env.get("ODYSSEY_TEST"), Some(&"1".to_string()));
    }

    #[test]
    fn network_mode_defaults_to_allow() {
        let policy = SandboxPolicy::default();
        assert_eq!(network_mode(&policy), SandboxNetworkMode::Allow);
    }

    #[test]
    fn network_mode_denies_when_deny_listed() {
        let mut policy = SandboxPolicy::default();
        policy.network.deny_domains.push("example.com".to_string());
        assert_eq!(network_mode(&policy), SandboxNetworkMode::Deny);
    }

    #[test]
    fn build_mounts_includes_external_overrides() {
        let workspace = tempdir().expect("workspace");
        let external_read = tempdir().expect("external_read");
        let external_write = tempdir().expect("external_write");

        let mut policy = SandboxPolicy::default();
        policy
            .filesystem
            .allow_read
            .push(external_read.path().to_string_lossy().to_string());
        policy
            .filesystem
            .allow_write
            .push(external_write.path().to_string_lossy().to_string());

        let mounts =
            build_mounts(SandboxMode::WorkspaceWrite, &policy, workspace.path()).expect("mounts");
        assert_eq!(mounts.len(), 3);

        let read_mount = mounts
            .iter()
            .find(|mount| mount.source == normalize_path(external_read.path()))
            .expect("read mount");
        assert_eq!(read_mount.writable, false);

        let write_mount = mounts
            .iter()
            .find(|mount| mount.source == normalize_path(external_write.path()))
            .expect("write mount");
        assert_eq!(write_mount.writable, true);
    }

    #[test]
    fn build_mounts_rejects_missing_paths() {
        let workspace = tempdir().expect("workspace");
        let external = tempdir().expect("external");
        let missing = external.path().join("missing");

        let mut policy = SandboxPolicy::default();
        policy
            .filesystem
            .allow_read
            .push(missing.to_string_lossy().to_string());

        let err = build_mounts(SandboxMode::WorkspaceWrite, &policy, workspace.path())
            .expect_err("missing path");
        match err {
            crate::SandboxError::InvalidConfig(message) => {
                assert_eq!(message.contains("does not exist"), true);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn build_prepared_sandbox_uses_network_and_env() {
        let workspace = tempdir().expect("workspace");
        let mut policy = SandboxPolicy::default();
        policy
            .env
            .set
            .insert("ODYSSEY_ENV".to_string(), "yes".to_string());
        policy.network.deny_domains.push("example.com".to_string());

        let ctx = crate::SandboxContext {
            workspace_root: workspace.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy,
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        assert_eq!(prepared.network, SandboxNetworkMode::Deny);
        assert_eq!(prepared.env.get("ODYSSEY_ENV"), Some(&"yes".to_string()));
        assert_eq!(prepared.working_dir, normalize_path(workspace.path()));
        assert_eq!(prepared.mounts.is_empty(), false);
    }

    #[tokio::test]
    async fn run_local_process_captures_output() {
        let workspace = tempdir().expect("workspace");
        let ctx = crate::SandboxContext {
            workspace_root: workspace.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");

        let mut spec = CommandSpec::new("sh");
        spec.args.extend([
            "-c".to_string(),
            "printf 'out'; printf 'err' 1>&2".to_string(),
        ]);

        #[derive(Default)]
        struct RecordingSink {
            stdout: String,
            stderr: String,
        }

        impl super::CommandOutputSink for RecordingSink {
            fn stdout(&mut self, chunk: &str) {
                self.stdout.push_str(chunk);
            }

            fn stderr(&mut self, chunk: &str) {
                self.stderr.push_str(chunk);
            }
        }

        let mut sink = RecordingSink::default();
        let result = run_local_process(spec, &prepared, &mut sink)
            .await
            .expect("run");
        assert_eq!(result.stdout, "out");
        assert_eq!(result.stderr, "err");
        assert_eq!(sink.stdout, "out");
        assert_eq!(sink.stderr, "err");
        assert_eq!(result.status_code, Some(0));
    }
}

/// Build environment variables for sandboxed commands.
fn build_env(policy: &SandboxPolicy) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if policy.env.allow.is_empty() {
        for (key, value) in std::env::vars() {
            if policy
                .env
                .deny
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(&key))
            {
                continue;
            }
            env.insert(key, value);
        }
    } else {
        for key in &policy.env.allow {
            if policy
                .env
                .deny
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(key))
            {
                continue;
            }
            if let Ok(value) = std::env::var(key) {
                env.insert(key.clone(), value);
            }
        }
    }
    for (key, value) in &policy.env.set {
        env.insert(key.clone(), value.clone());
    }
    env
}

/// Determine network mode based on policy.
fn network_mode(policy: &SandboxPolicy) -> SandboxNetworkMode {
    let allow_configured = !policy.network.allow_domains.is_empty();
    let deny_configured = !policy.network.deny_domains.is_empty();
    if !allow_configured && !deny_configured {
        return SandboxNetworkMode::Allow;
    }
    if allow_configured && !deny_configured {
        warn!(
            "sandbox allow_domains configured but domain filtering is not enforced; allowing network"
        );
        return SandboxNetworkMode::Allow;
    }
    warn!("sandbox deny_domains configured; network access disabled");
    SandboxNetworkMode::Deny
}

/// Build mount list for sandbox execution.
pub fn build_mounts(
    mode: SandboxMode,
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> Result<Vec<Mount>, SandboxError> {
    let workspace_root = normalize_path(workspace_root);
    let workspace_writable = matches!(
        mode,
        SandboxMode::WorkspaceWrite | SandboxMode::DangerFullAccess
    );
    let mut mounts = Vec::new();
    mounts.push(Mount {
        source: workspace_root.clone(),
        target: workspace_root.clone(),
        writable: workspace_writable,
    });

    let mut overrides: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for path in normalize_patterns(&workspace_root, &policy.filesystem.allow_read)? {
        if path.starts_with(&workspace_root) {
            continue;
        }
        overrides.entry(path).or_insert(false);
    }
    for path in normalize_patterns(&workspace_root, &policy.filesystem.allow_exec)? {
        if path.starts_with(&workspace_root) {
            continue;
        }
        overrides.entry(path).or_insert(false);
    }
    for path in normalize_patterns(&workspace_root, &policy.filesystem.allow_write)? {
        if path.starts_with(&workspace_root) {
            continue;
        }
        overrides.insert(path, true);
    }

    for (path, writable) in overrides {
        if !path.exists() {
            return Err(SandboxError::InvalidConfig(format!(
                "sandbox mount path does not exist: {}",
                path.display()
            )));
        }
        mounts.push(Mount {
            source: path.clone(),
            target: path,
            writable,
        });
    }

    debug!(
        "sandbox mounts built (count={}, workspace_writable={})",
        mounts.len(),
        workspace_writable
    );
    Ok(mounts)
}

/// Build a prepared sandbox from context.
pub fn build_prepared_sandbox(ctx: &SandboxContext) -> Result<PreparedSandbox, SandboxError> {
    let access = AccessPolicy::new(ctx.mode, &ctx.policy, &ctx.workspace_root)?;
    let env = build_env(&ctx.policy);
    let network = network_mode(&ctx.policy);
    let mounts = build_mounts(ctx.mode, &ctx.policy, &ctx.workspace_root)?;
    info!(
        "prepared sandbox (mode={:?}, mounts={}, env_keys={})",
        ctx.mode,
        mounts.len(),
        env.len()
    );
    Ok(PreparedSandbox {
        access,
        env,
        limits: ctx.policy.limits.clone(),
        network,
        working_dir: normalize_path(&ctx.workspace_root),
        mounts,
    })
}

/// Buffering sink that captures stdout/stderr for non-streaming runs.
#[derive(Default)]
struct BufferingSink {
    stdout: String,
    stderr: String,
}

impl CommandOutputSink for BufferingSink {
    /// Append stdout chunk.
    fn stdout(&mut self, chunk: &str) {
        self.stdout.push_str(chunk);
    }

    /// Append stderr chunk.
    fn stderr(&mut self, chunk: &str) {
        self.stderr.push_str(chunk);
    }
}

/// Run a command locally with the prepared sandbox configuration.
async fn run_local_process(
    spec: CommandSpec,
    prepared: &PreparedSandbox,
    sink: &mut dyn CommandOutputSink,
) -> Result<CommandResult, SandboxError> {
    debug!(
        "running local process (args_len={}, has_cwd={})",
        spec.args.len(),
        spec.cwd.is_some()
    );
    let mut command = Command::new(&spec.command);
    command.args(&spec.args);
    command.env_clear();
    for (key, value) in &prepared.env {
        command.env(key, value);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    } else {
        command.current_dir(&prepared.working_dir);
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    #[cfg(target_os = "linux")]
    {
        let limits = prepared.limits.clone();
        unsafe {
            command.pre_exec(move || crate::provider::linux::apply_rlimits(&limits));
        }
    }

    let mut child = command.spawn().map_err(SandboxError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stdout_buf, stderr_buf) = stream_child_output(stdout, stderr, sink).await?;

    let status = child.wait().await.map_err(SandboxError::Io)?;

    Ok(CommandResult {
        status_code: status.code(),
        stdout: stdout_buf,
        stderr: stderr_buf,
    })
}

/// Stream child stdout/stderr while capturing full buffers.
pub async fn stream_child_output(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    sink: &mut dyn CommandOutputSink,
) -> Result<(String, String), SandboxError> {
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();

    let mut stdout_reader = stdout.map(tokio::io::BufReader::new);
    let mut stderr_reader = stderr.map(tokio::io::BufReader::new);

    let mut stdout_done = stdout_reader.is_none();
    let mut stderr_done = stderr_reader.is_none();

    let mut stdout_chunk = vec![0u8; 8192];
    let mut stderr_chunk = vec![0u8; 8192];

    while !stdout_done || !stderr_done {
        tokio::select! {
            read = async {
                if let Some(reader) = stdout_reader.as_mut() {
                    reader.read(&mut stdout_chunk).await
                } else {
                    Ok(0)
                }
            }, if !stdout_done => {
                let read = read.map_err(SandboxError::Io)?;
                if read == 0 {
                    stdout_done = true;
                } else {
                    let chunk = String::from_utf8_lossy(&stdout_chunk[..read]);
                    stdout_buf.push_str(&chunk);
                    sink.stdout(&chunk);
                }
            }
            read = async {
                if let Some(reader) = stderr_reader.as_mut() {
                    reader.read(&mut stderr_chunk).await
                } else {
                    Ok(0)
                }
            }, if !stderr_done => {
                let read = read.map_err(SandboxError::Io)?;
                if read == 0 {
                    stderr_done = true;
                } else {
                    let chunk = String::from_utf8_lossy(&stderr_chunk[..read]);
                    stderr_buf.push_str(&chunk);
                    sink.stderr(&chunk);
                }
            }
        }
    }

    Ok((stdout_buf, stderr_buf))
}

/// Build a displayable command string relative to working directory.
pub fn command_display(command: &Path, working_dir: &Path) -> Result<String, SandboxError> {
    if command.is_absolute() {
        return Ok(command.display().to_string());
    }
    if command.components().count() > 1 {
        let absolute = normalize_path(&working_dir.join(command));
        return Ok(absolute.display().to_string());
    }
    Ok(command.display().to_string())
}

/// Add bind mount args if the source exists.
pub fn bind_if_exists(args: &mut Vec<String>, flag: &str, source: &Path, target: &Path) {
    if source.exists() {
        args.push(flag.to_string());
        args.push(source.display().to_string());
        args.push(target.display().to_string());
    }
}
