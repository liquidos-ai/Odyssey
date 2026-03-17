mod executor;

use std::sync::Arc;

use autoagents_core::{
    agent::{AgentDeriveT, AgentHooks},
    tool::{ToolT, shared_tools_to_boxes},
};
pub(crate) use executor::{ExecutorRun, emit, run_executor};
use tera::Value;

#[derive(Clone, Debug, Default)]
struct OdysseyAgent {
    system_prompt: String,
    tools: Vec<Arc<dyn ToolT>>,
}

impl OdysseyAgent {
    fn new(system_prompt: String, tools: Vec<Arc<dyn ToolT>>) -> Self {
        Self {
            system_prompt,
            tools,
        }
    }
}

#[async_trait::async_trait]
impl AgentDeriveT for OdysseyAgent {
    type Output = String;

    fn description(&self) -> &str {
        &self.system_prompt
    }

    fn output_schema(&self) -> Option<Value> {
        None
    }

    fn name(&self) -> &str {
        "odyssey-agent"
    }

    fn tools(&self) -> Vec<Box<dyn ToolT>> {
        shared_tools_to_boxes(&self.tools)
    }
}

#[async_trait::async_trait]
impl AgentHooks for OdysseyAgent {}

#[cfg(test)]
mod tests {
    use super::OdysseyAgent;
    use autoagents_core::{
        agent::AgentDeriveT,
        tool::{ToolCallError, ToolRuntime, ToolT},
    };
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::sync::Arc;

    #[derive(Debug)]
    struct DummyTool(&'static str);

    impl ToolT for DummyTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "dummy"
        }

        fn args_schema(&self) -> Value {
            json!({ "type": "object" })
        }
    }

    #[async_trait::async_trait]
    impl ToolRuntime for DummyTool {
        async fn execute(&self, _args: Value) -> Result<Value, ToolCallError> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn odyssey_agent_exposes_prompt_identity_and_tools() {
        let tools: Vec<Arc<dyn ToolT>> = vec![Arc::new(DummyTool("search"))];
        let agent = OdysseyAgent::new("system prompt".to_string(), tools);

        assert_eq!(agent.name(), "odyssey-agent");
        assert_eq!(agent.description(), "system prompt");
        assert_eq!(agent.output_schema(), None);

        let boxed = agent.tools();
        assert_eq!(boxed.len(), 1);
        assert_eq!(boxed[0].name(), "search");
        assert_eq!(boxed[0].description(), "dummy");
    }
}
