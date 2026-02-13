//! Orchestrator integration tests with a mock LLM.

use autoagents_core::agent::prebuilt::executor::ReActAgent;
use autoagents_llm::LLMProvider;
use futures_util::StreamExt;
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_core::{AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator};
use odyssey_rs_memory::FileMemoryProvider;
use odyssey_rs_protocol::EventPayload;
use odyssey_rs_test_utils::{DummyTool, FixedLLM, RecordingLLM, StreamingLLM, base_tool_context};
use odyssey_rs_tools::{ToolRegistry, builtin_tool_registry, tool_to_adaptor};
use parking_lot::RwLock;
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

/// Orchestrator should execute a run using the mock LLM.
#[tokio::test]
async fn orchestrator_runs_with_mock_llm() {
    let llm: Arc<dyn LLMProvider> = Arc::new(FixedLLM::new("mock response"));
    let tools = builtin_tool_registry();
    let temp = tempdir().expect("tempdir");
    let mut config = OdysseyConfig::default();
    config.memory.path = Some(temp.path().join("memory").to_string_lossy().to_string());
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(
            config.memory.path.clone().expect("memory path"),
        ))
        .expect("memory provider"),
    );
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
            provider: llm.clone(),
        })
        .expect("register llm");
    orchestrator
        .register_agent(default_agent)
        .expect("register agent");
    let result = orchestrator
        .run(None, None, "Hello from test")
        .await
        .expect("run");
    assert_eq!(result.response, "mock response");
}

/// Orchestrator should merge registry tools with agent-defined tools.
#[tokio::test]
async fn orchestrator_merges_registry_and_agent_tools() {
    let temp = tempdir().expect("tempdir");
    let mut config = OdysseyConfig::default();
    config.memory.path = Some(temp.path().join("memory").to_string_lossy().to_string());
    let tools = {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool::new("RegistryTool")));
        registry
    };
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(
            config.memory.path.clone().expect("memory path"),
        ))
        .expect("memory provider"),
    );
    let ctx = Arc::new(RwLock::new(base_tool_context()));
    let agent_tool = tool_to_adaptor(Arc::new(DummyTool::new("AgentTool")), ctx);
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new(
            "Test agent".to_string(),
            vec![agent_tool],
        )),
        memory,
    );
    let (llm, seen_tools) = RecordingLLM::new("mock response");
    let llm: Arc<dyn LLMProvider> = Arc::new(llm);
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

    let result = orchestrator.run(None, None, "Hello from test").await;
    assert_eq!(result.expect("run").response, "mock response");

    let mut names = seen_tools.lock().clone();
    names.sort();
    assert_eq!(
        names,
        vec!["AgentTool".to_string(), "RegistryTool".to_string()]
    );
}

/// Orchestrator should stream agent deltas and turn lifecycle events.
#[tokio::test]
async fn orchestrator_streams_run_events() {
    let llm: Arc<dyn LLMProvider> = Arc::new(StreamingLLM::new(vec![
        "stream ".to_string(),
        "response".to_string(),
    ]));
    let tools = builtin_tool_registry();
    let temp = tempdir().expect("tempdir");
    let mut config = OdysseyConfig::default();
    config.memory.path = Some(temp.path().join("memory").to_string_lossy().to_string());
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(
            config.memory.path.clone().expect("memory path"),
        ))
        .expect("memory provider"),
    );
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

    let mut stream = orchestrator
        .run_stream(None, None, "Hello from stream test")
        .await
        .expect("run stream");
    let mut deltas = String::new();
    let mut saw_turn_started = false;
    let mut saw_turn_completed = false;
    let turn_id = stream.turn_id;
    while let Some(event) = stream.events.next().await {
        let event = event.expect("stream event");
        let payload = &event.payload;
        if let EventPayload::TurnStarted {
            turn_id: event_id, ..
        } = payload
        {
            if *event_id == turn_id {
                saw_turn_started = true;
            }
            continue;
        }
        if let EventPayload::AgentMessageDelta {
            turn_id: event_id,
            delta,
        } = payload
        {
            if *event_id == turn_id {
                deltas.push_str(delta);
            }
            continue;
        }
        if let EventPayload::TurnCompleted {
            turn_id: event_id, ..
        } = payload
            && *event_id == turn_id
        {
            saw_turn_completed = true;
            break;
        }
    }

    let result = stream.finish().await.expect("finish");
    assert_eq!(result.response, "stream response");
    assert_eq!(deltas, "stream response");
    assert_eq!(saw_turn_started, true);
    assert_eq!(saw_turn_completed, true);
}
