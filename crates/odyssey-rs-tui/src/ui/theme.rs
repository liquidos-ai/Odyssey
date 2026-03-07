//! Theme definitions for the Odyssey TUI.
//!
//! Each theme is a `const` value of [`Theme`].  The active theme is stored on
//! [`App`](crate::app::App) and read by every widget at render time.

use ratatui::style::Color;

// ── Theme struct ──────────────────────────────────────────────────────────────

/// A complete UI color palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Human-readable identifier used in slash commands and the viewer.
    pub name: &'static str,
    /// Primary accent – used for highlights, active borders, and the banner.
    pub primary: Color,
    /// Secondary accent – used for assistant badge and mild highlights.
    pub secondary: Color,
    /// Default body text color.
    pub text: Color,
    /// Dimmed text for labels, hints, and inactive elements.
    pub text_muted: Color,
    /// Default border color.
    pub border: Color,
    /// Border color when the widget is focused / active.
    pub border_active: Color,
    /// Warning / accent color (CPU bar medium, status line warnings, etc.).
    pub accent: Color,
    /// Background for popup panels (slash palette, viewers).
    pub bg_popup: Color,
    /// Background for the selected row inside a popup.
    pub bg_selected: Color,
    /// Badge background for user messages.
    pub user_badge_bg: Color,
    /// Badge background for system messages.
    pub system_badge_bg: Color,
}

// ── Bundled themes ────────────────────────────────────────────────────────────

/// Default Odyssey dark theme (orange / red).
pub const ODYSSEY: Theme = Theme {
    name: "odyssey",
    primary: Color::Rgb(236, 91, 43),
    secondary: Color::Rgb(238, 121, 72),
    text: Color::Rgb(238, 238, 238),
    text_muted: Color::Rgb(128, 128, 128),
    border: Color::Rgb(60, 60, 60),
    border_active: Color::Rgb(238, 121, 72),
    accent: Color::Rgb(229, 192, 123),
    bg_popup: Color::Rgb(20, 20, 20),
    bg_selected: Color::Rgb(40, 30, 25),
    user_badge_bg: Color::Rgb(107, 161, 230),
    system_badge_bg: Color::Rgb(60, 60, 60),
};

/// Atom One Dark.
pub const ONE_DARK: Theme = Theme {
    name: "one-dark",
    primary: Color::Rgb(97, 175, 239),
    secondary: Color::Rgb(152, 195, 121),
    text: Color::Rgb(171, 178, 191),
    text_muted: Color::Rgb(92, 99, 112),
    border: Color::Rgb(62, 68, 81),
    border_active: Color::Rgb(97, 175, 239),
    accent: Color::Rgb(229, 192, 123),
    bg_popup: Color::Rgb(33, 37, 43),
    bg_selected: Color::Rgb(44, 49, 58),
    user_badge_bg: Color::Rgb(97, 175, 239),
    system_badge_bg: Color::Rgb(62, 68, 81),
};

/// Dracula.
pub const DRACULA: Theme = Theme {
    name: "dracula",
    primary: Color::Rgb(189, 147, 249),
    secondary: Color::Rgb(80, 250, 123),
    text: Color::Rgb(248, 248, 242),
    text_muted: Color::Rgb(98, 114, 164),
    border: Color::Rgb(68, 71, 90),
    border_active: Color::Rgb(189, 147, 249),
    accent: Color::Rgb(241, 250, 140),
    bg_popup: Color::Rgb(40, 42, 54),
    bg_selected: Color::Rgb(68, 71, 90),
    user_badge_bg: Color::Rgb(80, 250, 123),
    system_badge_bg: Color::Rgb(68, 71, 90),
};

/// Nord.
pub const NORD: Theme = Theme {
    name: "nord",
    primary: Color::Rgb(136, 192, 208),
    secondary: Color::Rgb(143, 188, 187),
    text: Color::Rgb(236, 239, 244),
    text_muted: Color::Rgb(76, 86, 106),
    border: Color::Rgb(59, 66, 82),
    border_active: Color::Rgb(136, 192, 208),
    accent: Color::Rgb(235, 203, 139),
    bg_popup: Color::Rgb(46, 52, 64),
    bg_selected: Color::Rgb(59, 66, 82),
    user_badge_bg: Color::Rgb(129, 161, 193),
    system_badge_bg: Color::Rgb(67, 76, 94),
};

/// Gruvbox Dark.
pub const GRUVBOX: Theme = Theme {
    name: "gruvbox",
    primary: Color::Rgb(254, 128, 25),
    secondary: Color::Rgb(184, 187, 38),
    text: Color::Rgb(235, 219, 178),
    text_muted: Color::Rgb(146, 131, 116),
    border: Color::Rgb(80, 73, 69),
    border_active: Color::Rgb(254, 128, 25),
    accent: Color::Rgb(250, 189, 47),
    bg_popup: Color::Rgb(29, 32, 33),
    bg_selected: Color::Rgb(60, 56, 54),
    user_badge_bg: Color::Rgb(131, 165, 152),
    system_badge_bg: Color::Rgb(80, 73, 69),
};

/// Catppuccin Mocha.
pub const CATPPUCCIN: Theme = Theme {
    name: "catppuccin",
    primary: Color::Rgb(203, 166, 247),
    secondary: Color::Rgb(166, 227, 161),
    text: Color::Rgb(205, 214, 244),
    text_muted: Color::Rgb(108, 112, 134),
    border: Color::Rgb(49, 50, 68),
    border_active: Color::Rgb(203, 166, 247),
    accent: Color::Rgb(249, 226, 175),
    bg_popup: Color::Rgb(30, 30, 46),
    bg_selected: Color::Rgb(49, 50, 68),
    user_badge_bg: Color::Rgb(137, 180, 250),
    system_badge_bg: Color::Rgb(49, 50, 68),
};

/// All bundled themes in display order.
pub const AVAILABLE_THEMES: &[Theme] = &[ODYSSEY, ONE_DARK, DRACULA, NORD, GRUVBOX, CATPPUCCIN];
