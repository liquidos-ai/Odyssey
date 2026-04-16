//! Memory capture and compaction policies.

/// Policy for deciding what to capture in memory.
#[derive(Debug, Clone)]
pub struct MemoryCapturePolicy {
    /// Capture user/assistant messages.
    pub capture_messages: bool,
    /// Capture tool outputs.
    pub capture_tool_output: bool,
    /// Patterns that deny capture.
    pub deny_patterns: Vec<String>,
    /// Patterns to redact from captured content.
    pub redact_patterns: Vec<String>,
    /// Optional maximum message length to capture.
    pub max_message_chars: Option<usize>,
    /// Detect secrets using entropy heuristics.
    pub detect_secrets: bool,
    /// Entropy threshold for secret detection.
    pub secret_entropy_threshold: f32,
    /// Optional maximum tool output length to capture.
    pub max_tool_output_chars: Option<usize>,
    /// Replacement string for redactions.
    pub redaction_replacement: String,
}

impl Default for MemoryCapturePolicy {
    /// Default capture policy settings.
    fn default() -> Self {
        Self {
            capture_messages: true,
            capture_tool_output: false,
            deny_patterns: Vec::new(),
            redact_patterns: Vec::new(),
            max_message_chars: None,
            detect_secrets: true,
            secret_entropy_threshold: 3.7,
            max_tool_output_chars: None,
            redaction_replacement: "[REDACTED]".to_string(),
        }
    }
}

/// Policy for compacting long conversation histories.
#[derive(Debug, Clone)]
pub struct MemoryCompactionPolicy {
    /// Enable compaction.
    pub enabled: bool,
    /// Max messages before compaction.
    pub max_messages: usize,
    /// Max summary size in characters.
    pub summary_max_chars: usize,
    /// Optional max total character limit.
    pub max_total_chars: Option<usize>,
}

impl Default for MemoryCompactionPolicy {
    /// Default compaction policy settings.
    fn default() -> Self {
        Self {
            enabled: false,
            max_messages: 40,
            summary_max_chars: 1500,
            max_total_chars: None,
        }
    }
}
