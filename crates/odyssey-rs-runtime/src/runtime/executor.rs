use chrono::Utc;
use odyssey_rs_protocol::{AgentRef, EventMsg, ExecutionRequest, ModelSpec, Task, TurnContext};
use odyssey_rs_sandbox::SandboxMode;
use odyssey_rs_tools::{ToolContext, tools_to_adaptors};
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    RunOutput, RuntimeError,
    agent::{ExecutorRun, run_executor},
    memory::build_memory,
    resolver::{agent::ResolvedAgentSpec, agent::resolve_agent, llm::LLMResolver},
    runtime::{
        engine::OdysseyRuntimeInner,
        history::TurnHistoryCollector,
        prompt::build_system_prompt,
        tool_event::{RuntimeApprovalHandler, RuntimeToolEventSink},
    },
    sandbox::{build_permission_rules, build_sandbox_runtime, prepare_cell},
    session::{SessionRecord, TurnChatMessageRecord, TurnRecord},
    skill::BundleSkillStore,
    tool::select_tools,
};

pub(crate) struct ScheduleExecutor {
    runtime: Arc<OdysseyRuntimeInner>,
}

impl ScheduleExecutor {
    pub fn new(runtime: Arc<OdysseyRuntimeInner>) -> Self {
        Self { runtime }
    }
    pub(crate) async fn execute_request(
        &self,
        turn_id: Uuid,
        request: ExecutionRequest,
    ) -> Result<RunOutput, RuntimeError> {
        let session = self.runtime.sessions.get(request.session_id)?;
        let resolved = resolve_agent(
            &self.runtime.store,
            &AgentRef::from(session.agent_ref.clone()),
        )?;
        let sender = self.runtime.sessions.sender(request.session_id)?;
        let receiver = self.runtime.sessions.subscribe(request.session_id)?;
        let response = self
            .execute_resolved_agent(
                resolved,
                session.clone(),
                turn_id,
                request.input.clone(),
                request.turn_context.clone(),
                sender.clone(),
            )
            .await?;
        let chat_history = collect_turn_chat_history(turn_id, &request.input, &response, receiver);
        self.runtime.sessions.append_turn(
            request.session_id,
            TurnRecord::from_history(
                turn_id,
                &request.input,
                response.clone(),
                chat_history,
                Utc::now(),
            ),
        )?;
        Ok(RunOutput {
            session_id: request.session_id,
            turn_id,
            response,
        })
    }

    async fn execute_resolved_agent(
        &self,
        resolved: ResolvedAgentSpec,
        session: SessionRecord,
        turn_id: Uuid,
        task: Task,
        turn_context_override: Option<odyssey_rs_protocol::TurnContextOverride>,
        sender: broadcast::Sender<EventMsg>,
    ) -> Result<String, RuntimeError> {
        let session_id = session.id;
        let mode_override = self
            .runtime
            .config
            .sandbox_mode_override
            .or(turn_context_override
                .as_ref()
                .and_then(|ctx| ctx.sandbox_mode));
        let mode = effective_sandbox_mode(&resolved.manifest, mode_override);
        let sandbox_runtime = if mode == SandboxMode::DangerFullAccess {
            &self.runtime.host_sandbox
        } else {
            &Arc::new(build_sandbox_runtime(
                &self.runtime.config,
                SandboxMode::WorkspaceWrite,
            )?)
        };
        let cell = prepare_cell(
            sandbox_runtime,
            session_id,
            &resolved.agent.id,
            &resolved.install_path,
            &resolved.manifest,
            mode_override,
        )
        .await?;
        let permissions = build_permission_rules(&resolved.manifest);
        let event_sink = Arc::new(RuntimeToolEventSink {
            session_id,
            turn_id,
            sender: sender.clone(),
            working_dir: cell.work_dir.display().to_string(),
        });
        let approval_handler = Arc::new(RuntimeApprovalHandler {
            session_id,
            turn_id,
            sender: sender.clone(),
            approvals: self.runtime.approvals.clone(),
        });
        let skills = Arc::new(BundleSkillStore::load(&cell.root)?);
        let system_prompt = build_system_prompt(
            &resolved.agent.prompt,
            &skills,
            !resolved.manifest.skills.is_empty(),
        );
        let ctx = ToolContext {
            session_id,
            turn_id,
            bundle_root: cell.root.clone(),
            working_dir: cell.work_dir.clone(),
            sandbox: cell.sandbox,
            permission_rules: permissions,
            event_sink: Some(event_sink),
            approval_handler: Some(approval_handler),
            skills: Some(skills),
        };
        let selected = select_tools(&self.runtime.tools, &resolved.manifest, &resolved.agent);
        let adapted = tools_to_adaptors(selected, ctx.clone());
        let model = turn_context_override
            .as_ref()
            .and_then(|ctx| ctx.model.clone())
            .unwrap_or_else(|| ModelSpec {
                provider: session.model_provider.clone(),
                name: session.model_id.clone(),
                config: None, //TODO
            });
        let llm_resolver = LLMResolver::new(&model);
        let llm = llm_resolver.build_llm()?;
        let memory = build_memory(&resolved.manifest, &session.turns)?;
        let turn_context = build_turn_context(
            cell.work_dir.display().to_string(),
            model.clone(),
            mode,
            turn_context_override.as_ref(),
        );
        run_executor(ExecutorRun {
            executor_id: resolved.manifest.executor.id.clone(),
            llm,
            system_prompt,
            task,
            memory,
            tools: adapted,
            session_id,
            turn_id,
            sender,
            turn_context,
        })
        .await
    }
}

fn collect_turn_chat_history(
    turn_id: Uuid,
    task: &Task,
    response: &str,
    mut receiver: broadcast::Receiver<EventMsg>,
) -> Vec<TurnChatMessageRecord> {
    let mut collector = TurnHistoryCollector::new(turn_id, task);
    while let Ok(event) = receiver.try_recv() {
        collector.observe(event);
    }
    collector.finish(response)
}

fn effective_sandbox_mode(
    manifest: &odyssey_rs_manifest::BundleManifest,
    override_mode: Option<SandboxMode>,
) -> SandboxMode {
    override_mode.unwrap_or(manifest.sandbox.mode)
}

fn build_turn_context(
    cwd: String,
    model: ModelSpec,
    sandbox_mode: SandboxMode,
    override_ctx: Option<&odyssey_rs_protocol::TurnContextOverride>,
) -> TurnContext {
    let mut context = TurnContext {
        cwd: Some(cwd),
        model: Some(model),
        sandbox_mode: Some(sandbox_mode),
        approval_policy: None,
        metadata: serde_json::json!({}),
    };
    if let Some(override_ctx) = override_ctx {
        context.apply_override(override_ctx);
    }
    context
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use odyssey_rs_manifest::{
        BundleExecutor, BundleManifest, BundleMemory, BundleSandbox, BundleServer, BundleTool,
    };
    use odyssey_rs_protocol::{EventMsg, EventPayload};
    use odyssey_rs_protocol::{ModelSpec, Task, TurnContextOverride};
    use odyssey_rs_sandbox::SandboxMode;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tokio::sync::broadcast;
    use uuid::Uuid;

    use crate::runtime::executor::{
        build_turn_context, collect_turn_chat_history, effective_sandbox_mode,
    };

    fn manifest(mode: SandboxMode) -> BundleManifest {
        BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: "prebuilt".to_string(),
                id: "react".to_string(),
                config: json!({}),
            },
            memory: BundleMemory::default(),
            resources: Vec::new(),
            skills: Vec::new(),
            tools: vec![BundleTool {
                name: "Read".to_string(),
                source: "builtin".to_string(),
            }],
            server: BundleServer::default(),
            sandbox: BundleSandbox {
                mode,
                ..BundleSandbox::default()
            },
        }
    }

    #[test]
    fn collect_turn_chat_history_preserves_tool_use_and_result_ids() {
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let tool_call_id = Uuid::new_v4();
        let (sender, receiver) = broadcast::channel(32);

        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallStarted {
                turn_id,
                tool_call_id,
                tool_name: "Write".to_string(),
                arguments: json!({ "path": "helloworld.py" }),
            },
        });
        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallFinished {
                turn_id,
                tool_call_id,
                result: json!({ "error": "permission denied" }),
                success: false,
            },
        });
        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::TurnCompleted {
                turn_id,
                message: "The write failed.".to_string(),
            },
        });

        let history = collect_turn_chat_history(
            turn_id,
            &Task::new("create file"),
            "The write failed.",
            receiver,
        );

        assert_eq!(history.len(), 4);
        assert_eq!(history[0].content, "create file");
        assert_eq!(history[1].tool_calls[0].id, tool_call_id.to_string());
        assert_eq!(history[2].tool_calls[0].id, tool_call_id.to_string());
        assert_eq!(history[3].content, "The write failed.");
    }

    #[test]
    fn effective_sandbox_mode_prefers_override() {
        assert_eq!(
            effective_sandbox_mode(
                &manifest(SandboxMode::WorkspaceWrite),
                Some(SandboxMode::DangerFullAccess)
            ),
            SandboxMode::DangerFullAccess
        );
        assert_eq!(
            effective_sandbox_mode(&manifest(SandboxMode::WorkspaceWrite), None),
            SandboxMode::WorkspaceWrite
        );
    }

    #[test]
    fn build_turn_context_applies_overrides() {
        let context = build_turn_context(
            "/tmp/demo".to_string(),
            ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
                config: None,
            },
            SandboxMode::WorkspaceWrite,
            Some(&TurnContextOverride {
                metadata: json!({ "trace": "abc" }),
                ..TurnContextOverride::default()
            }),
        );

        assert_eq!(context.cwd.as_deref(), Some("/tmp/demo"));
        assert_eq!(
            context.metadata,
            json!({
                "trace": "abc"
            })
        );
    }
}
