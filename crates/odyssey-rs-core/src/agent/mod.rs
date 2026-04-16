//! AutoAgents adapter for Odyssey.

use async_trait::async_trait;
use autoagents_core::{
    agent::{AgentDeriveT, AgentHooks},
    tool::ToolT,
    tool::shared_tools_to_boxes,
};
use odyssey_rs_config::ToolPolicy;
use odyssey_rs_memory::MemoryProvider;
use std::{fmt::Debug, sync::Arc};

use crate::types::OdysseyAgentRuntime;

pub mod builder;
pub mod llm;
pub mod memory;
mod tool_messages;

pub trait AgentInstance: OdysseyAgentRuntime {
    /// Tool policy to apply for this agent.
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::allow_all()
    }

    /// Memory provider used to persist and recall session state.
    fn memory_provider(&self) -> Arc<dyn MemoryProvider>;
}

/// Odyssey agent wrapper used by the AutoAgents runtime.
#[derive(Clone, Default)]
pub struct OdysseyAgent {
    /// System prompt description for the agent.
    system_prompt: String,
    /// Shared tool instances available to the agent.
    tools: Vec<Arc<dyn ToolT>>,
}

impl Debug for OdysseyAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OdysseyAgent")
            .field("description", &self.system_prompt)
            .field("tools", &self.tools.len())
            .finish()
    }
}

impl OdysseyAgent {
    /// Create a new Odyssey agent wrapper.
    pub fn new(system_prompt: String, tools: Vec<Arc<dyn ToolT>>) -> Self {
        Self {
            system_prompt,
            tools,
        }
    }
}

#[async_trait]
impl AgentDeriveT for OdysseyAgent {
    type Output = String;

    fn description(&self) -> &str {
        &self.system_prompt
    }

    /// Return the output schema (none for string output).
    fn output_schema(&self) -> Option<serde_json::Value> {
        None
    }

    /// Return the agent name.
    fn name(&self) -> &str {
        "odyssey-agent"
    }

    /// Return boxed tool handles for AutoAgents.
    fn tools(&self) -> Vec<Box<dyn ToolT>> {
        shared_tools_to_boxes(&self.tools)
    }
}

#[async_trait]
/// No-op hooks implementation for OdysseyAgent.
impl AgentHooks for OdysseyAgent {}

#[cfg(test)]
mod tests {
    use super::OdysseyAgent;
    use autoagents_core::agent::AgentDeriveT;
    use odyssey_rs_test_utils::DummyToolRuntime;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[test]
    fn odyssey_agent_exposes_description_and_tools() {
        let tool = Arc::new(DummyToolRuntime::new("Dummy"));
        let agent = OdysseyAgent::new("prompt".to_string(), vec![tool]);

        assert_eq!(agent.description(), "prompt");
        assert_eq!(agent.name(), "odyssey-agent");
        let tools = agent.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "Dummy");
    }
}
