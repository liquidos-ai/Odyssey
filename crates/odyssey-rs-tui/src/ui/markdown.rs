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
            Event::Start(tag) => self.handle_start(tag),
            Event::End(tag) => self.handle_end(tag),
            Event::Rule => self.handle_rule(),
            Event::Code(code) => self.handle_inline_code(code.into_string()),
            Event::Text(text) => self.handle_text(text.into_string()),
            Event::SoftBreak => self.handle_soft_break(),
            Event::HardBreak => self.flush_line(),
            _ => {}
        }
    }

    fn handle_start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => self.handle_heading_start(level),
            Tag::Paragraph => self.apply_pending_item_prefix(),
            Tag::List(start) => {
                self.flush_line();
                self.list_stack.push(start);
            }
            Tag::Item => self.handle_item_start(),
            Tag::CodeBlock(kind) => self.handle_code_block_start(kind),
            Tag::BlockQuote(_) => self.handle_block_quote_start(),
            Tag::Strong => self.push_style(self.cur_style().add_modifier(Modifier::BOLD)),
            Tag::Emphasis => self.push_style(self.cur_style().add_modifier(Modifier::ITALIC)),
            Tag::Strikethrough => {
                self.push_style(self.cur_style().add_modifier(Modifier::CROSSED_OUT))
            }
            Tag::Link { .. } => self.push_style(
                self.cur_style()
                    .fg(self.theme.primary)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            _ => {}
        }
    }

    fn handle_end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line();
                self.blank_line();
            }
            TagEnd::Paragraph => {
                self.flush_line();
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.blank_line();
                }
            }
            TagEnd::Item => {
                self.apply_pending_item_prefix();
                self.flush_line();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.pop_style();
                self.flush_line();
                self.blank_line();
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link => {
                self.pop_style();
            }
            _ => {}
        }
    }

    fn handle_heading_start(&mut self, level: HeadingLevel) {
        self.flush_line();
        self.push_style(self.heading_style(level));
        let prefix = match level {
            HeadingLevel::H1 => "# ",
            HeadingLevel::H2 => "## ",
            HeadingLevel::H3 => "### ",
            _ => "#### ",
        };
        self.current_spans
            .push(Span::styled(prefix, self.cur_style()));
    }

    fn handle_item_start(&mut self) {
        self.flush_line();
        let depth = self.list_stack.len().saturating_sub(1);
        let indent = "  ".repeat(depth);
        let prefix = match self.list_stack.last_mut() {
            Some(Some(n)) => {
                let prefix = format!("{indent}{n}. ");
                *n += 1;
                prefix
            }
            _ => format!("{indent}• "),
        };
        self.pending_item_prefix = Some(prefix);
    }

    fn handle_code_block_start(&mut self, kind: CodeBlockKind<'_>) {
        self.flush_line();
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

    fn handle_block_quote_start(&mut self) {
        self.flush_line();
        let style = Style::default()
            .fg(self.theme.text_muted)
            .add_modifier(Modifier::ITALIC);
        self.push_style(style);
        self.current_spans
            .push(Span::styled("│ ", self.cur_style()));
    }

    fn handle_rule(&mut self) {
        self.flush_line();
        self.lines.push(Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(self.theme.border),
        )));
        self.blank_line();
    }

    fn handle_inline_code(&mut self, code: String) {
        self.apply_pending_item_prefix();
        let style = Style::default()
            .fg(self.theme.accent)
            .bg(self.theme.bg_popup);
        self.current_spans.push(Span::styled(code, style));
    }

    fn handle_text(&mut self, text: String) {
        if self.in_code_block {
            self.push_code_block_lines(&text);
            return;
        }

        self.apply_pending_item_prefix();
        let style = self.cur_style();
        self.current_spans.push(Span::styled(text, style));
    }

    fn push_code_block_lines(&mut self, text: &str) {
        let code_style = Style::default().fg(self.theme.accent);
        let trimmed = text.trim_end_matches('\n');
        for line in trimmed.lines() {
            self.lines
                .push(Line::from(Span::styled(line.to_owned(), code_style)));
        }
    }

    fn handle_soft_break(&mut self) {
        let style = self.cur_style();
        self.current_spans.push(Span::styled(" ", style));
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
