//! Keyboard input dispatch: routes key events to the appropriate handler.

use crate::app::{App, PendingPermission, ViewerKind};
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crate::handlers::{agent, bundle, model, session, slash};
use crate::ui::theme::AVAILABLE_THEMES;
use crate::ui::widgets::input::PROMPT;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::info;
use odyssey_rs_protocol::ApprovalDecision;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unicode_width::UnicodeWidthStr;

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
        app.input_cursor = 0;
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

    if handle_normal_shortcut_key(key, client, app, &sender, stream_handle).await? {
        return Ok(false);
    }

    if handle_navigation_key(key, app) {
        return Ok(false);
    }

    if key.code == KeyCode::Enter {
        handle_enter(key, client, app, sender, stream_handle).await?;
        return Ok(false);
    }

    if handle_edit_key(key, app) {
        return Ok(false);
    }

    Ok(false)
}

async fn handle_normal_shortcut_key(
    key: KeyEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: &mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::create_session(client, app, sender.clone(), stream_handle).await?;
            Ok(true)
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::refresh_sessions(client, app).await?;
            Ok(true)
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            session::activate_selected_session(client, app, sender.clone(), stream_handle).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_navigation_key(key: KeyEvent, app: &mut App) -> bool {
    match key.code {
        KeyCode::Tab => {
            app.open_viewer(ViewerKind::Agents);
            true
        }
        KeyCode::PageUp => {
            app.scroll_up(5);
            true
        }
        KeyCode::PageDown => {
            app.scroll_down(5);
            true
        }
        KeyCode::Left => {
            handle_left_key(app);
            true
        }
        KeyCode::Right => {
            handle_right_key(app);
            true
        }
        KeyCode::Up => {
            handle_up_key(app);
            true
        }
        KeyCode::Down => {
            handle_down_key(app);
            true
        }
        KeyCode::Home => {
            handle_home_key(app);
            true
        }
        KeyCode::End => {
            handle_end_key(app);
            true
        }
        _ => false,
    }
}

fn handle_left_key(app: &mut App) {
    if !app.input.is_empty() {
        move_cursor_left(app);
    }
}

fn handle_right_key(app: &mut App) {
    if !app.input.is_empty() {
        move_cursor_right(app);
    }
}

fn handle_up_key(app: &mut App) {
    if app.input.is_empty() || (app.history_index.is_some() && cursor_on_first_line(app)) {
        history_up(app);
    } else {
        move_cursor_up(app);
    }
}

fn handle_down_key(app: &mut App) {
    if app.history_index.is_some() && cursor_on_last_line(app) {
        history_down(app);
    } else if !app.input.is_empty() {
        move_cursor_down(app);
    } else {
        app.scroll_down(1);
    }
}

fn handle_home_key(app: &mut App) {
    if !app.input.is_empty() {
        app.input_cursor = 0;
    } else {
        app.scroll_to_top();
    }
}

fn handle_end_key(app: &mut App) {
    if !app.input.is_empty() {
        app.input_cursor = app.input.len();
    } else {
        app.enable_auto_scroll();
    }
}

fn handle_edit_key(key: KeyEvent, app: &mut App) -> bool {
    match key.code {
        KeyCode::Backspace => {
            handle_backspace(app);
            true
        }
        KeyCode::Delete => {
            handle_delete(app);
            true
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_input_char(app, ch);
            true
        }
        _ => false,
    }
}

fn handle_backspace(app: &mut App) {
    if app.input_cursor > 0 {
        let prev = prev_char_boundary(&app.input, app.input_cursor);
        app.input.drain(prev..app.input_cursor);
        app.input_cursor = prev;
        refresh_input_edit_state(app, true);
    }
}

fn handle_delete(app: &mut App) {
    if app.input_cursor < app.input.len() {
        let next = next_char_boundary(&app.input, app.input_cursor);
        app.input.drain(app.input_cursor..next);
        refresh_input_edit_state(app, false);
    }
}

fn insert_input_char(app: &mut App, ch: char) {
    app.input.insert(app.input_cursor, ch);
    app.input_cursor += ch.len_utf8();
    refresh_input_edit_state(app, true);
}

fn refresh_input_edit_state(app: &mut App, clear_history_index: bool) {
    app.show_slash_commands = app.input.trim_start().starts_with('/');
    app.slash_selected = 0;
    if clear_history_index {
        app.history_index = None;
    }
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
                app.input_cursor = 0;
                if let Err(err) =
                    slash::handle_slash_command(client, app, sender, stream_handle, command).await
                {
                    app.push_system_message(err);
                }
            } else {
                // Command needs arguments — complete the trigger and let the
                // user type the rest.
                app.input = format!("/{} ", entry.trigger);
                app.input_cursor = app.input.len();
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

// ── History helpers ──────────────────────────────────────────────────────────

/// Whether the cursor is on the first visual line of the input.
fn cursor_on_first_line(app: &App) -> bool {
    let width = app.input_inner_width.max(1) as usize;
    let prompt_w = UnicodeWidthStr::width(PROMPT);
    let (line, _) = visual_pos(&app.input, app.input_cursor, width, prompt_w);
    line == 0
}

/// Whether the cursor is on the last visual line of the input.
fn cursor_on_last_line(app: &App) -> bool {
    let width = app.input_inner_width.max(1) as usize;
    let prompt_w = UnicodeWidthStr::width(PROMPT);
    let (line, _) = visual_pos(&app.input, app.input_cursor, width, prompt_w);
    let total = crate::ui::input_line_count(&app.input, width) as usize;
    line + 1 >= total
}

fn history_up(app: &mut App) {
    if app.history.is_empty() {
        return;
    }
    let new_idx = match app.history_index {
        Some(idx) if idx > 0 => idx - 1,
        Some(0) => return, // already at oldest
        None => {
            // Start browsing: save current input as draft.
            app.history_draft.clone_from(&app.input);
            app.history.len() - 1
        }
        _ => return,
    };
    app.history_index = Some(new_idx);
    app.input.clone_from(&app.history[new_idx]);
    app.input_cursor = app.input.len();
}

fn history_down(app: &mut App) {
    let Some(idx) = app.history_index else {
        return;
    };
    if idx + 1 < app.history.len() {
        let new_idx = idx + 1;
        app.history_index = Some(new_idx);
        app.input.clone_from(&app.history[new_idx]);
        app.input_cursor = app.input.len();
    } else {
        // Past the newest entry — restore draft.
        app.history_index = None;
        app.input = std::mem::take(&mut app.history_draft);
        app.input_cursor = app.input.len();
    }
}

// ── Cursor movement helpers ──────────────────────────────────────────────────

/// Return the byte index of the previous character boundary before `pos`.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos.saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Return the byte index of the next character boundary after `pos`.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

fn move_cursor_left(app: &mut App) {
    if app.input_cursor > 0 {
        app.input_cursor = prev_char_boundary(&app.input, app.input_cursor);
    }
}

fn move_cursor_right(app: &mut App) {
    if app.input_cursor < app.input.len() {
        app.input_cursor = next_char_boundary(&app.input, app.input_cursor);
    }
}

/// Move cursor up one visual line, keeping the same visual column.
fn move_cursor_up(app: &mut App) {
    let width = app.input_inner_width.max(1) as usize;
    let prompt_w = UnicodeWidthStr::width(PROMPT);

    // Compute the visual column and line of the current cursor.
    let (cursor_line, cursor_col) = visual_pos(&app.input, app.input_cursor, width, prompt_w);

    if cursor_line == 0 {
        // Already on the first line — move cursor to start.
        app.input_cursor = 0;
        return;
    }

    // Find the byte offset that corresponds to (cursor_line - 1, cursor_col).
    app.input_cursor =
        byte_offset_for_visual(&app.input, cursor_line - 1, cursor_col, width, prompt_w);
}

/// Move cursor down one visual line, keeping the same visual column.
fn move_cursor_down(app: &mut App) {
    let width = app.input_inner_width.max(1) as usize;
    let prompt_w = UnicodeWidthStr::width(PROMPT);

    let (cursor_line, cursor_col) = visual_pos(&app.input, app.input_cursor, width, prompt_w);

    // Total number of visual lines.
    let total_lines = crate::ui::input_line_count(&app.input, width) as usize;
    if cursor_line + 1 >= total_lines {
        // Already on the last line — move cursor to end.
        app.input_cursor = app.input.len();
        return;
    }

    app.input_cursor =
        byte_offset_for_visual(&app.input, cursor_line + 1, cursor_col, width, prompt_w);
}

/// Compute (visual_line, visual_col) for a given byte offset in the input.
fn visual_pos(input: &str, byte_offset: usize, width: usize, prompt_w: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = prompt_w;

    for (i, ch) in input.char_indices() {
        if i >= byte_offset {
            break;
        }
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if col + ch_w > width && col > 0 {
            line += 1;
            col = 0;
        }
        col += ch_w;
    }
    (line, col)
}

/// Find the byte offset in `input` that corresponds to (target_line, target_col).
/// Clamps to the end of the target line if `target_col` is beyond the line length.
fn byte_offset_for_visual(
    input: &str,
    target_line: usize,
    target_col: usize,
    width: usize,
    prompt_w: usize,
) -> usize {
    let mut line = 0usize;
    let mut col = prompt_w;
    let mut last_byte = 0usize;

    for (i, ch) in input.char_indices() {
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if col + ch_w > width && col > 0 {
            if line == target_line {
                // We were on the target line but hit the end before reaching target_col.
                return i;
            }
            line += 1;
            col = 0;
        }
        if line == target_line && col >= target_col {
            return i;
        }
        col += ch_w;
        last_byte = i + ch.len_utf8();
    }
    // Past all characters — return end.
    last_byte
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
    // Save to history and reset browsing state.
    crate::history::push(&app.input);
    app.history.push(app.input.clone());
    app.history_index = None;
    app.history_draft.clear();

    app.show_slash_commands = false;
    if app.input.trim_start().starts_with('/') {
        let command = std::mem::take(&mut app.input);
        app.input_cursor = 0;
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
    use super::{handle_input, history_down, history_up};
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

    #[test]
    fn history_up_saves_draft_and_loads_latest_entry() {
        let mut app = App {
            input: "draft".to_string(),
            history: vec!["oldest".to_string(), "latest".to_string()],
            ..App::default()
        };

        history_up(&mut app);

        assert_eq!(app.history_index, Some(1));
        assert_eq!(app.history_draft, "draft");
        assert_eq!(app.input, "latest");
        assert_eq!(app.input_cursor, app.input.len());
    }

    #[test]
    fn history_down_restores_saved_draft_after_newest_entry() {
        let mut app = App {
            input: "latest".to_string(),
            history: vec!["oldest".to_string(), "latest".to_string()],
            history_index: Some(1),
            history_draft: "draft".to_string(),
            ..App::default()
        };

        history_down(&mut app);

        assert_eq!(app.history_index, None);
        assert_eq!(app.history_draft, "");
        assert_eq!(app.input, "draft");
        assert_eq!(app.input_cursor, app.input.len());
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
