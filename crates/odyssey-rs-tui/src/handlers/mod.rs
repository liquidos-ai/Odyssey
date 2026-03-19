//! Input and event handlers for the Odyssey TUI.

pub mod agent;
pub mod bundle;
pub mod input;
pub mod model;
pub mod session;
pub mod slash;

use crate::app::App;
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Dispatch one application event.
///
/// Returns `true` when the event loop should exit.
pub async fn handle_app_event(
    event: AppEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match event {
        AppEvent::Input(key) => input::handle_input(key, client, app, sender, stream_handle).await,
        AppEvent::Server(event) => {
            let Some(active_session) = app.active_session else {
                return Ok(false);
            };
            if event.session_id != active_session {
                return Ok(false);
            }
            app.apply_event(event);
            Ok(false)
        }
        AppEvent::StreamError(message) => {
            app.push_system_message(format!("stream error: {message}"));
            Ok(false)
        }
        AppEvent::ActionError(message) => {
            app.push_system_message(message);
            app.push_status("idle");
            Ok(false)
        }
        AppEvent::Scroll(delta) => {
            if app.viewer.is_some() {
                if delta < 0 {
                    app.viewer_scroll_up((-delta) as u16);
                } else if delta > 0 {
                    app.viewer_scroll_down(delta as u16);
                }
            } else if delta < 0 {
                app.scroll_up((-delta) as u16);
            } else if delta > 0 {
                app.scroll_down(delta as u16);
            }
            Ok(false)
        }
        AppEvent::Tick => {
            app.refresh_cpu();
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{agent, bundle, handle_app_event, model, session};
    use crate::app::{App, ViewerKind};
    use crate::client::AgentRuntimeClient;
    use crate::event::AppEvent;
    use chrono::Utc;
    use odyssey_rs_protocol::{EventMsg, EventPayload, SandboxMode};
    use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
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
        fs::create_dir_all(root.join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            format!(
                r#"{{
                    id: "{bundle_id}",
                    version: "0.1.0",
                    agent_spec: "agent.yaml",
                    executor: {{ type: "prebuilt", id: "react" }},
                    memory: {{ provider: {{ type: "prebuilt", id: "sliding_window" }} }},
                    resources: ["data"],
                    skills: [{{ name: "{skill_name}", path: "skills/{skill_name}" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    server: {{ enable_http: true }},
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: [], write: [] }} }},
                            network: [],
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
  deny: []
"#
            ),
        )
        .expect("write agent");
        fs::write(
            root.join("skills").join(skill_name).join("SKILL.md"),
            format!("# {skill_name}\n\n{skill_description}\n"),
        )
        .expect("write skill");
        fs::write(root.join("data").join("notes.txt"), "hello world\n").expect("write resource");
    }

    fn abort_stream(handle: &mut Option<JoinHandle<()>>) {
        if let Some(handle) = handle.take() {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn agent_model_and_bundle_handlers_refresh_and_validate_ids() {
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

        let client = Arc::new(AgentRuntimeClient::new(
            runtime,
            "local/alpha@0.1.0".to_string(),
        ));
        let mut app = App::new();
        app.bundle_ref = "local/alpha@0.1.0".to_string();

        bundle::refresh_bundles(&client, &mut app)
            .await
            .expect("refresh bundles");
        agent::refresh_agents(&client, &mut app)
            .await
            .expect("refresh agents");
        model::refresh_models(&client, &mut app)
            .await
            .expect("refresh models");

        assert_eq!(app.bundles.len(), 1);
        assert_eq!(app.agents, vec!["alpha-agent"]);
        assert_eq!(app.models, vec!["gpt-4.1-mini"]);

        agent::set_agent_by_id(&client, &mut app, "alpha-agent".to_string())
            .await
            .expect("set agent");
        model::set_model_by_id(&client, &mut app, "gpt-4.1-mini".to_string())
            .await
            .expect("set model");
        assert_eq!(app.active_agent.as_deref(), Some("alpha-agent"));
        assert_eq!(app.model_id, "gpt-4.1-mini");

        assert_eq!(
            agent::set_agent_by_id(&client, &mut app, "missing".to_string())
                .await
                .expect_err("missing agent"),
            "unknown agent: missing"
        );
        assert_eq!(
            model::set_model_by_id(&client, &mut app, "missing".to_string())
                .await
                .expect_err("missing model"),
            "unknown model: missing"
        );

        app.bundle_ref.clear();
        agent::refresh_agents(&client, &mut app)
            .await
            .expect("refresh empty agents");
        model::refresh_models(&client, &mut app)
            .await
            .expect("refresh empty models");
        assert!(app.agents.is_empty());
        assert!(app.models.is_empty());
    }

    #[tokio::test]
    async fn bundle_and_session_handlers_install_switch_and_join() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let alpha_project = temp.path().join("alpha-project");
        let beta_project = temp.path().join("beta-project");
        fs::create_dir_all(&alpha_project).expect("create alpha project");
        fs::create_dir_all(&beta_project).expect("create beta project");
        write_bundle_project(
            &alpha_project,
            "alpha",
            "alpha-agent",
            "gpt-4.1-mini",
            "repo-hygiene",
            "Keep commits focused.",
        );
        write_bundle_project(
            &beta_project,
            "beta",
            "beta-agent",
            "gpt-4.1",
            "deploy-checks",
            "Verify release readiness.",
        );

        let client = Arc::new(AgentRuntimeClient::new(
            runtime.clone(),
            "local/alpha@0.1.0".to_string(),
        ));
        let mut app = App::new();
        app.cwd = temp.path().display().to_string();
        let (sender, _receiver) = mpsc::channel(16);
        let mut stream_handle = None;

        bundle::install_bundle(
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
            "alpha-project".to_string(),
        )
        .await
        .expect("install bundle");
        assert_eq!(app.bundle_ref, "local/alpha@0.1.0");
        assert_eq!(app.active_agent.as_deref(), Some("alpha-agent"));
        assert!(app.active_session.is_some());
        assert_eq!(app.status, "bundle set: local/alpha@0.1.0");
        assert_eq!(app.skills.len(), 1);
        abort_stream(&mut stream_handle);

        runtime
            .build_and_install(&beta_project)
            .expect("install beta bundle");
        bundle::refresh_bundles(&client, &mut app)
            .await
            .expect("refresh bundles");
        app.selected_bundle = app
            .bundles
            .iter()
            .position(|bundle| bundle.id == "beta")
            .expect("beta bundle index");
        bundle::activate_selected_bundle(&client, &mut app, sender.clone(), &mut stream_handle)
            .await
            .expect("activate selected bundle");
        assert_eq!(app.bundle_ref, "local/beta@0.1.0");
        assert_eq!(app.active_agent.as_deref(), Some("beta-agent"));
        assert!(app.active_session.is_some());
        assert_eq!(app.status, "bundle set: local/beta@0.1.0");

        let joined_session_id = runtime
            .create_session("local/beta@0.1.0")
            .expect("create joined session")
            .id;
        session::join_session(
            &client,
            &mut app,
            joined_session_id,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("join session");
        assert_eq!(app.active_session, Some(joined_session_id));
        assert_eq!(app.status, "session joined");

        session::refresh_sessions(&client, &mut app)
            .await
            .expect("refresh sessions");
        app.selected_session = app
            .sessions
            .iter()
            .position(|session| session.id == joined_session_id)
            .expect("joined session index");
        session::activate_selected_session(&client, &mut app, sender.clone(), &mut stream_handle)
            .await
            .expect("activate session");
        assert_eq!(app.active_session, Some(joined_session_id));
        assert_eq!(app.status, "session selected");

        let mut no_session_app = App::new();
        no_session_app.input = "hello".to_string();
        session::send_message(&client, &mut no_session_app, sender)
            .await
            .expect("send message without session");
        assert_eq!(no_session_app.status, "no active session");
        abort_stream(&mut stream_handle);
    }

    #[tokio::test]
    async fn handle_app_event_updates_matching_sessions_and_ui_state() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let client = Arc::new(AgentRuntimeClient::new(runtime, String::new()));
        let (sender, _receiver) = mpsc::channel(8);
        let mut stream_handle = None;
        let mut app = App::new();
        let active_session = Uuid::new_v4();
        app.active_session = Some(active_session);
        app.chat_max_scroll = 10;
        app.viewer = Some(ViewerKind::Sessions);
        app.viewer_max_scroll = 10;

        let matching = EventMsg {
            id: Uuid::new_v4(),
            session_id: active_session,
            created_at: Utc::now(),
            payload: EventPayload::Error {
                turn_id: None,
                message: "boom".to_string(),
            },
        };
        assert!(
            !handle_app_event(
                AppEvent::Server(matching),
                &client,
                &mut app,
                sender.clone(),
                &mut stream_handle,
            )
            .await
            .expect("handle matching server event")
        );
        assert_eq!(app.status, "idle");
        assert_eq!(
            app.messages.last().expect("error message").content,
            "error: boom"
        );

        let before = app.messages.len();
        let non_matching = EventMsg {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            created_at: Utc::now(),
            payload: EventPayload::Error {
                turn_id: None,
                message: "ignored".to_string(),
            },
        };
        handle_app_event(
            AppEvent::Server(non_matching),
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("handle non matching server event");
        assert_eq!(app.messages.len(), before);

        handle_app_event(
            AppEvent::StreamError("stream lost".to_string()),
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("handle stream error");
        assert_eq!(
            app.messages.last().expect("stream error").content,
            "stream error: stream lost"
        );

        handle_app_event(
            AppEvent::ActionError("action failed".to_string()),
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("handle action error");
        assert_eq!(app.status, "idle");
        assert_eq!(
            app.messages.last().expect("action error").content,
            "action failed"
        );

        handle_app_event(
            AppEvent::Scroll(3),
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("handle viewer scroll");
        assert_eq!(app.viewer_scroll, 3);

        app.viewer = None;
        handle_app_event(
            AppEvent::Scroll(2),
            &client,
            &mut app,
            sender.clone(),
            &mut stream_handle,
        )
        .await
        .expect("handle chat scroll");
        assert_eq!(app.scroll, 10);

        handle_app_event(
            AppEvent::Tick,
            &client,
            &mut app,
            sender,
            &mut stream_handle,
        )
        .await
        .expect("handle tick");
    }
}
