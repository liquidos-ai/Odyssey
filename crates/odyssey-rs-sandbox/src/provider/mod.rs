use async_trait::async_trait;
use log::info;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::SandboxError;
use crate::types::{
    AccessDecision, AccessMode, CommandResult, CommandSpec, SandboxContext, SandboxHandle,
    SandboxLimits, SandboxMountBinding, SandboxNetworkMode, SandboxPolicy,
};
use odyssey_rs_protocol::SandboxMode;

#[cfg(target_os = "linux")]
pub mod linux;
pub mod local;
pub mod noop;

const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const DEFAULT_WALL_CLOCK_SECONDS: u64 = 60;
const DEFAULT_STDIO_BYTES: usize = 64 * 1024;
const SAFE_ENV_VARS: &[&str] = &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ"];

#[derive(Debug, Default, Clone)]
pub struct DependencyReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError>;

    async fn run_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError>;

    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError>;

    fn check_access(&self, handle: &SandboxHandle, path: &Path, mode: AccessMode)
    -> AccessDecision;

    fn spawn_command(
        &self,
        _handle: &SandboxHandle,
        _spec: CommandSpec,
    ) -> Result<Command, SandboxError> {
        Err(SandboxError::Unsupported(
            "sandbox backend does not support long-lived protocol transports".to_string(),
        ))
    }

    fn dependency_report(&self) -> DependencyReport {
        DependencyReport::default()
    }

    async fn shutdown(&self, handle: SandboxHandle);
}

pub trait CommandOutputSink: Send {
    fn stdout(&mut self, chunk: &str);
    fn stderr(&mut self, chunk: &str);
}

#[derive(Debug, Clone)]
pub struct Mount {
    pub(crate) source: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) writable: bool,
}

#[derive(Debug, Clone)]
pub struct PreparedSandbox {
    pub(crate) access: AccessPolicy,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) allowed_env_keys: BTreeSet<String>,
    pub(crate) limits: SandboxLimits,
    pub(crate) network: SandboxNetworkMode,
    pub(crate) working_dir: PathBuf,
    pub(crate) mounts: Vec<Mount>,
}

#[derive(Debug, Clone)]
struct AccessRules {
    roots: Vec<PathBuf>,
    allow_all: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AccessPolicy {
    pub(crate) workspace_root: PathBuf,
    read: AccessRules,
    write: AccessRules,
    exec: AccessRules,
}

impl AccessPolicy {
    fn new(
        mode: SandboxMode,
        policy: &SandboxPolicy,
        workspace_root: &Path,
    ) -> Result<Self, SandboxError> {
        let workspace_root = canonicalize_existing_path(workspace_root)?;

        let read = match mode {
            SandboxMode::DangerFullAccess => AccessRules {
                roots: Vec::new(),
                allow_all: true,
            },
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => {
                let mut roots = vec![workspace_root.clone()];
                roots.extend(standard_system_exec_roots());
                roots.extend(normalize_existing_roots(
                    &workspace_root,
                    &policy.filesystem.read_roots,
                )?);
                AccessRules {
                    roots: dedupe_roots(roots),
                    allow_all: false,
                }
            }
        };

        let write = match mode {
            SandboxMode::DangerFullAccess => AccessRules {
                roots: Vec::new(),
                allow_all: true,
            },
            SandboxMode::ReadOnly => AccessRules {
                roots: normalize_existing_roots(&workspace_root, &policy.filesystem.write_roots)?,
                allow_all: false,
            },
            SandboxMode::WorkspaceWrite => {
                let mut roots = vec![workspace_root.clone()];
                roots.extend(normalize_existing_roots(
                    &workspace_root,
                    &policy.filesystem.write_roots,
                )?);
                AccessRules {
                    roots: dedupe_roots(roots),
                    allow_all: false,
                }
            }
        };

        let exec = match mode {
            SandboxMode::DangerFullAccess => AccessRules {
                roots: Vec::new(),
                allow_all: true,
            },
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => {
                let roots =
                    normalize_existing_roots(&workspace_root, &policy.filesystem.exec_roots)?;
                AccessRules {
                    roots: dedupe_roots(roots),
                    allow_all: policy.filesystem.exec_allow_all,
                }
            }
        };

        Ok(Self {
            workspace_root,
            read,
            write,
            exec,
        })
    }

    fn check(&self, path: &Path, mode: AccessMode) -> AccessDecision {
        let working_dir = self.workspace_root.as_path();
        let resolved = match resolve_user_path(path, working_dir, &self.workspace_root) {
            Ok(path) => path,
            Err(err) => return AccessDecision::Deny(err.to_string()),
        };

        let rules = match mode {
            AccessMode::Read => &self.read,
            AccessMode::Write => &self.write,
            AccessMode::Execute => &self.exec,
        };

        if rules.allow_all || matches_any(&resolved, &rules.roots) {
            AccessDecision::Allow
        } else {
            AccessDecision::Deny(format!(
                "sandbox policy blocks {:?} access to {}",
                mode,
                resolved.display()
            ))
        }
    }
}

fn normalize_existing_roots(
    root: &Path,
    patterns: &[String],
) -> Result<Vec<PathBuf>, SandboxError> {
    let mut resolved = Vec::new();
    for pattern in patterns {
        reject_glob(pattern)?;
        let path = PathBuf::from(pattern);
        let joined = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        resolved.push(canonicalize_existing_path(&joined)?);
    }
    Ok(resolved)
}

fn reject_glob(pattern: &str) -> Result<(), SandboxError> {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        return Err(SandboxError::InvalidConfig(format!(
            "glob patterns are not supported in sandbox paths: {pattern}"
        )));
    }
    Ok(())
}

pub(crate) fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, SandboxError> {
    path.canonicalize().map_err(|err| {
        SandboxError::InvalidConfig(format!("failed to resolve {}: {err}", path.display()))
    })
}

fn dedupe_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.sort();
    roots.dedup();
    roots
}

fn matches_any(path: &Path, patterns: &[PathBuf]) -> bool {
    patterns.iter().any(|pattern| path.starts_with(pattern))
}

pub fn standard_system_exec_roots() -> Vec<PathBuf> {
    ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/opt"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .filter_map(|path| canonicalize_existing_path(&path).ok())
        .collect()
}

fn normalize_lexical(path: &Path) -> PathBuf {
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

fn resolve_user_path(
    user_path: &Path,
    working_dir: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, SandboxError> {
    if user_path.as_os_str().is_empty() {
        return Err(SandboxError::AccessDenied(
            "empty path is not allowed".to_string(),
        ));
    }
    for component in user_path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(SandboxError::AccessDenied(format!(
                "path traversal is not allowed: {}",
                user_path.display()
            )));
        }
    }

    let candidate = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        working_dir.join(user_path)
    };
    let candidate = normalize_lexical(&candidate);

    let mut unresolved = Vec::<OsString>::new();
    let mut cursor = candidate.as_path();
    loop {
        if cursor.exists() {
            let mut resolved = cursor.canonicalize().map_err(SandboxError::Io)?;
            for suffix in unresolved.iter().rev() {
                resolved.push(suffix);
            }
            return Ok(normalize_lexical(&resolved));
        }

        if cursor == workspace_root.parent().unwrap_or_else(|| Path::new("/"))
            && !workspace_root.exists()
        {
            return Err(SandboxError::AccessDenied(format!(
                "workspace root does not exist: {}",
                workspace_root.display()
            )));
        }

        let Some(name) = cursor.file_name() else {
            return Err(SandboxError::AccessDenied(format!(
                "path cannot be resolved safely: {}",
                user_path.display()
            )));
        };
        unresolved.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return Err(SandboxError::AccessDenied(format!(
                "path cannot be resolved safely: {}",
                user_path.display()
            )));
        };
        cursor = parent;
    }
}

fn effective_output_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_STDIO_BYTES)
}

fn effective_wall_clock(limit: Option<u64>) -> Option<std::time::Duration> {
    Some(std::time::Duration::from_secs(
        limit.unwrap_or(DEFAULT_WALL_CLOCK_SECONDS),
    ))
}

#[derive(Default)]
pub(crate) struct BufferingSink {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl CommandOutputSink for BufferingSink {
    fn stdout(&mut self, chunk: &str) {
        self.stdout.push_str(chunk);
    }

    fn stderr(&mut self, chunk: &str) {
        self.stderr.push_str(chunk);
    }
}

fn build_env(
    policy: &SandboxPolicy,
    workspace_root: &Path,
) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let inherit_keys = if policy.env.inherit.is_empty() {
        SAFE_ENV_VARS
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>()
    } else {
        policy.env.inherit.clone()
    };

    let mut env = BTreeMap::new();
    let mut allowed = BTreeSet::new();

    for key in inherit_keys {
        if let Ok(value) = std::env::var(&key) {
            allowed.insert(key.clone());
            env.insert(key, value);
        }
    }

    let tmp_dir = workspace_root.join(".tmp");
    allowed.insert("HOME".to_string());
    env.insert("HOME".to_string(), workspace_root.display().to_string());
    allowed.insert("TMPDIR".to_string());
    env.insert("TMPDIR".to_string(), tmp_dir.display().to_string());
    allowed.insert("ODYSSEY_SANDBOX".to_string());
    env.insert("ODYSSEY_SANDBOX".to_string(), "1".to_string());

    if !env.contains_key("PATH") {
        allowed.insert("PATH".to_string());
        env.insert("PATH".to_string(), DEFAULT_PATH.to_string());
    }

    for (key, value) in &policy.env.set {
        allowed.insert(key.clone());
        env.insert(key.clone(), value.clone());
    }

    (env, allowed)
}

pub(crate) fn merge_command_env(
    prepared: &PreparedSandbox,
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SandboxError> {
    let mut env = prepared.env.clone();
    for (key, value) in overrides {
        if !prepared.allowed_env_keys.contains(key) {
            return Err(SandboxError::AccessDenied(format!(
                "environment variable is not allowed by sandbox policy: {key}"
            )));
        }
        env.insert(key.clone(), value.clone());
    }
    Ok(env)
}

fn append_with_limit(
    raw: &[u8],
    buffer: &mut String,
    limit: usize,
    truncated: &mut bool,
) -> Option<String> {
    if *truncated {
        return None;
    }

    let current = buffer.len();
    if current >= limit {
        *truncated = true;
        return None;
    }

    let remaining = limit.saturating_sub(current);
    if raw.len() <= remaining {
        let chunk = String::from_utf8_lossy(raw).to_string();
        buffer.push_str(&chunk);
        return Some(chunk);
    }

    let chunk = String::from_utf8_lossy(&raw[..remaining]).to_string();
    buffer.push_str(&chunk);
    *truncated = true;
    Some(chunk)
}

async fn stream_child_output(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    sink: &mut dyn CommandOutputSink,
    limits: &SandboxLimits,
) -> Result<(String, String, bool, bool), SandboxError> {
    let mut stdout_buf = String::default();
    let mut stderr_buf = String::default();
    let stdout_limit = effective_output_limit(limits.stdout_bytes);
    let stderr_limit = effective_output_limit(limits.stderr_bytes);
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;

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
                } else if let Some(chunk) = append_with_limit(
                    &stdout_chunk[..read],
                    &mut stdout_buf,
                    stdout_limit,
                    &mut stdout_truncated,
                ) {
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
                } else if let Some(chunk) = append_with_limit(
                    &stderr_chunk[..read],
                    &mut stderr_buf,
                    stderr_limit,
                    &mut stderr_truncated,
                ) {
                    sink.stderr(&chunk);
                }
            }
        }
    }

    if stdout_truncated {
        let note = "\n...[stdout truncated by sandbox]";
        stdout_buf.push_str(note);
        sink.stdout(note);
    }
    if stderr_truncated {
        let note = "\n...[stderr truncated by sandbox]";
        stderr_buf.push_str(note);
        sink.stderr(note);
    }

    Ok((stdout_buf, stderr_buf, stdout_truncated, stderr_truncated))
}

#[cfg(unix)]
pub(crate) unsafe fn configure_child_unix(command: &mut Command, limits: &SandboxLimits) {
    let limits = limits.clone();
    unsafe {
        command.pre_exec(move || {
            let setpgid_result = libc::setpgid(0, 0);
            if setpgid_result != 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            crate::provider::linux::apply_rlimits(&limits)?;
            Ok(())
        });
    }
}

#[cfg(unix)]
async fn kill_process_tree(pid: u32) -> Result<(), SandboxError> {
    let pgid = -(pid as i32);
    let term_result = unsafe { libc::kill(pgid, libc::SIGTERM) };
    if term_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(SandboxError::Io(err));
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let kill_result = unsafe { libc::kill(pgid, libc::SIGKILL) };
    if kill_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(SandboxError::Io(err));
        }
    }

    Ok(())
}

#[cfg(not(unix))]
async fn kill_process_tree(_pid: u32) -> Result<(), SandboxError> {
    Ok(())
}

pub(crate) async fn collect_child_result(
    child: &mut tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    sink: &mut dyn CommandOutputSink,
    limits: &SandboxLimits,
) -> Result<CommandResult, SandboxError> {
    let execution = async {
        let (stdout, stderr, stdout_truncated, stderr_truncated) =
            stream_child_output(stdout, stderr, sink, limits).await?;
        let status = child.wait().await.map_err(SandboxError::Io)?;

        Ok(CommandResult {
            status_code: status.code(),
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    };

    if let Some(timeout) = effective_wall_clock(limits.wall_clock_seconds) {
        match tokio::time::timeout(timeout, execution).await {
            Ok(result) => result,
            Err(_) => {
                if let Some(pid) = child.id() {
                    kill_process_tree(pid).await?;
                }
                let _ = child.wait().await;
                Err(SandboxError::LimitExceeded(format!(
                    "command exceeded wall clock limit of {} seconds",
                    timeout.as_secs()
                )))
            }
        }
    } else {
        execution.await
    }
}

pub(crate) fn resolve_working_dir(
    spec: &CommandSpec,
    prepared: &PreparedSandbox,
) -> Result<PathBuf, SandboxError> {
    let cwd = spec.cwd.as_ref().unwrap_or(&prepared.working_dir);
    let resolved = resolve_user_path(cwd, &prepared.working_dir, &prepared.access.workspace_root)?;
    if !resolved.is_dir() {
        return Err(SandboxError::AccessDenied(format!(
            "working directory is not a directory: {}",
            resolved.display()
        )));
    }
    match prepared.access.check(&resolved, AccessMode::Read) {
        AccessDecision::Allow => Ok(resolved),
        AccessDecision::Deny(reason) => Err(SandboxError::AccessDenied(reason)),
    }
}

pub(crate) fn resolve_command_path(
    command: &Path,
    working_dir: &Path,
    prepared: &PreparedSandbox,
) -> Result<PathBuf, SandboxError> {
    if command.is_absolute() || command.components().count() > 1 {
        let resolved = resolve_user_path(command, working_dir, &prepared.access.workspace_root)?;
        return match prepared.access.check(&resolved, AccessMode::Execute) {
            AccessDecision::Allow => Ok(resolved),
            AccessDecision::Deny(reason) => Err(SandboxError::AccessDenied(reason)),
        };
    }

    let path_value = prepared
        .env
        .get("PATH")
        .cloned()
        .unwrap_or_else(|| DEFAULT_PATH.to_string());
    for root in std::env::split_paths(&path_value) {
        let candidate = root.join(command);
        if !candidate.exists() {
            continue;
        }
        let resolved = candidate.canonicalize().map_err(SandboxError::Io)?;
        if matches!(
            prepared.access.check(&resolved, AccessMode::Execute),
            AccessDecision::Allow
        ) {
            return Ok(resolved);
        }
    }

    Err(SandboxError::AccessDenied(format!(
        "executable is not permitted by sandbox policy: {}",
        command.display()
    )))
}

fn build_mounts_from_access(
    access: &AccessPolicy,
    mode: SandboxMode,
    mount_bindings: &[SandboxMountBinding],
) -> Vec<Mount> {
    if matches!(mode, SandboxMode::DangerFullAccess) {
        return Vec::new();
    }

    let workspace_writable = matches!(mode, SandboxMode::WorkspaceWrite);
    let mut mount_modes: BTreeMap<PathBuf, bool> = BTreeMap::new();
    mount_modes.insert(access.workspace_root.clone(), workspace_writable);

    for path in access.read.roots.iter().chain(access.exec.roots.iter()) {
        if !path.starts_with(&access.workspace_root) {
            mount_modes.entry(path.clone()).or_insert(false);
        }
    }
    for path in &access.write.roots {
        if !path.starts_with(&access.workspace_root) {
            mount_modes.insert(path.clone(), true);
        }
    }

    let mut mounts = mount_modes
        .into_iter()
        .map(|(path, writable)| Mount {
            source: path.clone(),
            target: path,
            writable,
        })
        .collect::<Vec<_>>();

    mounts.extend(mount_bindings.iter().map(|binding| Mount {
        source: PathBuf::from(&binding.source),
        target: PathBuf::from(&binding.target),
        writable: binding.writable,
    }));
    mounts
}

pub fn build_prepared_sandbox(ctx: &SandboxContext) -> Result<PreparedSandbox, SandboxError> {
    let workspace_root = canonicalize_existing_path(&ctx.workspace_root)?;
    std::fs::create_dir_all(workspace_root.join(".tmp")).map_err(SandboxError::Io)?;
    let access = AccessPolicy::new(ctx.mode, &ctx.policy, &workspace_root)?;
    let (env, allowed_env_keys) = build_env(&ctx.policy, &workspace_root);
    let mounts = build_mounts_from_access(&access, ctx.mode, &ctx.policy.filesystem.mount_bindings);
    info!(
        "prepared sandbox (mode={:?}, mounts={}, env_keys={})",
        ctx.mode,
        mounts.len(),
        env.len()
    );
    Ok(PreparedSandbox {
        access,
        env,
        allowed_env_keys,
        limits: ctx.policy.limits.clone(),
        network: ctx.policy.network.mode,
        working_dir: workspace_root,
        mounts,
    })
}

pub(crate) fn validate_host_execution_context(ctx: &SandboxContext) -> Result<(), SandboxError> {
    if ctx.mode != SandboxMode::DangerFullAccess {
        return Err(SandboxError::Unsupported(
            "host provider only supports danger_full_access; use bubblewrap for restricted sandbox modes".to_string(),
        ));
    }

    if matches!(ctx.policy.network.mode, SandboxNetworkMode::Disabled) {
        return Err(SandboxError::Unsupported(
            "host provider cannot enforce disabled network access".to_string(),
        ));
    }

    Ok(())
}

pub(crate) fn build_host_child_command(
    spec: CommandSpec,
    prepared: &PreparedSandbox,
) -> Result<Command, SandboxError> {
    let cwd = resolve_working_dir(&spec, prepared)?;
    let command = resolve_command_path(&spec.command, &cwd, prepared)?;
    let env = merge_command_env(prepared, &spec.env)?;

    let mut child_command = Command::new(&command);
    child_command.args(&spec.args);
    child_command.current_dir(&cwd);
    child_command.env_clear();
    child_command.envs(&env);

    #[cfg(unix)]
    unsafe {
        configure_child_unix(&mut child_command, &prepared.limits);
    }

    Ok(child_command)
}

pub(crate) async fn run_host_process(
    spec: CommandSpec,
    prepared: &PreparedSandbox,
    sink: &mut dyn CommandOutputSink,
) -> Result<CommandResult, SandboxError> {
    let mut child_command = build_host_child_command(spec, prepared)?;
    child_command.stdout(std::process::Stdio::piped());
    child_command.stderr(std::process::Stdio::piped());

    let mut child = child_command.spawn().map_err(SandboxError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    collect_child_result(&mut child, stdout, stderr, sink, &prepared.limits).await
}

pub fn command_display(command: &Path) -> String {
    command.display().to_string()
}

pub fn bind_if_exists(args: &mut Vec<String>, flag: &str, source: &Path, target: &Path) {
    if source.exists() {
        args.push(flag.to_string());
        args.push(source.display().to_string());
        args.push(target.display().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessPolicy, bind_if_exists, build_mounts_from_access, build_prepared_sandbox,
        command_display, effective_output_limit, effective_wall_clock, merge_command_env,
        normalize_existing_roots, normalize_lexical, reject_glob, resolve_command_path,
        resolve_user_path, resolve_working_dir, run_host_process, validate_host_execution_context,
    };
    use crate::{
        AccessDecision, AccessMode, CommandSpec, SandboxContext, SandboxFilesystemPolicy,
        SandboxLimits, SandboxMountBinding, SandboxNetworkMode, SandboxNetworkPolicy,
        SandboxPolicy,
    };
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn read_only_mode_denies_workspace_write_and_requires_explicit_exec_roots() {
        let temp = tempdir().expect("tempdir");
        let policy = SandboxPolicy::default();
        let access =
            AccessPolicy::new(SandboxMode::ReadOnly, &policy, temp.path()).expect("access");
        let inside = temp.path().join("file.txt");
        assert_eq!(
            access.check(&inside, AccessMode::Read),
            AccessDecision::Allow
        );
        assert!(matches!(
            access.check(&inside, AccessMode::Write),
            AccessDecision::Deny(_)
        ));
        assert!(matches!(
            access.check(Path::new("/bin/sh"), AccessMode::Execute),
            AccessDecision::Deny(_)
        ));
    }

    #[test]
    fn workspace_write_allows_within_workspace() {
        let temp = tempdir().expect("tempdir");
        let policy = SandboxPolicy::default();
        let access =
            AccessPolicy::new(SandboxMode::WorkspaceWrite, &policy, temp.path()).expect("access");
        let path = temp.path().join("bin");
        assert_eq!(access.check(&path, AccessMode::Read), AccessDecision::Allow);
        assert_eq!(
            access.check(&path, AccessMode::Write),
            AccessDecision::Allow
        );
    }

    #[test]
    fn reject_glob_blocks_patterns() {
        let err = reject_glob("/sandbox-root/*.txt").expect_err("glob rejected");
        assert_eq!(
            err.to_string(),
            "invalid configuration: glob patterns are not supported in sandbox paths: /sandbox-root/*.txt"
        );
    }

    #[test]
    fn normalize_existing_roots_requires_existing_paths() {
        let temp = tempdir().expect("tempdir");
        let err = normalize_existing_roots(temp.path(), &["missing".to_string()])
            .expect_err("missing path");
        assert!(err.to_string().contains("failed to resolve"));
    }

    #[test]
    fn normalize_lexical_resolves_components() {
        let path = Path::new("/sandbox-root/dir/../file.txt");
        assert_eq!(
            normalize_lexical(path),
            PathBuf::from("/sandbox-root/file.txt")
        );
    }

    #[test]
    fn resolve_user_path_rejects_parent_dir() {
        let temp = tempdir().expect("tempdir");
        let err = resolve_user_path(Path::new("../oops"), temp.path(), temp.path())
            .expect_err("parent dir rejected");
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn resolve_user_path_rejects_empty_path() {
        let temp = tempdir().expect("tempdir");
        let err = resolve_user_path(Path::new(""), temp.path(), temp.path())
            .expect_err("empty path rejected");
        assert_eq!(err.to_string(), "access denied: empty path is not allowed");
    }

    #[test]
    fn resolve_user_path_preserves_unresolved_suffix_inside_workspace() {
        let temp = tempdir().expect("tempdir");
        let existing = temp.path().join("existing");
        std::fs::create_dir_all(&existing).expect("create existing");

        let resolved =
            resolve_user_path(Path::new("existing/new-file.txt"), temp.path(), temp.path())
                .expect("resolve path");

        assert_eq!(resolved, existing.join("new-file.txt"));
    }

    #[test]
    fn command_display_returns_absolute_path() {
        assert_eq!(
            command_display(Path::new("/sandbox-root/bin/run")),
            "/sandbox-root/bin/run".to_string()
        );
    }

    #[test]
    fn bind_if_exists_adds_flag_when_present() {
        let temp = tempdir().expect("tempdir");
        let mut args = Vec::new();
        bind_if_exists(&mut args, "--ro-bind", temp.path(), temp.path());
        assert_eq!(args[0], "--ro-bind");
    }

    #[test]
    fn build_prepared_sandbox_uses_network_and_env_defaults() {
        let temp = tempdir().expect("tempdir");
        let mut policy = SandboxPolicy::default();
        policy.network.mode = SandboxNetworkMode::Disabled;
        policy.env.set.insert("FOO".to_string(), "BAR".to_string());
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy,
        };

        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        assert_eq!(prepared.network, SandboxNetworkMode::Disabled);
        assert_eq!(prepared.env.get("FOO"), Some(&"BAR".to_string()));
        assert!(prepared.env.contains_key("PATH"));
    }

    #[test]
    fn validate_host_execution_context_rejects_restricted_modes() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };

        let error = validate_host_execution_context(&ctx).expect_err("restricted mode rejected");
        assert!(error.to_string().contains("danger_full_access"));
    }

    #[test]
    fn validate_host_execution_context_rejects_disabled_network() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::DangerFullAccess,
            policy: SandboxPolicy {
                network: SandboxNetworkPolicy {
                    mode: SandboxNetworkMode::Disabled,
                },
                ..SandboxPolicy::default()
            },
        };

        let error = validate_host_execution_context(&ctx).expect_err("disabled network rejected");
        assert!(error.to_string().contains("disabled network"));
    }

    #[test]
    fn merge_command_env_rejects_unapproved_overrides() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let overrides = std::iter::once(("UNSAFE".to_string(), "1".to_string())).collect();

        let error = merge_command_env(&prepared, &overrides).expect_err("override rejected");
        assert_eq!(
            error.to_string(),
            "access denied: environment variable is not allowed by sandbox policy: UNSAFE"
        );
    }

    #[test]
    fn effective_limits_use_defaults() {
        assert_eq!(effective_output_limit(None), 64 * 1024);
        assert_eq!(effective_output_limit(Some(17)), 17);
        assert_eq!(
            effective_wall_clock(None),
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(
            effective_wall_clock(Some(7)),
            Some(std::time::Duration::from_secs(7))
        );
    }

    #[test]
    fn build_mounts_from_access_tracks_writable_roots_by_mode() {
        let temp = tempdir().expect("tempdir");
        let host_root = tempdir().expect("host root");
        let host_read = host_root.path().join("host-read");
        let host_write = host_root.path().join("host-write");
        std::fs::create_dir_all(&host_read).expect("create host read");
        std::fs::create_dir_all(&host_write).expect("create host write");
        let target_read = temp.path().join("mount").join("read").join("current");
        let target_write = temp.path().join("mount").join("write").join("current");
        std::fs::create_dir_all(&target_read).expect("create target read");
        std::fs::create_dir_all(&target_write).expect("create target write");
        let policy = SandboxPolicy {
            filesystem: SandboxFilesystemPolicy {
                read_roots: vec![target_read.display().to_string()],
                write_roots: vec![target_write.display().to_string()],
                exec_roots: Vec::new(),
                exec_allow_all: false,
                mount_bindings: vec![
                    SandboxMountBinding {
                        source: host_read.display().to_string(),
                        target: target_read.display().to_string(),
                        writable: false,
                    },
                    SandboxMountBinding {
                        source: host_write.display().to_string(),
                        target: target_write.display().to_string(),
                        writable: true,
                    },
                ],
            },
            ..SandboxPolicy::default()
        };
        let access =
            AccessPolicy::new(SandboxMode::WorkspaceWrite, &policy, temp.path()).expect("access");

        let mounts = build_mounts_from_access(
            &access,
            SandboxMode::WorkspaceWrite,
            &policy.filesystem.mount_bindings,
        );
        assert!(
            mounts
                .iter()
                .any(|mount| mount.source == temp.path() && mount.writable)
        );
        assert!(mounts.iter().any(|mount| mount.source == host_read
            && mount.target == target_read
            && !mount.writable));
        assert!(mounts.iter().any(|mount| mount.source == host_write
            && mount.target == target_write
            && mount.writable));

        let no_mounts = build_mounts_from_access(
            &access,
            SandboxMode::DangerFullAccess,
            &policy.filesystem.mount_bindings,
        );
        assert!(no_mounts.is_empty());
    }

    #[test]
    fn resolve_working_dir_rejects_file_paths() {
        let temp = tempdir().expect("tempdir");
        let file = temp.path().join("file.txt");
        std::fs::write(&file, "data").expect("write file");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let mut spec = CommandSpec::new("sh");
        spec.cwd = Some(file);

        let error = resolve_working_dir(&spec, &prepared).expect_err("file cwd rejected");
        assert!(
            error
                .to_string()
                .contains("working directory is not a directory")
        );
    }

    #[tokio::test]
    async fn run_host_process_captures_output_and_truncates() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::DangerFullAccess,
            policy: SandboxPolicy {
                limits: SandboxLimits {
                    stdout_bytes: Some(4),
                    stderr_bytes: Some(4),
                    ..SandboxLimits::default()
                },
                ..SandboxPolicy::default()
            },
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let mut spec = CommandSpec::new("sh");
        spec.args = vec![
            "-c".to_string(),
            "printf out123; printf err123 1>&2".to_string(),
        ];

        struct RecordingSink {
            stdout: String,
            stderr: String,
        }

        impl crate::provider::CommandOutputSink for RecordingSink {
            fn stdout(&mut self, chunk: &str) {
                self.stdout.push_str(chunk);
            }
            fn stderr(&mut self, chunk: &str) {
                self.stderr.push_str(chunk);
            }
        }

        let mut sink = RecordingSink {
            stdout: String::default(),
            stderr: String::default(),
        };
        let result = run_host_process(spec, &prepared, &mut sink)
            .await
            .expect("run");
        assert!(result.stdout_truncated);
        assert!(result.stderr_truncated);
        assert!(result.stdout.contains("truncated"));
        assert!(result.stderr.contains("truncated"));
        assert_eq!(result.status_code, Some(0));
    }

    #[tokio::test]
    async fn run_host_process_enforces_wall_clock_limit() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::DangerFullAccess,
            policy: SandboxPolicy {
                limits: SandboxLimits {
                    wall_clock_seconds: Some(0),
                    ..SandboxLimits::default()
                },
                ..SandboxPolicy::default()
            },
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let mut spec = CommandSpec::new("sh");
        spec.args = vec!["-c".to_string(), "sleep 1".to_string()];

        let mut sink = crate::provider::BufferingSink::default();
        let error = run_host_process(spec, &prepared, &mut sink)
            .await
            .expect_err("timeout expected");
        assert!(
            error
                .to_string()
                .contains("command exceeded wall clock limit of 0 seconds")
        );
    }

    #[tokio::test]
    async fn run_host_process_enforces_wall_clock_limit_after_stdio_closes() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::DangerFullAccess,
            policy: SandboxPolicy {
                limits: SandboxLimits {
                    wall_clock_seconds: Some(1),
                    ..SandboxLimits::default()
                },
                ..SandboxPolicy::default()
            },
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let mut spec = CommandSpec::new("sh");
        spec.args = vec![
            "-c".to_string(),
            "exec >/dev/null 2>&1; sleep 2".to_string(),
        ];

        let mut sink = crate::provider::BufferingSink::default();
        let error = run_host_process(spec, &prepared, &mut sink)
            .await
            .expect_err("timeout expected");
        assert!(
            error
                .to_string()
                .contains("command exceeded wall clock limit of 1 seconds")
        );
    }

    #[test]
    fn resolve_command_path_honors_exec_roots() {
        let temp = tempdir().expect("tempdir");
        let policy = SandboxPolicy {
            filesystem: SandboxFilesystemPolicy {
                exec_roots: vec!["/bin".to_string()],
                ..SandboxFilesystemPolicy::default()
            },
            ..SandboxPolicy::default()
        };
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::ReadOnly,
            policy,
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let resolved =
            resolve_command_path(Path::new("sh"), temp.path(), &prepared).expect("resolve");
        assert!(resolved.exists());
        assert!(resolved.file_name().is_some());
    }
}
