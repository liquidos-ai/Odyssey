use anyhow::{Context, Result};
use autoagents::llm::LLMProvider;
use autoagents::llm::backends::openai::OpenAI;
use autoagents::llm::builder::LLMBuilder;
use autoagents::prelude::ReActAgent;
use odyssey_rs::config::OdysseyConfig;
use odyssey_rs_sandbox::{LocalSandboxProvider, SandboxProvider};
use odyssey_rs_tools::builtin_tool_registry;
use std::path::PathBuf;
use std::sync::Arc;

use odyssey_rs::protocol::EventMsg;
use odyssey_rs::{
    core::{AgentBuilder, DEFAULT_AGENT_ID, EventSink, LLMEntry, OdysseyAgent, Orchestrator},
    init_logging,
    memory::FileMemoryProvider,
};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct EventBus {
    sender: broadcast::Sender<EventMsg>,
}

impl EventBus {
    pub fn new(buffer: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventMsg> {
        self.sender.subscribe()
    }
}

impl EventSink for EventBus {
    fn emit(&self, event: EventMsg) {
        let _ = self.sender.send(event);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    // File-based config example.
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("odyssey.json5");
    let config_display = config_path.display().to_string();
    println!("{config_display}");
    let config = OdysseyConfig::load_from_path(&config_path)
        .with_context(|| format!("failed to load config at {config_display}"))?;

    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY is required to run the memory_config example")?;
    let llm: Arc<dyn LLMProvider> = LLMBuilder::<OpenAI>::new()
        .api_key(api_key)
        .model("gpt-4.1")
        .build()
        .context("failed to build OpenAI LLM provider")?;

    let tools = builtin_tool_registry();
    let memory_root = config
        .memory
        .path
        .clone()
        .unwrap_or_else(|| ".odyssey/memory".to_string());
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(memory_root))
            .context("failed to create memory provider")?,
    );
    let sandbox_enabled = config.sandbox.enabled;
    // NoSandboxProvider executes locally; swap in a real sandbox for isolation.
    let sandbox_provider = if sandbox_enabled {
        Some(Arc::new(LocalSandboxProvider::default()) as Arc<dyn SandboxProvider>)
    } else {
        None
    };

    let event_bus = Arc::new(EventBus::new(100));

    let mut receiver = event_bus.subscribe();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                message = receiver.recv() => {
                    match message {
                        Ok(event) => {
                           println!("{:?}", event);
                        }

                        Err(broadcast::error::RecvError::Closed) => break,

                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                    }
                }
            }
        }

        println!("event forwarder task ended");
    });

    let orchestrator = Orchestrator::new(
        config,
        tools,
        sandbox_provider,
        None,
        None,
        Some(event_bus.clone()),
    )?;
    orchestrator.register_llm_provider(LLMEntry {
        id: "default_LLM".to_string(),
        provider: llm.clone(),
    })?;
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new(
            "Odyssey event listener".to_string(),
            Vec::new(),
        )),
        memory,
    );
    orchestrator.register_agent(default_agent)?;

    let result = orchestrator
        .run(None, None, "Send a short greeting.")
        .await?;
    println!("Response: {}", result.response);

    Ok(())
}
