use crate::RuntimeError;
use odyssey_rs_manifest::{BundleManifest, BundlePermissionAction};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    SandboxCellKey, SandboxCellSpec, SandboxLimits, SandboxNetworkMode, SandboxNetworkPolicy,
    SandboxPolicy, SandboxRuntime,
};
use odyssey_rs_tools::{PermissionAction, ToolSandbox};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) struct PreparedToolSandbox {
    pub sandbox: ToolSandbox,
    pub root: PathBuf,
    pub work_dir: PathBuf,
}

pub fn build_policy(bundle_root: &Path, manifest: &BundleManifest) -> SandboxPolicy {
    let map_bundle_paths = |entries: &[String]| -> Vec<String> {
        entries
            .iter()
            .map(|entry| bundle_root.join(entry).display().to_string())
            .collect()
    };
    let map_host_paths = |entries: &[String]| -> Vec<String> { entries.to_vec() };

    SandboxPolicy {
        filesystem: odyssey_rs_sandbox::SandboxFilesystemPolicy {
            read_roots: map_host_paths(&manifest.sandbox.permissions.filesystem.mounts.read),
            write_roots: map_host_paths(&manifest.sandbox.permissions.filesystem.mounts.write),
            exec_roots: map_bundle_paths(&manifest.sandbox.permissions.filesystem.exec),
        },
        network: SandboxNetworkPolicy {
            mode: if manifest.sandbox.permissions.network.is_empty() {
                SandboxNetworkMode::Disabled
            } else {
                SandboxNetworkMode::AllowAll
            },
        },
        limits: SandboxLimits {
            cpu_seconds: manifest.sandbox.resources.cpu,
            memory_bytes: manifest
                .sandbox
                .resources
                .memory_mb
                .map(|value| value * 1024 * 1024),
            ..SandboxLimits::default()
        },
        ..SandboxPolicy::default()
    }
}

pub fn build_mode(manifest: &BundleManifest, override_mode: Option<SandboxMode>) -> SandboxMode {
    override_mode.unwrap_or(manifest.sandbox.mode)
}

pub fn build_permission_rules(manifest: &BundleManifest) -> HashMap<String, PermissionAction> {
    manifest
        .sandbox
        .permissions
        .tools
        .rules
        .iter()
        .map(|rule| {
            let action = match rule.action {
                BundlePermissionAction::Allow => PermissionAction::Allow,
                BundlePermissionAction::Deny => PermissionAction::Deny,
                BundlePermissionAction::Ask => PermissionAction::Ask,
            };
            (rule.tool.clone(), action)
        })
        .collect()
}

pub async fn prepare_cell(
    sandbox: &SandboxRuntime,
    session_id: Uuid,
    agent_id: &str,
    bundle_root: &Path,
    manifest: &BundleManifest,
    override_mode: Option<SandboxMode>,
) -> Result<PreparedToolSandbox, RuntimeError> {
    verify_system_tools(&manifest.sandbox.system_tools)?;
    let mode = build_mode(manifest, override_mode);
    let key = SandboxCellKey::tooling(session_id, agent_id);
    let cell_root = sandbox.managed_cell_root(&key)?;
    let root = cell_root.join("app");
    stage_bundle(bundle_root, &root)?;
    let work_dir = root.clone();
    let policy = build_policy(&root, manifest);

    if sandbox.provider_name() == "host"
        && matches!(policy.network.mode, SandboxNetworkMode::Disabled)
    {
        return Err(RuntimeError::Sandbox(
            odyssey_rs_sandbox::SandboxError::Unsupported(
                "bundle disables network but runtime fell back to host execution without kernel isolation".to_string(),
            ),
        ));
    }

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

fn verify_system_tools(tools: &[String]) -> Result<(), RuntimeError> {
    for tool in tools {
        which::which(tool).map_err(|_| {
            RuntimeError::Sandbox(odyssey_rs_sandbox::SandboxError::DependencyMissing(
                format!("missing system tool: {tool}"),
            ))
        })?;
    }
    Ok(())
}

fn stage_bundle(source: &Path, target: &Path) -> Result<(), RuntimeError> {
    if target.exists() && target_has_entries(target)? {
        return Ok(());
    }
    copy_dir_all(source, target).map_err(|err| RuntimeError::Io {
        path: target.display().to_string(),
        message: err.to_string(),
    })?;
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
        build_mode, build_permission_rules, build_policy, extend_cell_filesystem_policy,
        stage_bundle, target_has_entries, verify_system_tools,
    };
    use odyssey_rs_manifest::{
        BundleExecutor, BundleManifest, BundleMemory, BundlePermissionAction, BundlePermissionRule,
        BundleSandbox, BundleSandboxFilesystem, BundleSandboxLimits, BundleSandboxMounts,
        BundleSandboxPermissions, BundleSandboxTools, ManifestVersion, ProviderKind,
    };
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_sandbox::{SandboxNetworkMode, SandboxPolicy};
    use odyssey_rs_tools::PermissionAction;
    use serde_json::Value;
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
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let policy = build_policy(Path::new("/bundle"), &manifest);

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
    fn stage_bundle_preserves_existing_workspace_changes() {
        let src = tempdir().expect("src");
        let dst = tempdir().expect("dst");
        std::fs::write(src.path().join("hello.txt"), "from bundle").expect("write src");
        std::fs::write(dst.path().join("hello.txt"), "modified").expect("write dst");

        stage_bundle(src.path(), dst.path()).expect("stage");

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
                    network: vec!["https://example.com".to_string()],
                    tools: BundleSandboxTools::default(),
                },
                system_tools: Vec::new(),
                resources: BundleSandboxLimits {
                    cpu: Some(3),
                    memory_mb: Some(64),
                    gpu: None,
                },
            },
        };

        let policy = build_policy(Path::new("/bundle-root"), &manifest);

        assert_eq!(
            policy.filesystem.exec_roots,
            vec!["/bundle-root/bin/run".to_string()]
        );
        assert_eq!(policy.network.mode, SandboxNetworkMode::AllowAll);
        assert_eq!(policy.limits.cpu_seconds, Some(3));
        assert_eq!(policy.limits.memory_bytes, Some(64 * 1024 * 1024));
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
                        mode: "default".to_string(),
                        rules: vec![
                            BundlePermissionRule {
                                action: BundlePermissionAction::Allow,
                                tool: "read".to_string(),
                            },
                            BundlePermissionRule {
                                action: BundlePermissionAction::Deny,
                                tool: "write".to_string(),
                            },
                            BundlePermissionRule {
                                action: BundlePermissionAction::Ask,
                                tool: "bash".to_string(),
                            },
                        ],
                    },
                },
                system_tools: Vec::new(),
                resources: BundleSandboxLimits::default(),
            },
        };

        let rules = build_permission_rules(&manifest);

        assert_eq!(rules.get("read"), Some(&PermissionAction::Allow));
        assert_eq!(rules.get("write"), Some(&PermissionAction::Deny));
        assert_eq!(rules.get("bash"), Some(&PermissionAction::Ask));
    }

    #[test]
    fn verify_system_tools_accepts_existing_binary_and_rejects_missing_one() {
        verify_system_tools(&["sh".to_string()]).expect("sh available");

        let error = verify_system_tools(&["odyssey-rs-missing-tool".to_string()])
            .expect_err("missing tool rejected");
        assert!(error.to_string().contains("missing system tool"));
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
