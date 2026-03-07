//! Header widget: ASCII art banner, session info, CPU/GPU gauges.

use crate::app::App;
use crate::ui::widgets::logo::logo_lines_compact;
use odyssey_rs_core::McpStatus;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const CPU_WIDGET_WIDTH: u16 = 22;

/// Draw the header bar with the ASCII art banner, info lines, and CPU widget.
pub fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let session = app
        .active_session
        .map(|id| {
            let s = id.to_string();
            s[..8.min(s.len())].to_string()
        })
        .unwrap_or_else(|| "none".to_string());
    let agent = app
        .active_agent
        .clone()
        .unwrap_or_else(|| "none".to_string());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(CPU_WIDGET_WIDTH)])
        .split(area);

    draw_header_left(frame, app, cols[0], &session, &agent);
    draw_cpu_widget(frame, app, cols[1]);
}

fn draw_header_left(frame: &mut Frame<'_>, app: &App, area: Rect, session: &str, agent: &str) {
    let t = &app.theme;

    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let label_style = Style::default().fg(t.text_muted);
    let value_style = Style::default().fg(t.text);
    let art_style = Style::default().fg(t.primary).add_modifier(Modifier::BOLD);
    let version_style = Style::default().fg(t.text_muted);

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Shared compact half-block logo — append version tag to the last row.
    let mut logo = logo_lines_compact(art_style);
    if let Some(last) = logo.last_mut() {
        last.spans
            .push(Span::styled(format!("  v{VERSION}"), version_style));
    }
    lines.extend(logo);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Welcome back, {}!", app.user_name),
        Style::default().fg(t.text),
    )));

    lines.push(Line::from(vec![
        Span::styled("  model ", label_style),
        Span::styled(app.model.as_str(), value_style),
        Span::styled("  cwd ", label_style),
        Span::styled(app.cwd.as_str(), value_style),
    ]));

    let (mcp_text, mcp_style) = mcp_status_badge(app);
    let mut session_spans = vec![
        Span::styled("  session ", label_style),
        Span::styled(session.to_string(), value_style),
        Span::styled("  agent ", label_style),
        Span::styled(agent.to_string(), value_style),
        Span::styled("  mcp ", label_style),
        Span::styled(mcp_text, mcp_style),
    ];
    if let Some(permission) = app.pending_permissions.front() {
        session_spans.push(Span::styled("  ", Style::default()));
        session_spans.push(Span::styled(
            format!(" {} ", permission.summary),
            Style::default()
                .fg(Color::Rgb(10, 10, 10))
                .bg(t.primary)
                .add_modifier(Modifier::BOLD),
        ));
        let remaining = app.pending_permissions.len().saturating_sub(1);
        if remaining > 0 {
            session_spans.push(Span::styled(
                format!(" +{remaining}"),
                Style::default().fg(t.primary),
            ));
        }
    }
    lines.push(Line::from(session_spans));

    let line_count = lines.len() as u16;
    let pad_top = inner.height.saturating_sub(line_count) / 2;
    let centered = Rect {
        x: inner.x,
        y: inner.y + pad_top,
        width: inner.width,
        height: inner.height.saturating_sub(pad_top),
    };

    frame.render_widget(Paragraph::new(lines), centered);
}

/// Draw the compact CPU/GPU usage gauge on the right side of the header.
pub fn draw_cpu_widget(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let cpu = app.cpu_usage;
    let cpu_color = usage_color(cpu, 50.0, 80.0, t.accent);

    let block = Block::default()
        .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(" CPU ", Style::default().fg(t.text_muted)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let bar_width = inner.width.saturating_sub(2);
    let filled = ((cpu / 100.0) * bar_width as f32).round() as u16;
    let empty = bar_width.saturating_sub(filled);

    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            format!(" {cpu:5.1}%"),
            Style::default().fg(cpu_color).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled("█".repeat(filled as usize), Style::default().fg(cpu_color)),
            Span::styled("░".repeat(empty as usize), Style::default().fg(t.border)),
        ]),
    ];

    if let Some(temp) = app.gpu_temp {
        let color = usage_color(temp, 60.0, 80.0, t.accent);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" GPU {temp:5.1}°C"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Map a gauge value to green / accent / red based on thresholds.
fn usage_color(value: f32, warn: f32, crit: f32, accent: Color) -> Color {
    if value < warn {
        Color::Rgb(120, 220, 140)
    } else if value < crit {
        accent
    } else {
        Color::Rgb(255, 110, 110)
    }
}

pub(crate) fn mcp_status_badge(app: &App) -> (String, Style) {
    let t = &app.theme;
    match &app.mcp_status {
        McpStatus::Disabled => ("disabled".to_string(), Style::default().fg(t.text_muted)),
        McpStatus::Connected {
            server_count,
            tool_count,
        } => (
            format!("connected {server_count}s/{tool_count}t"),
            Style::default()
                .fg(Color::Rgb(120, 220, 140))
                .add_modifier(Modifier::BOLD),
        ),
        McpStatus::Failed(message) => (
            format!("failed ({})", truncate_status(message, 18)),
            Style::default()
                .fg(Color::Rgb(255, 110, 110))
                .add_modifier(Modifier::BOLD),
        ),
    }
}

fn truncate_status(value: &str, max_chars: usize) -> String {
    let len = value.chars().count();
    if len <= max_chars {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max_chars).collect();
    format!("{truncated}…")
}
