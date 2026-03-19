use crate::tool::ToolError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Summary of a skill available to the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Render a compact textual summary.
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

#[cfg(test)]
mod tests {
    use super::{SkillProvider, SkillSummary};
    use crate::ToolError;
    use async_trait::async_trait;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    struct DummySkillProvider {
        skills: Vec<SkillSummary>,
    }

    #[async_trait]
    impl SkillProvider for DummySkillProvider {
        fn list(&self) -> Vec<SkillSummary> {
            self.skills.clone()
        }

        async fn load(&self, name: &str) -> Result<String, ToolError> {
            Ok(format!("loaded {name}"))
        }
    }

    #[test]
    fn summaries_sort_skills_by_name() {
        let provider = DummySkillProvider {
            skills: vec![
                SkillSummary {
                    name: "zebra".to_string(),
                    description: "last".to_string(),
                    path: PathBuf::from("skills/zebra/SKILL.md"),
                },
                SkillSummary {
                    name: "alpha".to_string(),
                    description: "first".to_string(),
                    path: PathBuf::from("skills/alpha/SKILL.md"),
                },
            ],
        };

        assert_eq!(
            provider
                .summaries()
                .into_iter()
                .map(|skill| skill.name)
                .collect::<Vec<_>>(),
            vec!["alpha".to_string(), "zebra".to_string()]
        );
    }

    #[test]
    fn render_summary_handles_empty_and_trimmed_descriptions() {
        let provider = DummySkillProvider {
            skills: vec![
                SkillSummary {
                    name: "alpha".to_string(),
                    description: "  tidy repo history  ".to_string(),
                    path: PathBuf::from("skills/alpha/SKILL.md"),
                },
                SkillSummary {
                    name: "beta".to_string(),
                    description: "   ".to_string(),
                    path: PathBuf::from("skills/beta/SKILL.md"),
                },
            ],
        };

        assert_eq!(
            provider.render_summary(),
            "- alpha: tidy repo history\n- beta"
        );
        assert_eq!(
            DummySkillProvider { skills: Vec::new() }.render_summary(),
            String::default()
        );
    }
}
