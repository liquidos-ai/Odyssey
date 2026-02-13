//! Skill store tests for discovery and filtering.

use odyssey_rs_config::{SettingSource, SkillsConfig};
use odyssey_rs_core::skills::SkillStore;
use odyssey_rs_protocol::SkillProvider;
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::tempdir;

/// Load skills from a project-scoped skills directory.
#[test]
fn loads_skills_from_project_directory() {
    let temp = tempdir().expect("tempdir");
    let skills_root = temp.path().join(".odyssey").join("skills").join("demo");
    fs::create_dir_all(&skills_root).expect("create skill dir");
    let skill_path = skills_root.join("SKILL.md");
    fs::write(
        &skill_path,
        r#"---
name: DemoSkill
description: Demo skill description
---

# DemoSkill

Use this skill for demos.
"#,
    )
    .expect("write skill");

    let config = SkillsConfig {
        setting_sources: vec![SettingSource::Project],
        paths: Vec::new(),
        allow: vec!["*".to_string()],
        deny: Vec::new(),
    };
    let store = SkillStore::load(&config, temp.path()).expect("load store");
    let summaries = store.summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "DemoSkill");
    assert_eq!(summaries[0].description, "Demo skill description");
}

/// Allow/deny lists should filter skills appropriately.
#[test]
fn allowlist_and_denylist_filter_skills() {
    let temp = tempdir().expect("tempdir");
    let base = temp.path().join(".odyssey").join("skills");
    fs::create_dir_all(base.join("allowed")).expect("allowed dir");
    fs::create_dir_all(base.join("blocked")).expect("blocked dir");
    fs::write(base.join("allowed").join("SKILL.md"), "# Allowed").expect("write allowed");
    fs::write(base.join("blocked").join("SKILL.md"), "# Blocked").expect("write blocked");

    let config = SkillsConfig {
        setting_sources: vec![SettingSource::Project],
        paths: Vec::new(),
        allow: vec!["Allowed".to_string()],
        deny: vec!["Blocked".to_string()],
    };
    let store = SkillStore::load(&config, temp.path()).expect("load store");
    let summaries = store.summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "Allowed");
}
