//! Slash-command palette overlay widget with keyboard-navigable selection.

use crate::app::App;
use crate::handlers::slash::filtered_commands;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Draw the slash-command palette as an overlay anchored to the bottom of `area`.
pub fn draw_slash_palette(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let filtered = filtered_commands(&app.input);
    let selected = app.slash_selected.min(filtered.len().saturating_sub(1));

    let hint_style = Style::default()
        .fg(t.text_muted)
        .add_modifier(Modifier::ITALIC);

    let mut cmd_lines: Vec<Line<'_>> = Vec::new();
    for (idx, entry) in filtered.iter().enumerate() {
        let is_sel = idx == selected;
        let row_bg = if is_sel { t.bg_selected } else { t.bg_popup };

        let marker = if is_sel {
            Span::styled("  ", Style::default().fg(t.primary).bg(row_bg))
        } else {
            Span::styled("   ", Style::default().bg(row_bg))
        };

        let cmd_style = if is_sel {
            Style::default()
                .fg(t.primary)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.primary).bg(row_bg)
        };

        let args_text = if entry.args.is_empty() {
            String::default()
        } else {
            format!(" {}", entry.args)
        };

        cmd_lines.push(Line::from(vec![
            marker,
            Span::styled(format!("/{}", entry.trigger), cmd_style),
            Span::styled(args_text, Style::default().fg(t.text_muted).bg(row_bg)),
            Span::styled("  ", Style::default().bg(row_bg)),
            Span::styled(entry.description, Style::default().fg(t.text).bg(row_bg)),
        ]));
    }

    if filtered.is_empty() {
        cmd_lines.push(Line::from(Span::styled(
            "  No matching commands",
            Style::default().fg(t.text_muted).bg(t.bg_popup),
        )));
    }

    let hint = Line::from(Span::styled(
        "  ↑↓ navigate  Tab/Enter select  Esc close",
        hint_style,
    ));

    let inner_height = (cmd_lines.len() + 2) as u16;
    let total_height = (inner_height + 2).min(area.height);

    let palette_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(total_height),
        width: area.width.saturating_sub(2).min(56),
        height: total_height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.primary))
        .title(Span::styled(
            " Commands ",
            Style::default().fg(t.primary).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.bg_popup));

    let mut lines: Vec<Line<'_>> = vec![Line::from("")];
    lines.extend(cmd_lines);
    lines.push(Line::from(""));
    lines.push(hint);

    frame.render_widget(Paragraph::new(lines).block(block), palette_area);
}
