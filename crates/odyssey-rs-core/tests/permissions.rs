//! Tests for permission engine behavior.

use odyssey_rs_config::{
    PathAccess, PermissionAction, PermissionMode, PermissionRule, PermissionsConfig,
};
use odyssey_rs_core::PermissionEngine;
use odyssey_rs_protocol::PermissionRequest;
use odyssey_rs_tools::{PermissionChecker, PermissionContext};
use pretty_assertions::assert_eq;
use uuid::Uuid;

/// Plan mode should block tool execution by default.
#[tokio::test]
async fn plan_mode_blocks_tools() {
    let config = PermissionsConfig {
        mode: PermissionMode::Plan,
        rules: Vec::new(),
    };
    let engine = PermissionEngine::new(config).expect("engine");
    let ctx = PermissionContext {
        session_id: Uuid::nil(),
        agent_id: "agent".to_string(),
        tool_name: None,
        turn_id: None,
    };

    let outcome = engine
        .authorize(
            &ctx,
            PermissionRequest::Tool {
                name: "Read".to_string(),
            },
        )
        .await
        .expect("outcome");

    assert_eq!(outcome.allowed, false);
    assert_eq!(
        outcome.reason.as_deref(),
        Some("plan mode blocks tool execution")
    );
}

/// Allow rules should override default mode decisions.
#[tokio::test]
async fn allow_rule_overrides_default() {
    let config = PermissionsConfig {
        mode: PermissionMode::Default,
        rules: vec![PermissionRule {
            action: PermissionAction::Allow,
            tool: Some("Read".to_string()),
            path: None,
            command: None,
            access: None,
        }],
    };
    let engine = PermissionEngine::new(config).expect("engine");
    let ctx = PermissionContext {
        session_id: Uuid::nil(),
        agent_id: "agent".to_string(),
        tool_name: None,
        turn_id: None,
    };

    let outcome = engine
        .authorize(
            &ctx,
            PermissionRequest::Tool {
                name: "Read".to_string(),
            },
        )
        .await
        .expect("outcome");

    assert_eq!(outcome.allowed, true);
    assert_eq!(outcome.reason, None);
}

/// Allowed tools should implicitly allow their path checks unless explicitly denied.
#[tokio::test]
async fn tool_allow_implies_path_allow() {
    let config = PermissionsConfig {
        mode: PermissionMode::Default,
        rules: vec![PermissionRule {
            action: PermissionAction::Allow,
            tool: Some("Read".to_string()),
            path: None,
            command: None,
            access: None,
        }],
    };
    let engine = PermissionEngine::new(config).expect("engine");
    let ctx = PermissionContext {
        session_id: Uuid::nil(),
        agent_id: "agent".to_string(),
        tool_name: Some("Read".to_string()),
        turn_id: None,
    };

    let outcome = engine
        .authorize(
            &ctx,
            PermissionRequest::Path {
                path: "README.md".to_string(),
                mode: PathAccess::Read,
            },
        )
        .await
        .expect("outcome");

    assert_eq!(outcome.allowed, true);
}

/// Explicit deny rules should override implicit tool allow.
#[tokio::test]
async fn deny_path_overrides_tool_allow() {
    let config = PermissionsConfig {
        mode: PermissionMode::Default,
        rules: vec![
            PermissionRule {
                action: PermissionAction::Allow,
                tool: Some("Write".to_string()),
                path: None,
                command: None,
                access: None,
            },
            PermissionRule {
                action: PermissionAction::Deny,
                tool: None,
                path: Some("secret.txt".to_string()),
                command: None,
                access: Some(PathAccess::Write),
            },
        ],
    };
    let engine = PermissionEngine::new(config).expect("engine");
    let ctx = PermissionContext {
        session_id: Uuid::nil(),
        agent_id: "agent".to_string(),
        tool_name: Some("Write".to_string()),
        turn_id: None,
    };

    let outcome = engine
        .authorize(
            &ctx,
            PermissionRequest::Path {
                path: "secret.txt".to_string(),
                mode: PathAccess::Write,
            },
        )
        .await
        .expect("outcome");

    assert_eq!(outcome.allowed, false);
    assert_eq!(outcome.reason.as_deref(), Some("denied by rule"));
}
