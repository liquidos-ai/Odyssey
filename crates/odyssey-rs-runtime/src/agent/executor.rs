use super::OdysseyAgent;
use crate::RuntimeError;
use autoagents_core::agent::prebuilt::executor::ReActAgent;
use autoagents_core::agent::{AgentBuilder, DirectAgent, task::Task};
use autoagents_core::tool::ToolT;
use autoagents_llm::LLMProvider;
use chrono::Utc;
use futures_util::StreamExt;
use odyssey_rs_protocol::{AutoAgentsEvent, AutoAgentsStreamChunk};
use odyssey_rs_protocol::{EventMsg, EventPayload, TurnContext};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

pub struct ExecutorRun {
    pub executor_id: String,
    pub llm: Arc<dyn LLMProvider>,
    pub system_prompt: String,
    pub task: Task,
    pub memory: Option<Box<dyn autoagents_core::agent::memory::MemoryProvider>>,
    pub tools: Vec<Arc<dyn ToolT>>,
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub sender: broadcast::Sender<EventMsg>,
    pub turn_context: TurnContext,
}

pub async fn run_executor(run: ExecutorRun) -> Result<String, RuntimeError> {
    match run.executor_id.as_str() {
        "react" => run_react(run).await,
        other => Err(RuntimeError::Unsupported(format!(
            "unsupported prebuilt executor: {other}"
        ))),
    }
}

async fn run_react(run: ExecutorRun) -> Result<String, RuntimeError> {
    let agent = ReActAgent::new(OdysseyAgent::new(run.system_prompt.clone(), run.tools));
    let mut builder = AgentBuilder::<ReActAgent<OdysseyAgent>, DirectAgent>::new(agent)
        .llm(run.llm)
        .stream(true);
    if let Some(memory) = run.memory {
        builder = builder.memory(memory);
    }
    let mut handle = builder
        .build()
        .await
        .map_err(|err| RuntimeError::Executor(err.to_string()))?;
    let events = handle.subscribe_events();
    let event_task = tokio::spawn(forward_autoagents_events(
        events,
        run.sender.clone(),
        run.session_id,
        run.turn_id,
        run.turn_context.clone(),
    ));
    let task = run.task.with_system_prompt(run.system_prompt);
    let stream = match handle.agent.run_stream(task).await {
        Ok(stream) => stream,
        Err(err) => {
            event_task.abort();
            return Err(RuntimeError::Executor(err.to_string()));
        }
    };
    let mut response = String::default();
    tokio::pin!(stream);
    while let Some(output) = stream.next().await {
        match output {
            Ok(output) => response = output,
            Err(err) => {
                event_task.abort();
                return Err(RuntimeError::Executor(err.to_string()));
            }
        }
    }
    match event_task.await {
        Ok(()) => {}
        Err(err) => {
            return Err(RuntimeError::Executor(format!(
                "failed to forward autoagents events: {err}"
            )));
        }
    }
    emit(
        &run.sender,
        run.session_id,
        EventPayload::TurnCompleted {
            turn_id: run.turn_id,
            message: response.clone(),
        },
    );
    Ok(response)
}

async fn forward_autoagents_events(
    mut events: autoagents_core::utils::BoxEventStream<AutoAgentsEvent>,
    sender: broadcast::Sender<EventMsg>,
    session_id: Uuid,
    turn_id: Uuid,
    turn_context: TurnContext,
) {
    let mut bridge = AutoagentsEventBridge::new(turn_id, turn_context);
    while let Some(event) = events.next().await {
        let mapped = bridge.map_event(event);
        for payload in mapped.payloads {
            emit(&sender, session_id, payload);
        }
        if mapped.terminal {
            break;
        }
    }
}

struct MappedEvent {
    payloads: Vec<EventPayload>,
    terminal: bool,
}

struct AutoagentsEventBridge {
    turn_id: Uuid,
    turn_context: TurnContext,
    reasoning_open: bool,
    tool_index_ids: HashMap<usize, String>,
    tool_call_ids: HashMap<String, Uuid>,
    started_tool_calls: HashSet<Uuid>,
}

impl AutoagentsEventBridge {
    fn new(turn_id: Uuid, turn_context: TurnContext) -> Self {
        Self {
            turn_id,
            turn_context,
            reasoning_open: false,
            tool_index_ids: HashMap::new(),
            tool_call_ids: HashMap::new(),
            started_tool_calls: HashSet::new(),
        }
    }

    fn map_event(&mut self, event: AutoAgentsEvent) -> MappedEvent {
        match event {
            AutoAgentsEvent::TurnStarted { .. } => MappedEvent {
                payloads: vec![EventPayload::TurnStarted {
                    turn_id: self.turn_id,
                    context: self.turn_context.clone(),
                }],
                terminal: false,
            },
            AutoAgentsEvent::StreamChunk { chunk, .. } => MappedEvent {
                payloads: self.map_stream_chunk(chunk),
                terminal: false,
            },
            AutoAgentsEvent::StreamToolCall { tool_call, .. } => MappedEvent {
                payloads: self.map_stream_tool_call(tool_call),
                terminal: false,
            },
            AutoAgentsEvent::ToolCallRequested {
                id,
                tool_name,
                arguments,
                ..
            } => MappedEvent {
                payloads: self.map_tool_call_requested(id, tool_name, arguments),
                terminal: false,
            },
            AutoAgentsEvent::ToolCallCompleted { id, result, .. } => MappedEvent {
                payloads: self.map_tool_call_finished(id, result, true),
                terminal: false,
            },
            AutoAgentsEvent::ToolCallFailed { id, error, .. } => MappedEvent {
                payloads: self.map_tool_call_finished(id, json!({ "error": error }), false),
                terminal: false,
            },
            AutoAgentsEvent::TaskError { error, .. } => {
                let mut payloads = self.close_reasoning_section();
                payloads.push(EventPayload::Error {
                    turn_id: Some(self.turn_id),
                    message: error,
                });
                MappedEvent {
                    payloads,
                    terminal: true,
                }
            }
            AutoAgentsEvent::TaskComplete { .. } | AutoAgentsEvent::StreamComplete { .. } => {
                MappedEvent {
                    payloads: self.close_reasoning_section(),
                    terminal: true,
                }
            }
            AutoAgentsEvent::TurnCompleted { .. }
            | AutoAgentsEvent::NewTask { .. }
            | AutoAgentsEvent::PublishMessage { .. }
            | AutoAgentsEvent::SendMessage { .. }
            | AutoAgentsEvent::TaskStarted { .. } => MappedEvent {
                payloads: Vec::new(),
                terminal: false,
            },
        }
    }

    fn map_stream_chunk(&mut self, chunk: AutoAgentsStreamChunk) -> Vec<EventPayload> {
        match chunk {
            AutoAgentsStreamChunk::Text(delta) => {
                let mut payloads = self.close_reasoning_section();
                payloads.push(EventPayload::AgentMessageDelta {
                    turn_id: self.turn_id,
                    delta,
                });
                payloads
            }
            AutoAgentsStreamChunk::ReasoningContent(delta) => {
                self.reasoning_open = true;
                vec![EventPayload::ReasoningDelta {
                    turn_id: self.turn_id,
                    delta,
                }]
            }
            AutoAgentsStreamChunk::ToolUseStart { index, id, .. } => {
                self.tool_index_ids.insert(index, id.clone());
                self.close_reasoning_section()
            }
            AutoAgentsStreamChunk::ToolUseInputDelta {
                index,
                partial_json,
            } => {
                let mut payloads = self.close_reasoning_section();
                let raw_tool_call_id = self
                    .tool_index_ids
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| index.to_string());
                let tool_call_id = self.tool_call_id(&raw_tool_call_id);
                payloads.push(EventPayload::ToolCallDelta {
                    turn_id: self.turn_id,
                    tool_call_id,
                    delta: json!({ "partial_json": partial_json }),
                });
                payloads
            }
            AutoAgentsStreamChunk::ToolUseComplete { tool_call, .. } => self
                .map_tool_call_requested(
                    tool_call.id,
                    tool_call.function.name,
                    tool_call.function.arguments,
                ),
            AutoAgentsStreamChunk::Done { .. } | AutoAgentsStreamChunk::Usage(_) => {
                self.close_reasoning_section()
            }
        }
    }

    fn map_stream_tool_call(&mut self, tool_call: Value) -> Vec<EventPayload> {
        let Some(tool_call_id) = tool_call.get("id").and_then(Value::as_str) else {
            return Vec::new();
        };
        let tool_name = tool_call
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let arguments = tool_call
            .get("function")
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
            .map(parse_json_value)
            .unwrap_or(tool_call.clone());
        self.map_tool_call_started(tool_call_id.to_string(), tool_name, arguments)
    }

    fn map_tool_call_requested(
        &mut self,
        raw_tool_call_id: String,
        tool_name: String,
        arguments: String,
    ) -> Vec<EventPayload> {
        let arguments = parse_json_value(&arguments);
        self.map_tool_call_started(raw_tool_call_id, tool_name, arguments)
    }

    fn map_tool_call_started(
        &mut self,
        raw_tool_call_id: String,
        tool_name: String,
        arguments: Value,
    ) -> Vec<EventPayload> {
        let mut payloads = self.close_reasoning_section();
        let tool_call_id = self.tool_call_id(&raw_tool_call_id);
        if self.started_tool_calls.insert(tool_call_id) {
            payloads.push(EventPayload::ToolCallStarted {
                turn_id: self.turn_id,
                tool_call_id,
                tool_name,
                arguments,
            });
        } else {
            payloads.push(EventPayload::ToolCallDelta {
                turn_id: self.turn_id,
                tool_call_id,
                delta: arguments,
            });
        }
        payloads
    }

    fn map_tool_call_finished(
        &mut self,
        raw_tool_call_id: String,
        result: Value,
        success: bool,
    ) -> Vec<EventPayload> {
        let mut payloads = self.close_reasoning_section();
        let tool_call_id = self.tool_call_id(&raw_tool_call_id);
        payloads.push(EventPayload::ToolCallFinished {
            turn_id: self.turn_id,
            tool_call_id,
            result,
            success,
        });
        payloads
    }

    fn close_reasoning_section(&mut self) -> Vec<EventPayload> {
        if self.reasoning_open {
            self.reasoning_open = false;
            return vec![EventPayload::ReasoningSectionBreak {
                turn_id: self.turn_id,
            }];
        }
        Vec::new()
    }

    fn tool_call_id(&mut self, raw_tool_call_id: &str) -> Uuid {
        *self
            .tool_call_ids
            .entry(raw_tool_call_id.to_string())
            .or_insert_with(Uuid::new_v4)
    }
}

fn parse_json_value(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
}

pub fn emit(sender: &broadcast::Sender<EventMsg>, session_id: Uuid, payload: EventPayload) {
    let _ = sender.send(EventMsg {
        id: Uuid::new_v4(),
        session_id,
        created_at: Utc::now(),
        payload,
    });
}

#[cfg(test)]
mod tests {
    use super::{AutoagentsEventBridge, emit, parse_json_value};
    use odyssey_rs_protocol::{AutoAgentsEvent, AutoAgentsStreamChunk};
    use odyssey_rs_protocol::{EventPayload, TurnContext};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use tokio::sync::broadcast;
    use uuid::Uuid;

    #[test]
    fn parse_json_value_falls_back_to_string() {
        assert_eq!(parse_json_value("{\"value\":1}"), json!({ "value": 1 }));
        assert_eq!(
            parse_json_value("not-json"),
            Value::String("not-json".to_string())
        );
    }

    #[test]
    fn reasoning_content_emits_section_break_before_text() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());

        let reasoning = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::ReasoningContent("plan".to_string()),
        });
        assert!(!reasoning.terminal);
        assert_eq!(reasoning.payloads.len(), 1);
        assert!(matches!(
            &reasoning.payloads[0],
            EventPayload::ReasoningDelta { turn_id: id, delta }
                if *id == turn_id && delta == "plan"
        ));

        let text = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::Text("done".to_string()),
        });
        assert_eq!(text.payloads.len(), 2);
        assert!(matches!(
            &text.payloads[0],
            EventPayload::ReasoningSectionBreak { turn_id: id } if *id == turn_id
        ));
        assert!(matches!(
            &text.payloads[1],
            EventPayload::AgentMessageDelta { turn_id: id, delta }
                if *id == turn_id && delta == "done"
        ));
    }

    #[test]
    fn tool_call_events_keep_a_stable_runtime_id() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());

        let started = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::ToolUseStart {
                index: 0,
                id: "call_1".to_string(),
                name: "search".to_string(),
            },
        });
        let requested = bridge.map_event(AutoAgentsEvent::ToolCallRequested {
            sub_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            id: "call_1".to_string(),
            tool_name: "search".to_string(),
            arguments: "{\"q\":\"rust\"}".to_string(),
        });
        let finished = bridge.map_event(AutoAgentsEvent::ToolCallCompleted {
            sub_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            id: "call_1".to_string(),
            tool_name: "search".to_string(),
            result: json!({ "ok": true }),
        });

        assert!(started.payloads.is_empty());

        let started_id = match &requested.payloads[0] {
            EventPayload::ToolCallStarted { tool_call_id, .. } => *tool_call_id,
            other => panic!("unexpected payload: {other:?}"),
        };
        let finished_id = match &finished.payloads[0] {
            EventPayload::ToolCallFinished { tool_call_id, .. } => *tool_call_id,
            other => panic!("unexpected payload: {other:?}"),
        };

        assert_eq!(started_id, finished_id);
    }

    #[test]
    fn turn_started_maps_context_into_runtime_event() {
        let turn_id = Uuid::new_v4();
        let context = TurnContext {
            cwd: Some("/workspace".to_string()),
            ..TurnContext::default()
        };
        let mut bridge = AutoagentsEventBridge::new(turn_id, context.clone());

        let started = bridge.map_event(AutoAgentsEvent::TurnStarted {
            sub_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            turn_number: 1,
            max_turns: 4,
        });

        assert!(!started.terminal);
        assert!(matches!(
            &started.payloads[0],
            EventPayload::TurnStarted { turn_id: id, context: mapped }
                if *id == turn_id && mapped.cwd == context.cwd
        ));
    }

    #[test]
    fn stream_tool_call_without_id_is_ignored() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());

        let payloads = bridge.map_event(AutoAgentsEvent::StreamToolCall {
            sub_id: Uuid::new_v4(),
            tool_call: json!({
                "function": {
                    "name": "search",
                    "arguments": "{\"q\":\"rust\"}"
                }
            }),
        });

        assert!(payloads.payloads.is_empty());
    }

    #[test]
    fn stream_tool_call_delta_uses_partial_json_and_stable_id() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());

        let _ = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::ToolUseStart {
                index: 2,
                id: "call_2".to_string(),
                name: "search".to_string(),
            },
        });
        let delta = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::ToolUseInputDelta {
                index: 2,
                partial_json: "{\"q\":\"rus".to_string(),
            },
        });
        let started = bridge.map_event(AutoAgentsEvent::StreamToolCall {
            sub_id: Uuid::new_v4(),
            tool_call: json!({
                "id": "call_2",
                "function": {
                    "name": "search",
                    "arguments": "{\"q\":\"rust\"}"
                }
            }),
        });

        let delta_id = match &delta.payloads[0] {
            EventPayload::ToolCallDelta {
                tool_call_id,
                delta,
                ..
            } => {
                assert_eq!(delta, &json!({ "partial_json": "{\"q\":\"rus" }));
                *tool_call_id
            }
            other => panic!("unexpected payload: {other:?}"),
        };
        let started_id = match &started.payloads[0] {
            EventPayload::ToolCallStarted {
                tool_call_id,
                arguments,
                ..
            } => {
                assert_eq!(arguments, &json!({ "q": "rust" }));
                *tool_call_id
            }
            other => panic!("unexpected payload: {other:?}"),
        };

        assert_eq!(delta_id, started_id);
    }

    #[test]
    fn tool_call_failed_marks_unsuccessful_completion() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());

        let failed = bridge.map_event(AutoAgentsEvent::ToolCallFailed {
            sub_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            id: "call_3".to_string(),
            tool_name: "search".to_string(),
            error: "boom".to_string(),
        });

        assert!(matches!(
            &failed.payloads[0],
            EventPayload::ToolCallFinished { success, result, .. }
                if !success && result == &json!({ "error": "boom" })
        ));
    }

    #[test]
    fn task_error_closes_reasoning_and_terminates_bridge() {
        let turn_id = Uuid::new_v4();
        let mut bridge = AutoagentsEventBridge::new(turn_id, TurnContext::default());
        let _ = bridge.map_event(AutoAgentsEvent::StreamChunk {
            sub_id: Uuid::new_v4(),
            chunk: AutoAgentsStreamChunk::ReasoningContent("plan".to_string()),
        });

        let errored = bridge.map_event(AutoAgentsEvent::TaskError {
            sub_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            error: "failed".to_string(),
        });

        assert!(errored.terminal);
        assert!(matches!(
            &errored.payloads[0],
            EventPayload::ReasoningSectionBreak { turn_id: id } if *id == turn_id
        ));
        assert!(matches!(
            &errored.payloads[1],
            EventPayload::Error { turn_id: Some(id), message }
                if *id == turn_id && message == "failed"
        ));
    }

    #[tokio::test]
    async fn emit_sends_event_to_broadcast_subscribers() {
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let (sender, mut receiver) = broadcast::channel(1);

        emit(
            &sender,
            session_id,
            EventPayload::ReasoningSectionBreak { turn_id },
        );

        let message = receiver.recv().await.expect("receive event");
        assert_eq!(message.session_id, session_id);
        assert!(matches!(
            message.payload,
            EventPayload::ReasoningSectionBreak { turn_id: id } if id == turn_id
        ));
    }
}
