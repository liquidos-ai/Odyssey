//! Welcome hero panel shown in the chat area when no messages exist.

use crate::app::App;
use crate::ui::widgets::logo::{LOGO_HEIGHT, LOGO_ROWS};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Draw the welcome hero panel, vertically centred inside `area`.
pub fn draw_hero(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let w = area.width as usize;

    let logo_style = Style::default().fg(t.primary);
    let accent_style = Style::default().fg(t.primary);
    let dim_style = Style::default().fg(t.text_muted);
    let text_style = Style::default().fg(t.text);
    let key_style = Style::default().fg(t.accent);

    // Content block: LOGO_HEIGHT + 1 (blank) + 1 (info) + 1 (blank) + 1 (keys)
    let content_h = LOGO_HEIGHT + 4;
    let top_pad = area.height.saturating_sub(content_h) / 2;
    let mut lines: Vec<Line<'_>> = vec![Line::from(""); top_pad as usize];

    // ── Logo ──────────────────────────────────────────────────────────────────
    for row in &LOGO_ROWS {
        lines.push(centered(vec![Span::styled(*row, logo_style)], w));
    }

    lines.push(Line::from(""));

    // ── Info line: ▍ model  ·  agent  ·  bundle ─────────────────────────────
    let model = if app.model.is_empty() {
        "no model selected".to_string()
    } else {
        app.model.clone()
    };
    let agent = app.active_agent.clone().unwrap_or_else(|| "default".into());
    let bundle = if app.bundle_ref.is_empty() {
        "no bundle selected".to_string()
    } else {
        app.bundle_ref.clone()
    };

    lines.push(centered(
        vec![
            Span::styled("▍ ", accent_style),
            Span::styled(model, accent_style),
            Span::styled("  ·  ", dim_style),
            Span::styled(agent, text_style),
            Span::styled("  ·  ", dim_style),
            Span::styled(bundle, dim_style),
        ],
        w,
    ));

    lines.push(Line::from(""));

    // ── Keybinding hints ──────────────────────────────────────────────────────
    lines.push(centered(
        vec![
            Span::styled("↩", key_style),
            Span::styled(" send", dim_style),
            Span::styled("    /", key_style),
            Span::styled(" commands", dim_style),
            Span::styled("    tab", key_style),
            Span::styled(" agents", dim_style),
            Span::styled("    esc", key_style),
            Span::styled(" cancel", dim_style),
        ],
        w,
    ));

    frame.render_widget(Paragraph::new(lines), area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Prepend padding to visually centre the span group within `width` columns.
fn centered<'a>(spans: Vec<Span<'a>>, width: usize) -> Line<'a> {
    let content_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = if width > content_len {
        (width - content_len) / 2
    } else {
        0
    };
    let mut result: Vec<Span<'a>> = vec![Span::raw(" ".repeat(pad))];
    result.extend(spans);
    Line::from(result)
}
