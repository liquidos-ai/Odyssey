//! Keyboard input dispatch: routes key events to the appropriate handler.

use crate::app::{App, PendingPermission, ViewerKind};
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crate::handlers::{model, session, slash};
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
        KeyCode::Up => match kind {
            ViewerKind::Sessions => {
                if app.selected_session > 0 {
                    app.selected_session -= 1;
                }
            }
            ViewerKind::Skills => app.viewer_scroll_up(1),
            ViewerKind::Models => {
                if app.selected_model > 0 {
                    app.selected_model -= 1;
                }
            }
            ViewerKind::Themes => {
                if app.selected_theme > 0 {
                    app.selected_theme -= 1;
                }
            }
        },
        KeyCode::Down => match kind {
            ViewerKind::Sessions => {
                if app.selected_session + 1 < app.sessions.len() {
                    app.selected_session += 1;
                }
            }
            ViewerKind::Skills => app.viewer_scroll_down(1),
            ViewerKind::Models => {
                if app.selected_model + 1 < app.models.len() {
                    app.selected_model += 1;
                }
            }
            ViewerKind::Themes => {
                if app.selected_theme + 1 < AVAILABLE_THEMES.len() {
                    app.selected_theme += 1;
                }
            }
        },
        KeyCode::PageUp => app.viewer_scroll_up(5),
        KeyCode::PageDown => app.viewer_scroll_down(5),
        KeyCode::Home => app.viewer_scroll_up(u16::MAX),
        KeyCode::End => app.viewer_scroll_down(u16::MAX),
        KeyCode::Enter => match kind {
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
        },
        _ => {}
    }
    Ok(false)
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
