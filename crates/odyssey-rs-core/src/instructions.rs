//! Instruction discovery helpers for system prompts.

use crate::error::OdysseyCoreError;
use log::debug;
use std::path::{Path, PathBuf};

/// Bundle of instruction content and source paths.
#[derive(Debug, Clone)]
pub struct InstructionBundle {
    /// Combined instruction content.
    pub content: String,
    /// Source files contributing to the content.
    pub sources: Vec<PathBuf>,
}

/// Resolve instruction roots from configured paths and the current working directory.
pub fn resolve_instruction_roots(roots: &[String], cwd: &Path) -> Vec<PathBuf> {
    if roots.is_empty() {
        return vec![cwd.to_path_buf()];
    }
    roots
        .iter()
        .map(|root| {
            let path = PathBuf::from(root);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .collect()
}

/// Discover instruction files under the given roots.
pub fn discover_instructions(roots: &[PathBuf]) -> Result<InstructionBundle, OdysseyCoreError> {
    let file_order = ["ODYSSEY.md", "AGENTS.md", "CLAUDE.md"];
    let mut sources = Vec::new();
    let mut contents = Vec::new();

    for root in roots {
        if !root.exists() {
            continue;
        }
        for filename in file_order {
            let path = root.join(filename);
            if path.is_file() {
                let content = std::fs::read_to_string(&path)?;
                if !content.trim().is_empty() {
                    contents.push(content);
                }
                sources.push(path);
            }
        }
    }

    debug!(
        "instruction discovery complete (roots={}, sources={})",
        roots.len(),
        sources.len()
    );
    Ok(InstructionBundle {
        content: contents.join("\n\n"),
        sources,
    })
}

/// Normalize and validate an instruction root path.
pub fn normalize_root(root: impl AsRef<Path>) -> Result<PathBuf, OdysseyCoreError> {
    let root = root.as_ref();
    if root.exists() {
        Ok(root.to_path_buf())
    } else {
        Err(OdysseyCoreError::Parse(format!(
            "invalid instruction root: {}",
            root.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, contents).expect("write file");
    }

    #[test]
    fn resolve_instruction_roots_defaults_to_cwd() {
        let temp = tempdir().expect("tempdir");
        let roots: Vec<String> = Vec::new();
        let resolved = resolve_instruction_roots(&roots, temp.path());
        assert_eq!(resolved, vec![temp.path().to_path_buf()]);
    }

    #[test]
    fn resolve_instruction_roots_handles_relative_and_absolute() {
        let temp = tempdir().expect("tempdir");
        let absolute = temp.path().join("abs");
        let roots = vec![
            "relative".to_string(),
            absolute.to_string_lossy().to_string(),
        ];
        let resolved = resolve_instruction_roots(&roots, temp.path());
        assert_eq!(resolved[0], temp.path().join("relative"));
        assert_eq!(resolved[1], absolute);
    }

    #[test]
    fn discover_instructions_respects_order() {
        let root_a = tempdir().expect("root_a");
        let root_b = tempdir().expect("root_b");
        write_file(&root_a.path().join("ODYSSEY.md"), "odyssey");
        write_file(&root_a.path().join("AGENTS.md"), "agents");
        write_file(&root_b.path().join("CLAUDE.md"), "claude");

        let bundle =
            discover_instructions(&[root_a.path().to_path_buf(), root_b.path().to_path_buf()])
                .expect("bundle");

        assert_eq!(bundle.content, "odyssey\n\nagents\n\nclaude");
        assert_eq!(bundle.sources.len(), 3);
        assert_eq!(bundle.sources[0], root_a.path().join("ODYSSEY.md"));
        assert_eq!(bundle.sources[1], root_a.path().join("AGENTS.md"));
        assert_eq!(bundle.sources[2], root_b.path().join("CLAUDE.md"));
    }

    #[test]
    fn normalize_root_rejects_missing_paths() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let err = normalize_root(&missing).expect_err("should fail");
        match err {
            OdysseyCoreError::Parse(message) => {
                assert!(message.contains("invalid instruction root"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
