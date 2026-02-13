//! Agent registry and default agent resolution.

use super::agent_factory::AgentExecutorRunner;
use crate::error::OdysseyCoreError;
use crate::types::{AgentID, LLMProviderID};
use autoagents_llm::LLMProvider;
use log::{debug, info};
use odyssey_rs_config::{AgentSandboxConfig, MemoryConfig, PermissionMode, ToolPolicy};
use odyssey_rs_memory::MemoryProvider;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Stored configuration and runtime for a registered agent.
#[derive(Clone)]
pub(crate) struct AgentEntry {
    /// Agent identifier.
    pub(crate) id: String,
    /// Optional human-friendly description.
    pub(crate) description: Option<String>,
    /// Base prompt for the agent.
    #[allow(dead_code)]
    pub(crate) prompt: String,
    /// Optional model configuration.
    pub(crate) model: Option<odyssey_rs_config::ModelConfig>,
    /// Tool allow/deny policy.
    pub(crate) tool_policy: ToolPolicy,
    /// Optional permission mode override.
    pub(crate) permission_mode: Option<PermissionMode>,
    /// Optional sandbox overrides.
    pub(crate) sandbox: Option<AgentSandboxConfig>,
    /// Optional memory overrides.
    pub(crate) memory: Option<MemoryConfig>,
    /// Memory provider used by the agent runtime.
    pub(crate) memory_provider: Arc<dyn MemoryProvider>,
    /// Executor wrapper used to run the agent.
    pub(crate) executor: Arc<dyn AgentExecutorRunner>,
}

impl AgentEntry {
    /// Build a new agent entry for registration.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: String,
        description: Option<String>,
        prompt: String,
        model: Option<odyssey_rs_config::ModelConfig>,
        tool_policy: ToolPolicy,
        permission_mode: Option<PermissionMode>,
        sandbox: Option<AgentSandboxConfig>,
        memory: Option<MemoryConfig>,
        memory_provider: Arc<dyn MemoryProvider>,
        executor: Arc<dyn AgentExecutorRunner>,
    ) -> Self {
        Self {
            id,
            description,
            prompt,
            model,
            tool_policy,
            permission_mode,
            sandbox,
            memory,
            memory_provider,
            executor,
        }
    }
}

/// In-memory agent registry with default id tracking.
#[derive(Clone)]
pub(crate) struct AgentRegistry {
    agents: Arc<RwLock<HashMap<AgentID, AgentEntry>>>,
    default_agent_id: Arc<RwLock<AgentID>>,
}

impl AgentRegistry {
    pub(crate) fn new(default_id: AgentID) -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::default())),
            default_agent_id: Arc::new(RwLock::new(default_id)),
        }
    }

    pub(crate) fn insert_entry(&self, entry: AgentEntry) {
        let mut agents = self.agents.write();
        agents.insert(entry.id.clone(), entry);
    }

    /// Return the current default agent id.
    pub(crate) fn default_agent_id(&self) -> String {
        self.default_agent_id.read().clone()
    }

    /// Resolve a requested agent id to a registered id.
    pub(crate) fn resolve_agent_id(
        &self,
        agent_id: Option<&str>,
    ) -> Result<String, OdysseyCoreError> {
        let resolved = if let Some(agent_id) = agent_id {
            agent_id.to_string()
        } else {
            self.default_agent_id.read().clone()
        };
        if !self.agents.read().contains_key(&resolved) {
            return Err(OdysseyCoreError::UnknownAgent(resolved));
        }
        Ok(resolved)
    }

    /// Fetch a registered agent entry by id.
    pub(crate) fn get_entry(&self, agent_id: &str) -> Result<AgentEntry, OdysseyCoreError> {
        self.agents
            .read()
            .get(agent_id)
            .cloned()
            .ok_or_else(|| OdysseyCoreError::UnknownAgent(agent_id.to_string()))
    }

    /// List all registered agent ids.
    pub(crate) fn list_agents(&self) -> Vec<String> {
        self.agents.read().keys().cloned().collect()
    }

    /// Set the default agent id to an existing registered agent.
    pub(crate) fn set_default_agent_id(
        &self,
        agent_id: impl Into<String>,
    ) -> Result<(), OdysseyCoreError> {
        let agent_id = agent_id.into();
        let agents = self.agents.read();
        if !agents.contains_key(&agent_id) {
            return Err(OdysseyCoreError::UnknownAgent(agent_id));
        }
        debug!("setting default agent id (agent_id={})", agent_id);
        *self.default_agent_id.write() = agent_id;
        Ok(())
    }
}
#[derive(Clone)]
pub struct LLMEntry {
    pub id: String,
    pub provider: Arc<dyn LLMProvider>,
}

#[derive(Default)]
pub(crate) struct LLMRegistry {
    providers: Arc<RwLock<HashMap<LLMProviderID, LLMEntry>>>,
    default_provider: Arc<RwLock<LLMProviderID>>,
}

impl LLMRegistry {
    pub(crate) fn new(default_id: LLMProviderID) -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::default())),
            default_provider: Arc::new(RwLock::new(default_id)),
        }
    }

    pub(crate) fn insert_entry(&self, entry: LLMEntry) {
        let mut providers = self.providers.write();
        providers.insert(entry.id.clone(), entry);
    }

    pub(crate) fn list_llm_ids(&self) -> Vec<String> {
        self.providers.read().keys().cloned().collect()
    }

    pub(crate) fn get_entry(&self, llm_id: &str) -> Result<LLMEntry, OdysseyCoreError> {
        self.providers
            .read()
            .get(llm_id)
            .cloned()
            .ok_or_else(|| OdysseyCoreError::UnknownAgent(llm_id.to_string()))
    }

    /// Resolve a requested agent id to a registered id.
    pub(crate) fn resolve_llm_id(&self, llm_id: Option<&str>) -> Result<String, OdysseyCoreError> {
        let resolved = if let Some(llm_id) = llm_id {
            llm_id.to_string()
        } else {
            self.default_provider.read().clone()
        };
        if !self.providers.read().contains_key(&resolved) {
            return Err(OdysseyCoreError::UnknownAgent(resolved));
        }
        Ok(resolved)
    }

    #[allow(dead_code)]
    pub(crate) fn set_default_llm_entry(&self, entry: LLMEntry) -> Result<(), OdysseyCoreError> {
        let current_default = self.default_provider.read().clone();
        let entry_id = entry.id.clone();
        {
            let mut providers = self.providers.write();
            if providers.contains_key(&entry_id) && entry_id != current_default {
                return Err(OdysseyCoreError::Executor(format!(
                    "agent already registered: {entry_id}"
                )));
            }
            providers.insert(entry_id.clone(), entry);
        }
        info!("updated default agent entry (agent_id={})", entry_id);
        *self.default_provider.write() = entry_id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentEntry, AgentRegistry, LLMEntry, LLMRegistry};
    use crate::error::OdysseyCoreError;
    use crate::orchestrator::agent_factory::{AgentExecutorRunner, AgentInput};
    use async_trait::async_trait;
    use autoagents_core::tool::ToolT;
    use autoagents_llm::LLMProvider;
    use futures_util::Stream;
    use odyssey_rs_config::{PermissionMode, ToolPolicy};
    use odyssey_rs_protocol::{EventSink, TurnContext, TurnId};
    use odyssey_rs_test_utils::{FailingLLM, StubMemory};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    struct DummyExecutor;

    #[async_trait]
    impl AgentExecutorRunner for DummyExecutor {
        async fn run(
            &self,
            _input: AgentInput,
            _turn_id: TurnId,
            _turn_context: TurnContext,
            _tools: Vec<Arc<dyn ToolT>>,
            _llm: Arc<dyn LLMProvider>,
            _memory: Option<Box<dyn autoagents_core::agent::memory::MemoryProvider>>,
            _event_sink: Option<Arc<dyn EventSink>>,
        ) -> Result<String, OdysseyCoreError> {
            Err(OdysseyCoreError::Executor("dummy".to_string()))
        }

        async fn run_stream(
            &self,
            _input: AgentInput,
            _turn_id: TurnId,
            _turn_context: TurnContext,
            _tools: Vec<Arc<dyn ToolT>>,
            _llm: Arc<dyn LLMProvider>,
            _memory: Option<Box<dyn autoagents_core::agent::memory::MemoryProvider>>,
            _event_sink: Arc<dyn EventSink>,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<String, OdysseyCoreError>> + Send>>,
            OdysseyCoreError,
        > {
            Err(OdysseyCoreError::Executor("dummy".to_string()))
        }
    }

    fn entry(id: &str) -> AgentEntry {
        AgentEntry::new(
            id.to_string(),
            Some(format!("{id} desc")),
            format!("{id} prompt"),
            None,
            ToolPolicy::default(),
            Some(PermissionMode::Default),
            None,
            None,
            Arc::new(StubMemory::default()),
            Arc::new(DummyExecutor),
        )
    }

    #[test]
    fn agent_registry_tracks_default_and_entries() {
        let registry = AgentRegistry::new("agent-a".to_string());
        registry.insert_entry(entry("agent-a"));
        registry.insert_entry(entry("agent-b"));

        let mut agents = registry.list_agents();
        agents.sort();
        assert_eq!(agents, vec!["agent-a".to_string(), "agent-b".to_string()]);
        assert_eq!(registry.resolve_agent_id(None).unwrap(), "agent-a");
        assert_eq!(registry.get_entry("agent-b").unwrap().id, "agent-b");

        registry
            .set_default_agent_id("agent-b")
            .expect("set default");
        assert_eq!(registry.default_agent_id(), "agent-b".to_string());
    }

    #[test]
    fn agent_registry_rejects_unknown_default() {
        let registry = AgentRegistry::new("agent-a".to_string());
        registry.insert_entry(entry("agent-a"));

        let err = registry
            .resolve_agent_id(Some("missing"))
            .expect_err("unknown");
        match err {
            OdysseyCoreError::UnknownAgent(name) => assert_eq!(name, "missing".to_string()),
            other => panic!("unexpected error: {other:?}"),
        }

        let err = registry
            .set_default_agent_id("missing")
            .expect_err("unknown");
        match err {
            OdysseyCoreError::UnknownAgent(name) => assert_eq!(name, "missing".to_string()),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn llm_registry_resolves_default_entry() {
        let registry = LLMRegistry::new("primary".to_string());
        let entry = LLMEntry {
            id: "primary".to_string(),
            provider: Arc::new(FailingLLM::new("dummy")),
        };
        registry.insert_entry(entry);

        assert_eq!(registry.resolve_llm_id(None).unwrap(), "primary");
        assert_eq!(registry.get_entry("primary").unwrap().id, "primary");
    }
}
