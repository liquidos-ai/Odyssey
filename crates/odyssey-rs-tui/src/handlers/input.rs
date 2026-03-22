//! Keyboard input dispatch: routes key events to the appropriate handler.

use crate::app::{App, PendingPermission, ViewerKind};
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crate::handlers::{agent, bundle, model, session, slash};
use crate::ui::theme::AVAILABLE_THEMES;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::info;
use odyssey_rs_protocol::ApprovalDecision;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Handle a keyboard event.
///
/// Returns `true` when the application should exit.
pub async fn handle_input(
    key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    // Global quit / close shortcuts
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }
    if key.code == KeyCode::Esc {
        return handle_esc(app);
    }

    // Permission queue takes priority over everything else
    if let Some(permission) = app.pending_permissions.front().cloned()
        && matches!(
            key.code,
            KeyCode::Char('y') | KeyCode::Char('a') | KeyCode::Char('n')
        )
    {
        return handle_permission_input(key, client, app, permission).await;
    }

    // Viewer navigation
    if let Some(kind) = app.viewer {
        return handle_viewer_input(key, kind, client, app, sender, stream_handle).await;
    }

    // Normal input mode
    handle_normal_input(key, client, app, sender, stream_handle).await
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn handle_esc(app: &mut App) -> anyhow::Result<bool> {
    if app.viewer.is_some() {
        app.close_viewer();
        return Ok(false);
    }
    if app.show_slash_commands {
        app.show_slash_commands = false;
        app.input.clear();
        return Ok(false);
    }
    Ok(true)
}

async fn handle_viewer_input(
    key: KeyEvent,
    kind: ViewerKind,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Up => handle_viewer_up(app, kind),
        KeyCode::Down => handle_viewer_down(app, kind),
        KeyCode::PageUp => app.viewer_scroll_up(5),
        KeyCode::PageDown => app.viewer_scroll_down(5),
        KeyCode::Home => app.viewer_scroll_up(u16::MAX),
        KeyCode::End => app.viewer_scroll_down(u16::MAX),
        KeyCode::Enter => {
            handle_viewer_enter(kind, client, app, sender, stream_handle).await?;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_viewer_up(app: &mut App, kind: ViewerKind) {
    match kind {
        ViewerKind::Agents => decrement_selection(&mut app.selected_agent),
        ViewerKind::Bundles => decrement_selection(&mut app.selected_bundle),
        ViewerKind::Sessions => decrement_selection(&mut app.selected_session),
        ViewerKind::Skills => app.viewer_scroll_up(1),
        ViewerKind::Models => decrement_selection(&mut app.selected_model),
        ViewerKind::Themes => decrement_selection(&mut app.selected_theme),
    }
}

fn handle_viewer_down(app: &mut App, kind: ViewerKind) {
    match kind {
        ViewerKind::Agents => increment_selection(&mut app.selected_agent, app.agents.len()),
        ViewerKind::Bundles => increment_selection(&mut app.selected_bundle, app.bundles.len()),
        ViewerKind::Sessions => increment_selection(&mut app.selected_session, app.sessions.len()),
        ViewerKind::Skills => app.viewer_scroll_down(1),
        ViewerKind::Models => increment_selection(&mut app.selected_model, app.models.len()),
        ViewerKind::Themes => increment_selection(&mut app.selected_theme, AVAILABLE_THEMES.len()),
    }
}

async fn handle_viewer_enter(
    kind: ViewerKind,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    match kind {
        ViewerKind::Agents => {
            agent::activate_selected_agent(app)?;
            app.close_viewer();
        }
        ViewerKind::Bundles => {
            bundle::activate_selected_bundle(client, app, sender, stream_handle)
                .await
                .map_err(anyhow::Error::msg)?;
            app.close_viewer();
        }
        ViewerKind::Sessions => {
            session::activate_selected_session(client, app, sender, stream_handle).await?;
            app.close_viewer();
        }
        ViewerKind::Models => {
            model::activate_selected_model(app)?;
            app.close_viewer();
        }
        ViewerKind::Themes => {
            app.apply_theme_at(app.selected_theme);
            let name = app.theme.name;
            app.push_status(format!("theme set: {name}"));
            app.close_viewer();
        }
        ViewerKind::Skills => {}
    }
    Ok(())
}

fn decrement_selection(selected: &mut usize) {
    if *selected > 0 {
        *selected -= 1;
    }
}

fn increment_selection(selected: &mut usize, len: usize) {
    if *selected + 1 < len {
        *selected += 1;
    }
}

async fn handle_normal_input(
    key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    // When the palette is open, intercept navigation / selection keys first.
    if app.show_slash_commands
        && handle_palette_key(key, client, app, sender.clone(), stream_handle).await?
    {
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::create_session(client, app, sender, stream_handle).await?;
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::refresh_sessions(client, app).await?;
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::activate_selected_session(client, app, sender, stream_handle).await?;
        }
        KeyCode::PageUp => app.scroll_up(5),
        KeyCode::PageDown => app.scroll_down(5),
        KeyCode::Up => app.scroll_up(1),
        KeyCode::Down => app.scroll_down(1),
        KeyCode::Home => app.scroll_to_top(),
        KeyCode::End => app.enable_auto_scroll(),
        KeyCode::Enter => handle_enter(key, client, app, sender, stream_handle).await?,
        KeyCode::Backspace => {
            app.input.pop();
            app.show_slash_commands = app.input.trim_start().starts_with('/');
            app.slash_selected = 0;
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.push(ch);
            app.show_slash_commands = app.input.trim_start().starts_with('/');
            app.slash_selected = 0;
        }
        _ => {}
    }
    Ok(false)
}

/// Handle a key press while the slash palette is visible.
///
/// Returns `true` when the key was consumed (caller should skip normal handling).
async fn handle_palette_key(
    key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    let filtered = slash::filtered_commands(&app.input);
    let count = filtered.len();

    match key.code {
        KeyCode::Up => {
            if app.slash_selected > 0 {
                app.slash_selected -= 1;
            }
            Ok(true)
        }
        KeyCode::Down => {
            if count > 0 && app.slash_selected + 1 < count {
                app.slash_selected += 1;
            }
            Ok(true)
        }
        KeyCode::Tab | KeyCode::Enter if count > 0 => {
            let entry = filtered[app.slash_selected.min(count - 1)];
            app.show_slash_commands = false;
            app.slash_selected = 0;

            if entry.args.is_empty() {
                // No arguments needed — execute the command immediately.
                let command = format!("/{}", entry.trigger);
                app.input.clear();
                if let Err(err) =
                    slash::handle_slash_command(client, app, sender, stream_handle, command).await
                {
                    app.push_system_message(err);
                }
            } else {
                // Command needs arguments — complete the trigger and let the
                // user type the rest.
                app.input = format!("/{} ", entry.trigger);
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn handle_enter(
    _key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    if app.input.trim().is_empty() {
        app.show_slash_commands = false;
        return Ok(());
    }
    app.show_slash_commands = false;
    if app.input.trim_start().starts_with('/') {
        let command = std::mem::take(&mut app.input);
        if let Err(err) =
            slash::handle_slash_command(client, app, sender, stream_handle, command).await
        {
            app.push_system_message(err);
        }
    } else if app.input.trim_start().starts_with('!') {
        session::send_command(client, app, sender).await?;
    } else {
        session::send_message(client, app, sender).await?;
    }
    Ok(())
}

/// Handle keyboard input for a pending permission prompt.
pub async fn handle_permission_input(
    key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    permission: PendingPermission,
) -> anyhow::Result<bool> {
    let decision = match key.code {
        KeyCode::Char('y') => Some(ApprovalDecision::AllowOnce),
        KeyCode::Char('a') => Some(ApprovalDecision::AllowAlways),
        KeyCode::Char('n') => Some(ApprovalDecision::Deny),
        KeyCode::Esc => {
            app.pending_permissions.pop_front();
            return Ok(false);
        }
        _ => None,
    };
    if let Some(decision) = decision {
        info!(
            "sending permission decision (request_id={}, decision={:?})",
            permission.request_id, decision
        );
        let resolved = client
            .resolve_permission(permission.request_id, decision)
            .await
            .unwrap_or(false);
        app.pending_permissions.pop_front();
        app.push_status(if resolved {
            "permission sent"
        } else {
            "permission request not found"
        });
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::handle_input;
    use crate::app::App;
    use crate::client::AgentRuntimeClient;
    use crate::event::AppEvent;
    use crate::handlers;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

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

    fn write_bundle_project(root: &Path, bundle_id: &str, agent_id: &str) {
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
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
                    skills: [{{ name: "repo-hygiene", path: "skills/repo-hygiene" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: [], write: [] }} }},
                            network: ["*"]
                        }},
                        system_tools: ["sh"],
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
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
"#
            ),
        )
        .expect("write agent");
        fs::write(root.join("README.md"), format!("# {bundle_id}\n")).expect("write readme");
        fs::write(
            root.join("skills").join("repo-hygiene").join("SKILL.md"),
            "Keep commits focused.\n",
        )
        .expect("write skill");
        fs::write(
            root.join("resources").join("data").join("notes.txt"),
            "hello world\n",
        )
        .expect("write resource");
    }

    #[tokio::test]
    async fn enter_routes_bang_prefix_to_session_command_execution() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project(&project, "alpha", "alpha-agent");
        runtime.build_and_install(&project).expect("install bundle");

        let client = Arc::new(AgentRuntimeClient::new(
            runtime,
            "local/alpha@0.1.0".to_string(),
        ));
        let mut app = App {
            bundle_ref: "local/alpha@0.1.0".to_string(),
            input: "!printf input-route".to_string(),
            ..App::default()
        };
        let (sender, mut receiver) = mpsc::channel::<AppEvent>(32);
        let mut stream_handle = None;

        handlers::session::create_session(&client, &mut app, sender.clone(), &mut stream_handle)
            .await
            .expect("create session");

        let exit = handle_input(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &client,
            &mut app,
            sender,
            &mut stream_handle,
        )
        .await
        .expect("handle enter");

        assert!(!exit);
        assert_eq!(app.input, "");
        assert_eq!(app.status, "running");
        assert!(app.messages.is_empty());

        let mut saw_begin = false;
        let mut saw_end = false;
        for _ in 0..4 {
            let Some(event) = receiver.recv().await else {
                break;
            };
            if let AppEvent::Server(message) = event {
                match message.payload {
                    odyssey_rs_protocol::EventPayload::ExecCommandBegin { .. } => {
                        saw_begin = true;
                    }
                    odyssey_rs_protocol::EventPayload::ExecCommandEnd { .. } => {
                        saw_end = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(saw_begin);
        assert!(saw_end);
        if let Some(handle) = stream_handle.take() {
            handle.abort();
        }
    }
}
