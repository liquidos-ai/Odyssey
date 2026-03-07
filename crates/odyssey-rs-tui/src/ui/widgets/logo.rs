//! Reusable "odyssey" half-block pixel-art logo widget.
//!
//! ## Usage
//!
//! **Embed in a line vector** (e.g. inside a header `Paragraph`):
//! ```ignore
//! lines.extend(logo_lines(Style::default().fg(theme.primary)));
//! ```
//!
//! **Render as a standalone centred widget** (e.g. on the hero screen):
//! ```ignore
//! frame.render_widget(Logo::new(Style::default().fg(theme.primary)), area);
//! ```
//!
//! ## Font design
//!
//! Each letter is laid out on a 6-column × 6-row pixel grid.
//! Two pixel rows are packed into one terminal row using Unicode half-blocks:
//!   `' '` = neither half  `▀` = upper half  `▄` = lower half  `█` = both
//!
//! ```text
//!  o        d        y        s        e
//!  .####.   ####..   #....#   .####.   .####.
//!  #....#   #...#.   #....#   #.....   #....#
//!  #....#   #....#   .####.   #####.   ######
//!  #....#   #....#   ...#..   .#####   #.....
//!  #....#   #...#.   ...#..   .....#   .....#
//!  .####.   ####..   ...#..   .####.   .####.
//! ```
//!
//! Packed: `o=["▄▀▀▀▀▄","█    █","▀▄▄▄▄▀"]`  `d=["█▀▀▀▄ ","█    █","█▄▄▄▀ "]`
//!         `y=["█    █"," ▀▄▄▀ ","   █  "]`  `s=["▄▀▀▀▀ ","▀████▄"," ▄▄▄▄▀"]`
//!         `e=["▄▀▀▀▀▄","█▀▀▀▀▀"," ▄▄▄▄▀"]`
//!
//! "odyssey" = o d y s s e y → 7 × 6 cols + 6 × 1-col gap = **48 chars wide**.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

/// Visible width of the logo in terminal columns.
#[allow(dead_code)]
pub const LOGO_WIDTH: u16 = 48;
/// Height of the logo in terminal rows (3 rows = 6 half-block pixel rows).
pub const LOGO_HEIGHT: u16 = 3;

/// The three raw row strings that compose the logo.
pub const LOGO_ROWS: [&str; 3] = [
    "▄▀▀▀▀▄ █▀▀▀▄  █    █ ▄▀▀▀▀  ▄▀▀▀▀  ▄▀▀▀▀▄ █    █",
    "█    █ █    █  ▀▄▄▀  ▀████▄  ▀████▄ █▀▀▀▀▀  ▀▄▄▀ ",
    "▀▄▄▄▄▀ █▄▄▄▀     █    ▄▄▄▄▀  ▄▄▄▄▀  ▄▄▄▄▀    █  ",
];

/// Return the full logo as three styled [`Line`]s ready to embed in any [`Paragraph`].
///
/// The lines are not padded — callers are responsible for centering if needed.
pub fn logo_lines(style: Style) -> Vec<Line<'static>> {
    LOGO_ROWS
        .iter()
        .map(|row| Line::from(Span::styled(*row, style)))
        .collect()
}

// ── Compact variant (2 rows, 34 chars wide) ───────────────────────────────────
//
// Each letter uses a 4-column × 4-row pixel grid packed into 2 terminal rows.
//
// ```text
//  o      d      y      s      e
//  .##.   ###.   #..#   .###   .###
//  #..#   #..#   #..#   #...   ###.
//  #..#   #..#   .##.   ...#   #...
//  .##.   ###.   ..#.   ###.   .###
// ```
//
// Packed:  o=["▄▀▀▄","▀▄▄▀"]  d=["█▀▀▄","█▄▄▀"]  y=["▀▄▄▀","  █ "]
//          s=["▄▀▀▀","▄▄▄▀"]  e=["▄██▀","▀▄▄▄"]
//
// "odyssey" = o d y s s e y → 7 × 4 cols + 6 × 1-col gap = **34 chars wide**.

/// Height of the compact logo in terminal rows.
#[allow(dead_code)]
pub const LOGO_COMPACT_HEIGHT: u16 = 2;

/// The two raw row strings for the compact logo.
pub const LOGO_ROWS_COMPACT: [&str; 2] = [
    "▄▀▀▄ █▀▀▄ ▀▄▄▀ ▄▀▀▀ ▄▀▀▀ ▄██▀ ▀▄▄▀",
    "▀▄▄▀ █▄▄▀   █  ▄▄▄▀ ▄▄▄▀ ▀▄▄▄   █ ",
];

/// Return the compact 2-row logo as styled [`Line`]s.
pub fn logo_lines_compact(style: Style) -> Vec<Line<'static>> {
    LOGO_ROWS_COMPACT
        .iter()
        .map(|row| Line::from(Span::styled(*row, style)))
        .collect()
}

// ── Widget ────────────────────────────────────────────────────────────────────

/// A self-contained widget that renders the "odyssey" logo **centred** (both
/// horizontally and vertically) inside the given [`Rect`].
#[allow(dead_code)]
pub struct Logo {
    style: Style,
}

impl Logo {
    #[allow(dead_code)]
    pub fn new(style: Style) -> Self {
        Self { style }
    }
}

impl Widget for Logo {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let h_pad = area.width.saturating_sub(LOGO_WIDTH) / 2;
        let v_pad = area.height.saturating_sub(LOGO_HEIGHT) / 2;

        let lines: Vec<Line<'static>> = logo_lines(self.style)
            .into_iter()
            .map(|line| {
                let mut spans = vec![Span::raw(" ".repeat(h_pad as usize))];
                spans.extend(line.spans);
                Line::from(spans)
            })
            .collect();

        let render_area = Rect {
            y: area.y + v_pad,
            height: area.height.saturating_sub(v_pad),
            ..area
        };

        Paragraph::new(lines).render(render_area, buf);
    }
}
