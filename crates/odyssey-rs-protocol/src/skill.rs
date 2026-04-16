use crate::tool::ToolError;
use async_trait::async_trait;
use std::path::PathBuf;

/// Summary of a skill available to the orchestrator.
#[derive(Debug, Clone)]
pub struct SkillSummary {
    /// Skill name.
    pub name: String,
    /// Short description of the skill.
    pub description: String,
    /// Path to the skill file.
    pub path: PathBuf,
}

/// Skill provider interface used by tools.
#[async_trait]
pub trait SkillProvider: Send + Sync {
    /// List available skill summaries.
    fn list(&self) -> Vec<SkillSummary>;

    /// Load a skill by name.
    async fn load(&self, name: &str) -> Result<String, ToolError>;

    /// Return sorted skill summaries.
    fn summaries(&self) -> Vec<SkillSummary> {
        let mut list = self.list();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn render_summary(&self) -> String {
        let summaries = self.list();
        if summaries.is_empty() {
            return String::default();
        }
        summaries
            .into_iter()
            .map(|skill| {
                if skill.description.trim().is_empty() {
                    format!("- {}", skill.name)
                } else {
                    format!("- {}: {}", skill.name, skill.description.trim())
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
