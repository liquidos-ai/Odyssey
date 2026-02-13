//! Memory provider implementations and policy enforcement.

use crate::error::MemoryError;
use crate::model::MemoryRecord;
use crate::policy::{MemoryCapturePolicy, MemoryCompactionPolicy};
use crate::recall::{MemoryRecallMode, MemoryRecallOptions};
use async_trait::async_trait;
use chrono::Utc;
use log::{debug, info};
use regex::Regex;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[async_trait]
/// Memory provider abstraction used by the orchestrator.
pub trait MemoryProvider: Send + Sync {
    /// Store a memory record without applying capture policy.
    async fn store(&self, record: MemoryRecord) -> Result<(), MemoryError>;

    /// Store a memory record after applying capture policy.
    async fn store_with_policy(
        &self,
        record: MemoryRecord,
        _policy: &MemoryCapturePolicy,
    ) -> Result<bool, MemoryError> {
        self.store(record).await?;
        Ok(true)
    }

    /// Recall memory records for a session.
    async fn recall(
        &self,
        session_id: Uuid,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Recall with options (mode selection, scoring).
    async fn recall_with_options(
        &self,
        session_id: Uuid,
        query: Option<&str>,
        limit: usize,
        options: MemoryRecallOptions,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        if matches!(options.mode, MemoryRecallMode::Text) {
            return self.recall(session_id, query, limit).await;
        }
        self.recall(session_id, query, limit).await
    }

    /// Recall global memory records for system prompt assembly.
    async fn recall_initial(
        &self,
        query: Option<&str>,
        limit: usize,
        options: MemoryRecallOptions,
    ) -> Result<Option<Vec<MemoryRecord>>, MemoryError> {
        let _ = (query, limit, options);
        Ok(None)
    }

    /// Compact memory for a session if supported.
    async fn compact(
        &self,
        _session_id: Uuid,
        _policy: &MemoryCompactionPolicy,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        Ok(None)
    }
}

/// File-backed memory provider storing JSONL records per session.
#[derive(Debug, Clone)]
pub struct FileMemoryProvider {
    /// Root directory for memory records.
    root: PathBuf,
}

impl FileMemoryProvider {
    /// Create a new file-backed provider under the given root.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        info!("initialized file memory provider (root={})", root.display());
        Ok(Self { root })
    }

    /// Path to the session JSONL file.
    fn session_path(&self, session_id: Uuid) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    /// Path to the temporary session file.
    fn temp_path(&self, session_id: Uuid) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl.tmp"))
    }

    /// Load all records for a session.
    fn load_records(&self, session_id: Uuid) -> Result<Vec<MemoryRecord>, MemoryError> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new().read(true).open(path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: MemoryRecord = serde_json::from_str(&line)?;
            records.push(record);
        }
        Ok(records)
    }

    /// Rewrite a session's records atomically.
    fn write_records(&self, session_id: Uuid, records: &[MemoryRecord]) -> Result<(), MemoryError> {
        let path = self.session_path(session_id);
        let temp_path = self.temp_path(session_id);
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temp_path)?;
            for record in records {
                let line = serde_json::to_string(record)?;
                writeln!(file, "{line}")?;
            }
        }
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(temp_path, path)?;
        Ok(())
    }
}

#[async_trait]
impl MemoryProvider for FileMemoryProvider {
    /// Store a record by appending to the session file.
    async fn store(&self, record: MemoryRecord) -> Result<(), MemoryError> {
        let path = self.session_path(record.session_id);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let line = serde_json::to_string(&record)?;
        writeln!(file, "{line}")?;
        debug!(
            "stored memory record (session_id={}, role={}, content_len={})",
            record.session_id,
            record.role,
            record.content.len()
        );
        Ok(())
    }

    /// Store a record after applying capture policy.
    async fn store_with_policy(
        &self,
        record: MemoryRecord,
        policy: &MemoryCapturePolicy,
    ) -> Result<bool, MemoryError> {
        let Some(record) = apply_capture_policy(record, policy)? else {
            return Ok(false);
        };
        self.store(record).await?;
        Ok(true)
    }

    /// Recall memory records matching a query.
    async fn recall(
        &self,
        session_id: Uuid,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let mut records = self.load_records(session_id)?;
        if let Some(query) = query {
            records.retain(|record| record.content.contains(query));
        }
        let start = records.len().saturating_sub(limit);
        debug!(
            "recall memory (session_id={}, returned={})",
            session_id,
            records.len().saturating_sub(start)
        );
        Ok(records[start..].to_vec())
    }

    /// Compact memory records based on policy.
    async fn compact(
        &self,
        session_id: Uuid,
        policy: &MemoryCompactionPolicy,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        if !policy.enabled {
            return Ok(None);
        }
        let mut records = self.load_records(session_id)?;
        if records.is_empty() {
            return Ok(None);
        }

        let mut needs_compaction = records.len() > policy.max_messages;
        if let Some(max_total) = policy.max_total_chars {
            let total_chars: usize = records
                .iter()
                .map(|record| record.content.chars().count())
                .sum();
            needs_compaction = needs_compaction || total_chars > max_total;
        }
        if !needs_compaction {
            return Ok(None);
        }

        let mut removed = Vec::new();
        if policy.max_messages == 0 {
            removed.append(&mut records);
        } else if records.len() > policy.max_messages {
            let split = records.len().saturating_sub(policy.max_messages);
            removed.extend(records.drain(..split));
        }

        if let Some(max_total) = policy.max_total_chars {
            let mut total_chars: usize = records
                .iter()
                .map(|record| record.content.chars().count())
                .sum();
            while total_chars > max_total && !records.is_empty() {
                let record = records.remove(0);
                total_chars = total_chars.saturating_sub(record.content.chars().count());
                removed.push(record);
            }
        }

        let summary = build_summary_record(session_id, &removed, policy.summary_max_chars);
        let mut next_records = Vec::new();
        if let Some(summary_record) = summary.clone() {
            next_records.push(summary_record);
        }
        next_records.extend(records);
        self.write_records(session_id, &next_records)?;
        info!(
            "memory compacted (session_id={}, removed={}, remaining={})",
            session_id,
            removed.len(),
            next_records.len()
        );
        Ok(summary)
    }
}

/// Apply capture policy to a record, returning None if filtered.
fn apply_capture_policy(
    record: MemoryRecord,
    policy: &MemoryCapturePolicy,
) -> Result<Option<MemoryRecord>, MemoryError> {
    let kind = record
        .metadata
        .get("kind")
        .and_then(serde_json::Value::as_str);
    let is_tool_output = kind == Some("tool_output");
    if is_tool_output && !policy.capture_tool_output {
        return Ok(None);
    }
    if !is_tool_output && !policy.capture_messages {
        return Ok(None);
    }

    if !policy.deny_patterns.is_empty() {
        let mut deny_patterns = Vec::new();
        for pattern in &policy.deny_patterns {
            let regex = Regex::new(pattern).map_err(|err| MemoryError::Regex(err.to_string()))?;
            deny_patterns.push(regex);
        }
        if deny_patterns
            .iter()
            .any(|regex| regex.is_match(&record.content))
        {
            return Ok(None);
        }
    }

    let mut content = record.content;

    if !policy.redact_patterns.is_empty() {
        for pattern in &policy.redact_patterns {
            let regex = Regex::new(pattern).map_err(|err| MemoryError::Regex(err.to_string()))?;
            content = regex
                .replace_all(&content, policy.redaction_replacement.as_str())
                .to_string();
        }
    }

    if policy.detect_secrets {
        content = redact_high_entropy(
            &content,
            policy.secret_entropy_threshold,
            policy.redaction_replacement.as_str(),
        );
    }

    let max_chars = if is_tool_output {
        policy.max_tool_output_chars.or(policy.max_message_chars)
    } else {
        policy.max_message_chars
    };
    if let Some(max_chars) = max_chars {
        content = truncate_chars(&content, max_chars);
    }

    Ok(Some(MemoryRecord { content, ..record }))
}

/// Truncate a string to a maximum character count.
fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

/// Build a summary record for compacted entries.
fn build_summary_record(
    session_id: Uuid,
    removed: &[MemoryRecord],
    summary_max_chars: usize,
) -> Option<MemoryRecord> {
    if removed.is_empty() || summary_max_chars == 0 {
        return None;
    }
    let mut summary_parts = Vec::new();
    for record in removed {
        let snippet = format!("{}: {}", record.role, record.content);
        summary_parts.push(snippet);
    }
    let summary_text = summary_parts.join("\n");
    let summary_text = truncate_chars(&summary_text, summary_max_chars);
    if summary_text.trim().is_empty() {
        return None;
    }
    Some(MemoryRecord {
        id: Uuid::new_v4(),
        session_id,
        role: "system".to_string(),
        content: summary_text,
        metadata: serde_json::json!({
            "summary": true,
            "count": removed.len()
        }),
        created_at: Utc::now(),
    })
}

/// Redact high-entropy tokens that resemble secrets.
fn redact_high_entropy(content: &str, threshold: f32, replacement: &str) -> String {
    let Ok(regex) = Regex::new(r"[A-Za-z0-9+/=]{20,}") else {
        return content.to_string();
    };
    regex
        .replace_all(content, |caps: &regex::Captures<'_>| {
            let token = caps.get(0).map_or("", |m| m.as_str());
            if shannon_entropy(token) >= threshold {
                replacement.to_string()
            } else {
                token.to_string()
            }
        })
        .to_string()
}

/// Calculate Shannon entropy for a token string.
fn shannon_entropy(token: &str) -> f32 {
    let mut counts = [0usize; 256];
    let bytes = token.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    for byte in bytes {
        counts[*byte as usize] += 1;
    }
    let len = bytes.len() as f32;
    let mut entropy = 0.0;
    for count in counts.iter().copied().filter(|count| *count > 0) {
        let p = count as f32 / len;
        entropy -= p * p.log2();
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::{
        FileMemoryProvider, MemoryProvider, apply_capture_policy, redact_high_entropy,
        truncate_chars,
    };
    use crate::{MemoryCapturePolicy, MemoryCompactionPolicy, MemoryRecord};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn base_record(content: &str) -> MemoryRecord {
        MemoryRecord {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            role: "user".to_string(),
            content: content.to_string(),
            metadata: json!({}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn capture_policy_skips_tool_output_when_disabled() {
        let mut record = base_record("tool output");
        record.metadata = json!({ "kind": "tool_output" });
        let policy = MemoryCapturePolicy {
            capture_messages: true,
            capture_tool_output: false,
            ..MemoryCapturePolicy::default()
        };
        let filtered = apply_capture_policy(record, &policy).expect("policy");
        assert_eq!(filtered, None);
    }

    #[test]
    fn capture_policy_redacts_and_truncates() {
        let record = base_record("token-1234");
        let policy = MemoryCapturePolicy {
            redact_patterns: vec!["token".to_string()],
            redaction_replacement: "REDACTED".to_string(),
            max_message_chars: Some(5),
            detect_secrets: false,
            ..MemoryCapturePolicy::default()
        };
        let filtered = apply_capture_policy(record, &policy)
            .expect("policy")
            .expect("record");
        assert_eq!(filtered.content, "REDAC");
    }

    #[test]
    fn capture_policy_detects_secrets_with_low_threshold() {
        let record = base_record("token ABCDEFGHIJKLMNOPQRSTUVWX");
        let policy = MemoryCapturePolicy {
            detect_secrets: true,
            secret_entropy_threshold: 0.1,
            redaction_replacement: "[X]".to_string(),
            ..MemoryCapturePolicy::default()
        };
        let filtered = apply_capture_policy(record, &policy)
            .expect("policy")
            .expect("record");
        assert!(filtered.content.contains("[X]"));
    }

    #[test]
    fn truncate_chars_handles_limits() {
        assert_eq!(truncate_chars("hello", 0), "");
        assert_eq!(truncate_chars("hello", 3), "hel");
        assert_eq!(truncate_chars("hello", 10), "hello");
    }

    #[tokio::test]
    async fn compact_rewrites_session_with_summary() {
        let temp = tempdir().expect("tempdir");
        let provider = FileMemoryProvider::new(temp.path()).expect("provider");
        let session_id = Uuid::new_v4();

        let record_a = MemoryRecord {
            session_id,
            ..base_record("one")
        };
        let record_b = MemoryRecord {
            session_id,
            ..base_record("two")
        };
        let record_c = MemoryRecord {
            session_id,
            ..base_record("three")
        };

        provider.store(record_a).await.expect("store a");
        provider.store(record_b).await.expect("store b");
        provider.store(record_c.clone()).await.expect("store c");

        let policy = MemoryCompactionPolicy {
            enabled: true,
            max_messages: 1,
            summary_max_chars: 200,
            max_total_chars: None,
        };
        let summary = provider
            .compact(session_id, &policy)
            .await
            .expect("compact")
            .expect("summary");

        assert_eq!(summary.metadata["summary"], json!(true));
        assert_eq!(summary.metadata["count"], json!(2));

        let records = provider.recall(session_id, None, 10).await.expect("recall");
        assert_eq!(records.len(), 2);
        assert_eq!(records[1], record_c);
    }

    #[test]
    fn redact_high_entropy_uses_replacement() {
        let redacted = redact_high_entropy("ABCDEFGHIJKLMNOPQRSTUVWX", 0.1, "[X]");
        assert_eq!(redacted, "[X]");
    }
}
