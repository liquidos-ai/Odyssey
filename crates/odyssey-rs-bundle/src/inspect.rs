use crate::reference::BundleStore;
use crate::{BundleError, BundleMetadata};

pub fn inspect_bundle(store: &BundleStore, reference: &str) -> Result<BundleMetadata, BundleError> {
    Ok(store.resolve(reference)?.metadata)
}

#[cfg(test)]
mod tests {
    use super::inspect_bundle;
    use crate::BundleStore;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_bundle_project(root: &Path) {
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
        fs::create_dir_all(root.join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            r#"{
                id: "demo",
                version: "0.1.0",
                agent_spec: "agent.yaml",
                executor: { type: "prebuilt", id: "react" },
                memory: { provider: { type: "prebuilt", id: "sliding_window" } },
                resources: ["data"],
                skills: [{ name: "repo-hygiene", path: "skills/repo-hygiene" }],
                tools: [{ name: "Read", source: "builtin" }],
                server: { enable_http: true },
                sandbox: {
                    permissions: {
                        filesystem: { exec: [], mounts: { read: [], write: [] } },
                        network: [],
                        tools: { mode: "default", rules: [] }
                    },
                    system_tools: [],
                    resources: {}
                }
            }"#,
        )
        .expect("write manifest");
        fs::write(
            root.join("agent.yaml"),
            r#"id: demo
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
  deny: []
"#,
        )
        .expect("write agent");
        fs::write(
            root.join("skills").join("repo-hygiene").join("SKILL.md"),
            "# Repo Hygiene\n\nKeep commits focused.\n",
        )
        .expect("write skill");
        fs::write(root.join("data").join("notes.txt"), "hello world\n").expect("write resource");
    }

    #[test]
    fn inspect_bundle_returns_resolved_metadata() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(&project_root);

        let store = BundleStore::new(temp.path().join("store"));
        let install = store
            .build_and_install(&project_root)
            .expect("build install");

        let metadata = inspect_bundle(&store, "local/demo@0.1.0").expect("inspect");

        assert_eq!(metadata.id, install.metadata.id);
        assert_eq!(metadata.version, install.metadata.version);
        assert_eq!(metadata.digest, install.metadata.digest);
    }
}
