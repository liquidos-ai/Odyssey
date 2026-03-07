//! Application state layer for the Odyssey TUI.
//!
//! The `App` struct is split across several submodules for clarity:
//!
//! - [`state`]  – struct definition, constructors, and basic setters
//! - [`scroll`] – chat-view and viewer-overlay scroll logic
//! - [`events`] – orchestrator protocol-event application
//! - [`render`] – rendering chat messages to styled ratatui lines
//! - [`types`]  – shared enums and lightweight value types

mod events;
mod render;
mod scroll;
pub mod state;
pub mod types;

pub use state::App;
#[allow(unused_imports)]
pub use types::{ChatEntry, ChatRole, PendingPermission, ViewerKind};
