use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BundleRefKind {
    Installed,
    Path,
    File,
    Remote,
    Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    pub raw: String,
    pub kind: BundleRefKind,
    pub namespace: Option<String>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub digest: Option<String>,
}

impl BundleRef {
    pub fn parse(value: &str) -> Self {
        if value.ends_with(".odybundle") {
            return Self {
                raw: value.to_string(),
                kind: BundleRefKind::File,
                namespace: None,
                id: None,
                version: None,
                digest: None,
            };
        }
        if value.starts_with('.') || value.starts_with('/') {
            return Self {
                raw: value.to_string(),
                kind: BundleRefKind::Path,
                namespace: None,
                id: None,
                version: None,
                digest: None,
            };
        }
        if let Some((repo, digest_suffix)) = value.split_once("@sha256:") {
            let (namespace, id) = parse_repo(repo);
            return Self {
                raw: value.to_string(),
                kind: BundleRefKind::Digest,
                namespace,
                id,
                version: None,
                digest: Some(format!("sha256:{digest_suffix}")),
            };
        }
        if value.contains('/') {
            let (repo, version) = if let Some((repo, version)) = value.rsplit_once('@') {
                (repo.to_string(), Some(version.to_string()))
            } else {
                match value.rsplit_once(':') {
                    Some((repo, version)) if !repo.contains("://") => {
                        (repo.to_string(), Some(version.to_string()))
                    }
                    _ => (value.to_string(), None),
                }
            };
            let (namespace, id) = parse_repo(&repo);
            return Self {
                raw: value.to_string(),
                kind: BundleRefKind::Remote,
                namespace,
                id,
                version,
                digest: None,
            };
        }

        let (id, version) = match value.split_once('@') {
            Some((id, version)) => (id.to_string(), Some(version.to_string())),
            None => (value.to_string(), None),
        };
        Self {
            raw: value.to_string(),
            kind: BundleRefKind::Installed,
            namespace: Some("local".to_string()),
            id: Some(id),
            version,
            digest: None,
        }
    }

    pub fn repository(&self) -> Option<String> {
        let namespace = self.namespace.as_ref()?;
        let id = self.id.as_ref()?;
        Some(format!("{namespace}/{id}"))
    }
}

fn parse_repo(value: &str) -> (Option<String>, Option<String>) {
    match value.split_once('/') {
        Some((namespace, id)) if !namespace.trim().is_empty() && !id.trim().is_empty() => {
            (Some(namespace.to_string()), Some(id.to_string()))
        }
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{BundleRef, BundleRefKind};
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_installed_reference_with_version() {
        let parsed = BundleRef::parse("demo@1.2.3");

        assert_eq!(parsed.kind, BundleRefKind::Installed);
        assert_eq!(parsed.namespace.as_deref(), Some("local"));
        assert_eq!(parsed.id.as_deref(), Some("demo"));
        assert_eq!(parsed.version.as_deref(), Some("1.2.3"));
        assert_eq!(parsed.repository().as_deref(), Some("local/demo"));
    }

    #[test]
    fn parse_path_and_file_references() {
        let path_ref = BundleRef::parse("./bundles/demo");
        assert_eq!(path_ref.kind, BundleRefKind::Path);
        assert_eq!(path_ref.repository(), None);

        let file_ref = BundleRef::parse("/tmp/demo.odybundle");
        assert_eq!(file_ref.kind, BundleRefKind::File);
        assert_eq!(file_ref.repository(), None);
    }

    #[test]
    fn parse_remote_reference_with_version() {
        let parsed = BundleRef::parse("team/demo:0.2.0");

        assert_eq!(parsed.kind, BundleRefKind::Remote);
        assert_eq!(parsed.namespace.as_deref(), Some("team"));
        assert_eq!(parsed.id.as_deref(), Some("demo"));
        assert_eq!(parsed.version.as_deref(), Some("0.2.0"));
        assert_eq!(parsed.digest, None);
        assert_eq!(parsed.repository().as_deref(), Some("team/demo"));
    }

    #[test]
    fn parse_namespaced_installed_reference_with_at_version() {
        let parsed = BundleRef::parse("team/demo@0.2.0");

        assert_eq!(parsed.kind, BundleRefKind::Remote);
        assert_eq!(parsed.namespace.as_deref(), Some("team"));
        assert_eq!(parsed.id.as_deref(), Some("demo"));
        assert_eq!(parsed.version.as_deref(), Some("0.2.0"));
        assert_eq!(parsed.digest, None);
        assert_eq!(parsed.repository().as_deref(), Some("team/demo"));
    }

    #[test]
    fn parse_digest_reference() {
        let parsed = BundleRef::parse("team/demo@sha256:abc123");

        assert_eq!(parsed.kind, BundleRefKind::Digest);
        assert_eq!(parsed.namespace.as_deref(), Some("team"));
        assert_eq!(parsed.id.as_deref(), Some("demo"));
        assert_eq!(parsed.version, None);
        assert_eq!(parsed.digest.as_deref(), Some("sha256:abc123"));
        assert_eq!(parsed.repository().as_deref(), Some("team/demo"));
    }

    #[test]
    fn parse_repo_rejects_empty_segments() {
        let parsed = BundleRef::parse("team/:latest");

        assert_eq!(parsed.kind, BundleRefKind::Remote);
        assert_eq!(parsed.namespace, None);
        assert_eq!(parsed.id, None);
        assert_eq!(parsed.repository(), None);
    }
}
