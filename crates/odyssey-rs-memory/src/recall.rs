//! Memory recall configuration.

/// Recall modes supported by memory providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRecallMode {
    /// Text-only recall.
    Text,
    /// Vector-only recall.
    Vector,
    /// Hybrid recall combining text and vector.
    Hybrid,
}

/// Recall options for memory retrieval.
#[derive(Debug, Clone, Copy)]
pub struct MemoryRecallOptions {
    /// Recall mode to use.
    pub mode: MemoryRecallMode,
    /// Optional minimum score filter.
    pub min_score: Option<f32>,
}

impl Default for MemoryRecallOptions {
    /// Default recall options.
    fn default() -> Self {
        Self {
            mode: MemoryRecallMode::Text,
            min_score: None,
        }
    }
}
