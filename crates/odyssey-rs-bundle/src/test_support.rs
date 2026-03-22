use std::fs;
use std::path::Path;

pub fn write_bundle_project(
    root: &Path,
    id: &str,
    version: &str,
    resource_relative_path: &str,
    resource_contents: &str,
) {
    fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
    let resource_path = root.join("resources").join(resource_relative_path);
    if let Some(parent) = resource_path.parent() {
        fs::create_dir_all(parent).expect("create resource dir");
    }
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
                        tools: {{ mode: "default", rules: [] }}
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
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
"#
        ),
    )
    .expect("write agent");
    fs::write(root.join("README.md"), format!("# {id}\n")).expect("write readme");
    fs::write(
        root.join("skills").join("repo-hygiene").join("SKILL.md"),
        "# Repo Hygiene\n",
    )
    .expect("write skill");
    fs::write(resource_path, resource_contents).expect("write resource");
}
