//! System prompt assembly for orchestrator and subagent turns.

use super::memory::{format_memory_records, recall_options_from_config};
use crate::error::OdysseyCoreError;
use crate::instructions::resolve_instruction_roots;
use odyssey_rs_config::MemoryConfig;
use odyssey_rs_memory::MemoryProvider;
use odyssey_rs_protocol::SkillProvider;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Prompt profile controls small formatting differences between agent types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptProfile {
    /// Full system prompt for the orchestrator.
    OrchestratorDefault,
    /// Subagent prompt with reduced orchestration context.
    SubagentFocused,
}

/// Builds system prompts from base prompt, instructions, memory recall, and skills.
#[derive(Clone)]
pub struct PromptBuilder {
    /// Memory provider for recall.
    memory_provider: Arc<dyn MemoryProvider>,
    /// Optional skill store for skill summaries.
    skill_store: Option<Arc<dyn SkillProvider>>,
}

impl PromptBuilder {
    /// Create a new prompt builder with memory and skill dependencies.
    pub fn new(
        memory_provider: Arc<dyn MemoryProvider>,
        skill_store: Option<Arc<dyn SkillProvider>>,
    ) -> Self {
        Self {
            memory_provider,
            skill_store,
        }
    }

    /// Build the system prompt for a single turn.
    pub async fn build_system_prompt(
        &self,
        additional_instructions: &str,
        memory_config: &MemoryConfig,
        profile: PromptProfile,
    ) -> Result<String, OdysseyCoreError> {
        let cwd = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
        let instruction_roots = resolve_instruction_roots(&memory_config.instruction_roots, &cwd);
        let bootstrap_sections = load_bootstrap_sections(&instruction_roots)?;
        let recall_options = recall_options_from_config(&memory_config.recall);
        let recall_records = self
            .memory_provider
            .recall_initial(None, memory_config.recall_k, recall_options)
            .await
            .map_err(|err| OdysseyCoreError::Memory(err.to_string()))?;

        let mut sections = Vec::new();
        let trimmed_additional_instructions = additional_instructions.trim();
        let header = build_header_section(trimmed_additional_instructions, &cwd);
        if !header.trim().is_empty() {
            sections.push(header);
        }
        if profile == PromptProfile::OrchestratorDefault {
            sections.extend(bootstrap_sections);
        }
        let recall_content = if let Some(records) = recall_records {
            format_memory_records(&records)
        } else {
            String::new()
        };
        if recall_content.trim().is_empty() {
            sections.push("## Memory\n\n".to_string());
        } else {
            sections.push(format!("## Memory\n\n{recall_content}"));
        }

        sections.push("## Active Skills\n\nNo always-loaded skills.".to_string());
        sections.push(render_skill_section(self.skill_store.as_ref()));
        sections.push(build_footer_section());

        if sections.is_empty() {
            Ok(String::new())
        } else {
            Ok(sections.join("\n\n---\n\n"))
        }
    }
}

const BOOTSTRAP_FILES: [&str; 5] = ["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md", "IDENTITY.md"];

fn build_header_section(additional_instructions: &str, cwd: &std::path::Path) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M (%A)");
    let runtime = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let workspace = cwd.display();
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let memory_path = cwd.join("memory").join("MEMORY.md");
    let daily_notes = cwd.join("memory").join(format!("{date}.md"));
    let skills_path = cwd.join("skills");
    let memory_path_display = memory_path.display();
    let daily_notes_display = daily_notes.display();
    let skills_path_display = skills_path.display();

    let mut header = format!(
        "# Odyssey 🛠️ (built by liquidOS)\n\n\
You are Odyssey, an assistant built by liquidOS. You have access to tools that let you:\n\
- Read, write, and edit files in the workspace\n\
- Execute shell commands\n\
- Use web search and fetch web pages\n\
- Send messages to specific chat channels\n\
- Spawn subagents for background tasks\n\n\
## Current Time\n\
{now}\n\n\
## Runtime\n\
{runtime}\n\n\
## Workspace\n\
Your workspace: {workspace}\n\
- Memory files: {memory_path_display}\n\
- Daily notes: {daily_notes_display}\n\
- Custom skills: {skills_path_display}/{{skill-name}}/SKILL.md\n\n\
IMPORTANT BEHAVIOR RULES:\n\
- For direct user conversation replies: respond with normal text only (do **not** call the message tool).\n\
- Use the `message` tool **only** to send messages to external chat channels (WhatsApp, Telegram, Feishu) when explicitly required.\n\
- When invoking tools, always include a brief explanation in the assistant response about:\n\
  1. What tool you will call,\n\
  2. Why you call it,\n\
  3. How you will use the result.\n\
- When you store something to memory, append or write to {workspace}/memory/MEMORY.md and explain what you stored."
    );

    if !additional_instructions.is_empty() {
        header.push_str("\n\n## Additional Instructions\n");
        header.push_str(additional_instructions);
    }

    header
}

fn build_footer_section() -> String {
    "(If session metadata is provided it is appended here:)\n\n\
## Current Session\n\
Channel: {CHANNEL}\n\
Chat ID: {CHAT_ID}\n\n\
Additional implementation notes (practical)\n\n\
Separator: use \\n\\n---\\n\\n between parts when programmatically joining sections (matching your ContextBuilder).\n\n\
Bootstrap files: include each file's contents under ## <FILENAME> to preserve context and make it searchable by the model.\n\n\
Memory: prefer summarized memory that fits into the system prompt rather than dumping long raw logs; keep larger memory on disk and have the agent read it as needed.\n\n\
Skills:\n\n\
Put full content for always-loaded (critical) skills.\n\n\
For others, include only a short summary and an instruction: \"To use, call read_file on skills/<skill>/SKILL.md\".\n\n\
Size control: if the workspace contains many/large bootstrap files or long memory, summarize or truncate older content to avoid token bloat.\n\n\
Session info: append session (channel / chat id) as the last block of the system prompt so the model knows the active context.\n\n\
Media: do not embed large binary data in the system prompt. Images and other media belong in the user message; encode them as data-URI there if needed.\n\n\
Tool metadata: when adding tool outputs into the conversation, append them as separate tool messages (role \"tool\" with name and content) rather than embedding them inside the system prompt."
        .to_string()
}

fn load_bootstrap_sections(roots: &[PathBuf]) -> Result<Vec<String>, OdysseyCoreError> {
    let mut sections = Vec::new();
    let mut seen = HashSet::new();

    if !roots.is_empty() {
        sections.push(
            "## BOOTSTRAP FILES\n\n(Include any of these files if present: AGENTS.md, SOUL.md, USER.md, TOOLS.md, IDENTITY.md)"
                .to_string(),
        );
    }

    for root in roots {
        for filename in BOOTSTRAP_FILES {
            let path = root.join(filename);
            if !path.is_file() || !seen.insert(path.clone()) {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            if content.trim().is_empty() {
                continue;
            }
            sections.push(format!("## {filename}\n\n{content}"));
        }
    }
    Ok(sections)
}

fn render_skill_section(store: Option<&Arc<dyn SkillProvider>>) -> String {
    let Some(store) = store else {
        return "## Skills\n\nNo skills available.".to_string();
    };
    let summary = store.render_summary();
    if summary.trim().is_empty() {
        return "## Skills\n\nNo skills available.".to_string();
    }
    format!(
        "## Skills\n\nThe following skills extend your capabilities. To use any skill, read its `SKILL.md` using the `read_file` tool.\n\n{summary}"
    )
}

#[cfg(test)]
mod tests {
    use super::{PromptBuilder, PromptProfile};
    use odyssey_rs_config::MemoryConfig;
    use odyssey_rs_memory::MemoryRecord;
    use odyssey_rs_protocol::SkillSummary;
    use odyssey_rs_test_utils::{StubMemory, StubSkillProvider};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn build_system_prompt_includes_memory_and_skills() {
        let record = MemoryRecord {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: "user".to_string(),
            content: "remember this".to_string(),
            metadata: json!({}),
            created_at: chrono::Utc::now(),
        };
        let memory = Arc::new(StubMemory::with_initial(vec![record]));
        let skills = Arc::new(StubSkillProvider::new(
            vec![SkillSummary {
                name: "Checklist".to_string(),
                description: "Keeps steps clear.".to_string(),
                path: "skills/checklist/SKILL.md".into(),
            }],
            "content",
        ));

        let builder = PromptBuilder::new(memory, Some(skills));
        let prompt = builder
            .build_system_prompt(
                "Extra instructions.",
                &MemoryConfig::default(),
                PromptProfile::OrchestratorDefault,
            )
            .await
            .expect("prompt");

        assert!(prompt.contains("## Memory"));
        assert!(prompt.contains("## Skills"));
        assert!(prompt.contains("Extra instructions."));
        assert!(prompt.contains("Checklist: Keeps steps clear."));
    }

    #[tokio::test]
    async fn build_system_prompt_handles_empty_skills() {
        let memory = Arc::new(StubMemory::with_initial(Vec::new()));
        let builder = PromptBuilder::new(memory, None);
        let prompt = builder
            .build_system_prompt(
                "",
                &MemoryConfig::default(),
                PromptProfile::OrchestratorDefault,
            )
            .await
            .expect("prompt");
        assert!(prompt.contains("No skills available."));
        assert_eq!(prompt.contains("Additional Instructions"), false);
    }
}
