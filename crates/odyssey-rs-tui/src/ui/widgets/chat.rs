//! Chat transcript widget with scroll and scrollbar.

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};

/// Draw the scrollable chat transcript panel.
pub fn draw_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = app.theme;
    let lines = app.render_lines();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(" Chat ", Style::default().fg(t.text_muted)));

    let inner = block.inner(area);
    let content_width = inner.width.saturating_sub(1);
    let content_height = inner.height as usize;

    let total_lines = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(content_width)
        .max(1);

    let max_scroll = total_lines.saturating_sub(content_height) as u16;
    app.update_scroll_bounds(max_scroll);
    let scroll = app.scroll;

    let chat_inner = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };

    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        chat_inner,
    );

    if total_lines > content_height {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(total_lines)
            .position(scroll as usize)
            .viewport_content_length(content_height);
        let scrollbar_area = Rect {
            x: inner.x + inner.width.saturating_sub(1),
            y: inner.y,
            width: 1,
            height: inner.height,
        };
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(t.border))
                .thumb_style(Style::default().fg(t.text_muted)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}
