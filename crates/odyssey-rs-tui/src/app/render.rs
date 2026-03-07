//! Chat transcript rendering: converts `ChatEntry` list to styled ratatui `Line`s.
//!
//! Assistant messages are rendered as Markdown; all other roles use plain text.

use crate::app::state::App;
use crate::app::types::ChatRole;
use crate::ui::markdown::render_markdown;
use crate::ui::theme::Theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

impl App {
    /// Render all chat messages into styled lines for the chat widget.
    pub fn render_lines(&self) -> Vec<Line<'static>> {
        if self.messages.is_empty() {
            return vec![Line::from(Span::styled(
                " No messages yet. Type a message below to start.",
                Style::default().fg(self.theme.text_muted),
            ))];
        }

        let mut lines: Vec<Line<'static>> = Vec::with_capacity(self.messages.len() * 3);

        for (idx, entry) in self.messages.iter().enumerate() {
            let (prefix, prefix_style) = role_badge_style(&entry.role, &self.theme);

            let content_style = entry
                .color
                .map(|c| Style::default().fg(c))
                .unwrap_or_else(|| default_content_style(&entry.role, &self.theme));

            lines.push(Line::from(vec![Span::styled(prefix, prefix_style)]));

            match entry.role {
                ChatRole::Assistant => {
                    // Render assistant content as Markdown.
                    let md_lines = render_markdown(&entry.content, &self.theme, content_style);
                    lines.extend(md_lines);
                    // Ensure a blank separator between messages.
                    if idx + 1 < self.messages.len()
                        && lines.last().map(|l| !l.spans.is_empty()) == Some(true)
                    {
                        lines.push(Line::from(Span::raw("")));
                    }
                }
                _ => {
                    // Plain text for user / system / permission messages.
                    let mut content_iter = entry.content.lines();
                    if let Some(first) = content_iter.next() {
                        if !first.is_empty() {
                            lines
                                .push(Line::from(Span::styled(format!(" {first}"), content_style)));
                        }
                        for line in content_iter {
                            lines.push(Line::from(Span::styled(format!(" {line}"), content_style)));
                        }
                    }
                    if idx + 1 < self.messages.len() {
                        lines.push(Line::from(Span::raw("")));
                    }
                }
            }
        }

        lines.push(Line::from(Span::raw("")));
        lines
    }
}

// ── Style helpers ─────────────────────────────────────────────────────────────

fn role_badge_style(role: &ChatRole, t: &Theme) -> (&'static str, Style) {
    let dark = Color::Rgb(10, 10, 10);
    match role {
        ChatRole::User => (
            " you ",
            Style::default()
                .fg(dark)
                .bg(t.user_badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        ChatRole::Assistant => (
            " assistant ",
            Style::default()
                .fg(dark)
                .bg(t.secondary)
                .add_modifier(Modifier::BOLD),
        ),
        ChatRole::System => (
            " system ",
            Style::default()
                .fg(dark)
                .bg(t.system_badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        ChatRole::Permission => (
            " permission ",
            Style::default()
                .fg(dark)
                .bg(t.primary)
                .add_modifier(Modifier::BOLD),
        ),
    }
}

fn default_content_style(role: &ChatRole, t: &Theme) -> Style {
    match role {
        ChatRole::User | ChatRole::Assistant => Style::default().fg(t.text),
        ChatRole::System => Style::default().fg(t.text_muted),
        ChatRole::Permission => Style::default().fg(t.primary),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_messages_returns_placeholder() {
        let app = App::new();
        let lines = app.render_lines();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn single_message_produces_badge_and_content() {
        let mut app = App::new();
        app.push_user_message("hello".into());
        let lines = app.render_lines();
        assert_eq!(lines.len(), 3); // badge + content + trailing blank
    }

    #[test]
    fn multiline_message_produces_one_line_per_content_line() {
        let mut app = App::new();
        app.push_user_message("line1\nline2\nline3".into());
        let lines = app.render_lines();
        assert_eq!(lines.len(), 5); // badge(1) + 3 lines + trailing(1)
    }

    #[test]
    fn two_messages_have_separator_between_them() {
        let mut app = App::new();
        app.push_user_message("hi".into());
        app.push_system_message("sys".into());
        let lines = app.render_lines();
        assert_eq!(lines.len(), 6);
    }
}
