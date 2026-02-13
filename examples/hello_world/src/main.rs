use anyhow::{Context, Result};
use autoagents::llm::LLMProvider;
use autoagents::llm::backends::openai::OpenAI;
use autoagents::llm::builder::LLMBuilder;
use autoagents::prelude::ReActAgent;
use odyssey_rs::config::{
    OdysseyConfig, PermissionAction, PermissionRule, PermissionsConfig, SandboxConfig,
    SettingSource, SkillsConfig,
};
use odyssey_rs::core::orchestrator::DEFAULT_LLM_ID;
use odyssey_rs::core::orchestrator::prompt::PromptProfile;
use odyssey_rs::core::skills::SkillStore;
use odyssey_rs::core::{
    AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator, PromptBuilder,
};
use odyssey_rs::init_logging;
use odyssey_rs::memory::FileMemoryProvider;
#[cfg(target_os = "linux")]
use odyssey_rs_sandbox::BubblewrapProvider;
#[cfg(not(target_os = "linux"))]
use odyssey_rs_sandbox::LocalSandboxProvider;
use odyssey_rs_sandbox::SandboxProvider;
use odyssey_rs_tools::builtin_tool_registry;
use std::{path::Path, sync::Arc};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Hello!".to_string());

    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY is required to run the hello_world example")?;
    let llm: Arc<dyn LLMProvider> = LLMBuilder::<OpenAI>::new()
        .api_key(api_key)
        .model("gpt-4.1")
        .build()
        .context("failed to build OpenAI LLM provider")?;

    let sandbox_provider = {
        #[cfg(target_os = "linux")]
        {
            "bubblewrap"
        }
        #[cfg(not(target_os = "linux"))]
        {
            "local"
        }
    };

    let config = OdysseyConfig::builder()
        .permissions(PermissionsConfig {
            mode: odyssey_rs::config::PermissionMode::Default,
            rules: vec![PermissionRule {
                action: PermissionAction::Allow,
                tool: Some("Write".to_string()),
                path: None,
                command: None,
                access: None,
            }],
        })
        .sandbox(SandboxConfig {
            enabled: true,
            provider: Some(sandbox_provider.to_string()),
            ..SandboxConfig::default()
        })
        .skills(SkillsConfig {
            setting_sources: vec![SettingSource::User],
            allow: vec!["*".into()],
            deny: vec![],
            paths: vec!["./configs/skills".into()],
        })
        .build();

    let tools = builtin_tool_registry();
    let memory = Arc::new(FileMemoryProvider::new(Path::new(".odyssey/memory")).unwrap());
    let memory_clone = memory.clone();
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let skill_store =
        Arc::new(SkillStore::load(&config.skills, &cwd).context("failed to load skills")?); //TODO: WHy do we need cwd here?
    let agent_description = "Odyssey hello world agent".to_string();
    let system_prompt = PromptBuilder::new(memory_clone.clone(), Some(skill_store.clone()))
        .build_system_prompt(
            &agent_description,
            &config.memory,
            PromptProfile::OrchestratorDefault,
        )
        .await
        .context("failed to build system prompt")?;

    let llm_clone = llm.clone();

    let odyssey_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.into(),
        ReActAgent::new(OdysseyAgent::new(system_prompt, vec![])),
        memory_clone,
    );

    let sandbox: Option<Arc<dyn SandboxProvider>> = {
        #[cfg(target_os = "linux")]
        {
            Some(Arc::new(
                BubblewrapProvider::new().context("failed to init bubblewrap provider")?,
            ))
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Not yet supported.
            Some(Arc::new(LocalSandboxProvider::default()))
        }
    };

    let orchestrator = Orchestrator::new(config, tools, sandbox, None, Some(skill_store), None)?;

    orchestrator.register_llm_provider(LLMEntry {
        id: DEFAULT_LLM_ID.into(),
        provider: llm_clone.clone(),
    })?;

    orchestrator.register_agent(odyssey_agent)?;

    let result = orchestrator
        .run(Some(DEFAULT_AGENT_ID), Some(DEFAULT_LLM_ID), prompt) //TODO: Add the capability to take multi model data, like iamges
        .await?;
    let response = result.response;

    println!("{response}");

    Ok(())
}
