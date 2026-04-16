//! Tests for layered configuration loading.

use super::*;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Write JSON5 contents to a path, creating parent directories if needed.
fn write_json5(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("dir");
    }
    fs::write(path, contents).expect("write");
}

/// Verify that a minimal config parses with defaults.
#[test]
fn parse_minimal_config() {
    let json5 = "{}";
    let config = OdysseyConfig::load_from_str(json5).expect("config");
    assert_eq!(config.tools.output_policy.replacement, "[REDACTED]");
}

/// Reject unexpected top-level config keys.
#[test]
fn rejects_unknown_top_level_key() {
    let json5 = r#"{ unexpected: true }"#;
    let err = OdysseyConfig::load_from_str(json5).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("unknown key"));
}

/// Reject invalid permission mode values.
#[test]
fn rejects_invalid_permission_mode() {
    let json5 = r#"{ permissions: { mode: "unsafe" } }"#;
    let err = OdysseyConfig::load_from_str(json5).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("permissions.mode"));
}

/// Ensure repo config takes precedence over cwd config.
#[test]
fn layered_config_prefers_repo_over_cwd() {
    let temp = TempDir::new().expect("tmp");
    let root = temp.path();
    let project_root = root.join("project");
    fs::create_dir_all(project_root.join(".git")).expect("git");
    let cwd = project_root.join("subdir");
    fs::create_dir_all(&cwd).expect("cwd");

    let system_config = root.join("system.json5");
    write_json5(
        &system_config,
        "{ tools: { output_policy: { replacement: \"system\" } } }",
    );

    let user_config = root.join("user.json5");
    write_json5(
        &user_config,
        "{ tools: { output_policy: { replacement: \"user\" } } }",
    );

    let project_config = project_root.join(DEFAULT_CONFIG_FILE);
    write_json5(
        &project_config,
        "{ tools: { output_policy: { replacement: \"project\" } } }",
    );

    let cwd_config = cwd.join(DEFAULT_CONFIG_FILE);
    write_json5(
        &cwd_config,
        "{ tools: { output_policy: { replacement: \"cwd\" } } }",
    );

    let repo_config = project_root
        .join(DEFAULT_CONFIG_DIR)
        .join(DEFAULT_CONFIG_FILE);
    write_json5(
        &repo_config,
        "{ tools: { output_policy: { replacement: \"repo\" } } }",
    );

    let mut options = LayeredConfigOptions::new(&cwd);
    options.system_config_path = Some(system_config);
    options.user_config_path = Some(user_config);
    options.requirements_path = None;

    let layered = OdysseyConfig::load_layered_with_options(options).expect("layered");
    assert_eq!(
        layered.config.tools.output_policy.replacement,
        "repo".to_string()
    );
}

#[test]
fn requirements_lock_overrides() {
    let temp = TempDir::new().expect("tmp");
    let root = temp.path();
    let project_root = root.join("project");
    fs::create_dir_all(project_root.join(".git")).expect("git");
    let cwd = project_root.join("subdir");
    fs::create_dir_all(&cwd).expect("cwd");

    let system_config = root.join("system.json5");
    write_json5(
        &system_config,
        "{ tools: { output_policy: { replacement: \"system\" } } }",
    );

    let requirements = root.join("requirements.json5");
    write_json5(
        &requirements,
        "{ tools: { output_policy: { replacement: \"locked\" } } }",
    );

    let runtime_config = root.join("runtime.json5");
    write_json5(
        &runtime_config,
        "{ tools: { output_policy: { replacement: \"runtime\" } } }",
    );

    let mut options = LayeredConfigOptions::new(&cwd);
    options.system_config_path = Some(system_config);
    options.user_config_path = None;
    options.requirements_path = Some(requirements);
    options.runtime_paths = vec![runtime_config];

    let layered = OdysseyConfig::load_layered_with_options(options).expect("layered");
    assert_eq!(
        layered.config.tools.output_policy.replacement,
        "locked".to_string()
    );
}

#[test]
fn runtime_override_wins_without_constraints() {
    let temp = TempDir::new().expect("tmp");
    let root = temp.path();
    let project_root = root.join("project");
    fs::create_dir_all(project_root.join(".git")).expect("git");
    let cwd = project_root.join("subdir");
    fs::create_dir_all(&cwd).expect("cwd");

    let system_config = root.join("system.json5");
    write_json5(
        &system_config,
        "{ tools: { output_policy: { replacement: \"system\" } } }",
    );

    let runtime_config = root.join("runtime.json5");
    write_json5(
        &runtime_config,
        "{ tools: { output_policy: { replacement: \"runtime\" } } }",
    );

    let mut options = LayeredConfigOptions::new(&cwd);
    options.system_config_path = Some(system_config);
    options.user_config_path = None;
    options.requirements_path = None;
    options.runtime_paths = vec![runtime_config];

    let layered = OdysseyConfig::load_layered_with_options(options).expect("layered");
    assert_eq!(
        layered.config.tools.output_policy.replacement,
        "runtime".to_string()
    );
}

#[test]
fn constraints_prevent_list_override() {
    let temp = TempDir::new().expect("tmp");
    let root = temp.path();
    let project_root = root.join("project");
    fs::create_dir_all(project_root.join(".git")).expect("git");

    let system_config = root.join("system.json5");
    write_json5(&system_config, "{ skills: { paths: [\"*\"] } }");

    let requirements = root.join("requirements.json5");
    write_json5(&requirements, "{ skills: { paths: [\"core\"] } }");

    let runtime_config = root.join("runtime.json5");
    write_json5(&runtime_config, "{ skills: { paths: [\"runtime\"] } }");

    let mut options = LayeredConfigOptions::new(&project_root);
    options.system_config_path = Some(system_config);
    options.user_config_path = None;
    options.requirements_path = Some(requirements);
    options.runtime_paths = vec![runtime_config];

    let layered = OdysseyConfig::load_layered_with_options(options).expect("layered");
    assert_eq!(layered.config.skills.paths, vec!["core".to_string()]);
}
