//! Sandbox provider traits and shared helpers.

use async_trait::async_trait;
use log::{debug, info, warn};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::SandboxError;
use crate::types::{
    AccessDecision, AccessMode, CommandLandlockPolicy, CommandResult, CommandSpec, SandboxContext,
    SandboxHandle, SandboxLimits, SandboxNetworkMode, SandboxPolicy,
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
const LANDLOCK_HELPER_ENV: &str = "ODYSSEY_SANDBOX_INTERNAL_LANDLOCK_HELPER";
const LANDLOCK_HELPER_NAME: &str = "odyssey-rs-sandbox-internal-landlock-helper";

/// Report of missing dependencies for a sandbox provider.
#[derive(Debug, Default, Clone)]
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

    /// Build a long-lived child command for protocols like MCP stdio.
    fn spawn_command(
        &self,
        _handle: &SandboxHandle,
        _spec: CommandSpec,
    ) -> Result<Command, SandboxError> {
        Err(SandboxError::Unsupported(
            "sandbox backend does not support long-lived protocol transports".to_string(),
        ))
    }

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
pub struct Mount {
    /// Source path on the host.
    pub(crate) source: PathBuf,
    /// Target path inside the sandbox.
    pub(crate) target: PathBuf,
    /// Whether the mount is writable.
    pub(crate) writable: bool,
}

/// Fully prepared sandbox execution plan.
#[derive(Debug, Clone)]
pub struct PreparedSandbox {
    /// Access policy derived from config.
    pub(crate) access: AccessPolicy,
    /// Environment variables to inject.
    pub(crate) env: BTreeMap<String, String>,
    /// Command-overridable environment keys.
    pub(crate) allowed_env_keys: BTreeSet<String>,
    /// Resource limits.
    pub(crate) limits: SandboxLimits,
    /// Network policy.
    pub(crate) network: SandboxNetworkMode,
    /// Default working directory.
    pub(crate) working_dir: PathBuf,
    /// Mount list for the sandbox.
    pub(crate) mounts: Vec<Mount>,
}

/// Allow rules for a specific access mode.
#[derive(Debug, Clone)]
struct AccessRules {
    roots: Vec<PathBuf>,
    allow_all: bool,
}

/// Aggregated access policy for read/write/exec.
#[derive(Debug, Clone)]
pub(crate) struct AccessPolicy {
    workspace_root: PathBuf,
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
        let system_roots = system_runtime_roots();

        let read = match mode {
            SandboxMode::DangerFullAccess => AccessRules {
                roots: Vec::new(),
                allow_all: true,
            },
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => {
                let mut roots = vec![workspace_root.clone()];
                roots.extend(system_roots.iter().cloned());
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
                let mut roots = system_roots;
                roots.push(workspace_root.clone());
                roots.extend(normalize_existing_roots(
                    &workspace_root,
                    &policy.filesystem.exec_roots,
                )?);
                AccessRules {
                    roots: dedupe_roots(roots),
                    allow_all: false,
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

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, SandboxError> {
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

fn system_runtime_roots() -> Vec<PathBuf> {
    ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/opt"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .filter_map(|path| canonicalize_existing_path(&path).ok())
        .collect()
}

/// Resolve the helper binary used to apply Landlock before exec.
pub fn resolve_internal_landlock_helper_path() -> Result<PathBuf, SandboxError> {
    if let Some(value) = std::env::var_os(LANDLOCK_HELPER_ENV) {
        return canonicalize_existing_path(Path::new(&value));
    }

    if let Ok(path) = which::which(LANDLOCK_HELPER_NAME) {
        return canonicalize_existing_path(&path);
    }

    let current_exe = std::env::current_exe().map_err(SandboxError::Io)?;
    let mut search_roots = Vec::new();
    if let Some(parent) = current_exe.parent() {
        search_roots.push(parent.to_path_buf());
        if let Some(grandparent) = parent.parent() {
            search_roots.push(grandparent.to_path_buf());
        }
    }

    for root in &search_roots {
        let candidate = root.join(LANDLOCK_HELPER_NAME);
        if candidate.exists() {
            return canonicalize_existing_path(&candidate);
        }
    }

    if let Some(path) = try_auto_build_landlock_helper(&search_roots)? {
        return Ok(path);
    }

    Err(SandboxError::DependencyMissing(format!(
        "internal Landlock helper '{}' not found; set {} or place the binary on PATH",
        LANDLOCK_HELPER_NAME, LANDLOCK_HELPER_ENV
    )))
}

fn try_auto_build_landlock_helper(
    search_roots: &[PathBuf],
) -> Result<Option<PathBuf>, SandboxError> {
    let (workspace_root, profile) = match infer_workspace_and_profile(search_roots) {
        Some(values) => values,
        None => return Ok(None),
    };
    let helper_source = workspace_root
        .join("crates")
        .join("odyssey-rs-sandbox")
        .join("src")
        .join("bin")
        .join(format!("{LANDLOCK_HELPER_NAME}.rs"));
    if !helper_source.exists() {
        return Ok(None);
    }

    let target_dir = workspace_root.join("target").join(&profile);
    let helper_binary = target_dir.join(LANDLOCK_HELPER_NAME);
    if helper_binary.exists() {
        return canonicalize_existing_path(&helper_binary).map(Some);
    }

    warn!(
        "Landlock helper missing from PATH; attempting automatic build (workspace={}, profile={})",
        workspace_root.display(),
        profile
    );
    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&workspace_root)
        .arg("build")
        .arg("-p")
        .arg("odyssey-rs-sandbox")
        .arg("--bin")
        .arg(LANDLOCK_HELPER_NAME);
    if profile == "release" {
        cmd.arg("--release");
    }
    let output = cmd.output().map_err(SandboxError::Io)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(SandboxError::DependencyMissing(format!(
            "failed to auto-build internal Landlock helper: {stderr}"
        )));
    }

    if helper_binary.exists() {
        return canonicalize_existing_path(&helper_binary).map(Some);
    }
    Err(SandboxError::DependencyMissing(format!(
        "auto-build succeeded but helper binary not found at {}",
        helper_binary.display()
    )))
}

fn infer_workspace_and_profile(search_roots: &[PathBuf]) -> Option<(PathBuf, String)> {
    for root in search_roots {
        let mut current = root.clone();
        loop {
            if current.join("Cargo.toml").exists()
                && current
                    .join("crates")
                    .join("odyssey-rs-sandbox")
                    .join("Cargo.toml")
                    .exists()
            {
                let profile = root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| *name == "release" || *name == "debug")
                    .unwrap_or("debug")
                    .to_string();
                return Some((current, profile));
            }
            if !current.pop() {
                break;
            }
        }
    }

    let cwd = std::env::current_dir().ok()?;
    let mut current = cwd;
    loop {
        if current.join("Cargo.toml").exists()
            && current
                .join("crates")
                .join("odyssey-rs-sandbox")
                .join("Cargo.toml")
                .exists()
        {
            return Some((current, "debug".to_string()));
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn wrap_command_with_landlock(
    command: PathBuf,
    args: Vec<String>,
    policy: Option<&CommandLandlockPolicy>,
) -> Result<(PathBuf, Vec<String>), SandboxError> {
    let Some(policy) = policy else {
        return Ok((command, args));
    };

    let launcher = resolve_internal_landlock_helper_path()?;
    let mut launcher_args = Vec::new();
    for root in &policy.read_roots {
        launcher_args.push("--read".to_string());
        launcher_args.push(canonicalize_existing_path(root)?.display().to_string());
    }
    for root in &policy.write_roots {
        launcher_args.push("--write".to_string());
        launcher_args.push(canonicalize_existing_path(root)?.display().to_string());
    }
    for root in &policy.exec_roots {
        launcher_args.push("--exec".to_string());
        launcher_args.push(canonicalize_existing_path(root)?.display().to_string());
    }
    launcher_args.push("--".to_string());
    launcher_args.push(command_display(&command));
    launcher_args.extend(args);

    Ok((launcher, launcher_args))
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

        if cursor == workspace_root.parent().unwrap_or(Path::new("/")) && !workspace_root.exists() {
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

/// Buffering sink that captures stdout/stderr for non-streaming runs.
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

    allowed.insert("HOME".to_string());
    env.insert("HOME".to_string(), workspace_root.display().to_string());
    allowed.insert("TMPDIR".to_string());
    env.insert("TMPDIR".to_string(), "/tmp".to_string());
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
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
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
    // SAFETY: pre_exec runs in the child just before exec. We only call async-signal-safe libc
    // functions (`setpgid`) and the rlimit setter used by the Linux backend.
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
    // SAFETY: kill is called with a negative process group id created by setpgid in the child.
    let term_result = unsafe { libc::kill(pgid, libc::SIGTERM) };
    if term_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(SandboxError::Io(err));
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // SAFETY: same as SIGTERM above, this is best-effort cleanup for the process group.
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
    let output_future = stream_child_output(stdout, stderr, sink, limits);
    tokio::pin!(output_future);

    let (stdout, stderr, stdout_truncated, stderr_truncated) =
        if let Some(timeout) = effective_wall_clock(limits.wall_clock_seconds) {
            tokio::select! {
                output = &mut output_future => output?,
                _ = tokio::time::sleep(timeout) => {
                    if let Some(pid) = child.id() {
                        kill_process_tree(pid).await?;
                    }
                    let _ = child.wait().await;
                    return Err(SandboxError::LimitExceeded(format!(
                        "command exceeded wall clock limit of {} seconds",
                        timeout.as_secs()
                    )));
                }
            }
        } else {
            output_future.await?
        };

    let status = child.wait().await.map_err(SandboxError::Io)?;

    Ok(CommandResult {
        status_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
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

fn build_mounts_from_access(access: &AccessPolicy, mode: SandboxMode) -> Vec<Mount> {
    if matches!(mode, SandboxMode::DangerFullAccess) {
        return Vec::new();
    }

    let workspace_writable = matches!(mode, SandboxMode::WorkspaceWrite);
    let mut mount_modes: BTreeMap<PathBuf, bool> = BTreeMap::new();
    mount_modes.insert(access.workspace_root.clone(), workspace_writable);

    for path in access.read.roots.iter().chain(access.exec.roots.iter()) {
        mount_modes.entry(path.clone()).or_insert(false);
    }
    for path in &access.write.roots {
        mount_modes.insert(path.clone(), true);
    }

    mount_modes
        .into_iter()
        .map(|(path, writable)| Mount {
            source: path.clone(),
            target: path,
            writable,
        })
        .collect()
}

/// Build a prepared sandbox from context.
pub fn build_prepared_sandbox(ctx: &SandboxContext) -> Result<PreparedSandbox, SandboxError> {
    let workspace_root = canonicalize_existing_path(&ctx.workspace_root)?;
    let access = AccessPolicy::new(ctx.mode, &ctx.policy, &workspace_root)?;
    let (env, allowed_env_keys) = build_env(&ctx.policy, &workspace_root);
    let mounts = build_mounts_from_access(&access, ctx.mode);
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

/// Build a host child command with the prepared sandbox configuration.
pub(crate) fn build_host_child_command(
    spec: CommandSpec,
    prepared: &PreparedSandbox,
) -> Result<Command, SandboxError> {
    let cwd = resolve_working_dir(&spec, prepared)?;
    let command = resolve_command_path(&spec.command, &cwd, prepared)?;
    let env = merge_command_env(prepared, &spec.env)?;
    let (command, args) = wrap_command_with_landlock(command, spec.args, spec.landlock.as_ref())?;

    debug!(
        "building host child command (command={}, args_len={}, cwd={})",
        command.display(),
        args.len(),
        cwd.display()
    );

    let mut child_command = Command::new(&command);
    child_command.args(&args);
    child_command.current_dir(&cwd);
    child_command.env_clear();
    child_command.envs(&env);

    #[cfg(unix)]
    unsafe {
        configure_child_unix(&mut child_command, &prepared.limits);
    }

    Ok(child_command)
}

/// Run a host process with the prepared sandbox configuration.
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

/// Build a displayable command string relative to working directory.
pub fn command_display(command: &Path) -> String {
    command.display().to_string()
}

/// Add bind mount args if the source exists.
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
        AccessPolicy, bind_if_exists, build_prepared_sandbox, command_display,
        normalize_existing_roots, normalize_lexical, reject_glob, resolve_command_path,
        resolve_user_path, run_host_process,
    };
    use crate::{
        AccessDecision, AccessMode, CommandSpec, SandboxContext, SandboxFilesystemPolicy,
        SandboxLimits, SandboxNetworkMode, SandboxPolicy,
    };
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn read_only_mode_allows_system_exec_but_denies_workspace_write() {
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
        assert_eq!(
            matches!(
                access.check(Path::new("/bin/sh"), AccessMode::Execute),
                AccessDecision::Allow
            ),
            Path::new("/bin/sh").exists()
        );
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
        let err = reject_glob("/tmp/*.txt").expect_err("glob rejected");
        assert_eq!(
            err.to_string(),
            "invalid configuration: glob patterns are not supported in sandbox paths: /tmp/*.txt"
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
        let path = Path::new("/tmp/dir/../file.txt");
        assert_eq!(normalize_lexical(path), PathBuf::from("/tmp/file.txt"));
    }

    #[test]
    fn resolve_user_path_rejects_parent_dir() {
        let temp = tempdir().expect("tempdir");
        let err = resolve_user_path(Path::new("../oops"), temp.path(), temp.path())
            .expect_err("parent dir rejected");
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn command_display_returns_absolute_path() {
        assert_eq!(
            command_display(Path::new("/tmp/bin/run")),
            "/tmp/bin/run".to_string()
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
        assert_eq!(prepared.env.contains_key("PATH"), true);
    }

    #[tokio::test]
    async fn run_host_process_captures_output_and_truncates() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
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
            stdout: String::new(),
            stderr: String::new(),
        };
        let result = run_host_process(spec, &prepared, &mut sink)
            .await
            .expect("run");
        assert_eq!(result.stdout_truncated, true);
        assert_eq!(result.stderr_truncated, true);
        assert!(result.stdout.contains("truncated"));
        assert!(result.stderr.contains("truncated"));
        assert_eq!(result.status_code, Some(0));
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
        assert_eq!(resolved.exists(), true);
        assert!(resolved.file_name().is_some());
    }
}
