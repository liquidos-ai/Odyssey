use directories::BaseDirs;
use odyssey_rs_protocol::SandboxMode;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub cache_root: PathBuf,
    pub session_root: PathBuf,
    pub sandbox_root: PathBuf,
    pub bind_addr: String,
    pub sandbox_mode_override: Option<SandboxMode>,
    pub hub_url: String,
    pub worker_count: usize,
    pub queue_capacity: usize,
}

impl RuntimeConfig {
    pub fn from_default_dirs() -> Self {
        let dirs = BaseDirs::new().expect("base dirs");
        let root = dirs.home_dir().join(".odyssey");
        Self {
            cache_root: root.join("bundles"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:8472".to_string(),
            sandbox_mode_override: None,
            hub_url: "http://127.0.0.1:8473".to_string(), //TODO: Default should be to the actual
            //URL
            worker_count: 4,
            queue_capacity: 128,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let dirs = BaseDirs::new().expect("base dirs");
        let root = dirs.home_dir().join(".odyssey");
        Self {
            cache_root: root.join("bundles"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:8472".to_string(),
            sandbox_mode_override: None,
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 4,
            queue_capacity: 128,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_dirs_point_into_odyssey_home() {
        let config = RuntimeConfig::from_default_dirs();

        assert!(config.cache_root.ends_with(".odyssey/bundles"));
        assert!(config.session_root.ends_with(".odyssey/sessions"));
        assert!(config.sandbox_root.ends_with(".odyssey/sandbox"));
        assert_eq!(config.bind_addr, "127.0.0.1:8472");
        assert_eq!(config.hub_url, "http://127.0.0.1:8473");
        assert_eq!(config.worker_count, 4);
        assert_eq!(config.queue_capacity, 128);
        assert!(config.sandbox_mode_override.is_none());
    }

    #[test]
    fn default_impl_matches_default_dirs() {
        let config = RuntimeConfig::default();
        let from_dirs = RuntimeConfig::from_default_dirs();

        assert_eq!(config.cache_root, from_dirs.cache_root);
        assert_eq!(config.session_root, from_dirs.session_root);
        assert_eq!(config.sandbox_root, from_dirs.sandbox_root);
        assert_eq!(config.bind_addr, from_dirs.bind_addr);
        assert_eq!(config.hub_url, from_dirs.hub_url);
        assert_eq!(config.worker_count, from_dirs.worker_count);
        assert_eq!(config.queue_capacity, from_dirs.queue_capacity);
    }
}
