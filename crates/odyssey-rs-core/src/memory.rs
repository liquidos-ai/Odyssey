//! Minimal persistent memory support for Odyssey.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MemoryCapturePolicy {
    pub capture_messages: bool,
    pub capture_tool_output: bool,
}

impl Default for MemoryCapturePolicy {
    fn default() -> Self {
        Self {
            capture_messages: true,
            capture_tool_output: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryCompactionPolicy {
    pub enabled: bool,
    pub max_messages: usize,
    pub summary_max_chars: usize,
    pub max_total_chars: Option<usize>,
}

impl Default for MemoryCompactionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_messages: 256,
            summary_max_chars: 1024,
            max_total_chars: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRecallMode {
    Text,
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryRecallOptions {
    pub mode: MemoryRecallMode,
    pub min_score: Option<f32>,
}

impl Default for MemoryRecallOptions {
    fn default() -> Self {
        Self {
            mode: MemoryRecallMode::Text,
            min_score: None,
        }
    }
}

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    async fn store(&self, record: MemoryRecord) -> Result<(), MemoryError>;

    async fn store_with_policy(
        &self,
        record: MemoryRecord,
        policy: &MemoryCapturePolicy,
    ) -> Result<bool, MemoryError> {
        if is_tool_output_record(&record) && !policy.capture_tool_output {
            return Ok(false);
        }
        if !is_tool_output_record(&record) && !policy.capture_messages {
            return Ok(false);
        }
        self.store(record).await?;
        Ok(true)
    }

    async fn recall(
        &self,
        session_id: Uuid,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;

    async fn recall_with_options(
        &self,
        session_id: Uuid,
        query: Option<&str>,
        limit: usize,
        _options: MemoryRecallOptions,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.recall(session_id, query, limit).await
    }

    async fn recall_initial(
        &self,
        _query: Option<&str>,
        _limit: usize,
        _options: MemoryRecallOptions,
    ) -> Result<Option<Vec<MemoryRecord>>, MemoryError> {
        Ok(None)
    }

    async fn compact(
        &self,
        session_id: Uuid,
        policy: &MemoryCompactionPolicy,
    ) -> Result<Option<MemoryRecord>, MemoryError>;
}

#[derive(Debug, Clone)]
enum MemoryLayout {
    SessionRoot(PathBuf),
    SharedFile(PathBuf),
}

#[derive(Debug, Clone)]
pub struct FileMemoryProvider {
    layout: MemoryLayout,
}

impl FileMemoryProvider {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let path = path.as_ref().to_path_buf();
        let layout = if looks_like_file_path(&path) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if !path.exists() {
                OpenOptions::new().create(true).append(true).open(&path)?;
            }
            MemoryLayout::SharedFile(path)
        } else {
            fs::create_dir_all(&path)?;
            MemoryLayout::SessionRoot(path)
        };
        Ok(Self { layout })
    }

    fn session_path(&self, session_id: Uuid) -> PathBuf {
        match &self.layout {
            MemoryLayout::SessionRoot(root) => root.join(format!("{session_id}.jsonl")),
            MemoryLayout::SharedFile(path) => path.clone(),
        }
    }

    fn temp_path(&self, session_id: Uuid) -> PathBuf {
        match &self.layout {
            MemoryLayout::SessionRoot(root) => root.join(format!("{session_id}.tmp")),
            MemoryLayout::SharedFile(path) => path.with_extension("tmp"),
        }
    }

    fn load_records(&self, session_id: Uuid) -> Result<Vec<MemoryRecord>, MemoryError> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new().read(true).open(&path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: MemoryRecord = serde_json::from_str(&line)?;
            if matches!(self.layout, MemoryLayout::SharedFile(_)) && record.session_id != session_id
            {
                continue;
            }
            records.push(record);
        }
        Ok(records)
    }

    fn load_all_records(&self) -> Result<Vec<MemoryRecord>, MemoryError> {
        let MemoryLayout::SharedFile(path) = &self.layout else {
            return Ok(Vec::new());
        };
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
            records.push(serde_json::from_str(&line)?);
        }
        Ok(records)
    }

    fn append_record(&self, record: &MemoryRecord) -> Result<(), MemoryError> {
        let path = self.session_path(record.session_id);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let line = serde_json::to_string(record)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    fn write_records(&self, session_id: Uuid, records: &[MemoryRecord]) -> Result<(), MemoryError> {
        match &self.layout {
            MemoryLayout::SessionRoot(_) => {
                let path = self.session_path(session_id);
                let temp_path = self.temp_path(session_id);
                write_jsonl_file(&temp_path, records)?;
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                fs::rename(temp_path, path)?;
            }
            MemoryLayout::SharedFile(path) => {
                let mut all_records = self
                    .load_all_records()?
                    .into_iter()
                    .filter(|record| record.session_id != session_id)
                    .collect::<Vec<_>>();
                all_records.extend_from_slice(records);
                all_records.sort_by_key(|record| record.created_at);
                let temp_path = self.temp_path(session_id);
                write_jsonl_file(&temp_path, &all_records)?;
                if path.exists() {
                    fs::remove_file(path)?;
                }
                fs::rename(temp_path, path)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MemoryProvider for FileMemoryProvider {
    async fn store(&self, record: MemoryRecord) -> Result<(), MemoryError> {
        self.append_record(&record)
    }

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
        if limit == 0 || records.len() <= limit {
            return Ok(records);
        }
        let start = records.len().saturating_sub(limit);
        Ok(records[start..].to_vec())
    }

    async fn compact(
        &self,
        session_id: Uuid,
        policy: &MemoryCompactionPolicy,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        if !policy.enabled {
            return Ok(None);
        }
        let mut records = self.load_records(session_id)?;
        let mut changed = false;
        if records.len() > policy.max_messages {
            let split = records.len().saturating_sub(policy.max_messages);
            records.drain(..split);
            changed = true;
        }
        if let Some(max_total_chars) = policy.max_total_chars {
            let mut total_chars = records
                .iter()
                .map(|record| record.content.len())
                .sum::<usize>();
            while total_chars > max_total_chars && !records.is_empty() {
                let removed = records.remove(0);
                total_chars = total_chars.saturating_sub(removed.content.len());
                changed = true;
            }
        }
        if changed {
            self.write_records(session_id, &records)?;
        }
        Ok(None)
    }
}

fn looks_like_file_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext, "jsonl" | "json" | "txt" | "md"))
        .unwrap_or(false)
}

fn is_tool_output_record(record: &MemoryRecord) -> bool {
    record
        .metadata
        .get("kind")
        .and_then(serde_json::Value::as_str)
        == Some("tool_output")
}

fn write_jsonl_file(path: &Path, records: &[MemoryRecord]) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    for record in records {
        let line = serde_json::to_string(record)?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FileMemoryProvider, MemoryProvider, MemoryRecord};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn record(session_id: Uuid, content: &str) -> MemoryRecord {
        MemoryRecord {
            id: Uuid::new_v4(),
            session_id,
            role: "user".to_string(),
            content: content.to_string(),
            metadata: json!({}),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn stores_per_session_under_directory_root() {
        let temp = tempdir().expect("tempdir");
        let provider = FileMemoryProvider::new(temp.path()).expect("provider");
        let session_id = Uuid::new_v4();
        provider
            .store(record(session_id, "hello"))
            .await
            .expect("store");
        let records = provider.recall(session_id, None, 10).await.expect("recall");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].content, "hello");
    }

    #[tokio::test]
    async fn shared_file_filters_by_session() {
        let temp = tempdir().expect("tempdir");
        let provider = FileMemoryProvider::new(temp.path().join("memory.jsonl")).expect("provider");
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();
        provider
            .store(record(session_a, "a1"))
            .await
            .expect("store a1");
        provider
            .store(record(session_b, "b1"))
            .await
            .expect("store b1");
        provider
            .store(record(session_a, "a2"))
            .await
            .expect("store a2");
        let records = provider.recall(session_a, None, 10).await.expect("recall");
        assert_eq!(
            records
                .iter()
                .map(|record| record.content.clone())
                .collect::<Vec<_>>(),
            vec!["a1".to_string(), "a2".to_string()]
        );
    }
}
