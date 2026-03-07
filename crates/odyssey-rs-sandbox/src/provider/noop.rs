//! Explicitly unsafe no-op sandbox provider.

use crate::provider::local::HostExecProvider;

/// Backwards-compatible no-op name for callers that opt into unsafe host execution.
pub type NoSandboxProvider = HostExecProvider;
