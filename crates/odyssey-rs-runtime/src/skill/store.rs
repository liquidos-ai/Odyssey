use crate::RuntimeError;
use odyssey_rs_tools::{SkillEntry, SkillProvider, ToolError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

#[derive(Clone, Default)]
pub struct BundleSkillStore {
    skills: Arc<HashMap<String, SkillEntry>>,
}

impl BundleSkillStore {
    pub fn load(root: &Path) -> Result<Self, RuntimeError> {
        let skills_root = root.join("skills");
        let mut skills = HashMap::new();
        if !skills_root.exists() {
            return Ok(Self::default());
        }
        for entry in WalkDir::new(&skills_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() && entry.file_name() == "SKILL.md" {
                let path = entry.path().to_path_buf();
                let name = path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| RuntimeError::Executor("invalid skill path".to_string()))?
                    .to_string();
                let description = fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| {
                        content
                            .lines()
                            .map(str::trim)
                            .find(|line| !line.is_empty() && !line.starts_with('#'))
                            .map(ToString::to_string)
                    })
                    .unwrap_or_default();
                skills.insert(
                    name.clone(),
                    SkillEntry {
                        name,
                        description,
                        path,
                    },
                );
            }
        }
        Ok(Self {
            skills: Arc::new(skills),
        })
    }

    pub fn render_prompt_section(&self) -> String {
        let mut skills = self.skills.values().cloned().collect::<Vec<_>>();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        if skills.is_empty() {
            return "## Skills\n\nNo skills available.".to_string();
        }
        let summary = skills
            .into_iter()
            .map(|skill| {
                if skill.description.trim().is_empty() {
                    format!("- {}", skill.name)
                } else {
                    format!("- {}: {}", skill.name, skill.description.trim())
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "## Skills\n\nThe following skills extend your capabilities. To use any skill, read its `SKILL.md` using the `Skill` tool.\n\n{summary}"
        )
    }
}

impl SkillProvider for BundleSkillStore {
    fn list(&self) -> Vec<SkillEntry> {
        self.skills.values().cloned().collect()
    }

    fn load(&self, name: &str) -> Result<String, ToolError> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| ToolError::ExecutionFailed(format!("skill {name} not found")))?;
        fs::read_to_string(&skill.path).map_err(|err| ToolError::ExecutionFailed(err.to_string()))
    }
}

#[allow(dead_code)]
fn _to_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
