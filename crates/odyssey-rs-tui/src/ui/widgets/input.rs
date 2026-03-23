//! Text input widget with cursor positioning.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

/// The prompt prefix shown before user input.
pub const PROMPT: &str = "❯ ";

/// Draw the input box.
pub fn draw_input(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
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
    let width = inner.width.max(1) as usize;

    // Store inner width so the input handler can use it for Up/Down navigation.
    app.input_inner_width = width as u16;

    let prompt_style = Style::default().fg(t.primary).add_modifier(Modifier::BOLD);
    let prompt_w = UnicodeWidthStr::width(PROMPT);

    if app.input.is_empty() && !has_permission {
        // Placeholder — single line, no wrapping needed.
        let line = Line::from(vec![
            Span::styled(PROMPT, prompt_style),
            Span::styled("Type a message...", Style::default().fg(t.text_muted)),
        ]);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(line), inner);
        // Cursor right after the prompt.
        frame.set_cursor_position((inner.x + prompt_w as u16, inner.y));
        return;
    }

    // ── Manually wrap prompt + input into visual lines ──────────────────────
    let input_style = Style::default().fg(t.text);
    let input = app.input.as_str();
    let cursor_byte = app.input_cursor.min(input.len());
    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut col = 0usize;
    let mut spans: Vec<Span<'_>> = Vec::new();

    // Track cursor visual position.
    let mut cursor_vis_line: u16 = 0;
    let mut cursor_vis_col: u16 = 0;
    let mut cursor_found = cursor_byte == 0;

    // Add the prompt on the first line.
    spans.push(Span::styled(PROMPT, prompt_style));
    col += prompt_w;

    // Set cursor position if it's at byte 0 (right after prompt).
    if cursor_byte == 0 {
        cursor_vis_col = col as u16;
        cursor_vis_line = 0;
    }

    // Walk through the input character-by-character, breaking into lines.
    let mut chunk_start = 0;
    for (i, ch) in input.char_indices() {
        let ch_w = UnicodeWidthStr::width(ch.encode_utf8(&mut [0u8; 4]) as &str);
        if col + ch_w > width && col > 0 {
            // Flush current chunk before this character.
            if chunk_start < i {
                spans.push(Span::styled(&input[chunk_start..i], input_style));
            }
            lines.push(Line::from(std::mem::take(&mut spans)));
            chunk_start = i;
            col = 0;
        }
        // Check if cursor is at this character's position (before it).
        if !cursor_found && i == cursor_byte {
            cursor_vis_line = lines.len() as u16;
            cursor_vis_col = col as u16;
            cursor_found = true;
        }
        col += ch_w;
    }

    // Cursor at end of input.
    if !cursor_found {
        cursor_vis_line = lines.len() as u16;
        cursor_vis_col = col as u16;
    }

    // Flush remaining text.
    if chunk_start < input.len() {
        spans.push(Span::styled(&input[chunk_start..], input_style));
    }
    lines.push(Line::from(spans));

    let visible = inner.height;

    // Scroll so the cursor line is always visible.
    let scroll = if cursor_vis_line >= visible {
        cursor_vis_line - visible + 1
    } else {
        0
    };

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);

    if !has_permission {
        let cursor_x = inner.x + cursor_vis_col;
        let cursor_y = inner.y + cursor_vis_line - scroll;
        frame.set_cursor_position((cursor_x, cursor_y.min(inner.y + visible.saturating_sub(1))));
    }
}
