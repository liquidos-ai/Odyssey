//! Local runtime client for the Odyssey TUI.

use crate::event::AppEvent;
use anyhow::Result;
use log::info;
use odyssey_rs_bundle::{BundleInstallSummary, BundleMetadata, BundleStore};
use odyssey_rs_protocol::{
    AgentRef, ApprovalDecision, ExecutionRequest, Session, SessionFilter, SessionSpec,
    SessionSummary, SkillSummary, Task,
};
use odyssey_rs_runtime::{OdysseyRuntime, RunOutput, SessionCommandOutput};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

/// Local client that wraps an embedded runtime engine.
#[derive(Clone)]
pub struct AgentRuntimeClient {
    runtime: Arc<OdysseyRuntime>,
    store: BundleStore,
    bundle_ref: Arc<std::sync::RwLock<String>>,
}

impl AgentRuntimeClient {
    /// Create a new local client.
    pub fn new(runtime: Arc<OdysseyRuntime>, bundle_ref: String) -> Self {
        Self {
            store: runtime.bundle_store(),
            runtime,
            bundle_ref: Arc::new(std::sync::RwLock::new(bundle_ref)),
        }
    }

    /// Return the currently selected bundle reference.
    pub fn bundle_ref(&self) -> String {
        self.bundle_ref
            .read()
            .expect("bundle ref lock poisoned")
            .clone()
    }

    /// Validate and switch the active bundle reference.
    pub async fn select_bundle(&self, bundle_ref: String) -> Result<BundleMetadata> {
        let metadata = self.store.resolve(&bundle_ref)?.metadata;
        *self.bundle_ref.write().expect("bundle ref lock poisoned") = bundle_ref;
        Ok(metadata)
    }

    /// Build and install a bundle project, returning the installed bundle reference.
    pub async fn install_bundle(&self, path: impl AsRef<Path>) -> Result<String> {
        let path = path.as_ref();
        let install = self.store.build_and_install(path)?;
        Ok(format!(
            "{}/{}@{}",
            install.metadata.namespace, install.metadata.id, install.metadata.version
        ))
    }

    /// List available agent ids for the configured bundle.
    pub async fn list_agents(&self) -> Result<Vec<String>> {
        self.runtime
            .list_agents(self.bundle_ref())
            .map_err(Into::into)
    }

    /// List available sessions.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        Ok(self.runtime.list_sessions(Some(&SessionFilter {
            agent_ref: Some(AgentRef::from(self.bundle_ref())),
        })))
    }

    /// List installed bundles from the default bundle store.
    pub async fn list_bundles(&self) -> Result<Vec<BundleInstallSummary>> {
        self.store.list_installed().map_err(Into::into)
    }

    /// Create a session for the configured bundle.
    pub async fn create_session(&self, _agent_id: Option<String>) -> Result<Uuid> {
        Ok(self
            .runtime
            .create_session(SessionSpec::from(self.bundle_ref()))?
            .id)
    }

    /// Fetch a session by id.
    pub async fn get_session(&self, session_id: Uuid) -> Result<Session> {
        self.runtime.get_session(session_id).map_err(Into::into)
    }

    /// Submit a prompt and wait for the run result.
    pub async fn send_message(
        &self,
        session_id: Uuid,
        task: Task,
        _agent_id: Option<String>,
        _llm_id: String,
    ) -> Result<RunOutput> {
        let prompt = &task.prompt;
        if prompt.trim().is_empty() {
            anyhow::bail!("prompt cannot be empty");
        }
        let request_id = Uuid::new_v4();
        self.runtime
            .run(ExecutionRequest {
                request_id,
                session_id,
                input: task,
                turn_context: None,
            })
            .await
            .map_err(Into::into)
    }

    /// Execute a direct process invocation in the current session sandbox.
    pub async fn run_session_command(
        &self,
        session_id: Uuid,
        command_line: impl AsRef<str>,
    ) -> Result<SessionCommandOutput> {
        self.runtime
            .run_session_command(session_id, command_line)
            .await
            .map_err(Into::into)
    }

    /// Resolve a permission request.
    pub async fn resolve_permission(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
    ) -> Result<bool> {
        self.runtime
            .resolve_approval(request_id, decision)
            .map_err(Into::into)
    }

    /// List skill summaries.
    pub async fn list_skills(&self) -> Result<Vec<SkillSummary>> {
        self.runtime
            .list_skills(self.bundle_ref())
            .map_err(Into::into)
    }

    /// List configured model ids for the bundle.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        self.runtime
            .list_models(self.bundle_ref())
            .map_err(Into::into)
    }

    /// Stream events for a session.
    pub async fn stream_events(
        &self,
        session_id: Uuid,
        sender: mpsc::Sender<AppEvent>,
    ) -> Result<()> {
        let mut receiver = self.runtime.subscribe_session(session_id)?;
        info!("subscribing to runtime event stream (session_id={session_id})");
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let _ = sender.send(AppEvent::Server(event)).await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AgentRuntimeClient;
    use odyssey_rs_protocol::ApprovalDecision;
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_protocol::Task;
    use odyssey_rs_runtime::{OdysseyRuntime, RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn runtime_config(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            cache_root: root.join("cache"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: Some(SandboxMode::DangerFullAccess),
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 2,
            queue_capacity: 32,
        }
    }

    fn write_bundle_project(
        root: &Path,
        bundle_id: &str,
        agent_id: &str,
        model_name: &str,
        skill_name: &str,
        skill_description: &str,
    ) {
        fs::create_dir_all(root.join("skills").join(skill_name)).expect("create skill dir");
        fs::create_dir_all(root.join("resources").join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            format!(
                r#"{{
                    id: "{bundle_id}",
                    version: "0.1.0",
                    manifest_version: "odyssey.bundle/v1",
                    readme: "README.md",
                    agent_spec: "agent.yaml",
                    executor: {{ type: "prebuilt", id: "react" }},
                    memory: {{ type: "prebuilt", id: "sliding_window" }},
                    skills: [{{ name: "{skill_name}", path: "skills/{skill_name}" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: [], write: [] }} }},
                            network: ["*"],
                            tools: {{ mode: "default", rules: [] }}
                        }},
                        system_tools: [],
                        resources: {{}}
                    }}
                }}"#
            ),
        )
        .expect("write manifest");
        fs::write(
            root.join("agent.yaml"),
            format!(
                r#"id: {agent_id}
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: {model_name}
tools:
  allow: ["Read", "Skill"]
"#
            ),
        )
        .expect("write agent");
        fs::write(root.join("README.md"), format!("# {bundle_id}\n")).expect("write readme");
        fs::write(
            root.join("skills").join(skill_name).join("SKILL.md"),
            format!("# {skill_name}\n\n{skill_description}\n"),
        )
        .expect("write skill");
        fs::write(
            root.join("resources").join("data").join("notes.txt"),
            "hello world\n",
        )
        .expect("write resource");
    }

    #[tokio::test]
    async fn client_installs_switches_and_lists_bundle_metadata() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(OdysseyRuntime::new(runtime_config(temp.path())).expect("runtime"));

        let first_project = temp.path().join("alpha-project");
        let second_project = temp.path().join("beta-project");
        fs::create_dir_all(&first_project).expect("create first project");
        fs::create_dir_all(&second_project).expect("create second project");
        write_bundle_project(
            &first_project,
            "alpha",
            "alpha-agent",
            "gpt-4.1-mini",
            "repo-hygiene",
            "Keep commits focused.",
        );
        write_bundle_project(
            &second_project,
            "beta",
            "beta-agent",
            "gpt-4.1",
            "deploy-checks",
            "Verify release readiness.",
        );

        let client = AgentRuntimeClient::new(runtime.clone(), "local/alpha@0.1.0".to_string());
        let installed_ref = client
            .install_bundle(&first_project)
            .await
            .expect("install first bundle");
        runtime
            .build_and_install(&second_project)
            .expect("install second bundle");

        assert_eq!(installed_ref, "local/alpha@0.1.0");
        assert_eq!(client.bundle_ref(), "local/alpha@0.1.0");
        assert_eq!(
            client.list_agents().await.expect("agents"),
            vec!["alpha-agent"]
        );
        assert_eq!(
            client.list_models().await.expect("models"),
            vec!["gpt-4.1-mini"]
        );

        let skills = client.list_skills().await.expect("skills");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "repo-hygiene");
        assert_eq!(skills[0].description, "Keep commits focused.");

        let bundles = client.list_bundles().await.expect("bundles");
        assert_eq!(bundles.len(), 2);

        let selected = client
            .select_bundle("local/beta@0.1.0".to_string())
            .await
            .expect("select bundle");
        assert_eq!(selected.id, "beta");
        assert_eq!(client.bundle_ref(), "local/beta@0.1.0");
        assert_eq!(
            client.list_agents().await.expect("beta agents"),
            vec!["beta-agent"]
        );
        assert_eq!(
            client.list_models().await.expect("beta models"),
            vec!["gpt-4.1"]
        );
    }

    #[tokio::test]
    async fn client_filters_sessions_and_rejects_empty_prompts() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));

        let first_project = temp.path().join("alpha-project");
        let second_project = temp.path().join("beta-project");
        fs::create_dir_all(&first_project).expect("create first project");
        fs::create_dir_all(&second_project).expect("create second project");
        write_bundle_project(
            &first_project,
            "alpha",
            "alpha-agent",
            "gpt-4.1-mini",
            "repo-hygiene",
            "Keep commits focused.",
        );
        write_bundle_project(
            &second_project,
            "beta",
            "beta-agent",
            "gpt-4.1",
            "deploy-checks",
            "Verify release readiness.",
        );

        runtime
            .build_and_install(&first_project)
            .expect("install first bundle");
        runtime
            .build_and_install(&second_project)
            .expect("install second bundle");

        let client = AgentRuntimeClient::new(runtime.clone(), "local/alpha@0.1.0".to_string());
        let session_id = client.create_session(None).await.expect("create session");
        runtime
            .create_session("local/beta@0.1.0")
            .expect("create other session");

        let sessions = client.list_sessions().await.expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session_id);
        assert_eq!(sessions[0].agent_id, "alpha-agent");

        let session = client.get_session(session_id).await.expect("get session");
        assert_eq!(session.agent_ref, "local/alpha@0.1.0");
        assert_eq!(session.agent_id, "alpha-agent");
        assert!(session.messages.is_empty());

        let error = client
            .send_message(
                session_id,
                Task::new("   ".to_string()),
                None,
                "gpt-4.1-mini".to_string(),
            )
            .await
            .expect_err("empty prompt should fail");
        assert_eq!(error.to_string(), "prompt cannot be empty");

        assert!(
            !client
                .resolve_permission(Uuid::new_v4(), ApprovalDecision::AllowOnce)
                .await
                .expect("resolve unknown request")
        );
    }

    #[tokio::test]
    async fn client_runs_direct_session_commands() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project(
            &project,
            "alpha",
            "alpha-agent",
            "gpt-4.1-mini",
            "repo-hygiene",
            "Keep commits focused.",
        );
        runtime.build_and_install(&project).expect("install bundle");

        let client = AgentRuntimeClient::new(runtime, "local/alpha@0.1.0".to_string());
        let session_id = client.create_session(None).await.expect("create session");
        let output = client
            .run_session_command(session_id, "printf client-direct")
            .await
            .expect("run session command");

        assert_eq!(output.session_id, session_id);
        assert_eq!(output.stdout, "client-direct");
        assert_eq!(output.stderr, "");
        assert_eq!(output.status_code, Some(0));
    }
}
