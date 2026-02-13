//! Session persistence integration tests.

use autoagents_core::agent::prebuilt::executor::ReActAgent;
use autoagents_llm::LLMProvider;
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_core::{AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator};
use odyssey_rs_memory::FileMemoryProvider;
use odyssey_rs_test_utils::FixedLLM;
use odyssey_rs_tools::builtin_tool_registry;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

/// Sessions should resume from the configured state store.
#[tokio::test]
async fn resumes_session_from_state_store() {
    let temp = tempdir().expect("tempdir");
    let mut config = OdysseyConfig::default();
    config.sessions.enabled = true;
    config.sessions.path = Some(temp.path().join("sessions").to_string_lossy().to_string());
    config.memory.path = Some(temp.path().join("memory").to_string_lossy().to_string());

    let llm: Arc<dyn LLMProvider> = Arc::new(FixedLLM::new("persisted response"));
    let tools = builtin_tool_registry();
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(
            config.memory.path.clone().expect("memory path"),
        ))
        .expect("memory provider"),
    );
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new("Test agent".to_string(), Vec::new())),
        memory.clone(),
    );
    let orchestrator = Orchestrator::new(config.clone(), tools, None, None, None, None)
        .expect("build orchestrator");
    orchestrator
        .register_llm_provider(LLMEntry {
            id: "default_LLM".to_string(),
            provider: llm.clone(),
        })
        .expect("register llm");
    orchestrator
        .register_agent(default_agent)
        .expect("register agent");
    let result = orchestrator.run(None, None, "hello").await.expect("run");

    let tools = builtin_tool_registry();
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new("Test agent".to_string(), Vec::new())),
        memory,
    );
    let orchestrator =
        Orchestrator::new(config, tools, None, None, None, None).expect("build orchestrator");
    orchestrator
        .register_llm_provider(LLMEntry {
            id: "default_LLM".to_string(),
            provider: llm,
        })
        .expect("register llm");
    orchestrator
        .register_agent(default_agent)
        .expect("register agent");
    let session = orchestrator
        .resume_session(result.session_id)
        .expect("resume session");

    assert_eq!(session.id, result.session_id);
    assert!(session.messages.len() >= 2);
}
