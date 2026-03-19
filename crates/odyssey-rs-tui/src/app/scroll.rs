//! Scroll-state management for the chat view and the viewer overlay.

use crate::app::state::App;
use std::cmp::min;

impl App {
    // ── Chat view scrolling ──────────────────────────────────────────────────

    /// Scroll the chat view upward by `lines` and disable auto-scroll.
    pub fn scroll_up(&mut self, lines: u16) {
        self.auto_scroll = false;
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Scroll the chat view downward by `lines`.
    ///
    /// Re-enables auto-scroll when the view reaches the bottom.
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll = min(self.scroll.saturating_add(lines), self.chat_max_scroll);
        if self.scroll >= self.chat_max_scroll {
            self.auto_scroll = true;
        }
    }

    /// Jump to the very top of the chat and disable auto-scroll.
    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        self.scroll = 0;
    }

    /// Pin the view to the bottom and enable auto-scroll.
    pub fn enable_auto_scroll(&mut self) {
        self.auto_scroll = true;
        self.scroll = self.chat_max_scroll;
    }

    /// Recalculate scroll bounds after a layout change.
    ///
    /// The view only snaps to the new bottom when auto-scroll is active
    /// **or** the user was already pinned to the exact previous bottom.
    pub fn update_scroll_bounds(&mut self, max_scroll: u16) {
        let was_at_bottom = self.scroll >= self.chat_max_scroll;
        self.chat_max_scroll = max_scroll;
        if self.auto_scroll || was_at_bottom {
            self.scroll = max_scroll;
            self.auto_scroll = true;
        } else {
            self.scroll = self.scroll.min(max_scroll);
        }
    }

    // ── Viewer overlay scrolling ─────────────────────────────────────────────

    /// Scroll the viewer upward by `lines`.
    pub fn viewer_scroll_up(&mut self, lines: u16) {
        self.viewer_scroll = self.viewer_scroll.saturating_sub(lines);
    }

    /// Scroll the viewer downward by `lines`, clamped to `viewer_max_scroll`.
    pub fn viewer_scroll_down(&mut self, lines: u16) {
        self.viewer_scroll = min(
            self.viewer_scroll.saturating_add(lines),
            self.viewer_max_scroll,
        );
    }

    /// Recalculate viewer scroll bounds and clamp the current offset.
    pub fn update_viewer_scroll_bounds(&mut self, max_scroll: u16) {
        self.viewer_max_scroll = max_scroll;
        self.viewer_scroll = self.viewer_scroll.min(max_scroll);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> App {
        App {
            chat_max_scroll: 100,
            ..App::default()
        }
    }

    #[test]
    fn scroll_up_is_clamped_at_zero() {
        let mut app = make_app();
        app.scroll = 5;
        app.scroll_up(10);
        assert_eq!(app.scroll, 0);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn scroll_down_is_clamped_at_max() {
        let mut app = make_app();
        app.scroll = 90;
        app.scroll_down(20);
        assert_eq!(app.scroll, 100);
        assert!(app.auto_scroll);
    }

    #[test]
    fn scroll_to_top_disables_auto_scroll() {
        let mut app = make_app();
        app.auto_scroll = true;
        app.scroll = 50;
        app.scroll_to_top();
        assert_eq!(app.scroll, 0);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn enable_auto_scroll_pins_to_bottom() {
        let mut app = make_app();
        app.auto_scroll = false;
        app.scroll = 20;
        app.enable_auto_scroll();
        assert_eq!(app.scroll, 100);
        assert!(app.auto_scroll);
    }

    #[test]
    fn update_scroll_bounds_snaps_when_auto_scroll_on() {
        let mut app = make_app();
        app.auto_scroll = true;
        app.scroll = 100;
        app.update_scroll_bounds(150);
        assert_eq!(app.scroll, 150);
        assert!(app.auto_scroll);
    }

    #[test]
    fn update_scroll_bounds_preserves_position_when_scrolled_up() {
        let mut app = make_app();
        app.auto_scroll = false;
        app.scroll = 40;
        app.chat_max_scroll = 100;
        app.update_scroll_bounds(150);
        assert_eq!(app.scroll, 40); // not snapped
        assert!(!app.auto_scroll);
    }

    #[test]
    fn update_scroll_bounds_snaps_when_at_exact_bottom() {
        let mut app = make_app();
        app.auto_scroll = false;
        app.scroll = 100;
        app.chat_max_scroll = 100;
        app.update_scroll_bounds(150);
        assert_eq!(app.scroll, 150);
        assert!(app.auto_scroll);
    }

    #[test]
    fn viewer_scroll_up_clamped_at_zero() {
        let mut app = App {
            viewer_scroll: 3,
            ..App::default()
        };
        app.viewer_scroll_up(10);
        assert_eq!(app.viewer_scroll, 0);
    }

    #[test]
    fn viewer_scroll_down_clamped_at_max() {
        let mut app = App {
            viewer_max_scroll: 20,
            ..App::default()
        };
        app.viewer_scroll_down(50);
        assert_eq!(app.viewer_scroll, 20);
    }
}
