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
use odyssey_rs::protocol::EventPayload;
use odyssey_rs_sandbox::{BubblewrapProvider, LocalSandboxProvider, SandboxProvider};
use odyssey_rs_tools::builtin_tool_registry;
use std::io::{self, Write};
use std::{path::Path, sync::Arc};
use tokio_stream::StreamExt;

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

    let sandbox_provider = if std::env::consts::OS == "linux" {
        "bubblewrap"
    } else {
        "local"
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

    let sandbox: Option<Arc<dyn SandboxProvider>> = if std::env::consts::OS == "linux" {
        Some(Arc::new(BubblewrapProvider::new().unwrap()))
    } else {
        //Not yet supported
        Some(Arc::new(LocalSandboxProvider::default()))
    };

    let orchestrator = Orchestrator::new(config, tools, sandbox, None, Some(skill_store), None)?;

    orchestrator.register_llm_provider(LLMEntry {
        id: DEFAULT_LLM_ID.into(),
        provider: llm_clone.clone(),
    })?;

    // orchestrator.register_agent(BasicAgent::new(MathAgent {}))?;
    orchestrator.register_agent(odyssey_agent)?;

    let stream = orchestrator
        .run_stream(Some(DEFAULT_AGENT_ID), Some(DEFAULT_LLM_ID), prompt)
        .await?;

    let mut stream = stream;
    let turn_id = stream.turn_id;
    let mut deltas = String::new();

    while let Some(event) = stream.events.next().await {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                eprintln!("stream event error: {error}");
                continue;
            }
        };

        match event.payload {
            EventPayload::TurnStarted {
                turn_id: event_turn_id,
                ..
            } if event_turn_id == turn_id => {
                println!("--- streaming ---");
            }
            EventPayload::AgentMessageDelta {
                turn_id: event_turn_id,
                delta,
            } if event_turn_id == turn_id => {
                if let Err(err) = io::stdout().write_all(delta.as_bytes()) {
                    eprintln!("failed to write delta: {err}");
                }
                if let Err(err) = io::stdout().flush() {
                    eprintln!("failed to flush stdout: {err}");
                }
                deltas.push_str(&delta);
            }
            EventPayload::TurnCompleted {
                turn_id: event_turn_id,
                ..
            } if event_turn_id == turn_id => {
                break;
            }
            _ => {}
        }
    }

    let result = stream.finish().await?;
    println!("\nFinal: {}", result.response);
    if deltas != result.response {
        eprintln!(
            "warning: streamed content differed from final response (streamed_len={}, final_len={})",
            deltas.len(),
            result.response.len()
        );
    }

    Ok(())
}
