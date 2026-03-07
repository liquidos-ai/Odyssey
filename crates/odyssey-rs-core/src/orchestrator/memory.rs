//! Memory config mapping and formatting helpers.

use crate::memory::{
    MemoryCapturePolicy, MemoryCompactionPolicy, MemoryRecallOptions, MemoryRecord,
};
use odyssey_rs_config::MemoryConfig;

pub(crate) fn capture_policy_from_config(config: &MemoryConfig) -> MemoryCapturePolicy {
    MemoryCapturePolicy {
        capture_messages: true,
        capture_tool_output: config.capture_tool_output,
    }
}

pub(crate) fn compaction_policy_from_config(_config: &MemoryConfig) -> MemoryCompactionPolicy {
    MemoryCompactionPolicy::default()
}

pub(crate) fn recall_options_from_config(_config: &MemoryConfig) -> MemoryRecallOptions {
    MemoryRecallOptions::default()
}

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
