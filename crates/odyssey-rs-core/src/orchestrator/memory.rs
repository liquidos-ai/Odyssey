//! Memory config mapping and formatting helpers.

use odyssey_rs_config::MemoryRecallMode;
use odyssey_rs_memory::{
    MemoryCapturePolicy, MemoryCompactionPolicy, MemoryRecallOptions, MemoryRecord,
};

/// Translate memory capture policy from config into runtime policy.
pub(crate) fn capture_policy_from_config(
    config: &odyssey_rs_config::MemoryCapturePolicy,
) -> MemoryCapturePolicy {
    MemoryCapturePolicy {
        capture_messages: config.capture_messages,
        capture_tool_output: config.capture_tool_output,
        deny_patterns: config.deny_patterns.clone(),
        redact_patterns: config.redact_patterns.clone(),
        max_message_chars: config.max_message_chars,
        detect_secrets: config.detect_secrets,
        secret_entropy_threshold: config.secret_entropy_threshold,
        max_tool_output_chars: config.max_tool_output_chars,
        redaction_replacement: "[REDACTED]".to_string(),
    }
}

/// Translate memory compaction policy from config into runtime policy.
pub(crate) fn compaction_policy_from_config(
    config: &odyssey_rs_config::MemoryCompactionPolicy,
) -> MemoryCompactionPolicy {
    MemoryCompactionPolicy {
        enabled: config.enabled,
        max_messages: config.max_messages,
        summary_max_chars: config.summary_max_chars,
        max_total_chars: config.max_total_chars,
    }
}

/// Translate memory recall config into runtime options.
pub(crate) fn recall_options_from_config(
    config: &odyssey_rs_config::MemoryRecallConfig,
) -> MemoryRecallOptions {
    MemoryRecallOptions {
        mode: recall_mode_from_config(config.mode),
        min_score: config.min_score,
    }
}

/// Format memory records for prompt injection.
#[allow(dead_code)]
pub(crate) fn format_memory_records(records: &[MemoryRecord]) -> String {
    let mut lines = Vec::new();
    for record in records {
        let summary = record
            .metadata
            .get("summary")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let kind = record
            .metadata
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let label = if summary {
            "summary".to_string()
        } else if kind == "tool_output" {
            record
                .metadata
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .map(|tool| format!("tool:{tool}"))
                .unwrap_or_else(|| "tool".to_string())
        } else {
            record.role.clone()
        };
        lines.push(format!("{label}: {}", record.content));
    }
    lines.join("\n")
}

/// Map memory recall mode from config to runtime enum.
fn recall_mode_from_config(mode: MemoryRecallMode) -> odyssey_rs_memory::MemoryRecallMode {
    match mode {
        MemoryRecallMode::Text => odyssey_rs_memory::MemoryRecallMode::Text,
        MemoryRecallMode::Vector => odyssey_rs_memory::MemoryRecallMode::Vector,
        MemoryRecallMode::Hybrid => odyssey_rs_memory::MemoryRecallMode::Hybrid,
    }
}
