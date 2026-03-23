//! TUI rendering entry point.
//!
//! Call [`draw`] once per event-loop tick to refresh the entire frame.

pub mod markdown;
pub mod theme;
pub mod widgets;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use unicode_width::UnicodeWidthStr;

use widgets::chat::draw_chat;
use widgets::header::draw_header;
use widgets::hero::draw_hero;
use widgets::input::draw_input;
use widgets::slash_palette::draw_slash_palette;
use widgets::status_bar::draw_status_bar;
use widgets::viewer::{draw_viewer, draw_viewer_footer};

/// Height of the header bar (6 content lines + 2 border lines).
const HEADER_HEIGHT: u16 = 8;

/// Maximum fraction of the terminal height the input box may occupy.
const INPUT_MAX_PERCENT: u16 = 40;

/// Count the number of visual lines the prompt + input occupy at the given width.
pub fn input_line_count(input: &str, width: usize) -> u16 {
    let prompt_w = UnicodeWidthStr::width(widgets::input::PROMPT);
    let mut col = prompt_w;
    let mut lines: u16 = 1;
    for ch in input.chars() {
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if col + ch_w > width && col > 0 {
            lines += 1;
            col = 0;
        }
        col += ch_w;
    }
    lines
}

/// Compute the height needed for the input box (content lines + 2 border lines),
/// capped so it never exceeds `INPUT_MAX_PERCENT`% of the terminal height.
fn input_height(app: &App, area_width: u16, area_height: u16) -> u16 {
    let inner_w = area_width.saturating_sub(2).max(1) as usize;
    let lines = input_line_count(&app.input, inner_w);
    let max_h = (area_height * INPUT_MAX_PERCENT / 100).max(3);
    (lines + 2).min(max_h)
}

/// Draw the complete TUI frame for the current application state.
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();

    if app.viewer.is_some() {
        let [header, content, footer, status] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Min(0),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .areas(area);

        draw_header(frame, app, header);
        draw_viewer(frame, app, content);
        draw_viewer_footer(frame, app, footer);
        draw_status_bar(frame, app, status);
    } else if app.messages.is_empty() {
        // Hero screen: no header, give all vertical space to the hero.
        let ih = input_height(app, area.width, area.height);
        let [hero, input, status] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(ih),
                Constraint::Length(1),
            ])
            .areas(area);

        draw_hero(frame, app, hero);
        if app.show_slash_commands {
            draw_slash_palette(frame, app, hero);
        }
        draw_input(frame, app, input);
        draw_status_bar(frame, app, status);
    } else {
        // Chat screen: show full header.
        let ih = input_height(app, area.width, area.height);
        let [header, chat, input, status] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Min(0),
                Constraint::Length(ih),
                Constraint::Length(1),
            ])
            .areas(area);

        draw_header(frame, app, header);
        draw_chat(frame, app, chat);
        if app.show_slash_commands {
            draw_slash_palette(frame, app, chat);
        }
        draw_input(frame, app, input);
        draw_status_bar(frame, app, status);
    }
}

#[cfg(test)]
mod tests {
    use super::draw;
    use crate::app::types::{ChatEntry, ChatRole, PendingPermission};
    use crate::app::{App, ViewerKind};
    use chrono::Utc;
    use odyssey_rs_bundle::BundleInstallSummary;
    use odyssey_rs_protocol::{SessionSummary, SkillSummary};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use uuid::Uuid;

    fn base_app() -> App {
        let session_id = Uuid::new_v4();
        App {
            user_name: "Ada".to_string(),
            model_id: "gpt-4.1-mini".to_string(),
            model: "gpt-4.1-mini".to_string(),
            cwd: "/workspace/demo".to_string(),
            bundle_ref: "local/demo@0.1.0".to_string(),
            status: "running".to_string(),
            cpu_usage: 72.5,
            gpu_temp: Some(64.0),
            active_agent: Some("planner".to_string()),
            active_session: Some(session_id),
            agents: vec!["planner".to_string(), "reviewer".to_string()],
            models: vec!["gpt-4.1-mini".to_string(), "gpt-4.1".to_string()],
            bundles: vec![BundleInstallSummary {
                namespace: "local".to_string(),
                id: "demo".to_string(),
                version: "0.1.0".to_string(),
                path: "fixtures/demo".into(),
            }],
            sessions: vec![SessionSummary {
                id: session_id,
                agent_id: "planner".to_string(),
                message_count: 2,
                created_at: Utc::now(),
            }],
            skills: vec![SkillSummary {
                name: "repo-hygiene".to_string(),
                description: "Keep repositories clean".to_string(),
                path: "fixtures/skills/repo-hygiene/SKILL.md".into(),
            }],
            ..App::default()
        }
    }

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("create terminal");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("draw terminal");
        format!("{}", terminal.backend())
    }

    #[test]
    fn draws_hero_input_status_and_slash_palette() {
        let mut app = base_app();
        app.messages.clear();
        app.input = "/se".to_string();
        app.show_slash_commands = true;

        let rendered = render(&mut app, 100, 30);

        assert!(rendered.contains("gpt-4.1-mini"));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("se"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn draws_chat_header_permission_state_and_status() {
        let mut app = base_app();
        app.messages = vec![
            ChatEntry {
                role: ChatRole::User,
                content: "hello odyssey".to_string(),
                color: None,
            },
            ChatEntry {
                role: ChatRole::Assistant,
                content: "working on it".to_string(),
                color: None,
            },
        ];
        app.pending_permissions.push_back(PendingPermission {
            request_id: Uuid::new_v4(),
            summary: "filesystem write".to_string(),
        });
        app.input = "approve?".to_string();

        let rendered = render(&mut app, 110, 32);

        assert!(rendered.contains("Welcome back, Ada!"));
        assert!(rendered.contains("Chat"));
        assert!(rendered.contains("filesystem write"));
        assert!(rendered.contains("Permission Pending"));
        assert!(rendered.contains("hello odyssey"));
        assert!(rendered.contains("working on it"));
    }

    #[test]
    fn draws_all_viewer_modes_and_footer() {
        let mut app = base_app();
        let cases = [
            (ViewerKind::Agents, "planner"),
            (ViewerKind::Bundles, "local/demo@0.1.0"),
            (ViewerKind::Sessions, "planner"),
            (ViewerKind::Skills, "repo-hygiene"),
            (ViewerKind::Models, "gpt-4.1-mini"),
            (ViewerKind::Themes, "odyssey"),
        ];

        for (viewer, needle) in cases {
            app.viewer = Some(viewer);
            let rendered = render(&mut app, 110, 34);
            assert!(rendered.contains("Actions"));
            assert!(rendered.contains("Esc to close"));
            assert!(rendered.contains(needle));
        }
    }
}
