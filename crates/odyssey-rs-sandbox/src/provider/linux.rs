//! Bubblewrap-based sandbox provider for Linux.

use async_trait::async_trait;
use log::{debug, info, warn};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use tokio::process::Command;

use crate::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxHandle, SandboxLimits, SandboxProvider,
    provider::{
        BufferingSink, DependencyReport, Mount, PreparedSandbox, bind_if_exists,
        build_prepared_sandbox, collect_child_result, command_display, configure_child_unix,
        merge_command_env, resolve_command_path, resolve_working_dir, wrap_command_with_landlock,
    },
};
use crate::{SandboxError, types::SandboxNetworkMode};
use odyssey_rs_protocol::SandboxMode;

/// Bubblewrap-backed sandbox provider.
#[derive(Debug)]
pub struct BubblewrapProvider {
    /// Path to the bwrap executable.
    bwrap_path: PathBuf,
    /// Prepared sandbox state keyed by handle id.
    state: parking_lot::RwLock<HashMap<uuid::Uuid, PreparedSandbox>>,
}

impl BubblewrapProvider {
    /// Create a new bubblewrap provider by resolving the bwrap binary.
    pub fn new() -> Result<Self, SandboxError> {
        let bwrap_path = which::which("bwrap").map_err(|_| {
            SandboxError::DependencyMissing("bubblewrap (bwrap) not found in PATH".to_string())
        })?;
        info!(
            "bubblewrap provider initialized (path={})",
            bwrap_path.display()
        );
        Ok(Self {
            bwrap_path,
            state: parking_lot::RwLock::new(HashMap::new()),
        })
    }

    /// Produce a dependency report for Linux bubblewrap requirements.
    fn dependency_report_linux() -> DependencyReport {
        let mut report = DependencyReport::default();
        if which::which("bwrap").is_err() {
            report
                .errors
                .push("bubblewrap (bwrap) not found in PATH".to_string());
        }
        if !Path::new("/proc/self/ns").exists() {
            report.warnings.push(
                "Linux namespaces do not appear to be available; bubblewrap may fail at runtime"
                    .to_string(),
            );
        }
        report
    }

    /// Build the bubblewrap command from the prepared sandbox and spec.
    fn build_command(
        &self,
        prepared: &PreparedSandbox,
        spec: &CommandSpec,
    ) -> Result<Command, SandboxError> {
        let cwd = resolve_working_dir(spec, prepared)?;
        let command = resolve_command_path(&spec.command, &cwd, prepared)?;
        let env = merge_command_env(prepared, &spec.env)?;
        let (command, args) =
            wrap_command_with_landlock(command, spec.args.clone(), spec.landlock.as_ref())?;

        let mut bwrap_args: Vec<String> = vec![
            "--die-with-parent".to_string(),
            "--new-session".to_string(),
            "--unshare-user".to_string(),
            "--uid".to_string(),
            "0".to_string(),
            "--gid".to_string(),
            "0".to_string(),
            "--unshare-ipc".to_string(),
            "--unshare-uts".to_string(),
            "--unshare-pid".to_string(),
            "--proc".to_string(),
            "/proc".to_string(),
        ];

        if matches!(prepared.network, SandboxNetworkMode::Disabled) {
            bwrap_args.push("--unshare-net".to_string());
        }

        append_etc_mounts(&mut bwrap_args);
        append_runtime_mounts(&mut bwrap_args);
        bwrap_args.push("--dev".to_string());
        bwrap_args.push("/dev".to_string());
        bwrap_args.push("--tmpfs".to_string());
        bwrap_args.push("/tmp".to_string());
        bwrap_args.push("--dir".to_string());
        bwrap_args.push("/runtime".to_string());
        bind_if_exists(
            &mut bwrap_args,
            "--bind",
            Path::new("/dev/pts"),
            Path::new("/dev/pts"),
        );

        for mount in &prepared.mounts {
            append_mount(&mut bwrap_args, mount)?;
        }

        bwrap_args.push("--chdir".to_string());
        bwrap_args.push(cwd.display().to_string());
        bwrap_args.push("--clearenv".to_string());
        for (key, value) in env {
            bwrap_args.push("--setenv".to_string());
            bwrap_args.push(key);
            bwrap_args.push(value);
        }

        bwrap_args.push("--".to_string());
        bwrap_args.push(command_display(&command));
        for arg in &args {
            bwrap_args.push(arg.clone());
        }

        let mut cmd = Command::new(&self.bwrap_path);
        cmd.args(&bwrap_args);
        Ok(cmd)
    }
}

#[async_trait]
impl SandboxProvider for BubblewrapProvider {
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        if ctx.mode == SandboxMode::DangerFullAccess {
            return Err(SandboxError::Unsupported(
                "bubblewrap provider does not support danger_full_access; use the host provider explicitly"
                    .to_string(),
            ));
        }
        let prepared = build_prepared_sandbox(ctx)?;
        let handle = SandboxHandle {
            id: uuid::Uuid::new_v4(),
        };
        self.state.write().insert(handle.id, prepared);
        info!("bubblewrap sandbox prepared (handle_id={})", handle.id);
        Ok(handle)
    }

    async fn run_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError> {
        let mut sink = BufferingSink::default();
        let result = self.run_command_streaming(handle, spec, &mut sink).await?;
        Ok(CommandResult {
            status_code: result.status_code,
            stdout: sink.stdout,
            stderr: sink.stderr,
            stdout_truncated: result.stdout_truncated,
            stderr_truncated: result.stderr_truncated,
        })
    }

    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        debug!(
            "bubblewrap run (handle_id={}, args_len={}, has_cwd={})",
            handle.id,
            spec.args.len(),
            spec.cwd.is_some()
        );
        let prepared = self
            .state
            .read()
            .get(&handle.id)
            .cloned()
            .ok_or_else(|| SandboxError::InvalidConfig("unknown sandbox handle".to_string()))?;
        run_bwrap_process(self, &prepared, spec, sink).await
    }

    fn check_access(
        &self,
        handle: &SandboxHandle,
        path: &Path,
        mode: AccessMode,
    ) -> AccessDecision {
        let state = self.state.read();
        let Some(prepared) = state.get(&handle.id) else {
            warn!(
                "bubblewrap access check failed (unknown handle_id={})",
                handle.id
            );
            return AccessDecision::Deny("unknown sandbox handle".to_string());
        };
        prepared.access.check(path, mode)
    }

    fn dependency_report(&self) -> DependencyReport {
        Self::dependency_report_linux()
    }

    fn spawn_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<Command, SandboxError> {
        let prepared = self
            .state
            .read()
            .get(&handle.id)
            .cloned()
            .ok_or_else(|| SandboxError::InvalidConfig("unknown sandbox handle".to_string()))?;
        let mut command = self.build_command(&prepared, &spec)?;

        #[cfg(unix)]
        unsafe {
            configure_child_unix(&mut command, &prepared.limits);
        }

        Ok(command)
    }

    async fn shutdown(&self, handle: SandboxHandle) {
        info!("bubblewrap sandbox shutdown (handle_id={})", handle.id);
        self.state.write().remove(&handle.id);
    }
}

/// Apply rlimits based on configured sandbox limits.
pub(crate) fn apply_rlimits(limits: &SandboxLimits) -> Result<(), std::io::Error> {
    fn set(limit: libc::__rlimit_resource_t, value: Option<u64>) -> Result<(), std::io::Error> {
        if let Some(value) = value {
            let rlim = libc::rlimit {
                rlim_cur: value as libc::rlim_t,
                rlim_max: value as libc::rlim_t,
            };
            // SAFETY: setrlimit is called in the child just before exec with validated integer values.
            let result = unsafe { libc::setrlimit(limit, &rlim) };
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    set(libc::RLIMIT_CPU, limits.cpu_seconds)?;
    set(libc::RLIMIT_AS, limits.memory_bytes)?;
    set(libc::RLIMIT_NOFILE, limits.nofile)?;
    set(libc::RLIMIT_NPROC, limits.pids)?;
    Ok(())
}

async fn run_bwrap_process(
    provider: &BubblewrapProvider,
    prepared: &PreparedSandbox,
    spec: CommandSpec,
    sink: &mut dyn CommandOutputSink,
) -> Result<CommandResult, SandboxError> {
    let mut cmd = provider.build_command(prepared, &spec)?;
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    unsafe {
        configure_child_unix(&mut cmd, &prepared.limits);
    }

    let mut child = cmd.spawn().map_err(SandboxError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let result = collect_child_result(&mut child, stdout, stderr, sink, &prepared.limits).await?;
    if result.status_code.unwrap_or(-1) != 0 {
        warn!("bubblewrap command exited non-zero");
    }
    Ok(result)
}

fn append_mount(args: &mut Vec<String>, mount: &Mount) -> Result<(), SandboxError> {
    if !mount.source.is_absolute() || !mount.target.is_absolute() {
        return Err(SandboxError::InvalidConfig(format!(
            "sandbox mount paths must be absolute: {} -> {}",
            mount.source.display(),
            mount.target.display()
        )));
    }
    if !mount.source.exists() {
        return Err(SandboxError::InvalidConfig(format!(
            "sandbox mount source does not exist: {}",
            mount.source.display()
        )));
    }
    let flag = if mount.writable {
        "--bind"
    } else {
        "--ro-bind"
    };
    args.push(flag.to_string());
    args.push(mount.source.display().to_string());
    args.push(mount.target.display().to_string());
    Ok(())
}

pub(crate) fn append_etc_mounts(args: &mut Vec<String>) {
    args.push("--dir".to_string());
    args.push("/etc".to_string());

    let file_mounts = [
        ("/etc/hosts", "/etc/hosts"),
        ("/etc/nsswitch.conf", "/etc/nsswitch.conf"),
        ("/etc/passwd", "/etc/passwd"),
        ("/etc/group", "/etc/group"),
        ("/etc/ld.so.cache", "/etc/ld.so.cache"),
    ];

    append_resolv_conf_mount(args);

    for (src, dst) in file_mounts {
        bind_if_exists(args, "--ro-bind", Path::new(src), Path::new(dst));
    }

    for (src, dst) in [("/etc/ssl", "/etc/ssl"), ("/etc/pki", "/etc/pki")] {
        bind_if_exists(args, "--ro-bind", Path::new(src), Path::new(dst));
    }
}

pub(crate) fn append_runtime_mounts(args: &mut Vec<String>) {
    for (src, dst) in [
        ("/lib", "/lib"),
        ("/lib64", "/lib64"),
        ("/usr/lib", "/usr/lib"),
        ("/usr/lib64", "/usr/lib64"),
        ("/usr/local/lib", "/usr/local/lib"),
    ] {
        bind_if_exists(args, "--ro-bind", Path::new(src), Path::new(dst));
    }
}

fn append_resolv_conf_mount(args: &mut Vec<String>) {
    let resolv_path = Path::new("/etc/resolv.conf");
    if let Ok(resolved) = fs::canonicalize(resolv_path)
        && resolved.as_path() != resolv_path
    {
        if let Some(parent) = resolved.parent() {
            let systemd_resolve_root = Path::new("/run/systemd/resolve");
            if parent.starts_with(systemd_resolve_root) {
                bind_if_exists(
                    args,
                    "--ro-bind",
                    systemd_resolve_root,
                    systemd_resolve_root,
                );
            }
        }
        bind_if_exists(args, "--ro-bind", &resolved, resolv_path);
        return;
    }
    bind_if_exists(args, "--ro-bind", resolv_path, resolv_path);
}

#[cfg(test)]
mod tests {
    use super::{BubblewrapProvider, Mount, append_mount, apply_rlimits};
    use crate::provider::build_prepared_sandbox;
    use crate::{CommandSpec, SandboxContext, SandboxLimits, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn append_mount_rejects_relative_paths() {
        let mount = Mount {
            source: PathBuf::from("relative"),
            target: PathBuf::from("/tmp/target"),
            writable: false,
        };
        let mut args = Vec::new();
        let err = append_mount(&mut args, &mount).expect_err("error");
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn append_mount_rejects_missing_sources() {
        let mount = Mount {
            source: PathBuf::from("/tmp/odyssey-missing"),
            target: PathBuf::from("/tmp/target"),
            writable: false,
        };
        let mut args = Vec::new();
        let err = append_mount(&mut args, &mount).expect_err("error");
        assert!(err.to_string().contains("source does not exist"));
    }

    #[test]
    fn append_mount_writes_bind_flags() {
        let temp = tempdir().expect("tempdir");
        let mount = Mount {
            source: temp.path().to_path_buf(),
            target: temp.path().to_path_buf(),
            writable: true,
        };
        let mut args = Vec::new();
        append_mount(&mut args, &mount).expect("append mount");
        assert_eq!(args[0], "--bind");
    }

    #[test]
    fn apply_rlimits_noop_when_unset() {
        let limits = SandboxLimits::default();
        apply_rlimits(&limits).expect("apply limits");
    }

    #[test]
    fn build_command_includes_env_and_args() {
        let temp = tempdir().expect("tempdir");
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let prepared = build_prepared_sandbox(&ctx).expect("prepared");
        let provider = BubblewrapProvider {
            bwrap_path: PathBuf::from("/usr/bin/bwrap"),
            state: parking_lot::RwLock::new(HashMap::new()),
        };

        let mut spec = CommandSpec::new("echo");
        spec.args.push("hello".to_string());

        let cmd = provider.build_command(&prepared, &spec).expect("cmd");
        let args = cmd
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(
            args.iter()
                .any(|arg| arg == "echo" || arg.ends_with("/echo"))
        );
        assert!(args.contains(&"hello".to_string()));
        assert!(args.contains(&"--clearenv".to_string()));
    }
}
