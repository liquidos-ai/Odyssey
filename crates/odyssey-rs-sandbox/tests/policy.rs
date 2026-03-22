use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, LocalSandboxProvider, SandboxContext, SandboxPolicy,
    SandboxProvider,
};
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[tokio::test]
async fn host_provider_rejects_workspace_mode() {
    let temp = tempdir().expect("tempdir");
    let ctx = SandboxContext {
        workspace_root: temp.path().to_path_buf(),
        mode: SandboxMode::WorkspaceWrite,
        policy: SandboxPolicy::default(),
    };
    let provider = LocalSandboxProvider::default();
    let error = provider
        .prepare(&ctx)
        .await
        .expect_err("workspace mode rejected");
    assert!(error.to_string().contains("danger_full_access"));
}

#[tokio::test]
async fn danger_full_access_still_allows_access_checks() {
    let temp = tempdir().expect("tempdir");
    let ctx = SandboxContext {
        workspace_root: temp.path().to_path_buf(),
        mode: SandboxMode::DangerFullAccess,
        policy: SandboxPolicy::default(),
    };
    let provider = LocalSandboxProvider::default();
    let handle = provider.prepare(&ctx).await.expect("prepare");

    let allowed = temp.path().join("file.txt");
    let denied = tempdir().expect("outside").path().join("outside.txt");

    assert_eq!(
        provider.check_access(&handle, &allowed, AccessMode::Read),
        AccessDecision::Allow
    );
    assert_eq!(
        provider.check_access(&handle, &denied, AccessMode::Write),
        AccessDecision::Allow
    );
}
