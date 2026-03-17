use crate::RuntimeError;
use std::fs;
use std::path::Path;

pub(crate) fn create_dir_all(path: &Path) -> Result<(), RuntimeError> {
    fs::create_dir_all(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

pub(crate) fn write_string(path: &Path, contents: &str) -> Result<(), RuntimeError> {
    fs::write(path, contents).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}
