//! Memory record model used by providers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persisted memory record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    /// Record identifier.
    pub id: Uuid,
    /// Session identifier.
    pub session_id: Uuid,
    /// Role or origin for the record.
    pub role: String,
    /// Record content.
    pub content: String,
    /// Additional metadata for recall and filtering.
    pub metadata: serde_json::Value,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}
