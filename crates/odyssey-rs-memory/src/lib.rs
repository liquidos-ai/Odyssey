//! Memory capture and recall support for Odyssey.

pub mod error;
pub mod model;
pub mod policy;
pub mod provider;
pub mod recall;

/// Memory error type.
pub use error::MemoryError;
/// Memory record model.
pub use model::MemoryRecord;
/// Capture and compaction policies.
pub use policy::{MemoryCapturePolicy, MemoryCompactionPolicy};
/// Memory provider interface and default file implementation.
pub use provider::{FileMemoryProvider, MemoryProvider};
/// Recall modes and options.
pub use recall::{MemoryRecallMode, MemoryRecallOptions};
