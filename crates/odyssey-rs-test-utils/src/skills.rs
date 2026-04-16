use async_trait::async_trait;
use odyssey_rs_protocol::{SkillProvider, SkillSummary, ToolError};

#[derive(Clone, Default)]
pub struct StubSkillProvider {
    summaries: Vec<SkillSummary>,
    content: String,
}

impl StubSkillProvider {
    pub fn new(summaries: Vec<SkillSummary>, content: impl Into<String>) -> Self {
        Self {
            summaries,
            content: content.into(),
        }
    }

    pub fn with_summaries(summaries: Vec<SkillSummary>) -> Self {
        Self {
            summaries,
            content: String::new(),
        }
    }
}

#[async_trait]
impl SkillProvider for StubSkillProvider {
    fn list(&self) -> Vec<SkillSummary> {
        self.summaries.clone()
    }

    async fn load(&self, _name: &str) -> Result<String, ToolError> {
        Ok(self.content.clone())
    }
}
