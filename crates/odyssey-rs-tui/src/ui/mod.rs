//! TUI rendering entry point.
//!
//! Call [`draw`] once per event-loop tick to refresh the entire frame.

pub mod markdown;
pub mod theme;
pub mod widgets;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use widgets::chat::draw_chat;
use widgets::header::draw_header;
use widgets::hero::draw_hero;
use widgets::input::draw_input;
use widgets::slash_palette::draw_slash_palette;
use widgets::status_bar::draw_status_bar;
use widgets::viewer::{draw_viewer, draw_viewer_footer};

/// Height of the header bar (6 content lines + 2 border lines).
const HEADER_HEIGHT: u16 = 8;

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
        let [hero, input, status] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(3),
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
        let [header, chat, input, status] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Min(0),
                Constraint::Length(3),
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
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn base_app() -> App {
        let session_id = Uuid::new_v4();
        let mut app = App::new();
        app.user_name = "Ada".to_string();
        app.model = "gpt-4.1-mini".to_string();
        app.model_id = app.model.clone();
        app.cwd = "/workspace/demo".to_string();
        app.bundle_ref = "local/demo@0.1.0".to_string();
        app.status = "running".to_string();
        app.cpu_usage = 72.5;
        app.gpu_temp = Some(64.0);
        app.active_agent = Some("planner".to_string());
        app.active_session = Some(session_id);
        app.agents = vec!["planner".to_string(), "reviewer".to_string()];
        app.models = vec!["gpt-4.1-mini".to_string(), "gpt-4.1".to_string()];
        app.bundles = vec![BundleInstallSummary {
            namespace: "local".to_string(),
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            path: PathBuf::from("/tmp/demo"),
        }];
        app.sessions = vec![SessionSummary {
            id: session_id,
            agent_id: "planner".to_string(),
            message_count: 2,
            created_at: Utc::now(),
        }];
        app.skills = vec![SkillSummary {
            name: "repo-hygiene".to_string(),
            description: "Keep repositories clean".to_string(),
            path: PathBuf::from("/tmp/skills/repo-hygiene/SKILL.md"),
        }];
        app
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

        assert_eq!(rendered.contains("gpt-4.1-mini"), true);
        assert_eq!(rendered.contains("Commands"), true);
        assert_eq!(rendered.contains("se"), true);
        assert_eq!(rendered.contains("Ctrl+C"), true);
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

        assert_eq!(rendered.contains("Welcome back, Ada!"), true);
        assert_eq!(rendered.contains("Chat"), true);
        assert_eq!(rendered.contains("filesystem write"), true);
        assert_eq!(rendered.contains("Permission Pending"), true);
        assert_eq!(rendered.contains("hello odyssey"), true);
        assert_eq!(rendered.contains("working on it"), true);
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
            assert_eq!(rendered.contains("Actions"), true);
            assert_eq!(rendered.contains("Esc to close"), true);
            assert_eq!(rendered.contains(needle), true);
        }
    }
}
