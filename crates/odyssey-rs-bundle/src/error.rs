use thiserror::Error;

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("manifest error: {0}")]
    Manifest(#[from] odyssey_rs_manifest::ManifestError),
    #[error("http error: {0}")]
    Http(String),
    #[error("io error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("bundle not found: {0}")]
    NotFound(String),
    #[error("invalid bundle: {0}")]
    Invalid(String),
    #[error("not implemented: {0}")]
    NotImplemented(String),
}
