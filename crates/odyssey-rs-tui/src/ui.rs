//! Rendering routines for the Odyssey TUI.

use crate::app::{App, ViewerKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
// ── Theme colors from theme.json (dark mode) ──────────────────────────

const PRIMARY: Color = Color::Rgb(236, 91, 43); // #EC5B2B
const SECONDARY: Color = Color::Rgb(238, 121, 72); // #EE7948
const TEXT: Color = Color::Rgb(238, 238, 238); // #eeeeee
const TEXT_MUTED: Color = Color::Rgb(128, 128, 128); // #808080
const BORDER: Color = Color::Rgb(60, 60, 60); // #3c3c3c (borderSubtle)
const BORDER_ACTIVE: Color = Color::Rgb(238, 121, 72); // #EE7948
const YELLOW: Color = Color::Rgb(229, 192, 123); // #e5c07b

const SLASH_PALETTE_HEIGHT: u16 = 12;
const HEADER_HEIGHT: u16 = 9; // 7 inner lines + 2 border lines

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HERO_ART: [&str; 2] = [
    " █▀▀█ █▀▀▄ █ █ █▀▀▄ █▀▀▄ █▀▀▀ █ █",
    " █▄▄█ █▄▄▀  █  ▀▄▄█ ▀▄▄█ █▄▄▄  █ ",
];

/// Draw the entire TUI frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();

    if app.viewer.is_some() {
        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT), // header bar
                Constraint::Min(0),                // viewer content
                Constraint::Length(3),             // input / footer
                Constraint::Length(1),             // status bar
            ])
            .split(area);

        draw_header(frame, app, root[0]);
        draw_viewer(frame, app, root[1]);
        draw_viewer_footer(frame, app, root[2]);
        draw_status_bar(frame, app, root[3]);
    } else {
        let root = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT), // header bar
                Constraint::Min(0),                // chat
                Constraint::Length(3),             // input
                Constraint::Length(1),             // status bar
            ])
            .split(area);

        draw_header(frame, app, root[0]);
        draw_chat(frame, app, root[1]);
        if app.show_slash_commands {
            draw_slash_palette(frame, root[1]);
        }
        draw_input(frame, app, root[2]);
        draw_status_bar(frame, app, root[3]);
    }
}

/// Draw the header with ASCII art centered vertically, info below, CPU gauge on right.
fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
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

    // Outer split: [left content] [right CPU widget]
    let cpu_widget_width: u16 = 22;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(cpu_widget_width)])
        .split(area);

    let left_block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER));

    let inner = left_block.inner(cols[0]);
    frame.render_widget(left_block, cols[0]);

    // Build all header lines
    let label_style = Style::default().fg(TEXT_MUTED);
    let value_style = Style::default().fg(TEXT);

    let mut lines: Vec<Line<'_>> = Vec::new();

    // ASCII art banner — version on the last line, to the right
    let art_style = Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD);
    for (i, art_line) in HERO_ART.iter().enumerate() {
        if i == HERO_ART.len() - 1 {
            // Last art line: append version
            lines.push(Line::from(vec![
                Span::styled(*art_line, art_style),
                Span::styled(format!("  v{VERSION}"), Style::default().fg(TEXT_MUTED)),
            ]));
        } else {
            lines.push(Line::from(Span::styled(*art_line, art_style)));
        }
    }

    // Padding + welcome
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Welcome back, {}!", app.user_name),
        Style::default().fg(TEXT),
    )));

    // Model + cwd line
    lines.push(Line::from(vec![
        Span::styled("  model ", label_style),
        Span::styled(app.model.as_str(), value_style),
        Span::styled("  cwd ", label_style),
        Span::styled(app.cwd.as_str(), value_style),
    ]));

    // Session + agent line
    let mut session_spans = vec![
        Span::styled("  session ", label_style),
        Span::styled(session, value_style),
        Span::styled("  agent ", label_style),
        Span::styled(agent, value_style),
    ];

    if let Some(permission) = app.pending_permissions.front() {
        session_spans.push(Span::styled("  ", Style::default()));
        session_spans.push(Span::styled(
            format!(" {} ", permission.summary),
            Style::default()
                .fg(Color::Rgb(10, 10, 10))
                .bg(PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
        let remaining = app.pending_permissions.len().saturating_sub(1);
        if remaining > 0 {
            session_spans.push(Span::styled(
                format!(" +{remaining}"),
                Style::default().fg(PRIMARY),
            ));
        }
    }

    lines.push(Line::from(session_spans));

    // Vertically center the content block
    let line_count = lines.len() as u16;
    let pad_top = inner.height.saturating_sub(line_count) / 2;
    let centered_area = Rect {
        x: inner.x,
        y: inner.y + pad_top,
        width: inner.width,
        height: inner.height.saturating_sub(pad_top),
    };

    let text = Paragraph::new(lines);
    frame.render_widget(text, centered_area);

    // ── Right: CPU usage widget ──
    draw_cpu_widget(frame, app, cols[1]);
}

/// Draw a compact CPU usage widget with a vertical bar gauge.
fn draw_cpu_widget(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let cpu = app.cpu_usage;
    let cpu_color = if cpu < 50.0 {
        Color::Rgb(120, 220, 140) // green
    } else if cpu < 80.0 {
        YELLOW
    } else {
        Color::Rgb(255, 110, 110) // red
    };

    let block = Block::default()
        .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" CPU ", Style::default().fg(TEXT_MUTED)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Bar gauge: fill proportional to CPU usage
    let bar_width = inner.width.saturating_sub(2); // padding
    let filled = ((cpu / 100.0) * bar_width as f32).round() as u16;
    let empty = bar_width.saturating_sub(filled);

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Percentage label
    lines.push(Line::from(Span::styled(
        format!(" {cpu:5.1}%"),
        Style::default().fg(cpu_color).add_modifier(Modifier::BOLD),
    )));

    // Bar
    let bar_filled = "█".repeat(filled as usize);
    let bar_empty = "░".repeat(empty as usize);
    lines.push(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(bar_filled, Style::default().fg(cpu_color)),
        Span::styled(bar_empty, Style::default().fg(BORDER)),
    ]));

    if let Some(temp) = app.gpu_temp {
        let gpu_color = if temp < 60.0 {
            Color::Rgb(120, 220, 140)
        } else if temp < 80.0 {
            YELLOW
        } else {
            Color::Rgb(255, 110, 110)
        };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" GPU {temp:5.1}°C"),
            Style::default().fg(gpu_color).add_modifier(Modifier::BOLD),
        )));
    }

    let gauge = Paragraph::new(lines);
    frame.render_widget(gauge, inner);
}

/// Draw the chat transcript with border and scrollbar.
fn draw_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let lines = app.render_lines();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" Chat ", Style::default().fg(TEXT_MUTED)));

    let inner = block.inner(area);
    let content_width = inner.width.saturating_sub(1); // -1 for scrollbar
    let content_height = inner.height as usize;

    // Use ratatui's own line_count to get the exact wrapped line total,
    // avoiding any mismatch with a hand-written wrap estimator.
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

    let chat = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(block, area);
    frame.render_widget(chat, chat_inner);

    // Scrollbar
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
                .style(Style::default().fg(BORDER))
                .thumb_style(Style::default().fg(TEXT_MUTED)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

/// Draw the input box with border and cursor.
fn draw_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let is_active = app.pending_permissions.is_empty();
    let border_color = if is_active { BORDER_ACTIVE } else { BORDER };
    let title = if !app.pending_permissions.is_empty() {
        " Permission Pending (y/a/n) "
    } else {
        " Input "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title,
            Style::default().fg(if is_active { SECONDARY } else { PRIMARY }),
        ));

    let inner = block.inner(area);

    let prompt_style = Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD);
    let input_text = if app.input.is_empty() && is_active {
        Line::from(vec![
            Span::styled(" ", prompt_style),
            Span::styled("Type a message...", Style::default().fg(TEXT_MUTED)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ", prompt_style),
            Span::styled(app.input.as_str(), Style::default().fg(TEXT)),
        ])
    };

    let paragraph = Paragraph::new(input_text);
    frame.render_widget(block, area);
    frame.render_widget(paragraph, inner);

    // Position cursor after input text
    if is_active {
        frame.set_cursor_position((inner.x + 2 + app.input.len() as u16, inner.y));
    }
}

/// Draw the status bar at the bottom.
fn draw_status_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let status_color = match app.status.as_str() {
        "running" => PRIMARY,
        "idle" => TEXT_MUTED,
        _ => YELLOW,
    };

    let shortcuts = vec![
        Span::styled(" Ctrl+C", Style::default().fg(TEXT_MUTED)),
        Span::styled(" quit", Style::default().fg(BORDER)),
        Span::styled("  Ctrl+N", Style::default().fg(TEXT_MUTED)),
        Span::styled(" new", Style::default().fg(BORDER)),
        Span::styled("  /", Style::default().fg(TEXT_MUTED)),
        Span::styled(" commands", Style::default().fg(BORDER)),
        Span::styled("  PgUp/PgDn", Style::default().fg(TEXT_MUTED)),
        Span::styled(" scroll", Style::default().fg(BORDER)),
    ];

    let right_text = format!(" {} ", app.status);

    // Calculate how much space the right side needs
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

    let left = Paragraph::new(Line::from(shortcuts));
    let right = Paragraph::new(Line::from(Span::styled(
        right_text,
        Style::default().fg(status_color),
    )));

    frame.render_widget(left, left_area);
    frame.render_widget(right, right_area);
}

fn draw_slash_palette(frame: &mut Frame<'_>, area: Rect) {
    let cmd_style = Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(TEXT_MUTED);
    let hint_style = Style::default()
        .fg(TEXT_MUTED)
        .add_modifier(Modifier::ITALIC);

    let lines = vec![
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("  /new", cmd_style),
            Span::styled("            ", desc_style),
            Span::styled("Create a new session", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /sessions", cmd_style),
            Span::styled("       ", desc_style),
            Span::styled("List all sessions", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /skills", cmd_style),
            Span::styled("         ", desc_style),
            Span::styled("List available skills", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /models", cmd_style),
            Span::styled("        ", desc_style),
            Span::styled("List available models", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /model <id>", cmd_style),
            Span::styled("    ", desc_style),
            Span::styled("Select model by id", desc_style),
        ]),
        Line::from(vec![
            Span::styled("  /join <id>", cmd_style),
            Span::styled("      ", desc_style),
            Span::styled("Join a session by ID", desc_style),
        ]),
        Line::from(vec![]),
        Line::from(Span::styled("  Esc to close", hint_style)),
    ];

    let height = SLASH_PALETTE_HEIGHT
        .min(area.height)
        .min(lines.len() as u16 + 2); // +2 for border

    let palette_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(height),
        width: area.width.saturating_sub(2).min(50),
        height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(PRIMARY))
        .title(Span::styled(
            " Commands ",
            Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(20, 20, 20)));

    let palette = Paragraph::new(lines).block(block);
    frame.render_widget(palette, palette_area);
}

fn draw_viewer(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(kind) = app.viewer else {
        return;
    };
    let (title, lines) = match kind {
        ViewerKind::Sessions => (" Sessions ", render_session_lines(app)),
        ViewerKind::Skills => (" Skills ", render_skill_lines(app)),
        ViewerKind::Models => (" Models ", render_model_lines(app)),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            title,
            Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    let content_width = inner.width.saturating_sub(1);
    let content_height = inner.height as usize;

    let total_lines = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(content_width)
        .max(1);
    let max_scroll = total_lines.saturating_sub(content_height) as u16;
    app.update_viewer_scroll_bounds(max_scroll);
    let scroll = app.viewer_scroll;

    let viewer_inner = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };

    let viewer = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(block, area);
    frame.render_widget(viewer, viewer_inner);

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
                .style(Style::default().fg(BORDER))
                .thumb_style(Style::default().fg(TEXT_MUTED)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

fn draw_viewer_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let hint = match app.viewer {
        Some(ViewerKind::Sessions) => "Up/Down to navigate  Enter to select  Esc to close",
        Some(ViewerKind::Models) => "Up/Down to navigate  Enter to select  Esc to close",
        _ => "Esc to close",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" Actions ", Style::default().fg(TEXT_MUTED)));

    let paragraph = Paragraph::new(Line::from(Span::styled(
        format!(" {hint}"),
        Style::default().fg(TEXT_MUTED),
    )))
    .block(block);

    frame.render_widget(paragraph, area);
}

fn render_session_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if app.sessions.is_empty() {
        lines.push(Line::from(Span::styled(
            " No sessions found. Use /new to create one.",
            Style::default().fg(TEXT_MUTED),
        )));
        return lines;
    }

    // Column header
    lines.push(Line::from(vec![
        Span::styled("   ", Style::default()),
        Span::styled(
            "ID",
            Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled("          ", Style::default()),
        Span::styled(
            "Agent",
            Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled("          ", Style::default()),
        Span::styled(
            "Messages",
            Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled("    ", Style::default()),
        Span::styled(
            "Created",
            Style::default().fg(TEXT_MUTED).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        " ─".to_string() + &"─".repeat(70),
        Style::default().fg(BORDER),
    )));

    for (idx, session) in app.sessions.iter().enumerate() {
        let is_selected = idx == app.selected_session;
        let id_str = {
            let s = session.id.to_string();
            s[..8.min(s.len())].to_string()
        };

        let (prefix, style) = if is_selected {
            (
                Span::styled(
                    "  ",
                    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
                ),
                Style::default().fg(PRIMARY),
            )
        } else {
            (
                Span::styled("   ", Style::default()),
                Style::default().fg(TEXT),
            )
        };

        lines.push(Line::from(vec![
            prefix,
            Span::styled(format!("{:<12}", id_str), style),
            Span::styled(format!("{:<15}", session.agent_id), style),
            Span::styled(
                format!("{:<12}", format!("{} msgs", session.message_count)),
                style,
            ),
            Span::styled(session.created_at.format("%Y-%m-%d").to_string(), style),
        ]));
    }
    lines
}

fn render_skill_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if app.skills.is_empty() {
        lines.push(Line::from(Span::styled(
            " No skills configured.",
            Style::default().fg(TEXT_MUTED),
        )));
        return lines;
    }

    for skill in &app.skills {
        let path = skill
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| skill.path.to_string_lossy().to_string());

        lines.push(Line::from(vec![Span::styled(
            format!(" {}", skill.name),
            Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("   {}", skill.description),
            Style::default().fg(TEXT),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("   {path}"),
            Style::default().fg(TEXT_MUTED),
        )]));
        lines.push(Line::from(Span::raw("")));
    }
    lines
}

fn render_model_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if app.models.is_empty() {
        lines.push(Line::from(Span::styled(
            " No models registered.",
            Style::default().fg(TEXT_MUTED),
        )));
        return lines;
    }

    for (idx, model_id) in app.models.iter().enumerate() {
        let is_selected = idx == app.selected_model;
        let is_active = model_id == &app.model_id;
        let line_style = if is_selected {
            Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT)
        };
        let active_style = if is_selected {
            Style::default().fg(PRIMARY)
        } else {
            Style::default().fg(SECONDARY)
        };
        let marker = if is_selected { ">" } else { " " };
        let active_tag = if is_active { " (active)" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(format!(" {marker} "), line_style),
            Span::styled(model_id.clone(), line_style),
            Span::styled(active_tag, active_style),
        ]));
    }

    lines
}
