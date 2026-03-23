//! Header widget: ASCII art banner, session info, CPU/GPU gauges.

use crate::app::App;
use crate::ui::widgets::logo::logo_lines_compact;
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
    let bundle = if app.bundle_ref.is_empty() {
        "none".to_string()
    } else {
        app.bundle_ref.clone()
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(CPU_WIDGET_WIDTH)])
        .split(area);

    draw_header_left(frame, app, cols[0], &session, &agent, &bundle);
    draw_cpu_widget(frame, app, cols[1]);
}

fn draw_header_left(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    session: &str,
    agent: &str,
    bundle: &str,
) {
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

    let mut session_spans = vec![
        Span::styled("  session ", label_style),
        Span::styled(session.to_string(), value_style),
        Span::styled("  agent ", label_style),
        Span::styled(agent.to_string(), value_style),
        Span::styled("  bundle ", label_style),
        Span::styled(bundle.to_string(), value_style),
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

    // Process memory usage
    let mem_label = format_bytes(app.mem_usage_bytes);
    let mem_color = if app.mem_usage_bytes < 256 * 1024 * 1024 {
        Color::Rgb(120, 220, 140)
    } else if app.mem_usage_bytes < 1024 * 1024 * 1024 {
        t.accent
    } else {
        Color::Rgb(255, 110, 110)
    };
    lines.push(Line::from(Span::styled(
        format!(" MEM {mem_label}"),
        Style::default().fg(mem_color).add_modifier(Modifier::BOLD),
    )));

    if let Some(temp) = app.gpu_temp {
        let color = usage_color(temp, 60.0, 80.0, t.accent);
        lines.push(Line::from(Span::styled(
            format!(" GPU {temp:5.1}°C"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Format a byte count as a human-readable string (e.g. "23.4 MB").
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
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
