//! Layered configuration loader with trust gating and constraints.
//!
//! Discovers configuration layers (system/user/project/etc), validates schema,
//! merges them with optional constraints, and produces a final `OdysseyConfig`.

mod layer_io;
mod merge;
mod schema;
mod utils;

#[cfg(test)]
mod tests;

use crate::{ConfigError, OdysseyConfig};
use log::{debug, info, warn};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Default config filename in local layers.
const DEFAULT_CONFIG_FILE: &str = "odyssey.json5";
/// Default config directory under user or repo roots.
const DEFAULT_CONFIG_DIR: &str = ".odyssey";
/// Marker files/dirs that identify a project root.
const DEFAULT_PROJECT_ROOT_MARKERS: &[&str] = &[".git"];

#[cfg(unix)]
/// Default system config path on Unix.
const SYSTEM_CONFIG_PATH: &str = "/etc/odyssey/odyssey.json5";
#[cfg(unix)]
/// Default requirements path on Unix.
const SYSTEM_REQUIREMENTS_PATH: &str = "/etc/odyssey/requirements.json5";
#[cfg(windows)]
/// Default system config path on Windows.
const SYSTEM_CONFIG_PATH: &str = "C:\\ProgramData\\odyssey\\odyssey.json5";
#[cfg(windows)]
/// Default requirements path on Windows.
const SYSTEM_REQUIREMENTS_PATH: &str = "C:\\ProgramData\\odyssey\\requirements.json5";

/// Effective config plus metadata about which layers were loaded.
#[derive(Debug, Clone)]
pub struct LayeredConfig {
    /// The merged, validated config.
    pub config: OdysseyConfig,
    /// Metadata for each layer considered during load.
    pub layers: Vec<ConfigLayer>,
}

/// Origin for a single config layer in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayerSource {
    /// Immutable requirements constraints.
    Requirements,
    /// System-wide configuration.
    System,
    /// User-specific configuration.
    User,
    /// Project root configuration.
    Project,
    /// Current working directory configuration.
    Cwd,
    /// Repo-local configuration.
    Repo,
    /// Runtime overrides (highest precedence).
    Runtime,
}

/// Metadata about a config layer, including disabled reason when untrusted.
#[derive(Debug, Clone)]
pub struct ConfigLayer {
    /// Layer origin (system, user, runtime, etc).
    pub source: ConfigLayerSource,
    /// Location on disk if present.
    pub path: Option<PathBuf>,
    /// Reason the layer was skipped or disabled.
    pub disabled_reason: Option<String>,
}

/// Schema validation mode for layered configs.
#[derive(Debug, Clone, Copy)]
enum SchemaMode {
    /// Partial validation for non-final layers.
    Partial,
    /// Full validation for the effective config.
    Full,
}

/// Options controlling layered config discovery and overrides.
#[derive(Debug, Clone)]
pub struct LayeredConfigOptions {
    /// Working directory used to resolve relative paths and local layers.
    pub cwd: PathBuf,
    /// Optional system config path (defaults to `/etc/odyssey/odyssey.json5` on Unix).
    pub system_config_path: Option<PathBuf>,
    /// Optional user config path (defaults to `~/.odyssey/odyssey.json5`).
    pub user_config_path: Option<PathBuf>,
    /// Optional requirements/constraints path for locked settings.
    pub requirements_path: Option<PathBuf>,
    /// Runtime override config paths applied last.
    pub runtime_paths: Vec<PathBuf>,
    /// Marker files/dirs used to detect the project root.
    pub project_root_markers: Vec<String>,
}

impl LayeredConfigOptions {
    /// Create options with default layer locations for the provided cwd.
    pub fn new(cwd: impl AsRef<Path>) -> Self {
        let cwd = cwd.as_ref().to_path_buf();
        Self {
            cwd,
            system_config_path: layer_io::default_system_config_path(),
            user_config_path: layer_io::default_user_config_path(),
            requirements_path: layer_io::default_requirements_path(),
            runtime_paths: Vec::new(),
            project_root_markers: DEFAULT_PROJECT_ROOT_MARKERS
                .iter()
                .map(|marker| marker.to_string())
                .collect(),
        }
    }

    /// Add a runtime override config path that is applied last.
    pub fn with_runtime_path(mut self, path: impl AsRef<Path>) -> Self {
        self.runtime_paths.push(path.as_ref().to_path_buf());
        self
    }
}

impl OdysseyConfig {
    /// Load a single config from a path (no layering).
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        info!("loading config from path: {}", path.as_ref().display());
        let contents = fs::read_to_string(path)?;
        let value: Value = json5::from_str(&contents)?;
        config_from_value(value, "config")
    }

    /// Load a single config from JSON5 contents (no layering).
    pub fn load_from_str(contents: &str) -> Result<Self, ConfigError> {
        debug!("loading config from raw contents (len={})", contents.len());
        let value: Value = json5::from_str(contents)?;
        config_from_value(value, "config")
    }

    /// Load a layered config stack using the default layer locations.
    pub fn load_layered(cwd: impl AsRef<Path>) -> Result<LayeredConfig, ConfigError> {
        info!(
            "loading layered config with defaults (cwd={})",
            cwd.as_ref().display()
        );
        let options = LayeredConfigOptions::new(cwd);
        Self::load_layered_with_options(options)
    }

    /// Load a layered config stack using explicit layer locations and overrides.
    ///
    /// Layer precedence (low -> high): requirements (constraints), system, user,
    /// project, cwd, repo, runtime overrides.
    pub fn load_layered_with_options(
        options: LayeredConfigOptions,
    ) -> Result<LayeredConfig, ConfigError> {
        let cwd = utils::normalize_path(&options.cwd)?;
        debug!("normalized cwd for config load: {}", cwd.display());
        let mut layers = Vec::new();
        let mut merge_layers = Vec::new();
        let mut seen_paths = HashSet::new();

        let requirements = layer_io::load_optional_layer(
            ConfigLayerSource::Requirements,
            options.requirements_path.as_deref(),
        )?;
        let requirements_value = requirements.as_ref().map(|layer| layer.value.clone());
        if let Some(layer) = requirements {
            debug!("loaded requirements layer");
            layers.push(layer.meta);
        }

        for (source, path) in [
            (
                ConfigLayerSource::System,
                options.system_config_path.as_deref(),
            ),
            (ConfigLayerSource::User, options.user_config_path.as_deref()),
        ] {
            if let Some(layer) = layer_io::load_optional_layer(source, path)? {
                debug!("loaded {:?} layer", source);
                layers.push(layer.meta.clone());
                merge_layers.push(layer);
            }
        }

        let project_root = utils::find_project_root(&cwd, &options.project_root_markers);
        let repo_root = project_root.clone();
        let local_disabled_reason = None;
        if let Some(project_root) = project_root.as_ref() {
            debug!("resolved project root: {}", project_root.display());
        } else {
            debug!("project root not found; skipping project/repo layers");
        }

        let mut local_layers = Vec::new();
        if let Some(project_root) = project_root.as_ref() {
            let path = project_root.join(DEFAULT_CONFIG_FILE);
            local_layers.push(LocalLayer {
                source: ConfigLayerSource::Project,
                path,
                disabled_reason: local_disabled_reason.clone(),
            });
        }

        let cwd_layer = LocalLayer {
            source: ConfigLayerSource::Cwd,
            path: cwd.join(DEFAULT_CONFIG_FILE),
            disabled_reason: local_disabled_reason.clone(),
        };
        local_layers.push(cwd_layer);

        if let Some(repo_root) = repo_root.as_ref() {
            let path = repo_root.join(DEFAULT_CONFIG_DIR).join(DEFAULT_CONFIG_FILE);
            local_layers.push(LocalLayer {
                source: ConfigLayerSource::Repo,
                path,
                disabled_reason: local_disabled_reason.clone(),
            });
        }

        for layer in local_layers {
            load_local_layer(layer, &mut layers, &mut merge_layers, &mut seen_paths)?;
        }

        for runtime_path in &options.runtime_paths {
            let loaded = layer_io::load_required_layer(ConfigLayerSource::Runtime, runtime_path)?;
            debug!("loaded runtime layer (path={})", runtime_path.display());
            layers.push(loaded.meta.clone());
            merge_layers.push(loaded);
        }

        let mut merged = Value::Object(serde_json::Map::new());
        if let Some(requirements_value) = &requirements_value {
            merge::merge_json_values(&mut merged, requirements_value);
        }

        for layer in merge_layers {
            merge::merge_json_with_constraints(
                &mut merged,
                &layer.value,
                requirements_value.as_ref(),
            );
        }

        let config = config_from_value(merged, "effective")?;
        info!("layered config loaded (layers={})", layers.len());
        Ok(LayeredConfig { config, layers })
    }

    /// Validate configuration invariants that cannot be expressed in serde.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for rule in &self.permissions.rules {
            if rule.tool.is_none() && rule.path.is_none() && rule.command.is_none() {
                return Err(ConfigError::Invalid(
                    "permission rules require tool, path, or command".to_string(),
                ));
            }
        }

        Ok(())
    }
}

/// Internal representation of a loaded config layer.
#[derive(Debug, Clone)]
struct LoadedLayer {
    meta: ConfigLayer,
    value: Value,
}

/// Internal representation for layer candidates on disk.
#[derive(Debug, Clone)]
struct LocalLayer {
    source: ConfigLayerSource,
    path: PathBuf,
    disabled_reason: Option<String>,
}

fn config_from_value(value: Value, label: &str) -> Result<OdysseyConfig, ConfigError> {
    schema::validate_layer_schema(&value, SchemaMode::Full, label)?;
    let config: OdysseyConfig = serde_json::from_value(value)?;
    config.validate()?;
    Ok(config)
}

fn load_local_layer(
    layer: LocalLayer,
    layers: &mut Vec<ConfigLayer>,
    merge_layers: &mut Vec<LoadedLayer>,
    seen_paths: &mut HashSet<PathBuf>,
) -> Result<(), ConfigError> {
    if !layer.path.exists() {
        debug!(
            "skipping missing layer (source={:?}, path={})",
            layer.source,
            layer.path.display()
        );
        return Ok(());
    }
    let unique = utils::unique_path(&layer.path);
    if !seen_paths.insert(unique) {
        debug!(
            "skipping duplicate layer (source={:?}, path={})",
            layer.source,
            layer.path.display()
        );
        return Ok(());
    }
    if let Some(disabled_reason) = layer.disabled_reason.clone() {
        warn!(
            "layer disabled (source={:?}, path={}, reason={})",
            layer.source,
            layer.path.display(),
            disabled_reason
        );
        layers.push(ConfigLayer {
            source: layer.source,
            path: Some(layer.path),
            disabled_reason: Some(disabled_reason),
        });
        return Ok(());
    }
    let loaded = layer_io::load_required_layer(layer.source, &layer.path)?;
    debug!(
        "loaded layer (source={:?}, path={})",
        layer.source,
        layer.path.display()
    );
    layers.push(loaded.meta.clone());
    merge_layers.push(loaded);
    Ok(())
}
