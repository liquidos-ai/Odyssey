use async_trait::async_trait;
use odyssey_rs_memory::{
    MemoryCompactionPolicy, MemoryError, MemoryProvider, MemoryRecallOptions, MemoryRecord,
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct StubMemory {
    recall_records: Vec<MemoryRecord>,
    initial_records: Option<Vec<MemoryRecord>>,
}

impl StubMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_recall(recall_records: Vec<MemoryRecord>) -> Self {
        Self {
            recall_records,
            initial_records: None,
        }
    }

    pub fn with_initial(initial_records: Vec<MemoryRecord>) -> Self {
        Self {
            recall_records: Vec::new(),
            initial_records: Some(initial_records),
        }
    }

    pub fn with_records(
        recall_records: Vec<MemoryRecord>,
        initial_records: Option<Vec<MemoryRecord>>,
    ) -> Self {
        Self {
            recall_records,
            initial_records,
        }
    }
}

#[async_trait]
impl MemoryProvider for StubMemory {
    async fn store(&self, _record: MemoryRecord) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn recall(
        &self,
        _session_id: Uuid,
        _query: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        Ok(self.recall_records.clone())
    }

    async fn recall_initial(
        &self,
        _query: Option<&str>,
        _limit: usize,
        _options: MemoryRecallOptions,
    ) -> Result<Option<Vec<MemoryRecord>>, MemoryError> {
        Ok(self.initial_records.clone())
    }

    async fn compact(
        &self,
        _session_id: Uuid,
        _policy: &MemoryCompactionPolicy,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        Ok(None)
    }
}
