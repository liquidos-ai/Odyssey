//! Skill discovery and loading for Odyssey.

use async_trait::async_trait;
use log::{debug, info};
use odyssey_rs_config::{SettingSource, SkillsConfig};
use odyssey_rs_protocol::{SkillProvider, SkillSummary, ToolError};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Errors returned when discovering or loading skills.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid skill frontmatter in {path}")]
    InvalidFrontmatter { path: String },
    #[error("skill is missing a name: {path}")]
    MissingName { path: String },
    #[error("skill not found: {name}")]
    NotFound { name: String },
    #[error("duplicate skill name: {name}")]
    DuplicateName { name: String },
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// In-memory skill store keyed by lowercase name.
#[derive(Debug, Clone, Default)]
pub struct SkillStore {
    skills: HashMap<String, SkillSummary>,
}

/// Helper to resolve skill roots from config.
#[derive(Debug, Clone)]
struct SkillLocator {
    config: SkillsConfig,
}

/// Parsed frontmatter for a skill file.
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

impl SkillStore {
    /// Load skills from configured locations.
    pub fn load(config: &SkillsConfig, cwd: &Path) -> Result<Self, SkillError> {
        let mut roots = SkillLocator::new(config).roots(cwd);
        roots.retain(|root| root.exists());
        roots.sort();
        roots.dedup();
        info!(
            "loading skills (roots={}, cwd={})",
            roots.len(),
            cwd.to_string_lossy()
        );

        let allow_all = config.allow.is_empty() || config.allow.iter().any(|entry| entry == "*");
        let allow_set = config
            .allow
            .iter()
            .map(|entry| entry.to_lowercase())
            .collect::<HashSet<_>>();
        let deny_set = config
            .deny
            .iter()
            .map(|entry| entry.to_lowercase())
            .collect::<HashSet<_>>();

        let mut skills = HashMap::new();
        for root in roots {
            debug!("scanning skills root: {}", root.display());
            for path in discover_skill_files(&root) {
                let summary = parse_skill_summary(&path)?;
                let key = summary.name.to_lowercase();
                if deny_set.contains(&key) {
                    continue;
                }
                if !allow_all && !allow_set.contains(&key) {
                    continue;
                }
                if skills.contains_key(&key) {
                    return Err(SkillError::DuplicateName { name: summary.name });
                }
                skills.insert(key, summary);
            }
        }
        info!("skills loaded (count={})", skills.len());

        Ok(Self { skills })
    }

    /// Return sorted skill summaries.
    fn summaries(&self) -> Vec<SkillSummary> {
        let mut list = self.skills.values().cloned().collect::<Vec<_>>();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Fetch a skill summary by name (case-insensitive).
    fn get(&self, name: &str) -> Option<&SkillSummary> {
        let key = name.to_lowercase();
        self.skills.get(&key)
    }

    /// Load the full contents for a skill by name.
    fn load_content(&self, name: &str) -> Result<String, SkillError> {
        let skill = self.get(name).ok_or_else(|| SkillError::NotFound {
            name: name.to_string(),
        })?;
        Ok(std::fs::read_to_string(&skill.path)?)
    }
}

impl SkillLocator {
    /// Create a new skill locator from config.
    fn new(config: &SkillsConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Resolve all configured skill roots.
    fn roots(&self, cwd: &Path) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for source in &self.config.setting_sources {
            match source {
                SettingSource::Project => roots.push(cwd.join(".odyssey").join("skills")),
                SettingSource::User => {
                    if let Some(home) =
                        directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
                    {
                        roots.push(home.join(".odyssey").join("skills"));
                    }
                }
                SettingSource::System => {
                    #[cfg(unix)]
                    {
                        roots.push(PathBuf::from("/etc/odyssey/skills"));
                    }
                }
            }
        }

        for path in &self.config.paths {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                roots.push(path);
            } else {
                roots.push(cwd.join(path));
            }
        }
        roots
    }
}

#[async_trait]
impl SkillProvider for SkillStore {
    /// List available skills for tool consumption.
    fn list(&self) -> Vec<SkillSummary> {
        self.summaries()
    }

    /// Load skill content for the tool layer.
    async fn load(&self, name: &str) -> Result<String, ToolError> {
        self.load_content(name)
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))
    }
}

/// Discover SKILL.md files under a root directory.
fn discover_skill_files(root: &Path) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }
    WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() == "SKILL.md")
        .map(|entry| entry.into_path())
        .collect()
}

/// Parse skill frontmatter and extract summary metadata.
fn parse_skill_summary(path: &Path) -> Result<SkillSummary, SkillError> {
    let contents = std::fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&contents, path)?;

    let mut name = frontmatter
        .as_ref()
        .and_then(|meta| meta.name.clone())
        .filter(|value| !value.trim().is_empty());
    let mut description = frontmatter
        .as_ref()
        .and_then(|meta| meta.description.clone())
        .unwrap_or_default();

    if name.is_none() {
        name = extract_heading(&body);
    }
    if name.is_none() {
        name = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map(|name| name.to_string());
    }

    if description.trim().is_empty() {
        description = extract_description(&body).unwrap_or_default();
    }

    let Some(name) = name else {
        return Err(SkillError::MissingName {
            path: path.display().to_string(),
        });
    };

    Ok(SkillSummary {
        name,
        description,
        path: path.to_path_buf(),
    })
}

/// Split YAML frontmatter from Markdown body.
fn split_frontmatter(
    contents: &str,
    path: &Path,
) -> Result<(Option<SkillFrontmatter>, String), SkillError> {
    let mut lines = contents.lines();
    let Some(first) = lines.next() else {
        return Ok((None, String::new()));
    };

    if first.trim() != "---" {
        return Ok((None, contents.to_string()));
    }

    let mut yaml_lines = Vec::new();
    let mut found_delimiter = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            found_delimiter = true;
            break;
        }
        yaml_lines.push(line);
    }

    if !found_delimiter {
        return Err(SkillError::InvalidFrontmatter {
            path: path.display().to_string(),
        });
    }

    let yaml = yaml_lines.join("\n");
    let metadata: SkillFrontmatter = serde_yaml::from_str(&yaml)?;
    let body = lines.collect::<Vec<_>>().join("\n");
    Ok((Some(metadata), body))
}

/// Extract a heading to use as skill name.
fn extract_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.strip_prefix("# ").map(|name| name.trim().to_string()))
}

/// Extract the first non-heading line as a description.
fn extract_description(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        return Some(line.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{SkillError, SkillStore};
    use odyssey_rs_config::{SettingSource, SkillsConfig};
    use odyssey_rs_protocol::SkillProvider;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_skill(path: &Path, contents: &str) {
        std::fs::create_dir_all(path).expect("create skill dir");
        std::fs::write(path.join("SKILL.md"), contents).expect("write skill");
    }

    fn config_for_root(root: &Path) -> SkillsConfig {
        SkillsConfig {
            setting_sources: Vec::new(),
            paths: vec![root.to_string_lossy().to_string()],
            allow: vec!["*".to_string()],
            deny: Vec::new(),
        }
    }

    #[test]
    fn skill_frontmatter_overrides_heading() {
        let temp = tempdir().expect("tempdir");
        let skill_dir = temp.path().join("focus");
        write_skill(
            &skill_dir,
            r#"---
name: Focus Mode
description: Keep context tight.
---

# Heading

Body
"#,
        );

        let config = config_for_root(temp.path());
        let store = SkillStore::load(&config, temp.path()).expect("store");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Focus Mode");
        assert_eq!(list[0].description, "Keep context tight.");
    }

    #[test]
    fn skill_falls_back_to_heading_and_description() {
        let temp = tempdir().expect("tempdir");
        let skill_dir = temp.path().join("fallback");
        write_skill(
            &skill_dir,
            r#"# Backup Skill

Use this when recovery is required.
More details later.
"#,
        );

        let config = config_for_root(temp.path());
        let store = SkillStore::load(&config, temp.path()).expect("store");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Backup Skill");
        assert_eq!(list[0].description, "Use this when recovery is required.");
    }

    #[test]
    fn skill_falls_back_to_directory_name() {
        let temp = tempdir().expect("tempdir");
        let skill_dir = temp.path().join("restore");
        write_skill(&skill_dir, "Content only.");

        let config = config_for_root(temp.path());
        let store = SkillStore::load(&config, temp.path()).expect("store");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "restore");
        assert_eq!(list[0].description, "Content only.");
    }

    #[test]
    fn skill_allow_and_deny_filters() {
        let temp = tempdir().expect("tempdir");
        write_skill(
            &temp.path().join("alpha"),
            r#"# Alpha

Alpha description.
"#,
        );
        write_skill(
            &temp.path().join("beta"),
            r#"# Beta

Beta description.
"#,
        );

        let config = SkillsConfig {
            setting_sources: vec![SettingSource::Project],
            paths: vec![temp.path().to_string_lossy().to_string()],
            allow: vec!["Alpha".to_string()],
            deny: vec!["beta".to_string()],
        };
        let store = SkillStore::load(&config, temp.path()).expect("store");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Alpha");
    }

    #[test]
    fn duplicate_skill_names_error() {
        let temp = tempdir().expect("tempdir");
        write_skill(
            &temp.path().join("one"),
            r#"---
name: Duplicate
---

First
"#,
        );
        write_skill(
            &temp.path().join("two"),
            r#"---
name: Duplicate
---

Second
"#,
        );

        let config = config_for_root(temp.path());
        let err = SkillStore::load(&config, temp.path()).expect_err("duplicate");
        match err {
            SkillError::DuplicateName { name } => assert_eq!(name, "Duplicate"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
