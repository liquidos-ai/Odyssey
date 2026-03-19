//! Reusable library API for launching the Odyssey Ratatui client.

mod app;
pub mod cli;
mod client;
mod event;
mod handlers;
mod spawn;
mod terminal;
mod tui_config;
mod ui;

use anyhow::anyhow;
use app::App;
use client::AgentRuntimeClient;
use event::AppEvent;
use log::debug;
use odyssey_rs_bundle::{BundleInstallSummary, BundleStore};
use odyssey_rs_runtime::OdysseyRuntime;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Configuration for a reusable TUI session.
#[derive(Debug, Clone, Default)]
pub struct TuiRunConfig {
    /// Bundle reference used for all sessions in the UI.
    pub bundle_ref: String,
    /// Optional user name shown in the header.
    pub user_name: Option<String>,
    /// Optional working directory shown in the header.
    pub cwd: Option<PathBuf>,
}

pub const DEFAULT_BUNDLE_REF: &str = "odyssey@latest";

/// Launch the Odyssey TUI against a pre-configured [`RuntimeEngine`].
pub async fn run(runtime: Arc<OdysseyRuntime>, config: TuiRunConfig) -> anyhow::Result<()> {
    let TuiRunConfig {
        bundle_ref,
        user_name,
        cwd,
    } = config;
    let cwd = cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine working directory"))?;
    let client = Arc::new(AgentRuntimeClient::new(runtime.clone(), bundle_ref.clone()));
    let bundle_store = BundleStore::new(runtime.config().cache_root.clone());
    let mut app = App {
        bundle_ref,
        ..App::default()
    };

    let persisted_tui_config = tui_config::TuiConfig::load();
    app.init_theme(&persisted_tui_config.theme);
    app.tui_config = persisted_tui_config;

    if let Ok(bundles) = client.list_bundles().await {
        app.set_bundles(bundles);
    }

    app.set_user_name(user_name.unwrap_or_else(terminal::resolve_user_name));
    app.cwd = cwd.display().to_string();
    if app.bundle_ref.is_empty() {
        app.open_viewer(app::ViewerKind::Bundles);
        app.push_system_message(
            "No bundles are installed. Use /bundle install <path> to install a local bundle."
                .to_string(),
        );
        app.push_status("install a local bundle to get started");
    } else {
        let bundle = bundle_store.resolve(&app.bundle_ref)?.metadata;
        app.set_active_model(bundle.agent_spec.model.name.clone());
        app.set_active_agent(bundle.agent_spec.id.clone());

        let agents = client.list_agents().await?;
        if agents.is_empty() {
            return Err(anyhow!("no agents available in bundle {}", app.bundle_ref));
        }
        debug!("loaded agents (count={})", agents.len());
        app.set_agents(agents);

        if let Ok(sessions) = client.list_sessions().await {
            app.set_sessions(sessions);
        }
        if let Ok(skills) = client.list_skills().await {
            app.set_skills(skills);
        }
        let mut models = client.list_models().await?;
        models.sort();
        app.set_models(models);
    }

    let mut terminal = terminal::setup_terminal()?;
    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);
    spawn::spawn_input_handler(tx.clone());
    spawn::spawn_tick(tx.clone());

    let mut stream_handle: Option<JoinHandle<()>> = None;
    if !app.bundle_ref.is_empty()
        && app.active_session.is_none()
        && let Err(err) =
            handlers::session::create_session(&client, &mut app, tx.clone(), &mut stream_handle)
                .await
    {
        app.push_status(format!("failed to initialize session: {err}"));
    }

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let Some(event) = rx.recv().await else {
            break;
        };
        if handlers::handle_app_event(event, &client, &mut app, tx.clone(), &mut stream_handle)
            .await?
        {
            break;
        }
    }

    terminal::restore_terminal(&mut terminal)?;
    Ok(())
}

pub fn resolve_bundle_ref(
    runtime: &OdysseyRuntime,
    requested: Option<String>,
) -> anyhow::Result<String> {
    let bundles = BundleStore::new(runtime.config().cache_root.clone());
    if let Some(bundle_ref) = requested.filter(|bundle| !bundle.trim().is_empty()) {
        return Ok(bundle_ref);
    }
    if bundles.resolve(DEFAULT_BUNDLE_REF).is_ok() {
        return Ok(DEFAULT_BUNDLE_REF.to_string());
    }

    Ok(bundles
        .list_installed()?
        .into_iter()
        .next()
        .map(bundle_summary_ref)
        .unwrap_or_default())
}

fn bundle_summary_ref(bundle: BundleInstallSummary) -> String {
    format!("{}/{}@{}", bundle.namespace, bundle.id, bundle.version)
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BUNDLE_REF, TuiRunConfig, bundle_summary_ref, resolve_bundle_ref};
    use odyssey_rs_bundle::BundleInstallSummary;
    use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn runtime_config(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            cache_root: root.join("cache"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: None,
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 2,
            queue_capacity: 32,
        }
    }

    fn write_bundle_project(root: &Path, bundle_id: &str) {
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
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
                    skills: [{{ name: "repo-hygiene", path: "skills/repo-hygiene" }}],
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
                r#"id: {bundle_id}
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
  deny: []
"#
            ),
        )
        .expect("write agent");
        fs::write(
            root.join("skills").join("repo-hygiene").join("SKILL.md"),
            "# Repo Hygiene\n\nKeep commits focused.\n",
        )
        .expect("write skill");
        fs::write(root.join("data").join("notes.txt"), "hello world\n").expect("write resource");
    }

    #[test]
    fn resolve_bundle_ref_prefers_explicit_request() {
        let temp = tempdir().expect("tempdir");
        let runtime = RuntimeEngine::new(runtime_config(temp.path())).expect("runtime");

        assert_eq!(
            resolve_bundle_ref(&runtime, Some("local/demo@1.2.3".to_string())).expect("resolve"),
            "local/demo@1.2.3"
        );
    }

    #[test]
    fn resolve_bundle_ref_uses_default_then_first_installed_bundle() {
        let temp = tempdir().expect("tempdir");
        let runtime = RuntimeEngine::new(runtime_config(temp.path())).expect("runtime");
        assert_eq!(resolve_bundle_ref(&runtime, None).expect("empty"), "");

        let odyssey_project = temp.path().join("odyssey-project");
        fs::create_dir_all(&odyssey_project).expect("create odyssey project");
        write_bundle_project(&odyssey_project, "odyssey");
        runtime
            .build_and_install(&odyssey_project)
            .expect("install odyssey");
        assert_eq!(
            resolve_bundle_ref(&runtime, None).expect("default bundle"),
            DEFAULT_BUNDLE_REF
        );

        let temp = tempdir().expect("tempdir");
        let runtime = RuntimeEngine::new(runtime_config(temp.path())).expect("runtime");
        let alpha_project = temp.path().join("alpha-project");
        fs::create_dir_all(&alpha_project).expect("create alpha project");
        write_bundle_project(&alpha_project, "alpha");
        runtime
            .build_and_install(&alpha_project)
            .expect("install alpha");
        assert_eq!(
            resolve_bundle_ref(&runtime, None).expect("first installed"),
            "local/alpha@0.1.0"
        );
    }

    #[test]
    fn bundle_summary_ref_formats_namespace_id_and_version() {
        assert_eq!(
            bundle_summary_ref(BundleInstallSummary {
                namespace: "team".to_string(),
                id: "demo".to_string(),
                version: "1.2.3".to_string(),
                path: "/workspace/team-demo".into(),
            }),
            "team/demo@1.2.3"
        );
        assert_eq!(TuiRunConfig::default().bundle_ref, "");
    }
}
