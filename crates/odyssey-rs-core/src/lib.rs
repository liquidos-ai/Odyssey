//! Core orchestration primitives for Odyssey.
//!
//! This crate owns the orchestrator, session handling, permissions, and agent
//! runtime integration used by the server and SDK.

pub mod error;
pub mod instructions;
pub mod orchestrator;
mod permission_store;
pub mod permissions;
pub mod skills;
pub mod types;

pub mod agent;
pub mod state;
pub mod tools;

pub use agent::OdysseyAgent;
pub use agent::builder::AgentBuilder;
/// Orchestrator facade and default agent helpers.
pub use odyssey_rs_protocol::EventSink;
pub use orchestrator::LLMEntry;
pub use orchestrator::{
    DEFAULT_AGENT_ID, Orchestrator, RunResult, RunStream, SystemPromptMode, prompt::PromptBuilder,
};
/// Permission hooks and enforcement primitives.
pub use permissions::{ApprovalHandler, HookDecision, PermissionEngine, PermissionHook};
