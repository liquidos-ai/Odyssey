use crate::RuntimeError;
use odyssey_rs_bundle::{BundleInstall, BundleStore};
use odyssey_rs_manifest::{AgentSpec, BundleManifest};

#[derive(Clone)]
pub struct LoadedBundle {
    pub install: BundleInstall,
    pub manifest: BundleManifest,
    pub agent: AgentSpec,
}

pub fn load_bundle(store: &BundleStore, reference: &str) -> Result<LoadedBundle, RuntimeError> {
    let install = store.resolve(reference)?;
    let manifest = install.metadata.bundle_manifest.clone();
    let agent = install.metadata.agent_spec.clone();
    Ok(LoadedBundle {
        install,
        manifest,
        agent,
    })
}

#[cfg(test)]
mod tests {
    use super::load_bundle;
    use odyssey_rs_bundle::BundleStore;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    fn write_bundle_project(root: &std::path::Path, id: &str, version: &str) {
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skills");
        fs::create_dir_all(root.join("resources").join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            format!(
                r#"{{
                    id: "{id}",
                    version: "{version}",
                    manifest_version: "odyssey.bundle/v1",
                    readme: "README.md",
                    agent_spec: "agent.yaml",
                    executor: {{ type: "prebuilt", id: "react" }},
                    memory: {{ type: "prebuilt", id: "sliding_window" }},
                    skills: [{{ name: "repo-hygiene", path: "skills/repo-hygiene" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: [], write: [] }} }},
                            network: [],
                            tools: {{ allow: [], ask: [], deny: [] }}
                        }},
                        system_tools: [],
                        resources: {{}}
                    }}
                }}"#
            ),
        )
        .expect("write manifest");
        fs::write(
            root.join("agent.yaml"),
            format!(
                r#"id: {id}
description: runtime loader test
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read"]
"#
            ),
        )
        .expect("write agent");
        fs::write(root.join("README.md"), format!("# {id}\n")).expect("write readme");
        fs::write(
            root.join("resources").join("data").join("notes.txt"),
            "hello world\n",
        )
        .expect("write resource");
    }

    #[test]
    fn load_bundle_returns_manifest_agent_and_install() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(&project_root, "demo", "0.1.0");
        let install = store
            .build_and_install(&project_root)
            .expect("build and install");

        let loaded = load_bundle(&store, "demo").expect("load bundle");

        assert_eq!(loaded.install.path, install.path);
        assert_eq!(loaded.manifest.id, "demo");
        assert_eq!(loaded.manifest.version, "0.1.0");
        assert_eq!(loaded.agent.id, "demo");
    }

    #[test]
    fn load_bundle_surfaces_store_resolution_errors() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        let error = match load_bundle(&store, "missing") {
            Ok(_) => panic!("missing bundle should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("bundle error"));
    }
}
