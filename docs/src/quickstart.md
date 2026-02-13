# Quickstart

This guide shows how to run Odyssey as an SDK and how to stream responses.

## Prerequisites
- Rust toolchain (edition 2024).
- `OPENAI_API_KEY` set in your environment.

## Minimal SDK usage
```rust
use anyhow::Result;
use autoagents::prelude::ReActAgent;
use autoagents_llm::LLMProvider;
use autoagents_llm::backends::openai::OpenAI;
use autoagents_llm::builder::LLMBuilder;
use odyssey_rs::config::OdysseyConfig;
use odyssey_rs::core::prompt::{PromptBuilder, PromptProfile};
use odyssey_rs::core::{AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator};
use odyssey_rs::core::skills::SkillStore;
use odyssey_rs::memory::FileMemoryProvider;
use odyssey_rs_tools::builtin_tool_registry;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let config = OdysseyConfig::default();
    let llm: Arc<dyn LLMProvider> = LLMBuilder::<OpenAI>::new()
        .api_key(std::env::var("OPENAI_API_KEY")?)
        .model("gpt-4.1-mini")
        .build()?;

    let tools = builtin_tool_registry();
    let memory_root = config
        .memory
        .path
        .clone()
        .unwrap_or_else(|| ".odyssey/memory".to_string());
    let memory = Arc::new(FileMemoryProvider::new(PathBuf::from(memory_root))?);
    let cwd = std::env::current_dir()?;
    let skill_store = Arc::new(SkillStore::load(&config.skills, &cwd)?);
    let system_prompt = PromptBuilder::new(memory.clone(), Some(skill_store.clone()))
        .build_system_prompt("", &config.memory, PromptProfile::OrchestratorDefault)
        .await?;

    let orchestrator = Orchestrator::new(
        config,
        tools,
        None,
        None,
        Some(skill_store),
        None,
    )?;
    orchestrator.register_llm_provider(LLMEntry {
        id: "default_LLM".to_string(),
        provider: llm,
    })?;
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new(system_prompt, Vec::new())),
        memory,
    );
    orchestrator.register_agent(default_agent)?;

    let result = orchestrator
        .run(None, None, "Explain Odyssey in one paragraph.")
        .await?;
    println!("{}", result.response);
    Ok(())
}
```

## Streaming usage
```rust
use futures_util::StreamExt;
use odyssey_rs::protocol::EventPayload;

let mut stream = orchestrator
    .run_stream(None, None, "Stream me the response.")
    .await?;

while let Some(event) = stream.events.next().await {
    let event = match event {
        Ok(event) => event,
        Err(_) => continue,
    };
    if let EventPayload::AgentMessageDelta { delta, .. } = event.payload {
        print!("{delta}");
    }
}

let result = stream.finish().await?;
println!("\nFinal: {}", result.response);
```

## Next steps
- `config.md` for JSON5 configuration and layering.
- `skills.md` for skill discovery and SKILL.md format.
- `permissions.md` for permission rules and approval flows.
- `architecture.md` for runtime flows and boundaries.
