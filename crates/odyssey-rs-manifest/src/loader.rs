use crate::bundle_manifest::{ManifestVersion, ProviderKind};
use crate::{AgentSpec, BundleManifest, ManifestError};
use std::fs;
use std::path::{Path, PathBuf};

pub struct BundleLoader<'a> {
    root: &'a Path,
}

impl<'a> BundleLoader<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root }
    }

    pub fn load_project(&self) -> Result<(BundleManifest, AgentSpec), ManifestError> {
        let manifest = self.load_bundle_manifest(&self.root.join("odyssey.bundle.json5"))?;
        let agent = self.load_agent_spec(&self.root.join(&manifest.agent_spec))?;
        self.validate_project(&manifest, &agent)?;
        Ok((manifest, agent))
    }

    pub fn load_bundle_manifest(&self, path: &Path) -> Result<BundleManifest, ManifestError> {
        let content = fs::read_to_string(path).map_err(|err| ManifestError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        let manifest = json5::from_str::<BundleManifest>(&content).map_err(|err| {
            ManifestError::Json5Parse {
                path: path.display().to_string(),
                message: err.to_string(),
            }
        })?;
        Ok(manifest)
    }

    pub fn load_agent_spec(&self, path: &Path) -> Result<AgentSpec, ManifestError> {
        let content = fs::read_to_string(path).map_err(|err| ManifestError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        serde_yaml::from_str::<AgentSpec>(&content).map_err(|err| ManifestError::YamlParse {
            path: path.display().to_string(),
            message: err.to_string(),
        })
    }

    pub fn validate_project(
        &self,
        manifest: &BundleManifest,
        agent: &AgentSpec,
    ) -> Result<(), ManifestError> {
        match &manifest.manifest_version {
            ManifestVersion::V1 => self.validate_v1(manifest, agent),
        }
    }

    fn validate_v1(
        &self,
        manifest: &BundleManifest,
        agent: &AgentSpec,
    ) -> Result<(), ManifestError> {
        if manifest.id.trim().is_empty() {
            return invalid(self.root, "bundle id cannot be empty");
        }
        if manifest.version.trim().is_empty() {
            return invalid(self.root, "bundle version cannot be empty");
        }
        if manifest.readme.trim().is_empty() {
            return invalid(self.root, "bundle readme cannot be empty");
        }
        if manifest.executor.kind != ProviderKind::Prebuilt {
            return invalid(self.root, "only prebuilt executors are supported in v1");
        }
        if manifest.memory.kind != ProviderKind::Prebuilt {
            return invalid(
                self.root,
                "only prebuilt memory providers are supported in v1",
            );
        }
        if manifest.executor.id.trim().is_empty() {
            return invalid(self.root, "executor id cannot be empty");
        }
        if manifest.memory.id.trim().is_empty() {
            return invalid(self.root, "memory provider id cannot be empty");
        }
        if agent.id.trim().is_empty() {
            return invalid(self.root, "agent id cannot be empty");
        }
        if agent.prompt.trim().is_empty() {
            return invalid(self.root, "agent prompt cannot be empty");
        }
        if agent.model.provider.trim().is_empty() || agent.model.name.trim().is_empty() {
            return invalid(self.root, "agent model provider and name are required");
        }
        ensure_relative_file(self.root, &manifest.readme, "readme path")?;
        for skill in &manifest.skills {
            ensure_relative_entry(self.root, &skill.path, "skill path")?;
        }
        for tool in &manifest.tools {
            if tool.source != "builtin" {
                return invalid(self.root, "only builtin tools are supported in v1");
            }
        }
        for path in &manifest.sandbox.permissions.filesystem.mounts.read {
            ensure_absolute_mount(self.root, path, "read mount")?;
        }
        for path in &manifest.sandbox.permissions.filesystem.mounts.write {
            ensure_absolute_mount(self.root, path, "write mount")?;
        }
        validate_sandbox_env(self.root, &manifest.sandbox.env)?;
        validate_network_permissions(self.root, &manifest.sandbox.permissions.network)?;
        validate_tool_permission_group(
            self.root,
            "sandbox.permissions.tools.allow",
            &manifest.sandbox.permissions.tools.allow,
        )?;
        validate_tool_permission_group(
            self.root,
            "sandbox.permissions.tools.ask",
            &manifest.sandbox.permissions.tools.ask,
        )?;
        validate_tool_permission_group(
            self.root,
            "sandbox.permissions.tools.deny",
            &manifest.sandbox.permissions.tools.deny,
        )?;
        Ok(())
    }
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

fn ensure_relative_file(root: &Path, value: &str, label: &str) -> Result<(), ManifestError> {
    ensure_relative_entry(root, value, label)?;
    let path = root.join(value);
    if !path.is_file() {
        return invalid(root, &format!("{label} must be a file"));
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

fn validate_network_permissions(root: &Path, values: &[String]) -> Result<(), ManifestError> {
    if values.is_empty() {
        return Ok(());
    }

    if values.len() == 1 && values[0] == "*" {
        return Ok(());
    }

    invalid(
        root,
        "sandbox.permissions.network only supports [] or [\"*\"] in v1",
    )
}

fn validate_sandbox_env(
    root: &Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<(), ManifestError> {
    for (target, source) in env {
        validate_env_name(root, target, "sandbox.env target")?;
        validate_env_name(root, source, "sandbox.env source")?;
    }
    Ok(())
}

fn validate_tool_permission_group(
    root: &Path,
    label: &str,
    values: &[String],
) -> Result<(), ManifestError> {
    for value in values {
        validate_tool_permission_value(root, label, value)?;
    }
    Ok(())
}

fn validate_tool_permission_value(
    root: &Path,
    label: &str,
    value: &str,
) -> Result<(), ManifestError> {
    if value.trim().is_empty() {
        return invalid(root, &format!("{label} entries cannot be empty"));
    }

    if let Some(open) = value.find('(') {
        if !value.ends_with(')') {
            return invalid(
                root,
                &format!("{label} entry `{value}` must end with `)` when using a granular matcher"),
            );
        }
        if value[..open].trim().is_empty() {
            return invalid(
                root,
                &format!("{label} entry `{value}` is missing a tool name"),
            );
        }
        let target = &value[open + 1..value.len() - 1];
        if target.trim().is_empty() {
            return invalid(
                root,
                &format!("{label} entry `{value}` is missing a matcher target"),
            );
        }
        if target.contains('(') || target.contains(')') {
            return invalid(
                root,
                &format!("{label} entry `{value}` cannot contain nested parentheses"),
            );
        }
        return Ok(());
    }

    if value.contains(')') {
        return invalid(
            root,
            &format!("{label} entry `{value}` has an unmatched closing parenthesis"),
        );
    }

    Ok(())
}

fn validate_env_name(root: &Path, value: &str, label: &str) -> Result<(), ManifestError> {
    if value.is_empty() {
        return invalid(root, &format!("{label} entries cannot be empty"));
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return invalid(root, &format!("{label} entries cannot be empty"));
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return invalid(
            root,
            &format!("{label} entry `{value}` must start with an ASCII letter or underscore"),
        );
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return invalid(
            root,
            &format!(
                "{label} entry `{value}` must contain only ASCII letters, digits, or underscores"
            ),
        );
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
    use super::BundleLoader;
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
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                skills: [],
                tools: [{ name: 'Read', source: 'builtin' }],
                sandbox: { permissions: { filesystem: { exec: [], mounts: { read: [], write: [] } }, network: [], tools: { allow: [], ask: [], deny: [] } }, system_tools: [], resources: {} }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "# demo\n").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n",
        )
        .expect("write agent");

        let bundle_loader = BundleLoader::new(temp.path());
        let (manifest, agent) = bundle_loader.load_project().expect("project");
        assert_eq!(manifest.executor.id, "react");
        assert_eq!(agent.id, "demo");
    }

    #[test]
    fn load_project_rejects_network_allowlists() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                sandbox: {
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: ['wttr.in'],
                        tools: { allow: [], ask: [], deny: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "hello").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\nmodel:\n  provider: openai\n  name: gpt-5\nprompt: hi\n",
        )
        .expect("write agent");

        let error = BundleLoader::new(temp.path())
            .load_project()
            .expect_err("network allowlist rejected");
        assert!(error.to_string().contains("only supports [] or [\"*\"]"));
    }

    #[test]
    fn load_project_rejects_relative_host_mounts() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                skills: [],
                tools: [{ name: 'Read', source: 'builtin' }],
                sandbox: {
                    permissions: {
                        filesystem: {
                            exec: [],
                            mounts: { read: ['tmp/project'], write: [] }
                        },
                        network: [],
                        tools: { allow: [], ask: [], deny: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "# demo\n").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n",
        )
        .expect("write agent");

        let bundle_loader = BundleLoader::new(temp.path());
        let error = bundle_loader
            .load_project()
            .expect_err("relative host mount should fail");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid manifest at {}: read mount must be an absolute host path",
                temp.path().display()
            )
        );
    }

    #[test]
    fn load_project_accepts_runtime_env_injection() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                sandbox: {
                    env: { OPENAI_API_KEY: 'OPENAI_API_KEY', APP_ENV: 'APP_ENV' },
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: [],
                        tools: { allow: [], ask: [], deny: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "hello").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\nmodel:\n  provider: openai\n  name: gpt-5\nprompt: hi\n",
        )
        .expect("write agent");

        let (manifest, _) = BundleLoader::new(temp.path())
            .load_project()
            .expect("runtime env config valid");
        assert_eq!(
            manifest.sandbox.env,
            std::collections::BTreeMap::from([
                ("APP_ENV".to_string(), "APP_ENV".to_string()),
                ("OPENAI_API_KEY".to_string(), "OPENAI_API_KEY".to_string()),
            ])
        );
    }

    #[test]
    fn load_project_rejects_invalid_env_names() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                sandbox: {
                    env: { OPENAI_API_KEY: 'NOT-VALID' },
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: [],
                        tools: { allow: [], ask: [], deny: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "hello").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\nmodel:\n  provider: openai\n  name: gpt-5\nprompt: hi\n",
        )
        .expect("write agent");

        let error = BundleLoader::new(temp.path())
            .load_project()
            .expect_err("invalid env name rejected");
        assert!(
            error
                .to_string()
                .contains("ASCII letters, digits, or underscores")
        );
    }

    #[test]
    fn load_project_rejects_invalid_granular_tool_permission() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                sandbox: {
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: [],
                        tools: { allow: ['Bash(find:*'], ask: [], deny: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "# demo\n").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n",
        )
        .expect("write agent");

        let bundle_loader = BundleLoader::new(temp.path());
        let error = bundle_loader
            .load_project()
            .expect_err("invalid granular tool permission rejected");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid manifest at {}: sandbox.permissions.tools.allow entry `Bash(find:*` must end with `)` when using a granular matcher",
                temp.path().display()
            )
        );
    }

    #[test]
    fn load_project_rejects_legacy_tool_rules() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("odyssey.bundle.json5"),
            r#"{
                id: 'demo',
                version: '0.1.0',
                manifest_version: 'odyssey.bundle/v1',
                readme: 'README.md',
                agent_spec: 'agent.yaml',
                executor: { type: 'prebuilt', id: 'react' },
                memory: { type: 'prebuilt', id: 'sliding_window' },
                sandbox: {
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: [],
                        tools: {
                            rules: [{ action: 'allow', tool: 'Read' }]
                        }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(temp.path().join("README.md"), "# demo\n").expect("write readme");
        fs::write(
            temp.path().join("agent.yaml"),
            "id: demo\ndescription: test\nprompt: hello\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: ['Read']\n",
        )
        .expect("write agent");

        let error = BundleLoader::new(temp.path())
            .load_project()
            .expect_err("legacy tool rules rejected");
        assert!(error.to_string().contains("unknown field `rules`"));
    }
}
