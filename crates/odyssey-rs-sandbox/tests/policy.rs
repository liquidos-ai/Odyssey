//! Sandbox policy enforcement tests.

use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, LocalSandboxProvider, SandboxContext, SandboxPolicy,
    SandboxProvider,
};
use pretty_assertions::assert_eq;
use tempfile::tempdir;

/// Workspace write mode should block paths outside the workspace.
#[tokio::test]
async fn workspace_mode_blocks_external_paths() {
    let temp = tempdir().expect("tempdir");
    let ctx = SandboxContext {
        workspace_root: temp.path().to_path_buf(),
        mode: SandboxMode::WorkspaceWrite,
        policy: SandboxPolicy::default(),
    };
    let provider = LocalSandboxProvider::new();
    let handle = provider.prepare(&ctx).await.expect("prepare");

    let inside = temp.path().join("file.txt");
    let outside = tempdir().expect("tempdir").path().join("outside.txt");

    assert_eq!(
        provider.check_access(&handle, &inside, AccessMode::Read),
        AccessDecision::Allow
    );
    assert!(matches!(
        provider.check_access(&handle, &outside, AccessMode::Write),
        AccessDecision::Deny(_)
    ));
}

/// Explicit read roots should restrict access to allowed paths.
#[tokio::test]
async fn read_roots_restrict_access() {
    let temp = tempdir().expect("tempdir");
    let allow_path = temp.path().join("allowed");
    std::fs::create_dir_all(&allow_path).expect("create allow dir");

    let mut policy = SandboxPolicy::default();
    policy
        .filesystem
        .read_roots
        .push(allow_path.to_string_lossy().to_string());

    let ctx = SandboxContext {
        workspace_root: temp.path().to_path_buf(),
        mode: SandboxMode::ReadOnly,
        policy,
    };
    let provider = LocalSandboxProvider::new();
    let handle = provider.prepare(&ctx).await.expect("prepare");

    let allowed = allow_path.join("file.txt");
    let denied = temp.path().join("other.txt");

    assert_eq!(
        provider.check_access(&handle, &allowed, AccessMode::Read),
        AccessDecision::Allow
    );
    assert!(matches!(
        provider.check_access(&handle, &denied, AccessMode::Write),
        AccessDecision::Deny(_)
    ));
}
