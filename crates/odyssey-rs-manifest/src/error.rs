use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("io error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("json5 parse error at {path}: {message}")]
    Json5Parse { path: String, message: String },
    #[error("yaml parse error at {path}: {message}")]
    YamlParse { path: String, message: String },
    #[error("invalid manifest at {path}: {message}")]
    Invalid { path: String, message: String },
}
