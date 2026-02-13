//! Helper utilities for config loader path handling.

use crate::ConfigError;
use std::path::{Path, PathBuf};

/// Normalize a path by canonicalizing when possible, preserving NotFound.
pub(super) fn normalize_path(path: &Path) -> Result<PathBuf, ConfigError> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(err) => Err(ConfigError::ReadFailed(err)),
    }
}

/// Produce a stable unique path used for de-duplication.
pub(super) fn unique_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Walk ancestors to find a directory containing any marker entries.
pub(super) fn find_project_root(cwd: &Path, markers: &[String]) -> Option<PathBuf> {
    for ancestor in cwd.ancestors() {
        if markers.iter().any(|marker| ancestor.join(marker).exists()) {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}
