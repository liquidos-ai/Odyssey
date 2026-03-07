//! Markdown → ratatui `Line` renderer using pulldown-cmark.
//!
//! Call [`render_markdown`] to convert a markdown string into a `Vec<Line<'static>>`
//! suitable for use with a ratatui `Paragraph` widget.

use crate::ui::theme::Theme;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Convert `text` (assumed to be Markdown) into styled ratatui lines.
///
/// `base_style` is the default style applied to plain text spans.
pub fn render_markdown(text: &str, theme: &Theme, base_style: Style) -> Vec<Line<'static>> {
    let mut r = MdRenderer::new(theme, base_style);
    let opts = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;
    for event in Parser::new_ext(text, opts) {
        r.handle(event);
    }
    r.finish()
}

// ── Renderer state ────────────────────────────────────────────────────────────

struct MdRenderer<'t> {
    theme: &'t Theme,
    base_style: Style,
    lines: Vec<Line<'static>>,
    /// Spans accumulated for the current logical line / paragraph.
    current_spans: Vec<Span<'static>>,
    /// Stack of inline styles (bold, italic, links, headings…).
    style_stack: Vec<Style>,
    /// Stack of list types: `None` = unordered, `Some(n)` = ordered starting at n.
    list_stack: Vec<Option<u64>>,
    /// Whether we are currently inside a fenced / indented code block.
    in_code_block: bool,
    /// Bullet / number prefix waiting to be prepended to the first span of a list item.
    pending_item_prefix: Option<String>,
}

impl<'t> MdRenderer<'t> {
    fn new(theme: &'t Theme, base_style: Style) -> Self {
        Self {
            theme,
            base_style,
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![base_style],
            list_stack: Vec::new(),
            in_code_block: false,
            pending_item_prefix: None,
        }
    }

    fn cur_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or(self.base_style)
    }

    fn push_style(&mut self, s: Style) {
        self.style_stack.push(s);
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    /// Move accumulated spans into a new `Line`.  No-op when spans are empty.
    fn flush_line(&mut self) {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            self.lines.push(Line::from(spans));
        }
    }

    fn blank_line(&mut self) {
        self.lines.push(Line::from(Span::raw("")));
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            // ── Headings ──────────────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                self.flush_line();
                let style = self.heading_style(level);
                self.push_style(style);
                // Visual prefix makes heading level obvious in terminal
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    _ => "#### ",
                };
                self.current_spans
                    .push(Span::styled(prefix, self.cur_style()));
            }
            Event::End(TagEnd::Heading(_)) => {
                self.pop_style();
                self.flush_line();
                self.blank_line();
            }

            // ── Paragraphs ────────────────────────────────────────────────────
            Event::Start(Tag::Paragraph) => {
                self.apply_pending_item_prefix();
            }
            Event::End(TagEnd::Paragraph) => {
                self.flush_line();
                // Only add blank line between top-level paragraphs, not inside list items
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }

            // ── Lists ─────────────────────────────────────────────────────────
            Event::Start(Tag::List(start)) => {
                self.flush_line();
                self.list_stack.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }
            Event::Start(Tag::Item) => {
                self.flush_line();
                let depth = self.list_stack.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let prefix = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let s = format!("{indent}{n}. ");
                        *n += 1;
                        s
                    }
                    _ => format!("{indent}• "),
                };
                self.pending_item_prefix = Some(prefix);
            }
            Event::End(TagEnd::Item) => {
                // Tight list items: text arrives directly without a Paragraph wrapper,
                // so the prefix may still be pending (empty item) or spans may be buffered.
                self.apply_pending_item_prefix();
                self.flush_line();
            }

            // ── Code blocks ───────────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                self.flush_line();
                // Show language tag for fenced blocks
                if let CodeBlockKind::Fenced(lang) = &kind
                    && !lang.is_empty()
                {
                    let lang_style = Style::default()
                        .fg(self.theme.text_muted)
                        .add_modifier(Modifier::ITALIC);
                    self.lines
                        .push(Line::from(Span::styled(format!(" {lang}"), lang_style)));
                }
                self.in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                self.in_code_block = false;
                self.blank_line();
            }

            // ── Block quotes ──────────────────────────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                self.flush_line();
                let style = Style::default()
                    .fg(self.theme.text_muted)
                    .add_modifier(Modifier::ITALIC);
                self.push_style(style);
                self.current_spans
                    .push(Span::styled("│ ", self.cur_style()));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.pop_style();
                self.flush_line();
                self.blank_line();
            }

            // ── Horizontal rule ───────────────────────────────────────────────
            Event::Rule => {
                self.flush_line();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(self.theme.border),
                )));
                self.blank_line();
            }

            // ── Inline: bold / italic / strikethrough ─────────────────────────
            Event::Start(Tag::Strong) => {
                let s = self.cur_style().add_modifier(Modifier::BOLD);
                self.push_style(s);
            }
            Event::End(TagEnd::Strong) => self.pop_style(),

            Event::Start(Tag::Emphasis) => {
                let s = self.cur_style().add_modifier(Modifier::ITALIC);
                self.push_style(s);
            }
            Event::End(TagEnd::Emphasis) => self.pop_style(),

            Event::Start(Tag::Strikethrough) => {
                let s = self.cur_style().add_modifier(Modifier::CROSSED_OUT);
                self.push_style(s);
            }
            Event::End(TagEnd::Strikethrough) => self.pop_style(),

            // ── Inline: links ─────────────────────────────────────────────────
            Event::Start(Tag::Link { .. }) => {
                let s = self
                    .cur_style()
                    .fg(self.theme.primary)
                    .add_modifier(Modifier::UNDERLINED);
                self.push_style(s);
            }
            Event::End(TagEnd::Link) => self.pop_style(),

            // ── Inline code ───────────────────────────────────────────────────
            Event::Code(code) => {
                self.apply_pending_item_prefix();
                let style = Style::default()
                    .fg(self.theme.accent)
                    .bg(self.theme.bg_popup);
                self.current_spans
                    .push(Span::styled(code.into_string(), style));
            }

            // ── Text ──────────────────────────────────────────────────────────
            Event::Text(text) => {
                if self.in_code_block {
                    let code_style = Style::default().fg(self.theme.accent);
                    let s = text.into_string();
                    // Code block text typically ends with '\n'; trim it.
                    let trimmed = s.trim_end_matches('\n');
                    for line in trimmed.split('\n') {
                        self.lines
                            .push(Line::from(Span::styled(line.to_owned(), code_style)));
                    }
                } else {
                    self.apply_pending_item_prefix();
                    let style = self.cur_style();
                    self.current_spans
                        .push(Span::styled(text.into_string(), style));
                }
            }

            // ── Breaks ───────────────────────────────────────────────────────
            Event::SoftBreak => {
                // Soft breaks become a space; Paragraph handles wrapping.
                let style = self.cur_style();
                self.current_spans.push(Span::styled(" ", style));
            }
            Event::HardBreak => {
                self.flush_line();
            }

            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        self.lines
    }

    /// If there is a pending list-item prefix, prepend it as the first span.
    fn apply_pending_item_prefix(&mut self) {
        if let Some(prefix) = self.pending_item_prefix.take() {
            let style = self.cur_style();
            self.current_spans.push(Span::styled(prefix, style));
        }
    }

    fn heading_style(&self, level: HeadingLevel) -> Style {
        let t = self.theme;
        match level {
            HeadingLevel::H1 => Style::default().fg(t.primary).add_modifier(Modifier::BOLD),
            HeadingLevel::H2 => Style::default()
                .fg(t.secondary)
                .add_modifier(Modifier::BOLD),
            HeadingLevel::H3 => Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        }
    }
}
