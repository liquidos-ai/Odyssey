//! IO helpers for reading config layers from disk.

use super::{
    ConfigLayer, ConfigLayerSource, DEFAULT_CONFIG_DIR, DEFAULT_CONFIG_FILE, LoadedLayer,
    SYSTEM_CONFIG_PATH, SYSTEM_REQUIREMENTS_PATH, SchemaMode, schema,
};
use crate::ConfigError;
use directories::UserDirs;
use log::debug;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Load an optional layer if the provided path exists.
pub(super) fn load_optional_layer(
    source: ConfigLayerSource,
    path: Option<&Path>,
) -> Result<Option<LoadedLayer>, ConfigError> {
    let path = match path {
        Some(path) => path,
        None => return Ok(None),
    };

    if !path.exists() {
        debug!(
            "optional layer missing (source={:?}, path={})",
            source,
            path.display()
        );
        return Ok(None);
    }

    Ok(Some(load_required_layer(source, path)?))
}

/// Load and validate a required layer from disk.
pub(super) fn load_required_layer(
    source: ConfigLayerSource,
    path: &Path,
) -> Result<LoadedLayer, ConfigError> {
    debug!(
        "loading config layer (source={:?}, path={})",
        source,
        path.display()
    );
    let contents = fs::read_to_string(path)?;
    let value: Value = json5::from_str(&contents)?;
    let label = layer_label(source, path);
    schema::validate_layer_schema(&value, SchemaMode::Partial, &label)?;
    Ok(LoadedLayer {
        meta: ConfigLayer {
            source,
            path: Some(path.to_path_buf()),
            disabled_reason: None,
        },
        value,
    })
}

/// Build a user-friendly label for schema validation errors.
pub(super) fn layer_label(source: ConfigLayerSource, path: &Path) -> String {
    let name = match source {
        ConfigLayerSource::Requirements => "requirements",
        ConfigLayerSource::System => "system",
        ConfigLayerSource::User => "user",
        ConfigLayerSource::Project => "project",
        ConfigLayerSource::Cwd => "cwd",
        ConfigLayerSource::Repo => "repo",
        ConfigLayerSource::Runtime => "runtime",
    };
    format!("{name}({})", path.display())
}

/// Default system config path on Unix; None elsewhere.
pub(super) fn default_system_config_path() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        Some(PathBuf::from(SYSTEM_CONFIG_PATH))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Default requirements path on Unix; None elsewhere.
pub(super) fn default_requirements_path() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        Some(PathBuf::from(SYSTEM_REQUIREMENTS_PATH))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Default user config path under the home directory.
pub(super) fn default_user_config_path() -> Option<PathBuf> {
    UserDirs::new().map(|dirs| {
        dirs.home_dir()
            .join(DEFAULT_CONFIG_DIR)
            .join(DEFAULT_CONFIG_FILE)
    })
}
