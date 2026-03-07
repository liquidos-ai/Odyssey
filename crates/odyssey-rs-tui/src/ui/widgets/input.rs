//! Text input widget with cursor positioning.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Draw the input box.
pub fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let has_permission = !app.pending_permissions.is_empty();
    let border_color = if has_permission {
        t.border
    } else {
        t.border_active
    };
    let title = if has_permission {
        " Permission Pending (y/a/n) "
    } else {
        " Input "
    };
    let title_color = if has_permission {
        t.primary
    } else {
        t.secondary
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(title_color)));

    let inner = block.inner(area);

    let prompt_style = Style::default().fg(t.primary).add_modifier(Modifier::BOLD);
    let input_line = if app.input.is_empty() && !has_permission {
        Line::from(vec![
            Span::styled(" ", prompt_style),
            Span::styled("Type a message...", Style::default().fg(t.text_muted)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ", prompt_style),
            Span::styled(app.input.as_str(), Style::default().fg(t.text)),
        ])
    };

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(input_line), inner);

    if !has_permission {
        frame.set_cursor_position((inner.x + 1 + app.input.len() as u16, inner.y));
    }
}
