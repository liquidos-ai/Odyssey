//! Persistent storage for approval decisions.

use crate::error::OdysseyCoreError;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use log::warn;
use odyssey_rs_protocol::ApprovalDecision;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const PERMISSION_FILENAME: &str = "permission.jsonl";

#[derive(Debug, Serialize, Deserialize)]
struct ApprovalRecord {
    workspace_root: String,
    request_key: String,
    decision: ApprovalDecision,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) struct ApprovalStore {
    workspace_root: String,
    path: PathBuf,
    cache: HashMap<String, ApprovalDecision>,
}

impl ApprovalStore {
    pub(crate) fn load_default(workspace_root: &Path) -> Result<Self, OdysseyCoreError> {
        let path = default_permission_path()?;
        match Self::load(workspace_root, path.clone()) {
            Ok(store) => Ok(store),
            Err(err) => {
                let path_display = path.display();
                warn!("failed to load approval store (path={path_display}): {err}");
                let workspace_root = canonical_workspace_root(workspace_root)?;
                Ok(Self {
                    workspace_root,
                    path,
                    cache: HashMap::new(),
                })
            }
        }
    }

    pub(crate) fn load(workspace_root: &Path, path: PathBuf) -> Result<Self, OdysseyCoreError> {
        let workspace_root = canonical_workspace_root(workspace_root)?;
        let cache = load_cached_approvals(&path, &workspace_root)?;
        Ok(Self {
            workspace_root,
            path,
            cache,
        })
    }

    pub(crate) fn lookup(&self, key: &str) -> Option<ApprovalDecision> {
        self.cache.get(key).copied()
    }

    pub(crate) fn record_allow_always(&mut self, key: String) -> Result<(), OdysseyCoreError> {
        if self.cache.contains_key(&key) {
            return Ok(());
        }
        let record = ApprovalRecord {
            workspace_root: self.workspace_root.clone(),
            request_key: key.clone(),
            decision: ApprovalDecision::AllowAlways,
            created_at: Utc::now(),
        };
        let serialized = serde_json::to_string(&record)
            .map_err(|err| OdysseyCoreError::Parse(err.to_string()))?;
        if let Some(parent) = self.path.parent() {
            create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{serialized}")?;
        self.cache.insert(key, ApprovalDecision::AllowAlways);
        Ok(())
    }
}

fn canonical_workspace_root(root: &Path) -> Result<String, OdysseyCoreError> {
    let canonical = root.canonicalize().map_err(OdysseyCoreError::Io)?;
    Ok(canonical.to_string_lossy().to_string())
}

fn default_permission_path() -> Result<PathBuf, OdysseyCoreError> {
    let cwd = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
    if let Some(home) = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        return Ok(home.join(".odyssey").join(PERMISSION_FILENAME));
    }
    Ok(cwd.join(".odyssey").join(PERMISSION_FILENAME))
}

fn load_cached_approvals(
    path: &Path,
    workspace_root: &str,
) -> Result<HashMap<String, ApprovalDecision>, OdysseyCoreError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(err) => return Err(OdysseyCoreError::Io(err)),
    };

    let reader = BufReader::new(file);
    let mut cache = HashMap::new();
    for line in reader.lines() {
        let line = line.map_err(OdysseyCoreError::Io)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<ApprovalRecord>(trimmed) {
            Ok(record) => {
                if record.workspace_root != workspace_root {
                    continue;
                }
                if record.decision == ApprovalDecision::AllowAlways {
                    cache.insert(record.request_key, record.decision);
                }
            }
            Err(err) => {
                warn!("invalid approval record ignored: {err}");
            }
        }
    }
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::{ApprovalRecord, ApprovalStore, canonical_workspace_root};
    use chrono::Utc;
    use odyssey_rs_protocol::ApprovalDecision;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn allow_always_persists_for_workspace() {
        let workspace = tempdir().expect("workspace");
        let store_path = workspace.path().join("permission.jsonl");
        let mut store = ApprovalStore::load(workspace.path(), store_path.clone()).expect("store");

        store
            .record_allow_always("tool:Read".to_string())
            .expect("record");

        let store = ApprovalStore::load(workspace.path(), store_path).expect("store reload");
        assert_eq!(
            store.lookup("tool:Read"),
            Some(ApprovalDecision::AllowAlways)
        );
    }

    #[test]
    fn ignores_invalid_or_other_workspace_records() {
        let workspace_a = tempdir().expect("workspace_a");
        let workspace_b = tempdir().expect("workspace_b");
        let file_dir = tempdir().expect("file_dir");
        let store_path = file_dir.path().join("permission.jsonl");
        let record = ApprovalRecord {
            workspace_root: canonical_workspace_root(workspace_a.path()).expect("workspace root"),
            request_key: "tool:Read".to_string(),
            decision: ApprovalDecision::AllowAlways,
            created_at: Utc::now(),
        };
        let serialized = serde_json::to_string(&record).expect("serialize");
        std::fs::write(&store_path, format!("not-json\n{serialized}\n")).expect("write file");

        let store = ApprovalStore::load(workspace_b.path(), store_path).expect("store");
        assert_eq!(store.lookup("tool:Read"), None);
    }
}
