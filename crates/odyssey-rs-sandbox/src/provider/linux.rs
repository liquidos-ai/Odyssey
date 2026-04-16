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
    SandboxHandle, SandboxLimits, SandboxNetworkMode, SandboxProvider,
    provider::{
        BufferingSink, Mount, PreparedSandbox, bind_if_exists, build_prepared_sandbox,
        command_display, stream_child_output,
    },
};
use crate::{DependencyReport, SandboxError};

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
        use std::path::Path;

        let mut report = DependencyReport::default();
        if which::which("bwrap").is_err() {
            report
                .errors
                .push("bubblewrap (bwrap) not found in PATH".to_string());
        }
        if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
            report
                .warnings
                .push("cgroup v2 not detected; resource limits rely on rlimits only".to_string());
        }
        report
    }

    /// Build the bubblewrap command from the prepared sandbox and spec.
    fn build_command(
        &self,
        prepared: &PreparedSandbox,
        spec: &CommandSpec,
    ) -> Result<Command, SandboxError> {
        let mut env = prepared.env.clone();
        for (key, value) in &spec.env {
            env.insert(key.clone(), value.clone());
        }
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

        if matches!(prepared.network, SandboxNetworkMode::Deny) {
            bwrap_args.push("--unshare-net".to_string());
        }

        for (src, dst) in base_system_mounts() {
            bind_if_exists(&mut bwrap_args, "--ro-bind", &src, &dst);
        }

        append_etc_mounts(&mut bwrap_args);

        bwrap_args.push("--dev".to_string());
        bwrap_args.push("/dev".to_string());
        bwrap_args.push("--tmpfs".to_string());
        bwrap_args.push("/dev/shm".to_string());
        bind_if_exists(
            &mut bwrap_args,
            "--bind",
            Path::new("/dev/pts"),
            Path::new("/dev/pts"),
        );
        bwrap_args.push("--tmpfs".to_string());
        bwrap_args.push("/tmp".to_string());
        bwrap_args.push("--dir".to_string());
        bwrap_args.push("/runtime".to_string());

        for mount in &prepared.mounts {
            append_mount(&mut bwrap_args, mount)?;
        }

        bwrap_args.push("--chdir".to_string());
        bwrap_args.push(prepared.working_dir.display().to_string());

        bwrap_args.push("--clearenv".to_string());
        for (key, value) in env {
            bwrap_args.push("--setenv".to_string());
            bwrap_args.push(key);
            bwrap_args.push(value);
        }

        bwrap_args.push("--".to_string());
        bwrap_args.push(command_display(&spec.command, &prepared.working_dir)?);
        for arg in &spec.args {
            bwrap_args.push(arg.clone());
        }

        let mut cmd = Command::new(&self.bwrap_path);
        cmd.args(&bwrap_args);
        Ok(cmd)
    }
}

#[async_trait]
impl SandboxProvider for BubblewrapProvider {
    /// Prepare sandbox state for a handle.
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        let prepared = build_prepared_sandbox(ctx)?;
        let handle = SandboxHandle {
            id: uuid::Uuid::new_v4(),
        };
        self.state.write().insert(handle.id, prepared);
        info!("bubblewrap sandbox prepared (handle_id={})", handle.id);
        Ok(handle)
    }

    /// Run a command in bubblewrap without streaming output.
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
        })
    }

    /// Run a command in bubblewrap with streaming output.
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

    /// Check access against the prepared sandbox policies.
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

    /// Return dependency report for the provider.
    fn dependency_report(&self) -> DependencyReport {
        Self::dependency_report_linux()
    }

    /// Shutdown and remove the prepared sandbox.
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

/// Run a bubblewrap command and stream output.
async fn run_bwrap_process(
    provider: &BubblewrapProvider,
    prepared: &PreparedSandbox,
    spec: CommandSpec,
    sink: &mut dyn CommandOutputSink,
) -> Result<CommandResult, SandboxError> {
    debug!(
        "starting bubblewrap process (args_len={}, has_cwd={})",
        spec.args.len(),
        spec.cwd.is_some()
    );
    let mut cmd = provider.build_command(prepared, &spec)?;
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let limits = prepared.limits.clone();
    #[cfg(target_os = "linux")]
    unsafe {
        cmd.pre_exec(move || apply_rlimits(&limits));
    }

    let mut child = cmd.spawn().map_err(SandboxError::Io)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stdout_buf, stderr_buf) = stream_child_output(stdout, stderr, sink).await?;

    let status = child.wait().await.map_err(SandboxError::Io)?;

    if status.code().unwrap_or(-1) != 0 {
        warn!("bubblewrap command exited non-zero");
    }
    Ok(CommandResult {
        status_code: status.code(),
        stdout: stdout_buf,
        stderr: stderr_buf,
    })
}

/// Base system mounts required for bubblewrap execution.
fn base_system_mounts() -> Vec<(PathBuf, PathBuf)> {
    [
        ("/usr", "/usr"),
        ("/lib", "/lib"),
        ("/lib64", "/lib64"),
        ("/bin", "/bin"),
        ("/sbin", "/sbin"),
        ("/opt", "/opt"),
    ]
    .into_iter()
    .map(|(src, dst)| (PathBuf::from(src), PathBuf::from(dst)))
    .collect()
}

/// Append a mount entry to the bubblewrap args.
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

/// Append default /etc mounts needed by bubblewrap.
fn append_etc_mounts(args: &mut Vec<String>) {
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

    let dir_mounts = [("/etc/ssl", "/etc/ssl"), ("/etc/pki", "/etc/pki")];

    for (src, dst) in dir_mounts {
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
    use super::{BubblewrapProvider, Mount, append_mount, apply_rlimits, base_system_mounts};
    use crate::provider::build_prepared_sandbox;
    use crate::{CommandSpec, SandboxContext, SandboxLimits, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn base_system_mounts_includes_expected_paths() {
        let mounts = base_system_mounts();
        let paths = mounts
            .iter()
            .map(|(src, _)| src.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"/usr".to_string()));
        assert!(paths.contains(&"/lib".to_string()));
        assert!(paths.contains(&"/bin".to_string()));
    }

    #[test]
    fn append_mount_rejects_relative_paths() {
        let mount = Mount {
            source: PathBuf::from("relative"),
            target: PathBuf::from("/tmp/target"),
            writable: false,
        };
        let mut args = Vec::new();
        let err = append_mount(&mut args, &mount).expect_err("error");
        match err {
            crate::SandboxError::InvalidConfig(message) => {
                assert!(message.contains("must be absolute"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
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
        match err {
            crate::SandboxError::InvalidConfig(message) => {
                assert!(message.contains("source does not exist"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn append_mount_writes_bind_flags() {
        let temp = tempdir().expect("tempdir");
        let mount = Mount {
            source: temp.path().to_path_buf(),
            target: temp.path().join("target"),
            writable: true,
        };
        let mut args = Vec::new();
        append_mount(&mut args, &mount).expect("append mount");
        assert_eq!(args[0], "--bind");
    }

    #[test]
    fn apply_rlimits_noop_when_unset() {
        let limits = SandboxLimits {
            cpu_seconds: None,
            memory_bytes: None,
            nofile: None,
            pids: None,
        };
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
        spec.env.insert("FOO".to_string(), "BAR".to_string());

        let cmd = provider.build_command(&prepared, &spec).expect("cmd");
        let args = cmd
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.contains(&"FOO".to_string()));
        assert!(args.contains(&"BAR".to_string()));
        assert!(args.iter().any(|arg| arg == "echo"));
    }
}
