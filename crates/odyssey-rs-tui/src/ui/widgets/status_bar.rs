//! Bottom status-bar widget.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draw the one-line status bar at the bottom of the screen.
pub fn draw_status_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let status_color = match app.status.as_str() {
        "running" => t.primary,
        "idle" => t.text_muted,
        _ => t.accent,
    };

    let shortcuts = Line::from(vec![
        Span::styled(" Ctrl+C", Style::default().fg(t.text_muted)),
        Span::styled(" quit", Style::default().fg(t.border)),
        Span::styled("  Ctrl+N", Style::default().fg(t.text_muted)),
        Span::styled(" new", Style::default().fg(t.border)),
        Span::styled("  /", Style::default().fg(t.text_muted)),
        Span::styled(" commands", Style::default().fg(t.border)),
        Span::styled("  PgUp/PgDn", Style::default().fg(t.text_muted)),
        Span::styled(" scroll", Style::default().fg(t.border)),
    ]);

    let right_text = format!(" {} ", app.status);
    let right_len = right_text.len() as u16;

    let left_area = Rect {
        width: area.width.saturating_sub(right_len),
        ..area
    };
    let right_area = Rect {
        x: area.x + area.width.saturating_sub(right_len),
        width: right_len,
        ..area
    };

    frame.render_widget(Paragraph::new(shortcuts), left_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            right_text,
            Style::default().fg(status_color),
        ))),
        right_area,
    );
}
