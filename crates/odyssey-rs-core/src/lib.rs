//! Core orchestration primitives for Odyssey.
//!
//! This crate owns the orchestrator, session handling, permissions, and agent
//! runtime integration used by the server and SDK.

#[path = "orchestrator/mod.rs"]
pub mod agent_runtime;
pub mod error;
pub mod instructions;
pub mod memory;
mod permission_store;
pub mod permissions;
pub mod skills;
pub mod types;

pub mod agent;
pub mod state;
pub mod tools;

pub use agent::OdysseyAgent;
pub use agent::builder::AgentBuilder;
pub use agent_runtime::LLMEntry;
pub use agent_runtime::{
    AgentRuntime, AgentRuntimeBuilder, DEFAULT_AGENT_ID, McpStatus, RunResult, RunStream,
    SystemPromptMode,
};
/// AgentRuntime facade and default agent helpers.
pub use odyssey_rs_protocol::EventSink;
/// Permission hooks and enforcement primitives.
pub use permissions::{ApprovalHandler, HookDecision, PermissionEngine, PermissionHook};
