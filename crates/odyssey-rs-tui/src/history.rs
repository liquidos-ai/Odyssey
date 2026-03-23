//! Persistent input history stored as JSON at `~/.odyssey/history.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Maximum number of entries kept in the history file.
const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HistoryFile {
    entries: Vec<String>,
}

/// Resolve the history file path: `~/.odyssey/history.json`.
fn history_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".odyssey").join("tui_history.json"))
}

/// Load saved history entries from disk (oldest first).
pub fn load() -> Vec<String> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<HistoryFile>(&raw)
        .map(|f| f.entries)
        .unwrap_or_default()
}

/// Append a single entry and persist. Duplicates of the most recent entry are
/// skipped. The file is capped at [`MAX_ENTRIES`].
pub fn push(entry: &str) {
    if entry.trim().is_empty() {
        return;
    }
    let mut entries = load();
    // Skip if identical to the last entry.
    if entries.last().map(String::as_str) == Some(entry) {
        return;
    }
    entries.push(entry.to_string());
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    let Some(path) = history_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = HistoryFile { entries };
    let _ = serde_json::to_string_pretty(&file).map(|json| std::fs::write(&path, json));
}
