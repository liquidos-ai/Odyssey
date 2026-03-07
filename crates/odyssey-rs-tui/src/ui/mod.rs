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
