mod bridge;

pub(crate) use bridge::{build_permission_rules, prepare_cell};
use odyssey_rs_sandbox::{SandboxMode, SandboxRuntime, default_provider_name};

use crate::{RuntimeConfig, RuntimeError};

pub(crate) fn build_sandbox_runtime(
    config: &RuntimeConfig,
    mode: SandboxMode,
) -> Result<SandboxRuntime, RuntimeError> {
    SandboxRuntime::from_provider_name(
        Some(default_provider_name(mode)),
        mode,
        config.sandbox_root.clone(),
    )
    .map_err(RuntimeError::from)
}
