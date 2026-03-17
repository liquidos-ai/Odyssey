use crate::{AgentSpec, BundleManifest, ManifestError};
use std::fs;
use std::path::{Path, PathBuf};

pub fn load_project(root: &Path) -> Result<(BundleManifest, AgentSpec), ManifestError> {
    let manifest = load_bundle_manifest(&root.join("odyssey.bundle.json5"))?;
    let agent = load_agent_spec(&root.join(&manifest.agent_spec))?;
    validate_project(root, &manifest, &agent)?;
    Ok((manifest, agent))
}

pub fn load_bundle_manifest(path: &Path) -> Result<BundleManifest, ManifestError> {
    let content = fs::read_to_string(path).map_err(|err| ManifestError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let manifest =
        json5::from_str::<BundleManifest>(&content).map_err(|err| ManifestError::Json5Parse {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
    Ok(manifest)
}

pub fn load_agent_spec(path: &Path) -> Result<AgentSpec, ManifestError> {
    let content = fs::read_to_string(path).map_err(|err| ManifestError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    serde_yaml::from_str::<AgentSpec>(&content).map_err(|err| ManifestError::YamlParse {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

fn validate_project(
    root: &Path,
    manifest: &BundleManifest,
    agent: &AgentSpec,
) -> Result<(), ManifestError> {
    if manifest.id.trim().is_empty() {
        return invalid(root, "bundle id cannot be empty");
    }
    if manifest.version.trim().is_empty() {
        return invalid(root, "bundle version cannot be empty");
    }
    if manifest.executor.kind != "prebuilt" {
        return invalid(root, "only prebuilt executors are supported in v1");
    }
    if manifest.memory.provider.kind != "prebuilt" {
        return invalid(root, "only prebuilt memory providers are supported in v1");
    }
    if manifest.executor.id.trim().is_empty() {
        return invalid(root, "executor id cannot be empty");
    }
    if manifest.memory.provider.id.trim().is_empty() {
        return invalid(root, "memory provider id cannot be empty");
    }
    if agent.id.trim().is_empty() {
        return invalid(root, "agent id cannot be empty");
    }
    if agent.prompt.trim().is_empty() {
        return invalid(root, "agent prompt cannot be empty");
    }
    if agent.model.provider.trim().is_empty() || agent.model.name.trim().is_empty() {
        return invalid(root, "agent model provider and name are required");
    }
    for resource in &manifest.resources {
        ensure_relative_entry(root, resource, "resource")?;
    }
    for skill in &manifest.skills {
        ensure_relative_entry(root, &skill.path, "skill path")?;
    }
    for tool in &manifest.tools {
        if tool.source != "builtin" {
            return invalid(root, "only builtin tools are supported in v1");
        }
    }
    for path in &manifest.sandbox.permissions.filesystem.mounts.read {
        ensure_absolute_mount(root, path, "read mount")?;
    }
    for path in &manifest.sandbox.permissions.filesystem.mounts.write {
        ensure_absolute_mount(root, path, "write mount")?;
    }
    Ok(())
}

fn ensure_relative_entry(root: &Path, value: &str, label: &str) -> Result<(), ManifestError> {
    if value.contains("wasm") || value.contains("store") {
        return invalid(root, &format!("{label} {value} is not supported in v1"));
    }
    let path = root.join(value);
    if !path.exists() {
        return Err(ManifestError::Invalid {
            path: path.display().to_string(),
            message: format!("{label} does not exist"),
        });
    }
    Ok(())
}

fn ensure_absolute_mount(root: &Path, value: &str, label: &str) -> Result<(), ManifestError> {
    if value.contains("wasm") || value.contains("store") {
        return invalid(root, &format!("{label} {value} is not supported in v1"));
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return invalid(root, &format!("{label} must be an absolute host path"));
    }
    Ok(())
}

fn invalid(root: &Path, message: &str) -> Result<(), ManifestError> {
    Err(ManifestError::Invalid {
        path: root.display().to_string(),
        message: message.to_string(),
    })
}

#[allow(dead_code)]
fn _normalize(root: &Path, value: &str) -> PathBuf {
    root.join(value)
}

#[cfg(test)]
mod tests {
    use super::load_project;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn load_project_validates_prebuilt_only() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { provider: { type: 'prebuilt', id: 'sliding_window' } },
                skills: [],
                resources: [],
                tools: [{ name: 'Read', source: 'builtin' }],
                server: { enable_http: true },
                sandbox: { permissions: { filesystem: { exec: [], mounts: { read: [], write: [] } }, network: [], tools: { mode: 'default', rules: [] } }, system_tools: [], resources: {} }
            }"#,
        )
        .expect("write manifest");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n  deny: []\n",
        )
        .expect("write agent");

        let (manifest, agent) = load_project(temp.path()).expect("project");
        assert_eq!(manifest.executor.id, "react");
        assert_eq!(agent.id, "demo");
    }

    #[test]
    fn load_project_rejects_relative_host_mounts() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { provider: { type: 'prebuilt', id: 'sliding_window' } },
                skills: [],
                resources: [],
                tools: [{ name: 'Read', source: 'builtin' }],
                server: { enable_http: true },
                sandbox: {
                    permissions: {
                        filesystem: {
                            exec: [],
                            mounts: { read: ['tmp/project'], write: [] }
                        },
                        network: [],
                        tools: { mode: 'default', rules: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n  deny: []\n",
        )
        .expect("write agent");

        let error = load_project(temp.path()).expect_err("relative host mount should fail");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid manifest at {}: read mount must be an absolute host path",
                temp.path().display()
            )
        );
    }
}
