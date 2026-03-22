use async_trait::async_trait;
use log::{info, warn};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::process::Command;

use crate::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxError, SandboxHandle, SandboxLimits, SandboxProvider,
    provider::{
        BufferingSink, DependencyReport, Mount, PreparedSandbox, bind_if_exists,
        build_prepared_sandbox, collect_child_result, command_display, configure_child_unix,
        merge_command_env, resolve_command_path, resolve_working_dir,
    },
    types::SandboxNetworkMode,
};
use odyssey_rs_protocol::SandboxMode;

fn sandbox_tmp_dir() -> PathBuf {
    PathBuf::from(std::path::MAIN_SEPARATOR.to_string()).join("tmp")
}

#[derive(Debug)]
pub struct BubblewrapProvider {
    bwrap_path: PathBuf,
    state: parking_lot::RwLock<HashMap<uuid::Uuid, PreparedSandbox>>,
}

impl BubblewrapProvider {
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

    fn build_command(
        &self,
        prepared: &PreparedSandbox,
        spec: &CommandSpec,
    ) -> Result<Command, SandboxError> {
        let cwd = resolve_working_dir(spec, prepared)?;
        let command = resolve_command_path(&spec.command, &cwd, prepared)?;
        let env = merge_command_env(prepared, &spec.env)?;
        let args = spec.args.clone();

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
        let sandbox_tmp = sandbox_tmp_dir();
        bwrap_args.push("--dev".to_string());
        bwrap_args.push("/dev".to_string());
        bwrap_args.push("--tmpfs".to_string());
        bwrap_args.push(sandbox_tmp.display().to_string());
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
        append_command_mount_if_needed(&mut bwrap_args, prepared, &command)?;

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

fn append_command_mount_if_needed(
    args: &mut Vec<String>,
    prepared: &PreparedSandbox,
    command: &Path,
) -> Result<(), SandboxError> {
    if !command.is_absolute() || path_is_mounted(command, prepared) {
        return Ok(());
    }

    let Some(parent) = command.parent() else {
        return Err(SandboxError::InvalidConfig(format!(
            "command path has no parent directory: {}",
            command.display()
        )));
    };
    bind_if_exists(args, "--ro-bind", parent, parent);
    Ok(())
}

fn path_is_mounted(path: &Path, prepared: &PreparedSandbox) -> bool {
    prepared
        .mounts
        .iter()
        .any(|mount| path.starts_with(&mount.target))
        || [
            "/usr", "/lib", "/lib64", "/bin", "/sbin", "/opt", "/etc", "/dev",
        ]
        .into_iter()
        .any(|root| path.starts_with(root))
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
        ("/etc/resolv.conf", "/etc/resolv.conf"),
        ("/etc/nsswitch.conf", "/etc/nsswitch.conf"),
        ("/etc/localtime", "/etc/localtime"),
        ("/etc/passwd", "/etc/passwd"),
        ("/etc/group", "/etc/group"),
    ];
    for (source, target) in file_mounts {
        bind_if_exists(args, "--ro-bind", Path::new(source), Path::new(target));
    }

    // HTTPS clients need access to the host CA trust store inside the sandbox.
    for dir in ["/etc/ssl", "/etc/pki", "/etc/ca-certificates"] {
        bind_if_exists(args, "--ro-bind", Path::new(dir), Path::new(dir));
    }
}

pub(crate) fn append_runtime_mounts(args: &mut Vec<String>) {
    for dir in ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/opt"] {
        bind_if_exists(args, "--ro-bind", Path::new(dir), Path::new(dir));
    }
    // Keep /run private so the sandbox cannot reach host Unix-domain sockets.
    args.push("--tmpfs".to_string());
    args.push("/run".to_string());
}

#[cfg(test)]
mod tests {
    use super::{append_etc_mounts, append_mount, append_runtime_mounts};
    use crate::provider::Mount;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn append_mount_rejects_relative_paths() {
        let mount = Mount {
            source: "relative".into(),
            target: "/sandbox/target".into(),
            writable: false,
        };
        let error = append_mount(&mut Vec::new(), &mount).expect_err("relative source rejected");
        assert!(
            error
                .to_string()
                .contains("sandbox mount paths must be absolute")
        );
    }

    #[test]
    fn append_mount_requires_existing_source() {
        let mount = Mount {
            source: "/path/that/does/not/exist".into(),
            target: "/sandbox/target".into(),
            writable: false,
        };
        let error = append_mount(&mut Vec::new(), &mount).expect_err("missing source rejected");
        assert_eq!(
            error.to_string(),
            "invalid configuration: sandbox mount source does not exist: /path/that/does/not/exist"
        );
    }

    #[test]
    fn append_mount_uses_bind_flag_for_writable_mounts() {
        let temp = tempdir().expect("tempdir");
        let mount = Mount {
            source: temp.path().to_path_buf(),
            target: temp.path().join("target"),
            writable: true,
        };
        let mut args = Vec::new();

        append_mount(&mut args, &mount).expect("append mount");

        assert_eq!(args[0], "--bind");
        assert_eq!(args[1], temp.path().display().to_string());
        assert_eq!(args[2], mount.target.display().to_string());
    }

    #[test]
    fn append_runtime_mounts_binds_existing_system_roots() {
        let mut args = Vec::new();
        append_runtime_mounts(&mut args);

        assert!(args.windows(3).any(|window| {
            window[0] == "--ro-bind" && window[1] == "/usr" && window[2] == "/usr"
        }));
        assert!(
            args.windows(2)
                .any(|window| { window[0] == "--tmpfs" && window[1] == "/run" })
        );
        assert!(!args.windows(3).any(|window| {
            (window[0] == "--bind" || window[0] == "--ro-bind")
                && window[1] == "/run"
                && window[2] == "/run"
        }));
    }

    #[test]
    fn append_etc_mounts_binds_common_certificate_roots_when_present() {
        let mut args = Vec::new();
        append_etc_mounts(&mut args);

        for dir in ["/etc/ssl", "/etc/pki", "/etc/ca-certificates"] {
            if !Path::new(dir).exists() {
                continue;
            }
            assert!(args.windows(3).any(|window| {
                window[0] == "--ro-bind" && window[1] == dir && window[2] == dir
            }));
        }
    }
}
