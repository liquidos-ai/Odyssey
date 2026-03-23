use crate::RuntimeError;
use odyssey_rs_manifest::{AgentSpec, BundleManifest, BundleSystemToolsMode};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    SandboxCellKey, SandboxCellSpec, SandboxLimits, SandboxMountBinding, SandboxNetworkMode,
    SandboxNetworkPolicy, SandboxPolicy, SandboxRuntime, standard_system_exec_roots,
};
use odyssey_rs_tools::{
    PermissionAction, ToolPermissionMatcher, ToolPermissionRule, ToolSandbox, WorkspaceMount,
};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub(crate) struct PreparedToolSandbox {
    pub sandbox: ToolSandbox,
    pub root: PathBuf,
    pub work_dir: PathBuf,
    pub workspace_mounts: Vec<WorkspaceMount>,
}

const AGENT_EXECUTION_COMPONENT: &str = "agent-execution";
const SESSION_COMMAND_COMPONENT: &str = "session-command";

/// Mount aliases need different on-disk prep depending on the backend:
/// restricted sandboxes need a real file/dir placeholder that bubblewrap can
/// over-mount, while host danger mode needs a symlink because no kernel mount
/// step happens later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountTargetPreparation {
    Placeholder,
    HostSymlink,
}

struct CellPreparationRequest<'a> {
    session_id: Uuid,
    agent_id: &'a str,
    bundle_root: &'a Path,
    manifest: &'a BundleManifest,
    override_mode: Option<SandboxMode>,
    component_id: &'static str,
}

pub fn build_policy(
    bundle_root: &Path,
    manifest: &BundleManifest,
) -> Result<SandboxPolicy, RuntimeError> {
    let current_dir = std::env::current_dir().map_err(|err| RuntimeError::Io {
        path: ".".to_string(),
        message: err.to_string(),
    })?;
    build_policy_with_resolvers(
        bundle_root,
        manifest,
        &[],
        false,
        read_process_env,
        &current_dir,
    )
}

pub fn build_operator_command_policy(
    bundle_root: &Path,
    manifest: &BundleManifest,
) -> Result<SandboxPolicy, RuntimeError> {
    let current_dir = std::env::current_dir().map_err(|err| RuntimeError::Io {
        path: ".".to_string(),
        message: err.to_string(),
    })?;
    build_policy_with_resolvers(
        bundle_root,
        manifest,
        &[],
        true,
        read_process_env,
        &current_dir,
    )
}

fn build_policy_with_resolvers<F>(
    bundle_root: &Path,
    manifest: &BundleManifest,
    extra_exec_roots: &[String],
    force_standard_exec_roots: bool,
    env_resolver: F,
    current_dir: &Path,
) -> Result<SandboxPolicy, RuntimeError>
where
    F: FnMut(&str) -> Option<String>,
{
    let map_bundle_paths = |entries: &[String]| -> Result<Vec<String>, RuntimeError> {
        entries
            .iter()
            .map(|entry| resolve_bundle_exec_root(bundle_root, entry))
            .collect()
    };
    let read_mounts = build_mount_bindings(
        bundle_root,
        current_dir,
        &manifest.sandbox.permissions.filesystem.mounts.read,
        false,
    );
    let write_mounts = build_mount_bindings(
        bundle_root,
        current_dir,
        &manifest.sandbox.permissions.filesystem.mounts.write,
        true,
    );
    let mount_bindings = read_mounts
        .iter()
        .chain(&write_mounts)
        .cloned()
        .collect::<Vec<_>>();
    let explicit_system_tools = resolve_system_tools(&manifest.sandbox.system_tools)?;
    let mut exec_roots = map_bundle_paths(&manifest.sandbox.permissions.filesystem.exec)?;
    exec_roots.extend(explicit_system_tools);
    let mut exec_allow_all = false;
    match manifest.sandbox.system_tools_mode {
        BundleSystemToolsMode::Explicit => {}
        BundleSystemToolsMode::Standard => {
            exec_roots.extend(
                standard_system_exec_roots()
                    .into_iter()
                    .map(|path| path.display().to_string()),
            );
        }
        BundleSystemToolsMode::All => {
            exec_allow_all = true;
        }
    }
    if force_standard_exec_roots && !exec_allow_all {
        exec_roots.extend(
            standard_system_exec_roots()
                .into_iter()
                .map(|path| path.display().to_string()),
        );
    }
    exec_roots.extend(extra_exec_roots.iter().cloned());
    exec_roots.sort();
    exec_roots.dedup();

    Ok(SandboxPolicy {
        filesystem: odyssey_rs_sandbox::SandboxFilesystemPolicy {
            read_roots: read_mounts
                .iter()
                .map(|mount| mount.target.clone())
                .collect(),
            write_roots: write_mounts
                .iter()
                .map(|mount| mount.target.clone())
                .collect(),
            exec_roots,
            exec_allow_all,
            mount_bindings,
        },
        env: odyssey_rs_sandbox::SandboxEnvPolicy {
            inherit: Vec::new(),
            set: resolve_manifest_env(&manifest.sandbox.env, env_resolver),
        },
        network: build_network_policy(&manifest.sandbox.permissions.network)?,
        limits: SandboxLimits {
            cpu_seconds: manifest.sandbox.resources.cpu,
            memory_bytes: manifest
                .sandbox
                .resources
                .memory_mb
                .map(|value| value * 1024 * 1024),
            ..SandboxLimits::default()
        },
    })
}

fn build_mount_bindings(
    bundle_root: &Path,
    current_dir: &Path,
    values: &[String],
    writable: bool,
) -> Vec<SandboxMountBinding> {
    values
        .iter()
        .map(|value| {
            let source = resolve_host_mount_path(value, current_dir);
            let target = mount_target_path(bundle_root, writable, value, Path::new(&source));
            SandboxMountBinding {
                source,
                target: target.display().to_string(),
                writable,
            }
        })
        .collect()
}

fn resolve_manifest_env(
    env: &std::collections::BTreeMap<String, String>,
    mut env_resolver: impl FnMut(&str) -> Option<String>,
) -> std::collections::BTreeMap<String, String> {
    env.iter()
        .filter_map(|(target, source)| env_resolver(source).map(|value| (target.clone(), value)))
        .collect()
}

fn read_process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn resolve_host_mount_path(value: &str, current_dir: &Path) -> String {
    if is_current_directory_mount(value) {
        return current_dir.display().to_string();
    }
    value.to_string()
}

fn is_current_directory_mount(value: &str) -> bool {
    let mut components = Path::new(value).components();
    components.next().is_some()
        && components.all(|component| component == std::path::Component::CurDir)
}

pub fn build_mode(manifest: &BundleManifest, override_mode: Option<SandboxMode>) -> SandboxMode {
    override_mode.unwrap_or(manifest.sandbox.mode)
}

pub fn build_permission_rules(agent: &AgentSpec) -> Vec<ToolPermissionRule> {
    let mut permissions = Vec::new();
    for rule in &agent.tools.allow {
        permissions.push(build_permission_rule(PermissionAction::Allow, rule));
    }
    for rule in &agent.tools.ask {
        permissions.push(build_permission_rule(PermissionAction::Ask, rule));
    }
    for rule in &agent.tools.deny {
        permissions.push(build_permission_rule(PermissionAction::Deny, rule));
    }

    permissions
}

fn build_permission_rule(action: PermissionAction, value: &str) -> ToolPermissionRule {
    let matcher = match ToolPermissionMatcher::parse(value) {
        Ok(matcher) => matcher,
        Err(_) => ToolPermissionMatcher {
            tool: value.to_string(),
            target: None,
        },
    };
    ToolPermissionRule { action, matcher }
}

pub async fn prepare_cell(
    sandbox: &SandboxRuntime,
    session_id: Uuid,
    agent_id: &str,
    bundle_root: &Path,
    manifest: &BundleManifest,
    override_mode: Option<SandboxMode>,
) -> Result<PreparedToolSandbox, RuntimeError> {
    prepare_cell_with_policy(
        sandbox,
        CellPreparationRequest {
            session_id,
            agent_id,
            bundle_root,
            manifest,
            override_mode,
            component_id: AGENT_EXECUTION_COMPONENT,
        },
        build_policy,
    )
    .await
}

pub async fn prepare_operator_command_cell(
    sandbox: &SandboxRuntime,
    session_id: Uuid,
    agent_id: &str,
    bundle_root: &Path,
    manifest: &BundleManifest,
    override_mode: Option<SandboxMode>,
) -> Result<PreparedToolSandbox, RuntimeError> {
    prepare_cell_with_policy(
        sandbox,
        CellPreparationRequest {
            session_id,
            agent_id,
            bundle_root,
            manifest,
            override_mode,
            component_id: SESSION_COMMAND_COMPONENT,
        },
        build_operator_command_policy,
    )
    .await
}

async fn prepare_cell_with_policy(
    sandbox: &SandboxRuntime,
    request: CellPreparationRequest<'_>,
    policy_builder: fn(&Path, &BundleManifest) -> Result<SandboxPolicy, RuntimeError>,
) -> Result<PreparedToolSandbox, RuntimeError> {
    let mode = build_mode(request.manifest, request.override_mode);
    let workspace_key = SandboxCellKey::tooling(request.session_id, request.agent_id);
    let key = SandboxCellKey::tooling_component(
        request.session_id,
        request.agent_id,
        request.component_id,
    );
    let cell_root = sandbox.managed_cell_root(&workspace_key)?;
    let root = cell_root.join("app");
    let policy = policy_builder(&root, request.manifest)?;
    validate_provider_support(sandbox.provider_name(), mode, &policy)?;
    stage_bundle_if_needed(request.bundle_root, &root, mode)?;
    validate_staged_bundle_exec_roots(
        &root,
        &request.manifest.sandbox.permissions.filesystem.exec,
    )?;
    prepare_host_mount_targets(
        &policy.filesystem.mount_bindings,
        host_mount_target_preparation(sandbox.provider_name(), mode),
    )?;
    let work_dir = root.clone();
    let lease_policy = policy.clone();

    let lease = sandbox
        .lease_cell(SandboxCellSpec::managed_component(
            key,
            cell_root,
            mode,
            lease_policy,
        ))
        .await?;

    Ok(PreparedToolSandbox {
        sandbox: ToolSandbox {
            provider: lease.provider(),
            handle: lease.handle(),
            lease: Some(lease),
        },
        root,
        work_dir,
        workspace_mounts: policy
            .filesystem
            .mount_bindings
            .iter()
            .map(|binding| WorkspaceMount {
                visible_root: PathBuf::from(&binding.target),
                host_root: PathBuf::from(&binding.source),
                writable: binding.writable,
            })
            .collect(),
    })
}

fn stage_bundle_if_needed(
    source: &Path,
    target: &Path,
    mode: SandboxMode,
) -> Result<(), RuntimeError> {
    if mode == SandboxMode::WorkspaceWrite && target_has_entries(target)? {
        return Ok(());
    }

    stage_bundle(source, target)
}

fn host_mount_target_preparation(provider_name: &str, mode: SandboxMode) -> MountTargetPreparation {
    if provider_name == "host" && mode == SandboxMode::DangerFullAccess {
        return MountTargetPreparation::HostSymlink;
    }
    MountTargetPreparation::Placeholder
}

fn prepare_host_mount_targets(
    mount_bindings: &[SandboxMountBinding],
    preparation: MountTargetPreparation,
) -> Result<(), RuntimeError> {
    for binding in mount_bindings {
        prepare_host_mount_target(
            Path::new(&binding.source),
            Path::new(&binding.target),
            preparation,
        )?;
    }
    Ok(())
}

fn prepare_host_mount_target(
    source: &Path,
    target: &Path,
    preparation: MountTargetPreparation,
) -> Result<(), RuntimeError> {
    let metadata = std::fs::metadata(source).map_err(|err| RuntimeError::Io {
        path: source.display().to_string(),
        message: err.to_string(),
    })?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| RuntimeError::Io {
            path: parent.display().to_string(),
            message: err.to_string(),
        })?;
    }
    reset_mount_target(target)?;
    if preparation == MountTargetPreparation::HostSymlink {
        return create_mount_target_symlink(source, target, metadata.is_dir());
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(target).map_err(|err| RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
    } else {
        std::fs::File::create(target).map_err(|err| RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
    }
    Ok(())
}

fn create_mount_target_symlink(
    source: &Path,
    target: &Path,
    is_dir: bool,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        let _ = is_dir;
        std::os::unix::fs::symlink(source, target).map_err(|err| RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(())
    }

    #[cfg(windows)]
    {
        let create = if is_dir {
            std::os::windows::fs::symlink_dir as fn(&Path, &Path) -> std::io::Result<()>
        } else {
            std::os::windows::fs::symlink_file as fn(&Path, &Path) -> std::io::Result<()>
        };
        create(source, target).map_err(|err| RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
        return Ok(());
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, target, is_dir);
        Err(RuntimeError::Unsupported(
            "host mount aliases require symlink support on this platform".to_string(),
        ))
    }
}

fn reset_mount_target(path: &Path) -> Result<(), RuntimeError> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };

    let result = if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir_all(path)
    };
    result.map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

fn mount_target_path(
    bundle_root: &Path,
    writable: bool,
    raw_value: &str,
    source: &Path,
) -> PathBuf {
    let base = bundle_root
        .join("mount")
        .join(if writable { "write" } else { "read" });
    if is_current_directory_mount(raw_value) {
        return base.join("current");
    }

    let mut path = base.join("abs");
    for component in source.components() {
        match component {
            Component::Prefix(prefix) => {
                path.push(format!(
                    "prefix-{}",
                    sanitize_mount_segment(&prefix.as_os_str().to_string_lossy())
                ));
            }
            Component::RootDir => {}
            Component::CurDir => path.push("current"),
            Component::ParentDir => path.push("parent"),
            Component::Normal(part) => path.push(part),
        }
    }
    path
}

fn sanitize_mount_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "mount".to_string()
    } else {
        sanitized
    }
}

fn resolve_bundle_exec_root(bundle_root: &Path, entry: &str) -> Result<String, RuntimeError> {
    validate_bundle_exec_entry(entry)?;
    Ok(bundle_root.join(entry).display().to_string())
}

fn validate_bundle_exec_entry(entry: &str) -> Result<(), RuntimeError> {
    if entry.trim().is_empty() {
        return invalid_exec_entry(entry, "entries cannot be empty");
    }

    let path = Path::new(entry);
    if path.is_absolute() {
        return invalid_exec_entry(entry, "entries must stay inside the staged bundle root");
    }

    for component in path.components() {
        if matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        ) {
            return invalid_exec_entry(entry, "entries must stay inside the staged bundle root");
        }
    }

    Ok(())
}

fn validate_staged_bundle_exec_roots(
    bundle_root: &Path,
    entries: &[String],
) -> Result<(), RuntimeError> {
    let bundle_root = bundle_root.canonicalize().map_err(|err| RuntimeError::Io {
        path: bundle_root.display().to_string(),
        message: err.to_string(),
    })?;

    for entry in entries {
        let candidate = bundle_root.join(entry);
        let resolved = candidate.canonicalize().map_err(|err| RuntimeError::Io {
            path: candidate.display().to_string(),
            message: err.to_string(),
        })?;
        if !resolved.starts_with(&bundle_root) {
            return invalid_exec_entry(
                entry,
                "entries must stay inside the staged bundle root after staging",
            );
        }
    }

    Ok(())
}

fn invalid_exec_entry(entry: &str, message: &str) -> Result<(), RuntimeError> {
    Err(RuntimeError::Sandbox(
        odyssey_rs_sandbox::SandboxError::InvalidConfig(format!(
            "sandbox.permissions.filesystem.exec entry `{entry}` {message}"
        )),
    ))
}

#[cfg(test)]
fn verify_system_tools(tools: &[String]) -> Result<(), RuntimeError> {
    let _ = resolve_system_tools(tools)?;
    Ok(())
}

fn stage_bundle(source: &Path, target: &Path) -> Result<(), RuntimeError> {
    let Some(parent) = target.parent() else {
        return Err(RuntimeError::Io {
            path: target.display().to_string(),
            message: "sandbox app root must have a parent directory".to_string(),
        });
    };

    std::fs::create_dir_all(parent).map_err(|err| RuntimeError::Io {
        path: parent.display().to_string(),
        message: err.to_string(),
    })?;

    let staging_root = parent.join(format!(".stage-{}", Uuid::new_v4().simple()));
    if let Err(err) = copy_dir_all(source, &staging_root) {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(RuntimeError::Io {
            path: staging_root.display().to_string(),
            message: err.to_string(),
        });
    }

    if target.exists()
        && let Err(err) = std::fs::remove_dir_all(target)
    {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        });
    }

    if let Err(err) = std::fs::rename(&staging_root, target) {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(RuntimeError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        });
    }
    Ok(())
}

fn build_network_policy(entries: &[String]) -> Result<SandboxNetworkPolicy, RuntimeError> {
    match entries {
        [] => Ok(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::Disabled,
        }),
        [entry] if entry == "*" => Ok(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::AllowAll,
        }),
        _ => Err(RuntimeError::Sandbox(
            odyssey_rs_sandbox::SandboxError::InvalidConfig(
                "sandbox.permissions.network only supports [] or [\"*\"] in v1".to_string(),
            ),
        )),
    }
}

fn resolve_system_tools(tools: &[String]) -> Result<Vec<String>, RuntimeError> {
    let mut resolved = Vec::with_capacity(tools.len());
    for tool in tools {
        let path = which::which(tool).map_err(|_| {
            RuntimeError::Sandbox(odyssey_rs_sandbox::SandboxError::DependencyMissing(
                format!("missing system tool: {tool}"),
            ))
        })?;
        let path = path.canonicalize().map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        resolved.push(path.display().to_string());
    }
    Ok(resolved)
}

fn validate_provider_support(
    provider_name: &str,
    mode: SandboxMode,
    policy: &SandboxPolicy,
) -> Result<(), RuntimeError> {
    if provider_name == "host" && mode != SandboxMode::DangerFullAccess {
        return Err(RuntimeError::Sandbox(
            odyssey_rs_sandbox::SandboxError::Unsupported(
                "host provider only supports danger_full_access; restricted bundle sandboxes require bubblewrap".to_string(),
            ),
        ));
    }

    if provider_name == "host" && matches!(policy.network.mode, SandboxNetworkMode::Disabled) {
        return Err(RuntimeError::Sandbox(
            odyssey_rs_sandbox::SandboxError::Unsupported(
                "bundle disables network but host execution cannot enforce that policy".to_string(),
            ),
        ));
    }

    Ok(())
}

fn target_has_entries(path: &Path) -> Result<bool, RuntimeError> {
    let mut entries = std::fs::read_dir(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?
        .is_some())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let source = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_all(&source, &target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MountTargetPreparation, build_mode, build_network_policy, build_operator_command_policy,
        build_permission_rules, build_policy, build_policy_with_resolvers, prepare_cell,
        prepare_host_mount_targets, prepare_operator_command_cell, stage_bundle,
        stage_bundle_if_needed, target_has_entries, validate_provider_support, verify_system_tools,
    };
    use odyssey_rs_manifest::{
        AgentSpec, AgentToolPolicy, BundleExecutor, BundleManifest, BundleMemory, BundleSandbox,
        BundleSandboxFilesystem, BundleSandboxLimits, BundleSandboxMounts,
        BundleSandboxPermissions, BundleSystemToolsMode, ManifestVersion, ProviderKind,
    };
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_sandbox::{
        LocalSandboxProvider, SandboxMountBinding, SandboxNetworkMode, SandboxPolicy,
        SandboxRuntime,
    };
    use odyssey_rs_tools::{PermissionAction, ToolPermissionMatcher, ToolPermissionRule};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn build_policy_includes_host_mounts() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions {
                    filesystem: BundleSandboxFilesystem {
                        exec: Vec::new(),
                        mounts: BundleSandboxMounts {
                            read: vec!["/sandbox-test/host-read".to_string()],
                            write: vec!["/sandbox-test/host-write".to_string()],
                        },
                    },
                    network: Vec::new(),
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy(Path::new("/bundle"), &manifest).expect("build policy");

        assert!(
            policy
                .filesystem
                .read_roots
                .contains(&"/bundle/mount/read/abs/sandbox-test/host-read".into())
        );
        assert!(
            policy
                .filesystem
                .write_roots
                .contains(&"/bundle/mount/write/abs/sandbox-test/host-write".into())
        );
        assert!(policy.filesystem.mount_bindings.iter().any(|binding| {
            binding.source == "/sandbox-test/host-read"
                && binding.target == "/bundle/mount/read/abs/sandbox-test/host-read"
                && !binding.writable
        }));
        assert!(policy.filesystem.mount_bindings.iter().any(|binding| {
            binding.source == "/sandbox-test/host-write"
                && binding.target == "/bundle/mount/write/abs/sandbox-test/host-write"
                && binding.writable
        }));
        assert!(
            !policy
                .filesystem
                .read_roots
                .contains(&"/sandbox-test/host-read".into())
        );
        assert_eq!(policy.network.mode, SandboxNetworkMode::Disabled);
    }

    #[test]
    fn build_policy_resolves_current_directory_mounts() {
        let temp = tempdir().expect("tempdir");
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions {
                    filesystem: BundleSandboxFilesystem {
                        exec: Vec::new(),
                        mounts: BundleSandboxMounts {
                            read: vec![".".to_string()],
                            write: Vec::new(),
                        },
                    },
                    network: Vec::new(),
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy_with_resolvers(
            Path::new("/bundle"),
            &manifest,
            &[],
            false,
            |_| None,
            temp.path(),
        )
        .expect("build policy");

        assert!(
            policy
                .filesystem
                .read_roots
                .contains(&"/bundle/mount/read/current".to_string())
        );
        assert!(policy.filesystem.mount_bindings.iter().any(|binding| {
            binding.source == temp.path().display().to_string()
                && binding.target == "/bundle/mount/read/current"
                && !binding.writable
        }));
    }

    #[test]
    fn prepare_host_mount_targets_creates_mount_placeholder() {
        let bundle_root = tempdir().expect("bundle root");
        let current_dir = tempdir().expect("current dir");
        let target_path = bundle_root
            .path()
            .join("mount")
            .join("read")
            .join("current");

        prepare_host_mount_targets(
            &[SandboxMountBinding {
                source: current_dir.path().display().to_string(),
                target: target_path.display().to_string(),
                writable: false,
            }],
            MountTargetPreparation::Placeholder,
        )
        .expect("prepare mount targets");

        let metadata = std::fs::metadata(&target_path).expect("target metadata");
        assert!(metadata.is_dir());
        assert!(
            !target_path
                .symlink_metadata()
                .expect("symlink metadata")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    #[cfg(unix)]
    fn prepare_host_mount_targets_symlink_mounts_in_host_mode() {
        let bundle_root = tempdir().expect("bundle root");
        let current_dir = tempdir().expect("current dir");
        std::fs::write(current_dir.path().join("mounted.txt"), "hello mount").expect("file");
        let target_path = bundle_root
            .path()
            .join("mount")
            .join("read")
            .join("current");

        prepare_host_mount_targets(
            &[SandboxMountBinding {
                source: current_dir.path().display().to_string(),
                target: target_path.display().to_string(),
                writable: false,
            }],
            MountTargetPreparation::HostSymlink,
        )
        .expect("prepare mount targets");

        let metadata = std::fs::symlink_metadata(&target_path).expect("target metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(&target_path).expect("read link"),
            current_dir.path()
        );
        let content =
            std::fs::read_to_string(target_path.join("mounted.txt")).expect("mounted content");
        assert_eq!(content, "hello mount");
    }

    #[test]
    fn build_mode_prefers_runtime_override() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions::default(),
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        assert_eq!(build_mode(&manifest, None), SandboxMode::ReadOnly);
        assert_eq!(
            build_mode(&manifest, Some(SandboxMode::DangerFullAccess)),
            SandboxMode::DangerFullAccess
        );
    }

    #[test]
    fn stage_bundle_replaces_existing_workspace_contents() {
        let src = tempdir().expect("src");
        let dst = tempdir().expect("dst");
        std::fs::write(src.path().join("hello.txt"), "from bundle").expect("write src");
        std::fs::write(dst.path().join("hello.txt"), "modified").expect("write dst");

        stage_bundle(src.path(), dst.path()).expect("stage");

        let content = std::fs::read_to_string(dst.path().join("hello.txt")).expect("read dst");
        assert_eq!(content, "from bundle");
    }

    #[test]
    fn workspace_write_stage_preserves_existing_sandbox_changes() {
        let src = tempdir().expect("src");
        let dst = tempdir().expect("dst");
        std::fs::write(src.path().join("hello.txt"), "from bundle").expect("write src");
        std::fs::write(dst.path().join("hello.txt"), "modified").expect("write dst");

        stage_bundle_if_needed(src.path(), dst.path(), SandboxMode::WorkspaceWrite)
            .expect("stage if needed");

        let content = std::fs::read_to_string(dst.path().join("hello.txt")).expect("read dst");
        assert_eq!(content, "modified");
    }

    #[test]
    fn build_policy_rejects_unsafe_exec_entries() {
        let mut manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::WorkspaceWrite,
                permissions: BundleSandboxPermissions {
                    filesystem: BundleSandboxFilesystem {
                        exec: vec!["../bin/run".to_string()],
                        mounts: BundleSandboxMounts::default(),
                    },
                    network: vec!["*".to_string()],
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let traversal = build_policy(Path::new("/bundle-root"), &manifest)
            .expect_err("traversal exec root should fail");
        assert!(
            traversal
                .to_string()
                .contains("must stay inside the staged bundle root")
        );

        manifest.sandbox.permissions.filesystem.exec = vec!["/usr/bin/sh".to_string()];
        let absolute = build_policy(Path::new("/bundle-root"), &manifest)
            .expect_err("absolute exec root should fail");
        assert!(
            absolute
                .to_string()
                .contains("must stay inside the staged bundle root")
        );
    }

    #[tokio::test]
    async fn agent_and_operator_cells_share_workspace_without_policy_collision() {
        let temp = tempdir().expect("tempdir");
        let sandbox = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("sandbox runtime");
        let bundle_root = temp.path().join("bundle");
        std::fs::create_dir_all(&bundle_root).expect("bundle root");
        std::fs::write(
            bundle_root.join("agent.yaml"),
            "id: demo\nprompt: hi\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\n",
        )
        .expect("agent");

        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::DangerFullAccess,
                permissions: BundleSandboxPermissions {
                    filesystem: BundleSandboxFilesystem::default(),
                    network: vec!["*".to_string()],
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let session_id = Uuid::new_v4();
        let agent = prepare_cell(&sandbox, session_id, "demo", &bundle_root, &manifest, None)
            .await
            .expect("agent cell");
        let command = prepare_operator_command_cell(
            &sandbox,
            session_id,
            "demo",
            &bundle_root,
            &manifest,
            None,
        )
        .await
        .expect("command cell");

        let agent_lease = agent.sandbox.lease.as_ref().expect("agent lease");
        let command_lease = command.sandbox.lease.as_ref().expect("command lease");

        assert_eq!(agent.root, command.root);
        assert_eq!(agent_lease.cell_root(), command_lease.cell_root());
        assert_eq!(agent_lease.workspace_root(), agent.root.as_path());
        assert_eq!(command_lease.workspace_root(), command.root.as_path());
        assert_ne!(
            agent_lease.key().component_id,
            command_lease.key().component_id
        );
    }

    #[test]
    fn build_policy_maps_exec_roots_and_resource_limits() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::WorkspaceWrite,
                permissions: BundleSandboxPermissions {
                    filesystem: BundleSandboxFilesystem {
                        exec: vec!["bin/run".to_string()],
                        mounts: BundleSandboxMounts::default(),
                    },
                    network: vec!["*".to_string()],
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: vec!["sh".to_string()],
                resources: BundleSandboxLimits {
                    cpu: Some(3),
                    memory_mb: Some(64),
                },
            },
        };

        let policy = build_policy(Path::new("/bundle-root"), &manifest).expect("build policy");
        let sh = which::which("sh")
            .expect("resolve sh")
            .canonicalize()
            .expect("canonicalize sh");

        assert!(
            policy
                .filesystem
                .exec_roots
                .contains(&"/bundle-root/bin/run".to_string())
        );
        assert!(
            policy
                .filesystem
                .exec_roots
                .contains(&sh.display().to_string())
        );
        assert_eq!(policy.network.mode, SandboxNetworkMode::AllowAll);
        assert_eq!(policy.limits.cpu_seconds, Some(3));
        assert_eq!(policy.limits.memory_bytes, Some(64 * 1024 * 1024));
    }

    #[test]
    fn build_policy_maps_manifest_env_into_sandbox_policy() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::WorkspaceWrite,
                permissions: BundleSandboxPermissions::default(),
                env: BTreeMap::from([
                    ("OPENAI_API_KEY".to_string(), "ODYSSEY_TEST_ENV".to_string()),
                    ("APP_ENV".to_string(), "ODYSSEY_TEST_APP_ENV".to_string()),
                ]),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy_with_resolvers(
            Path::new("/bundle-root"),
            &manifest,
            &[],
            false,
            |name| match name {
                "ODYSSEY_TEST_ENV" => Some("secret".to_string()),
                "ODYSSEY_TEST_APP_ENV" => Some("development".to_string()),
                _ => None,
            },
            Path::new("/runtime-cwd"),
        )
        .expect("build policy");
        assert!(policy.env.inherit.is_empty());
        assert_eq!(
            policy.env.set.get("OPENAI_API_KEY"),
            Some(&"secret".to_string())
        );
        assert_eq!(
            policy.env.set.get("APP_ENV"),
            Some(&"development".to_string())
        );
    }

    #[test]
    fn operator_command_policy_includes_standard_system_exec_roots() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions::default(),
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy =
            build_operator_command_policy(Path::new("/bundle-root"), &manifest).expect("policy");

        assert!(
            policy
                .filesystem
                .exec_roots
                .iter()
                .any(|path| path == "/usr" || path == "/bin")
        );
    }

    #[test]
    fn build_network_policy_rejects_partial_allowlists() {
        let error = build_network_policy(&["wttr.in".to_string()]).expect_err("reject allowlist");
        assert!(error.to_string().contains("only supports [] or [\"*\"]"));
    }

    #[test]
    fn validate_provider_support_rejects_host_for_restricted_modes() {
        let policy = SandboxPolicy::default();
        let error = validate_provider_support("host", SandboxMode::WorkspaceWrite, &policy)
            .expect_err("restricted host rejected");
        assert!(error.to_string().contains("danger_full_access"));
    }

    #[test]
    fn build_permission_rules_maps_agent_actions() {
        let agent = AgentSpec {
            id: "demo".to_string(),
            description: String::default(),
            prompt: "test".to_string(),
            model: odyssey_rs_protocol::ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
                config: None,
            },
            tools: AgentToolPolicy {
                allow: vec!["read".to_string(), "Bash(find:*)".to_string()],
                ask: vec!["bash".to_string(), "Bash(cargo test:*)".to_string()],
                deny: vec![
                    "write".to_string(),
                    "WebFetch(domain:liquidos.ai)".to_string(),
                ],
            },
        };

        let rules = build_permission_rules(&agent);

        assert_eq!(
            rules,
            vec![
                ToolPermissionRule {
                    action: PermissionAction::Allow,
                    matcher: ToolPermissionMatcher::parse("read").expect("read matcher"),
                },
                ToolPermissionRule {
                    action: PermissionAction::Allow,
                    matcher: ToolPermissionMatcher::parse("Bash(find:*)").expect("allow matcher"),
                },
                ToolPermissionRule {
                    action: PermissionAction::Ask,
                    matcher: ToolPermissionMatcher::parse("bash").expect("bash matcher"),
                },
                ToolPermissionRule {
                    action: PermissionAction::Ask,
                    matcher: ToolPermissionMatcher::parse("Bash(cargo test:*)")
                        .expect("ask matcher"),
                },
                ToolPermissionRule {
                    action: PermissionAction::Deny,
                    matcher: ToolPermissionMatcher::parse("write").expect("write matcher"),
                },
                ToolPermissionRule {
                    action: PermissionAction::Deny,
                    matcher: ToolPermissionMatcher::parse("WebFetch(domain:liquidos.ai)")
                        .expect("deny matcher"),
                },
            ]
        );
    }

    #[test]
    fn verify_system_tools_accepts_existing_binary_and_rejects_missing_one() {
        verify_system_tools(&["sh".to_string()]).expect("sh available");

        let error = verify_system_tools(&["odyssey-rs-missing-tool".to_string()])
            .expect_err("missing tool rejected");
        assert!(error.to_string().contains("missing system tool"));
    }

    #[test]
    fn build_policy_adds_standard_exec_roots_when_requested() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions::default(),
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Standard,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy(Path::new("/bundle-root"), &manifest).expect("build policy");
        assert!(
            policy
                .filesystem
                .exec_roots
                .iter()
                .any(|path| path == "/usr" || path == "/bin")
        );
        assert!(!policy.filesystem.exec_allow_all);
    }

    #[test]
    fn build_policy_allows_all_exec_paths_when_requested() {
        let manifest = BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: Vec::new(),
            sandbox: BundleSandbox {
                mode: SandboxMode::ReadOnly,
                permissions: BundleSandboxPermissions::default(),
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::All,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy(Path::new("/bundle-root"), &manifest).expect("build policy");
        assert!(policy.filesystem.exec_allow_all);
    }

    #[test]
    fn target_has_entries_distinguishes_empty_and_populated_directories() {
        let temp = tempdir().expect("tempdir");
        let empty = temp.path().join("empty");
        let populated = temp.path().join("populated");
        std::fs::create_dir_all(&empty).expect("create empty");
        std::fs::create_dir_all(&populated).expect("create populated");
        std::fs::write(populated.join("file.txt"), "data").expect("write file");

        assert!(!target_has_entries(&empty).expect("empty dir"));
        assert!(target_has_entries(&populated).expect("populated dir"));
    }

    #[test]
    fn stage_bundle_copies_nested_directories_into_empty_target() {
        let src = tempdir().expect("src");
        let dst = tempdir().expect("dst");
        let source_file = src.path().join("nested").join("bundle.txt");
        std::fs::create_dir_all(source_file.parent().expect("source parent"))
            .expect("create source nested");
        std::fs::write(&source_file, "hello").expect("write source file");

        let target = dst.path().join("app");
        stage_bundle(src.path(), &target).expect("stage bundle");

        let staged = std::fs::read_to_string(target.join("nested").join("bundle.txt"))
            .expect("read staged file");
        assert_eq!(staged, "hello");
    }
}
