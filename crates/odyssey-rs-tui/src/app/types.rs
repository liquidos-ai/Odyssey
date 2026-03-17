//! Core types shared across the TUI application layer.

use odyssey_rs_protocol::ApprovalDecision;
use ratatui::style::Color;
use uuid::Uuid;

/// Chat roles displayed in the UI.
#[derive(Debug, Clone)]
pub enum ChatRole {
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// System/status message.
    System,
    /// Permission prompt message.
    Permission,
}

/// Single chat entry rendered in the transcript.
#[derive(Debug, Clone)]
pub struct ChatEntry {
    /// Role that produced the message.
    pub role: ChatRole,
    /// Message content.
    pub content: String,
    /// Optional override color for the message text.
    pub color: Option<Color>,
}

/// Pending permission request displayed to the user.
#[derive(Debug, Clone)]
pub struct PendingPermission {
    /// Permission request id.
    pub request_id: Uuid,
    /// Summary text presented to the user.
    pub summary: String,
}

/// Viewer overlay types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerKind {
    Agents,
    Bundles,
    Sessions,
    Skills,
    Models,
    Themes,
}

// ── Theme-aware event colors ──────────────────────────────────────────────────

pub fn permission_color() -> Color {
    Color::Rgb(255, 153, 51)
}

pub fn tool_start_color() -> Color {
    Color::Rgb(120, 190, 255)
}

pub fn tool_success_color() -> Color {
    Color::Rgb(120, 220, 140)
}

pub fn tool_error_color() -> Color {
    Color::Rgb(255, 110, 110)
}

pub fn exec_command_color() -> Color {
    Color::Rgb(160, 200, 255)
}

pub fn exec_output_color() -> Color {
    Color::Rgb(170, 170, 170)
}

pub fn approval_color(decision: ApprovalDecision) -> Color {
    match decision {
        ApprovalDecision::AllowOnce | ApprovalDecision::AllowAlways => tool_success_color(),
        ApprovalDecision::Deny => tool_error_color(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        approval_color, exec_command_color, exec_output_color, permission_color, tool_error_color,
        tool_start_color, tool_success_color,
    };
    use odyssey_rs_protocol::ApprovalDecision;
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    #[test]
    fn event_colors_match_expected_palette() {
        assert_eq!(permission_color(), Color::Rgb(255, 153, 51));
        assert_eq!(tool_start_color(), Color::Rgb(120, 190, 255));
        assert_eq!(tool_success_color(), Color::Rgb(120, 220, 140));
        assert_eq!(tool_error_color(), Color::Rgb(255, 110, 110));
        assert_eq!(exec_command_color(), Color::Rgb(160, 200, 255));
        assert_eq!(exec_output_color(), Color::Rgb(170, 170, 170));
    }

    #[test]
    fn approval_color_follows_decision_state() {
        assert_eq!(
            approval_color(ApprovalDecision::AllowOnce),
            tool_success_color()
        );
        assert_eq!(
            approval_color(ApprovalDecision::AllowAlways),
            tool_success_color()
        );
        assert_eq!(approval_color(ApprovalDecision::Deny), tool_error_color());
    }
}
