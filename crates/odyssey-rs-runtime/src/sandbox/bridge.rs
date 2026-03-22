use crate::RuntimeError;
use odyssey_rs_manifest::{BundleManifest, BundleSystemToolsMode};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    SandboxCellKey, SandboxCellSpec, SandboxLimits, SandboxNetworkMode, SandboxNetworkPolicy,
    SandboxPolicy, SandboxRuntime, standard_system_exec_roots,
};
use odyssey_rs_tools::{PermissionAction, ToolPermissionMatcher, ToolPermissionRule, ToolSandbox};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) struct PreparedToolSandbox {
    pub sandbox: ToolSandbox,
    pub root: PathBuf,
    pub work_dir: PathBuf,
}

pub fn build_policy(
    bundle_root: &Path,
    manifest: &BundleManifest,
) -> Result<SandboxPolicy, RuntimeError> {
    build_policy_with_exec_roots(bundle_root, manifest, &[], false)
}

pub fn build_operator_command_policy(
    bundle_root: &Path,
    manifest: &BundleManifest,
) -> Result<SandboxPolicy, RuntimeError> {
    build_policy_with_exec_roots(bundle_root, manifest, &[], true)
}

fn build_policy_with_exec_roots(
    bundle_root: &Path,
    manifest: &BundleManifest,
    extra_exec_roots: &[String],
    force_standard_exec_roots: bool,
) -> Result<SandboxPolicy, RuntimeError> {
    let map_bundle_paths = |entries: &[String]| -> Vec<String> {
        entries
            .iter()
            .map(|entry| bundle_root.join(entry).display().to_string())
            .collect()
    };
    let map_host_paths = |entries: &[String]| -> Vec<String> { entries.to_vec() };
    let explicit_system_tools = resolve_system_tools(&manifest.sandbox.system_tools)?;
    let mut exec_roots = map_bundle_paths(&manifest.sandbox.permissions.filesystem.exec);
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
            read_roots: map_host_paths(&manifest.sandbox.permissions.filesystem.mounts.read),
            write_roots: map_host_paths(&manifest.sandbox.permissions.filesystem.mounts.write),
            exec_roots,
            exec_allow_all,
        },
        env: odyssey_rs_sandbox::SandboxEnvPolicy {
            inherit: Vec::new(),
            set: resolve_manifest_env(&manifest.sandbox.env),
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

fn resolve_manifest_env(
    env: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    env.iter()
        .filter_map(|(target, source)| {
            std::env::var(source)
                .ok()
                .map(|value| (target.clone(), value))
        })
        .collect()
}

pub fn build_mode(manifest: &BundleManifest, override_mode: Option<SandboxMode>) -> SandboxMode {
    override_mode.unwrap_or(manifest.sandbox.mode)
}

pub fn build_permission_rules(manifest: &BundleManifest) -> Vec<ToolPermissionRule> {
    let mut permissions = Vec::new();
    for rule in &manifest.sandbox.permissions.tools.allow {
        permissions.push(build_permission_rule(PermissionAction::Allow, rule));
    }
    for rule in &manifest.sandbox.permissions.tools.ask {
        permissions.push(build_permission_rule(PermissionAction::Ask, rule));
    }
    for rule in &manifest.sandbox.permissions.tools.deny {
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
        session_id,
        agent_id,
        bundle_root,
        manifest,
        override_mode,
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
        session_id,
        agent_id,
        bundle_root,
        manifest,
        override_mode,
        build_operator_command_policy,
    )
    .await
}

async fn prepare_cell_with_policy(
    sandbox: &SandboxRuntime,
    session_id: Uuid,
    agent_id: &str,
    bundle_root: &Path,
    manifest: &BundleManifest,
    override_mode: Option<SandboxMode>,
    policy_builder: fn(&Path, &BundleManifest) -> Result<SandboxPolicy, RuntimeError>,
) -> Result<PreparedToolSandbox, RuntimeError> {
    let mode = build_mode(manifest, override_mode);
    let key = SandboxCellKey::tooling(session_id, agent_id);
    let cell_root = sandbox.managed_cell_root(&key)?;
    let root = cell_root.join("app");
    let policy = policy_builder(&root, manifest)?;
    validate_provider_support(sandbox.provider_name(), mode, &policy)?;
    stage_bundle_if_needed(bundle_root, &root, mode)?;
    let work_dir = root.clone();

    let (read_roots, write_roots, exec_roots) =
        extend_cell_filesystem_policy(&policy, &cell_root, mode);

    let lease = sandbox
        .lease_cell(SandboxCellSpec::managed_component(
            key,
            mode,
            SandboxPolicy {
                filesystem: odyssey_rs_sandbox::SandboxFilesystemPolicy {
                    read_roots,
                    write_roots,
                    exec_roots,
                    exec_allow_all: policy.filesystem.exec_allow_all,
                },
                env: policy.env.clone(),
                network: policy.network.clone(),
                limits: policy.limits.clone(),
            },
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

fn extend_cell_filesystem_policy(
    policy: &SandboxPolicy,
    cell_root: &Path,
    mode: SandboxMode,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let cell_root = cell_root.display().to_string();

    let mut read_roots = policy.filesystem.read_roots.clone();
    read_roots.push(cell_root.clone());

    let mut write_roots = policy.filesystem.write_roots.clone();
    if matches!(
        mode,
        SandboxMode::WorkspaceWrite | SandboxMode::DangerFullAccess
    ) {
        write_roots.push(cell_root.clone());
    }

    let mut exec_roots = policy.filesystem.exec_roots.clone();
    exec_roots.push(cell_root);

    (read_roots, write_roots, exec_roots)
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
        build_mode, build_network_policy, build_operator_command_policy, build_permission_rules,
        build_policy, extend_cell_filesystem_policy, stage_bundle, stage_bundle_if_needed,
        target_has_entries, validate_provider_support, verify_system_tools,
    };
    use odyssey_rs_manifest::{
        BundleExecutor, BundleManifest, BundleMemory, BundleSandbox, BundleSandboxFilesystem,
        BundleSandboxLimits, BundleSandboxMounts, BundleSandboxPermissions, BundleSandboxTools,
        BundleSystemToolsMode, ManifestVersion, ProviderKind,
    };
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_sandbox::{SandboxNetworkMode, SandboxPolicy};
    use odyssey_rs_tools::{PermissionAction, ToolPermissionMatcher, ToolPermissionRule};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::path::Path;
    use tempfile::tempdir;

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
                    tools: BundleSandboxTools::default(),
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
                .contains(&"/sandbox-test/host-read".into())
        );
        assert!(
            policy
                .filesystem
                .write_roots
                .contains(&"/sandbox-test/host-write".into())
        );
        assert_eq!(policy.network.mode, SandboxNetworkMode::Disabled);
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
    fn read_only_mode_does_not_add_managed_cell_write_root() {
        let policy = SandboxPolicy::default();
        let cell_root = Path::new("/sandbox-test/cell");

        let (_, read_only_writes, _) =
            extend_cell_filesystem_policy(&policy, cell_root, SandboxMode::ReadOnly);
        let (_, workspace_writes, _) =
            extend_cell_filesystem_policy(&policy, cell_root, SandboxMode::WorkspaceWrite);

        assert!(!read_only_writes.contains(&"/sandbox-test/cell".to_string()));
        assert!(workspace_writes.contains(&"/sandbox-test/cell".to_string()));
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
                    tools: BundleSandboxTools::default(),
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

        unsafe {
            std::env::set_var("ODYSSEY_TEST_ENV", "secret");
            std::env::set_var("ODYSSEY_TEST_APP_ENV", "development");
        }
        let policy = build_policy(Path::new("/bundle-root"), &manifest).expect("build policy");
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
    fn build_permission_rules_maps_manifest_actions() {
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
                    filesystem: BundleSandboxFilesystem::default(),
                    network: Vec::new(),
                    tools: BundleSandboxTools {
                        allow: vec!["read".to_string(), "Bash(find:*)".to_string()],
                        ask: vec!["bash".to_string(), "Bash(cargo test:*)".to_string()],
                        deny: vec![
                            "write".to_string(),
                            "WebFetch(domain:liquidos.ai)".to_string(),
                        ],
                    },
                },
                env: BTreeMap::new(),
                system_tools_mode: BundleSystemToolsMode::Explicit,
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let rules = build_permission_rules(&manifest);

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
