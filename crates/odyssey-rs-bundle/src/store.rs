use crate::build::{BundleArtifact, BundleMetadata};
use crate::constants::{
    BUNDLE_IMPORTS_ROOT_DIR_NAME, BUNDLE_INSTALL_LAYOUT_DIR_NAME, BUNDLE_INSTALL_ROOT_DIR_NAME,
    BUNDLE_LOCAL_NAMESPACE, BUNDLE_ODYSSEY_EXPORT_FILE_FORMAT, OCI_LAYOUT_VERSION,
};
use crate::distribution::{publish_layout, pull_layout};
use crate::layout::{
    OciImageManifest, archive_entries, archive_entry_ranges, blob_path, collect_oci_entries,
    copy_blob_into_layout, parse_config_bytes, read_blob, read_config, read_manifest,
    sha256_digest, unpack_payload,
};
use crate::{BundleBuilder, BundleError, BundleProject};
use directories::BaseDirs;
use odyssey_rs_manifest::{BundleRef, BundleRefKind};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use tempfile::{Builder as TempDirBuilder, TempDir};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleInstall {
    pub path: PathBuf,
    pub metadata: BundleMetadata,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleInstallSummary {
    pub namespace: String,
    pub id: String,
    pub version: String,
    pub path: PathBuf,
}

/// Stores installed bundles and manages bundle build, import, export, publish, and resolution flows.
#[derive(Debug, Clone)]
pub struct BundleStore {
    pub root: PathBuf,
}

impl BundleStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Initiate from user default home localtion
    pub fn from_default_location() -> Result<Self, BundleError> {
        let dirs = BaseDirs::new().ok_or_else(|| {
            BundleError::Invalid("unable to resolve odyssey home dir".to_string())
        })?;
        Ok(Self::new(dirs.home_dir().join(".odyssey").join("bundles")))
    }

    /// Build and install the bundle
    pub fn build_and_install(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<BundleInstall, BundleError> {
        //Local bundle install
        self.build_and_install_with_namespace(project_root, "local")
    }

    pub fn build_and_install_with_namespace(
        &self,
        project_root: impl AsRef<Path>,
        namespace: impl AsRef<str>,
    ) -> Result<BundleInstall, BundleError> {
        fs::create_dir_all(self.installs_root()).map_err(|err| BundleError::Io {
            path: self.installs_root().display().to_string(),
            message: err.to_string(),
        })?;
        let project = BundleProject::load(project_root.as_ref().to_path_buf())?;
        let staging_parent = self.install_staging_parent();
        let staging_root = create_temp_dir(&staging_parent, "build-")?;

        let install = (|| -> Result<BundleInstall, BundleError> {
            // Build into a staging root first, then atomically move the completed bundle
            // into its final install path once all bundle files are written successfully.
            let artifact = BundleBuilder::new(project)
                .with_namespace(namespace.as_ref())
                .build(staging_root.path())?;

            self.persist_layout_blobs(&install_layout_root(&artifact.path))?;
            let install_path = self.validated_install_path(
                &artifact.metadata.namespace,
                &artifact.metadata.id,
                &artifact.metadata.version,
            )?;
            commit_staged_install(&artifact.path, &install_path)?;

            Ok(BundleInstall {
                path: install_path,
                metadata: artifact.metadata,
            })
        })();

        finish_staged_operation(install, staging_root, &staging_parent)
    }

    pub fn resolve(&self, input: &str) -> Result<BundleInstall, BundleError> {
        let as_path = Path::new(input);
        if as_path.exists() {
            return if as_path.is_dir() {
                self.load_from_path(as_path)
            } else {
                Err(BundleError::Invalid(format!(
                    "bundle archives must be imported before use: {}",
                    as_path.display()
                )))
            };
        }

        let reference = BundleRef::parse(input);
        match reference.kind {
            BundleRefKind::Installed => self.load_installed(&reference),
            BundleRefKind::Remote => self.load_remote_install(&reference),
            BundleRefKind::Digest => self.load_by_digest(&reference),
            BundleRefKind::Path => self.load_from_path(Path::new(&reference.raw)),
            BundleRefKind::File => Err(BundleError::Invalid(
                "bundle archives must be imported before use".to_string(),
            )),
        }
    }

    pub async fn publish(
        &self,
        source: &str,
        target: &str,
        hub_url: &str,
    ) -> Result<BundleMetadata, BundleError> {
        let reference = BundleRef::parse(target);
        let source_path = Path::new(source);
        let install = if source_path.is_dir() && source_path.join("odyssey.bundle.json5").exists() {
            let namespace = reference.namespace.clone().ok_or_else(|| {
                BundleError::Invalid("publish target must include a namespace".to_string())
            })?;
            self.build_and_install_with_namespace(source_path, &namespace)?
        } else {
            self.resolve(source)?
        };
        let layout_root = metadata_root(&install.path)
            .ok_or_else(|| BundleError::NotFound(source.to_string()))?;
        publish_layout(hub_url, &layout_root, &reference).await
    }

    pub fn list_installed(&self) -> Result<Vec<BundleInstallSummary>, BundleError> {
        let installs_root = self.installs_root();
        if !installs_root.exists() {
            return Ok(Vec::new());
        }
        let mut bundles = Vec::new();
        for namespace_entry in read_dir_entries(&installs_root)? {
            bundles.extend(collect_namespace_installs(namespace_entry)?);
        }
        bundles.sort_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then(a.id.cmp(&b.id))
                .then_with(|| compare_bundle_versions(&a.version, &b.version))
        });
        Ok(bundles)
    }

    pub async fn pull(&self, reference: &str, hub_url: &str) -> Result<BundleInstall, BundleError> {
        let parsed = BundleRef::parse(reference);
        let pulled = pull_layout(hub_url, &parsed).await?;
        let install = self.install_remote_layout(
            pulled.metadata,
            pulled.index_bytes,
            pulled.manifest_bytes,
            pulled.config_bytes,
            pulled.layers,
        )?;
        Ok(install)
    }

    pub fn export(
        &self,
        reference: &str,
        output: impl AsRef<Path>,
    ) -> Result<PathBuf, BundleError> {
        let install = self.resolve(reference)?;
        let layout_root = metadata_root(&install.path)
            .ok_or_else(|| BundleError::NotFound(install.path.display().to_string()))?;
        let entries = collect_oci_entries(&layout_root)?;
        //Pack the entries into a file
        let archive = archive_entries(&entries);
        let output = output.as_ref();
        let output_path = if output.is_dir() {
            output.join(format!(
                "{}-{}{}",
                install.metadata.id, install.metadata.version, BUNDLE_ODYSSEY_EXPORT_FILE_FORMAT
            ))
        } else {
            output.to_path_buf()
        };
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| BundleError::Io {
                path: parent.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::write(&output_path, archive).map_err(|err| BundleError::Io {
            path: output_path.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(output_path)
    }

    pub fn import(&self, archive_path: impl AsRef<Path>) -> Result<BundleInstall, BundleError> {
        let archive_path = archive_path.as_ref();
        let bytes = fs::read(archive_path).map_err(|err| BundleError::Io {
            path: archive_path.display().to_string(),
            message: err.to_string(),
        })?;
        let entry_ranges = archive_entry_ranges(&bytes)?;
        let staging_parent = self.import_staging_parent();
        let staging_root = create_temp_dir(&staging_parent, "staging-")?;
        let install = (|| -> Result<BundleInstall, BundleError> {
            materialize_archive_ranges(staging_root.path(), &bytes, &entry_ranges)?;
            self.install_from_layout(staging_root.path())
        })();
        finish_staged_operation(install, staging_root, &staging_parent)
    }

    fn load_from_path(&self, path: &Path) -> Result<BundleInstall, BundleError> {
        if !has_bundle_metadata(path) {
            return Err(BundleError::NotFound(path.display().to_string()));
        }
        let metadata = read_metadata(path)?;
        Ok(BundleInstall {
            path: path.to_path_buf(),
            metadata,
        })
    }

    fn load_installed(&self, reference: &BundleRef) -> Result<BundleInstall, BundleError> {
        let namespace = reference
            .namespace
            .clone()
            .unwrap_or_else(|| BUNDLE_LOCAL_NAMESPACE.to_string());
        validate_store_component(&namespace, "bundle namespace")?;
        let id = reference
            .id
            .as_ref()
            .ok_or_else(|| BundleError::Invalid("bundle id missing".to_string()))?;
        validate_store_component(id, "bundle id")?;
        let id_dir = self.installs_root().join(&namespace).join(id);
        if !id_dir.exists() {
            return Err(BundleError::NotFound(reference.raw.clone()));
        }
        let version = reference
            .version
            .clone()
            .unwrap_or_else(|| "latest".to_string());
        if version == "latest" {
            return self
                .load_latest_installed_from_id_dir(&id_dir)?
                .ok_or_else(|| BundleError::NotFound(reference.raw.clone()));
        }
        validate_store_component(&version, "bundle version")?;
        let bundle_path = self.validated_install_path(&namespace, id, &version)?;
        load_bundle_install(&bundle_path)
    }

    fn load_latest_installed_from_id_dir(
        &self,
        id_dir: &Path,
    ) -> Result<Option<BundleInstall>, BundleError> {
        read_dir_entries(id_dir)?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && has_bundle_metadata(path))
            .try_fold(None, |latest, bundle_path| {
                let candidate = load_bundle_install(&bundle_path)?;
                Ok::<_, BundleError>(Some(select_newer_bundle_install(latest, candidate)))
            })
    }

    fn load_remote_install(&self, reference: &BundleRef) -> Result<BundleInstall, BundleError> {
        self.load_installed(reference)
    }

    fn load_by_digest(&self, reference: &BundleRef) -> Result<BundleInstall, BundleError> {
        let namespace = reference
            .namespace
            .clone()
            .ok_or_else(|| BundleError::Invalid("bundle namespace missing".to_string()))?;
        validate_store_component(&namespace, "bundle namespace")?;
        let id = reference
            .id
            .clone()
            .ok_or_else(|| BundleError::Invalid("bundle id missing".to_string()))?;
        validate_store_component(&id, "bundle id")?;
        let digest = reference
            .digest
            .clone()
            .ok_or_else(|| BundleError::Invalid("bundle digest missing".to_string()))?;
        let repo_root = self.installs_root().join(namespace).join(id);
        for entry in fs::read_dir(&repo_root).map_err(|err| BundleError::Io {
            path: repo_root.display().to_string(),
            message: err.to_string(),
        })? {
            let path = entry
                .map_err(|err| BundleError::Io {
                    path: repo_root.display().to_string(),
                    message: err.to_string(),
                })?
                .path();
            if !path.is_dir() || !has_bundle_metadata(&path) {
                continue;
            }
            let metadata = read_metadata(&path)?;
            if metadata.digest == digest {
                return Ok(BundleInstall { path, metadata });
            }
        }
        Err(BundleError::NotFound(reference.raw.clone()))
    }

    fn install_from_layout(&self, layout_root: &Path) -> Result<BundleInstall, BundleError> {
        let (_, manifest, manifest_digest) = read_manifest(layout_root)?;
        let config = read_config(layout_root, &manifest)?;
        let metadata = BundleMetadata {
            namespace: config.namespace.clone(),
            id: config.id.clone(),
            version: config.version.clone(),
            digest: manifest_digest,
            readme: config.readme.clone(),
            bundle_manifest: config.bundle_manifest.clone(),
            agent_spec: config.agent_spec.clone(),
        };
        self.persist_layout_blobs(layout_root)?;
        self.install_layout_payload(
            layout_root,
            metadata,
            manifest.config.digest,
            manifest.layers,
        )
    }

    fn install_remote_layout(
        &self,
        metadata: BundleMetadata,
        index_bytes: Vec<u8>,
        manifest_bytes: Vec<u8>,
        config_bytes: Vec<u8>,
        layers: Vec<(String, Vec<u8>)>,
    ) -> Result<BundleInstall, BundleError> {
        let validated =
            self.validate_remote_layout_metadata(metadata, &manifest_bytes, &config_bytes)?;
        let validated_layers = validate_remote_layers(&validated.manifest, layers)?;
        self.persist_remote_layout_blobs(
            &validated.manifest_digest,
            &validated.config_digest,
            &manifest_bytes,
            &config_bytes,
            &validated_layers,
        )?;
        self.materialize_remote_layout_install(
            index_bytes,
            validated.config_digest,
            validated.expected_metadata,
            validated_layers,
        )
    }

    fn validate_remote_layout_metadata(
        &self,
        metadata: BundleMetadata,
        manifest_bytes: &[u8],
        config_bytes: &[u8],
    ) -> Result<ValidatedRemoteLayout, BundleError> {
        let manifest_digest = sha256_digest(manifest_bytes);
        if metadata.digest != manifest_digest {
            return Err(BundleError::Invalid(
                "hub returned metadata inconsistent with manifest digest".to_string(),
            ));
        }
        let manifest: OciImageManifest = serde_json::from_slice(manifest_bytes)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let config_digest = sha256_digest(config_bytes);
        if manifest.config.digest != config_digest {
            return Err(BundleError::Invalid(
                "hub returned config bytes that do not match manifest digest".to_string(),
            ));
        }
        let config = parse_config_bytes(config_bytes)?;
        let expected_metadata = BundleMetadata {
            namespace: config.namespace.clone(),
            id: config.id.clone(),
            version: config.version.clone(),
            digest: manifest_digest.clone(),
            readme: config.readme.clone(),
            bundle_manifest: config.bundle_manifest.clone(),
            agent_spec: config.agent_spec.clone(),
        };
        if metadata_matches_config(&metadata, &expected_metadata) {
            return Ok(ValidatedRemoteLayout {
                manifest,
                manifest_digest,
                config_digest,
                expected_metadata,
            });
        }
        Err(BundleError::Invalid(
            "hub returned metadata inconsistent with bundle config".to_string(),
        ))
    }

    fn persist_remote_layout_blobs(
        &self,
        manifest_digest: &str,
        config_digest: &str,
        manifest_bytes: &[u8],
        config_bytes: &[u8],
        validated_layers: &[(String, Vec<u8>)],
    ) -> Result<(), BundleError> {
        self.persist_blob_bytes(manifest_digest, manifest_bytes)?;
        self.persist_blob_bytes(config_digest, config_bytes)?;
        for (digest, bytes) in validated_layers {
            self.persist_blob_bytes(digest, bytes)?;
        }
        Ok(())
    }

    fn materialize_remote_layout_install(
        &self,
        index_bytes: Vec<u8>,
        config_digest: String,
        metadata: BundleMetadata,
        validated_layers: Vec<(String, Vec<u8>)>,
    ) -> Result<BundleInstall, BundleError> {
        let install_root =
            self.validated_install_path(&metadata.namespace, &metadata.id, &metadata.version)?;
        let staging_parent = self.install_staging_parent();
        let staging_root = create_temp_dir(&staging_parent, "pull-")?;
        let install = (|| -> Result<(), BundleError> {
            let staged_install_root = staging_root.path().join("install");
            let layout_root = install_layout_root(&staged_install_root);
            fs::create_dir_all(&layout_root).map_err(|err| BundleError::Io {
                path: layout_root.display().to_string(),
                message: err.to_string(),
            })?;
            write_install_layout_files(&layout_root, index_bytes)?;
            copy_blob_into_layout(&self.root, &layout_root, &metadata.digest)?;
            copy_blob_into_layout(&self.root, &layout_root, &config_digest)?;
            for (digest, _) in &validated_layers {
                copy_blob_into_layout(&self.root, &layout_root, digest)?;
            }
            for (_, bytes) in &validated_layers {
                unpack_payload(bytes, &staged_install_root)?;
            }
            self.write_metadata(&staged_install_root, &metadata)?;
            commit_staged_install(&staged_install_root, &install_root)
        })();
        finish_staged_operation(install, staging_root, &staging_parent)?;
        Ok(BundleInstall {
            path: install_root,
            metadata,
        })
    }

    fn install_layout_payload(
        &self,
        source_layout_root: &Path,
        metadata: BundleMetadata,
        config_digest: String,
        layers: Vec<crate::layout::OciDescriptor>,
    ) -> Result<BundleInstall, BundleError> {
        let install_root =
            self.validated_install_path(&metadata.namespace, &metadata.id, &metadata.version)?;
        let staging_parent = self.install_staging_parent();
        let staging_root = create_temp_dir(&staging_parent, "install-")?;
        let install = (|| -> Result<(), BundleError> {
            let staged_install_root = staging_root.path().join("install");
            let staged_layout_root = install_layout_root(&staged_install_root);
            fs::create_dir_all(&staged_install_root).map_err(|err| BundleError::Io {
                path: staged_install_root.display().to_string(),
                message: err.to_string(),
            })?;
            fs::create_dir_all(&staged_layout_root).map_err(|err| BundleError::Io {
                path: staged_layout_root.display().to_string(),
                message: err.to_string(),
            })?;
            for relative in ["oci-layout", "index.json"] {
                let src = source_layout_root.join(relative);
                let dst = staged_layout_root.join(relative);
                fs::copy(&src, &dst).map_err(|err| BundleError::Io {
                    path: dst.display().to_string(),
                    message: err.to_string(),
                })?;
            }
            copy_blob_into_layout(&self.root, &staged_layout_root, &metadata.digest)?;
            copy_blob_into_layout(&self.root, &staged_layout_root, &config_digest)?;
            for layer in &layers {
                copy_blob_into_layout(&self.root, &staged_layout_root, &layer.digest)?;
                let bytes = read_blob(source_layout_root, &layer.digest)?;
                unpack_payload(&bytes, &staged_install_root)?;
            }
            self.write_metadata(&staged_install_root, &metadata)?;
            commit_staged_install(&staged_install_root, &install_root)
        })();
        finish_staged_operation(install, staging_root, &staging_parent)?;
        Ok(BundleInstall {
            path: install_root,
            metadata,
        })
    }

    fn persist_layout_blobs(&self, layout_root: &Path) -> Result<(), BundleError> {
        let (_, manifest, manifest_digest) = read_manifest(layout_root)?;
        let digests = std::iter::once(manifest_digest)
            .chain(std::iter::once(manifest.config.digest.clone()))
            .chain(manifest.layers.into_iter().map(|layer| layer.digest))
            .collect::<Vec<_>>();
        for digest in digests {
            let bytes = read_blob(layout_root, &digest)?;
            self.persist_blob_bytes(&digest, &bytes)?;
        }
        Ok(())
    }

    fn persist_blob_bytes(&self, digest: &str, bytes: &[u8]) -> Result<(), BundleError> {
        let actual = sha256_digest(bytes);
        if actual != digest {
            return Err(BundleError::Invalid(format!(
                "blob bytes do not match digest {digest}"
            )));
        }
        let target = blob_path(&self.root, digest)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| BundleError::Io {
                path: parent.display().to_string(),
                message: err.to_string(),
            })?;
        }
        if !target.exists() {
            fs::write(&target, bytes).map_err(|err| BundleError::Io {
                path: target.display().to_string(),
                message: err.to_string(),
            })?;
        }
        Ok(())
    }

    fn write_metadata(
        &self,
        install_root: &Path,
        metadata: &BundleMetadata,
    ) -> Result<(), BundleError> {
        let layout_root = install_layout_root(install_root);
        fs::create_dir_all(&layout_root).map_err(|err| BundleError::Io {
            path: layout_root.display().to_string(),
            message: err.to_string(),
        })?;
        fs::write(
            layout_root.join("bundle.json"),
            serde_json::to_vec_pretty(metadata)
                .map_err(|err| BundleError::Invalid(err.to_string()))?,
        )
        .map_err(|err| BundleError::Io {
            path: layout_root.join("bundle.json").display().to_string(),
            message: err.to_string(),
        })
    }

    fn installs_root(&self) -> PathBuf {
        self.root.join(BUNDLE_INSTALL_ROOT_DIR_NAME)
    }

    fn install_staging_parent(&self) -> PathBuf {
        self.installs_root().join(".tmp")
    }

    fn import_staging_parent(&self) -> PathBuf {
        self.root.join(BUNDLE_IMPORTS_ROOT_DIR_NAME)
    }

    fn validated_install_path(
        &self,
        namespace: &str,
        id: &str,
        version: &str,
    ) -> Result<PathBuf, BundleError> {
        validate_store_component(namespace, "bundle namespace")?;
        validate_store_component(id, "bundle id")?;
        validate_store_component(version, "bundle version")?;
        Ok(self.installs_root().join(namespace).join(id).join(version))
    }
}

fn safe_relative_join(root: &Path, value: &str, label: &str) -> Result<PathBuf, BundleError> {
    let relative = Path::new(value);
    if relative.is_absolute() {
        return Err(BundleError::Invalid(format!(
            "{label} must be relative: {value}"
        )));
    }

    let mut normalized = PathBuf::default();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BundleError::Invalid(format!(
                    "{label} escapes destination: {value}"
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(BundleError::Invalid(format!("{label} cannot be empty")));
    }

    Ok(root.join(normalized))
}

fn install_layout_root(install_root: &Path) -> PathBuf {
    install_root.join(BUNDLE_INSTALL_LAYOUT_DIR_NAME)
}

fn metadata_root(path: &Path) -> Option<PathBuf> {
    if path.join("bundle.json").exists() {
        Some(path.to_path_buf())
    } else {
        let nested = install_layout_root(path);
        nested.join("bundle.json").exists().then_some(nested)
    }
}

fn has_bundle_metadata(path: &Path) -> bool {
    metadata_root(path).is_some()
}

struct ValidatedRemoteLayout {
    manifest: OciImageManifest,
    manifest_digest: String,
    config_digest: String,
    expected_metadata: BundleMetadata,
}

fn metadata_matches_config(actual: &BundleMetadata, expected: &BundleMetadata) -> bool {
    actual.namespace == expected.namespace
        && actual.id == expected.id
        && actual.version == expected.version
        && actual.readme == expected.readme
        && actual.digest == expected.digest
}

fn validate_remote_layers(
    manifest: &OciImageManifest,
    layers: Vec<(String, Vec<u8>)>,
) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
    let mut layer_bytes_by_digest = BTreeMap::new();
    for (digest, bytes) in layers {
        if layer_bytes_by_digest
            .insert(digest.clone(), bytes)
            .is_some()
        {
            return Err(BundleError::Invalid(format!(
                "hub returned duplicate layer digest {digest}"
            )));
        }
    }

    let mut validated_layers = Vec::with_capacity(manifest.layers.len());
    for layer in &manifest.layers {
        let Some(bytes) = layer_bytes_by_digest.remove(&layer.digest) else {
            return Err(BundleError::Invalid(format!(
                "hub response missing layer {}",
                layer.digest
            )));
        };
        if sha256_digest(&bytes) != layer.digest {
            return Err(BundleError::Invalid(format!(
                "hub returned layer bytes that do not match digest {}",
                layer.digest
            )));
        }
        validated_layers.push((layer.digest.clone(), bytes));
    }

    if let Some(extra_digest) = layer_bytes_by_digest.keys().next() {
        return Err(BundleError::Invalid(format!(
            "hub returned unexpected layer {extra_digest}"
        )));
    }

    Ok(validated_layers)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), BundleError> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|err| BundleError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

fn remove_dir_if_empty(path: &Path) -> Result<(), BundleError> {
    if !path.exists() {
        return Ok(());
    }
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(BundleError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        }),
    }
}

fn create_temp_dir(parent: &Path, prefix: &str) -> Result<TempDir, BundleError> {
    fs::create_dir_all(parent).map_err(|err| BundleError::Io {
        path: parent.display().to_string(),
        message: err.to_string(),
    })?;
    TempDirBuilder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .map_err(|err| BundleError::Io {
            path: parent.display().to_string(),
            message: err.to_string(),
        })
}

fn close_temp_dir(temp_dir: TempDir) -> Result<(), BundleError> {
    let path = temp_dir.path().to_path_buf();
    temp_dir.close().map_err(|err| BundleError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

fn finish_staged_operation<T>(
    result: Result<T, BundleError>,
    staging_root: TempDir,
    staging_parent: &Path,
) -> Result<T, BundleError> {
    let cleanup = close_temp_dir(staging_root).and_then(|_| remove_dir_if_empty(staging_parent));
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), Ok(())) => Err(err),
        (Ok(_), Err(err)) => Err(err),
        (Err(err), Err(_)) => Err(err),
    }
}

fn reserve_temp_path(parent: &Path, prefix: &str) -> Result<PathBuf, BundleError> {
    let temp_dir = create_temp_dir(parent, prefix)?;
    let path = temp_dir.path().to_path_buf();
    close_temp_dir(temp_dir)?;
    Ok(path)
}

fn commit_staged_install(staged_root: &Path, install_root: &Path) -> Result<(), BundleError> {
    let parent = install_root.parent().ok_or_else(|| {
        BundleError::Invalid(format!(
            "bundle install path has no parent: {}",
            install_root.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|err| BundleError::Io {
        path: parent.display().to_string(),
        message: err.to_string(),
    })?;
    let backup_parent = parent.join(".tmp");
    let backup_path = if install_root.exists() {
        Some(reserve_temp_path(&backup_parent, "backup-")?)
    } else {
        None
    };
    if let Some(backup_path) = &backup_path {
        fs::rename(install_root, backup_path).map_err(|err| BundleError::Io {
            path: backup_path.display().to_string(),
            message: err.to_string(),
        })?;
    }

    match fs::rename(staged_root, install_root) {
        Ok(()) => {
            if let Some(backup_path) = &backup_path {
                remove_dir_if_exists(backup_path)?;
                remove_dir_if_empty(&backup_parent)?;
            }
            Ok(())
        }
        Err(err) => {
            let install_message = err.to_string();
            if let Some(backup_path) = &backup_path {
                fs::rename(backup_path, install_root).map_err(|restore_err| BundleError::Io {
                    path: install_root.display().to_string(),
                    message: format!(
                        "failed to install staged bundle: {install_message}; failed to restore previous install: {restore_err}"
                    ),
                })?;
                remove_dir_if_empty(&backup_parent)?;
            }
            Err(BundleError::Io {
                path: install_root.display().to_string(),
                message: install_message,
            })
        }
    }
}

fn load_bundle_install(bundle_path: &Path) -> Result<BundleInstall, BundleError> {
    let metadata = read_metadata(bundle_path)?;
    Ok(BundleInstall {
        path: bundle_path.to_path_buf(),
        metadata,
    })
}

fn materialize_archive_ranges(
    root: &Path,
    archive_bytes: &[u8],
    entries: &[(String, Range<usize>)],
) -> Result<(), BundleError> {
    for (path, range) in entries {
        let target = safe_relative_join(root, path, "bundle archive entry")?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| BundleError::Io {
                path: parent.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::write(&target, &archive_bytes[range.clone()]).map_err(|err| BundleError::Io {
            path: target.display().to_string(),
            message: err.to_string(),
        })?;
    }
    Ok(())
}

fn select_newer_bundle_install(
    current: Option<BundleInstall>,
    candidate: BundleInstall,
) -> BundleInstall {
    match current {
        Some(current)
            if compare_bundle_versions(&candidate.metadata.version, &current.metadata.version)
                == Ordering::Greater =>
        {
            candidate
        }
        Some(current) => current,
        None => candidate,
    }
}

fn read_dir_entries(path: &Path) -> Result<Vec<fs::DirEntry>, BundleError> {
    fs::read_dir(path)
        .map_err(|err| BundleError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| BundleError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })
}

fn collect_namespace_installs(
    namespace_entry: fs::DirEntry,
) -> Result<Vec<BundleInstallSummary>, BundleError> {
    let namespace_path = namespace_entry.path();
    if !namespace_path.is_dir() {
        return Ok(Vec::new());
    }
    let namespace = namespace_entry.file_name().to_string_lossy().to_string();
    let mut bundles = Vec::new();
    for id_entry in read_dir_entries(&namespace_path)? {
        bundles.extend(collect_id_installs(&namespace, id_entry)?);
    }
    Ok(bundles)
}

fn collect_id_installs(
    namespace: &str,
    id_entry: fs::DirEntry,
) -> Result<Vec<BundleInstallSummary>, BundleError> {
    let id_path = id_entry.path();
    if !id_path.is_dir() {
        return Ok(Vec::new());
    }
    let mut bundles = Vec::new();
    for version_entry in read_dir_entries(&id_path)? {
        let bundle_path = version_entry.path();
        if !bundle_path.is_dir() || !has_bundle_metadata(&bundle_path) {
            continue;
        }
        let metadata = read_metadata(&bundle_path)?;
        bundles.push(BundleInstallSummary {
            namespace: namespace.to_string(),
            id: metadata.id,
            version: metadata.version,
            path: bundle_path,
        });
    }
    Ok(bundles)
}

fn write_install_layout_files(layout_root: &Path, index_bytes: Vec<u8>) -> Result<(), BundleError> {
    let oci_layout_path = layout_root.join("oci-layout");
    fs::write(
        &oci_layout_path,
        format!("{{\"imageLayoutVersion\":\"{}\"}}\n", OCI_LAYOUT_VERSION),
    )
    .map_err(|err| BundleError::Io {
        path: oci_layout_path.display().to_string(),
        message: err.to_string(),
    })?;

    let index_path = layout_root.join("index.json");
    fs::write(&index_path, index_bytes).map_err(|err| BundleError::Io {
        path: index_path.display().to_string(),
        message: err.to_string(),
    })
}

fn validate_store_component(value: &str, label: &str) -> Result<(), BundleError> {
    let path = Path::new(value);
    if value.trim().is_empty() {
        return Err(BundleError::Invalid(format!("{label} cannot be empty")));
    }
    if path.is_absolute() {
        return Err(BundleError::Invalid(format!(
            "{label} must be a relative path component"
        )));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BundleError::Invalid(format!(
            "{label} must not contain path separators or traversal segments"
        )));
    }
    Ok(())
}

fn compare_bundle_versions(left: &str, right: &str) -> Ordering {
    match (parse_semver_like(left), parse_semver_like(right)) {
        (Some(left), Some(right)) => compare_parsed_versions(&left, &right),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => left.cmp(right),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedVersion {
    core: [u64; 3],
    prerelease: Vec<VersionIdentifier>,
}

#[derive(Debug, PartialEq, Eq)]
enum VersionIdentifier {
    Numeric(u64),
    AlphaNumeric(String),
}

fn parse_semver_like(value: &str) -> Option<ParsedVersion> {
    let (without_build, _) = value.split_once('+').unwrap_or((value, ""));
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    let mut segments = core.split('.');
    let parsed = [
        segments.next()?.parse().ok()?,
        segments.next()?.parse().ok()?,
        segments.next()?.parse().ok()?,
    ];
    if segments.next().is_some() {
        return None;
    }

    let prerelease = match prerelease {
        Some(value) => value
            .split('.')
            .map(|segment| {
                if segment.is_empty() {
                    return None;
                }
                Some(match segment.parse::<u64>() {
                    Ok(value) => VersionIdentifier::Numeric(value),
                    Err(_) => VersionIdentifier::AlphaNumeric(segment.to_string()),
                })
            })
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };

    Some(ParsedVersion {
        core: parsed,
        prerelease,
    })
}

fn compare_parsed_versions(left: &ParsedVersion, right: &ParsedVersion) -> Ordering {
    left.core
        .cmp(&right.core)
        .then_with(|| compare_prerelease(&left.prerelease, &right.prerelease))
}

fn compare_prerelease(left: &[VersionIdentifier], right: &[VersionIdentifier]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => left
            .iter()
            .zip(right)
            .map(|(left, right)| match (left, right) {
                (VersionIdentifier::Numeric(left), VersionIdentifier::Numeric(right)) => {
                    left.cmp(right)
                }
                (VersionIdentifier::Numeric(_), VersionIdentifier::AlphaNumeric(_)) => {
                    Ordering::Less
                }
                (VersionIdentifier::AlphaNumeric(_), VersionIdentifier::Numeric(_)) => {
                    Ordering::Greater
                }
                (VersionIdentifier::AlphaNumeric(left), VersionIdentifier::AlphaNumeric(right)) => {
                    left.cmp(right)
                }
            })
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or_else(|| left.len().cmp(&right.len())),
    }
}

fn read_metadata(bundle_path: &Path) -> Result<BundleMetadata, BundleError> {
    let metadata_root = metadata_root(bundle_path)
        .ok_or_else(|| BundleError::NotFound(bundle_path.display().to_string()))?;
    let bytes = fs::read(metadata_root.join("bundle.json")).map_err(|err| BundleError::Io {
        path: metadata_root.join("bundle.json").display().to_string(),
        message: err.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| BundleError::Invalid(err.to_string()))
}

#[allow(dead_code)]
fn _artifact_to_install(artifact: BundleArtifact) -> BundleInstall {
    BundleInstall {
        path: artifact.path,
        metadata: artifact.metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BundleInstallSummary, BundleStore, collect_id_installs, collect_namespace_installs,
        commit_staged_install, load_bundle_install, read_dir_entries, select_newer_bundle_install,
    };
    use crate::BundleError;
    use crate::constants::ARCHIVE_MAGIC;
    use crate::layout::{blob_path, read_blob, read_manifest};
    use crate::test_support::write_bundle_project;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn build_install_and_resolve_variants_round_trip() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");

        let by_installed = store.resolve("demo@0.1.0").expect("resolve installed");
        let by_latest = store.resolve("demo").expect("resolve latest");
        let by_path = store
            .resolve(install.path.to_str().expect("install path"))
            .expect("resolve by path");
        let by_digest = store
            .resolve(&format!("local/demo@{}", install.metadata.digest))
            .expect("resolve by digest");

        assert_eq!(by_installed.metadata.digest, install.metadata.digest);
        assert_eq!(by_latest.metadata.version, "0.1.0");
        assert_eq!(by_path.path, install.path);
        assert_eq!(by_digest.metadata.digest, install.metadata.digest);
        assert_eq!(
            fs::read_to_string(install.path.join("agent.yaml")).expect("read agent"),
            "id: demo\ndescription: test bundle\nprompt: keep responses concise\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: [\"Read\", \"Skill\"]\n"
        );
        assert_eq!(
            fs::read_to_string(
                install
                    .path
                    .join("resources")
                    .join("data")
                    .join("notes.txt")
            )
            .expect("read resource"),
            "hello world\n"
        );
        assert!(!install.path.join("bundle.json").exists());
        assert!(!install.path.join("index.json").exists());
        assert!(!install.path.join("oci-layout").exists());
        assert!(!install.path.join("blobs").exists());
        assert!(install.path.join(".odyssey").join("bundle.json").exists());
        assert!(install.path.join(".odyssey").join("index.json").exists());
        assert!(install.path.join(".odyssey").join("oci-layout").exists());
        assert!(install.path.join(".odyssey").join("blobs").exists());
    }

    #[test]
    fn list_installed_returns_sorted_summaries() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        for (name, version) in [
            ("zeta", "0.1.0"),
            ("alpha", "0.10.0"),
            ("alpha", "0.2.0"),
            ("alpha", "0.1.0"),
        ] {
            let project_root = temp.path().join(format!("{name}-{version}"));
            fs::create_dir_all(&project_root).expect("create project");
            write_bundle_project(
                &project_root,
                name,
                version,
                "data/notes.txt",
                "hello world\n",
            );
            store
                .build_and_install(&project_root)
                .expect("build and install project");
        }

        let bundles = store.list_installed().expect("list installed");
        let summaries = bundles
            .into_iter()
            .map(|bundle: BundleInstallSummary| (bundle.namespace, bundle.id, bundle.version))
            .collect::<Vec<_>>();

        assert_eq!(
            summaries,
            vec![
                (
                    "local".to_string(),
                    "alpha".to_string(),
                    "0.1.0".to_string()
                ),
                (
                    "local".to_string(),
                    "alpha".to_string(),
                    "0.2.0".to_string()
                ),
                (
                    "local".to_string(),
                    "alpha".to_string(),
                    "0.10.0".to_string()
                ),
                ("local".to_string(), "zeta".to_string(), "0.1.0".to_string()),
            ]
        );
    }

    #[test]
    fn read_dir_entries_returns_files_and_directories() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("entries");
        fs::create_dir_all(root.join("nested")).expect("create nested dir");
        fs::write(root.join("note.txt"), "hello").expect("write note");

        let mut names = read_dir_entries(&root)
            .expect("read dir entries")
            .into_iter()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(names, vec!["nested".to_string(), "note.txt".to_string()]);
    }

    #[test]
    fn collect_id_installs_ignores_entries_without_bundle_metadata() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );
        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let id_path = install.path.parent().expect("id path");
        fs::create_dir_all(id_path.join("not-a-bundle")).expect("create invalid version dir");
        fs::write(id_path.join("README.txt"), "ignore me").expect("write stray file");

        let id_entry = read_dir_entries(id_path.parent().expect("namespace path"))
            .expect("read namespace entries")
            .into_iter()
            .find(|entry| entry.path() == id_path)
            .expect("find id entry");
        let installs = collect_id_installs("local", id_entry).expect("collect id installs");

        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].namespace, "local");
        assert_eq!(installs[0].id, "demo");
        assert_eq!(installs[0].version, "0.1.0");
        assert_eq!(installs[0].path, install.path);
    }

    #[test]
    fn collect_namespace_installs_aggregates_bundle_summaries() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        for version in ["0.1.0", "0.2.0"] {
            let project_root = temp.path().join(format!("demo-{version}"));
            fs::create_dir_all(&project_root).expect("create project");
            write_bundle_project(
                &project_root,
                "demo",
                version,
                "data/notes.txt",
                "hello world\n",
            );
            store
                .build_and_install(&project_root)
                .expect("build and install");
        }

        let namespace_path = store.installs_root().join("local");
        fs::write(store.installs_root().join("ignore.txt"), "ignore me").expect("write stray file");
        let namespace_entry = read_dir_entries(store.installs_root().as_path())
            .expect("read installs root entries")
            .into_iter()
            .find(|entry| entry.path() == namespace_path)
            .expect("find namespace entry");
        let mut summaries = collect_namespace_installs(namespace_entry)
            .expect("collect namespace installs")
            .into_iter()
            .map(|summary| (summary.namespace, summary.id, summary.version))
            .collect::<Vec<_>>();
        summaries.sort();

        assert_eq!(
            summaries,
            vec![
                ("local".to_string(), "demo".to_string(), "0.1.0".to_string()),
                ("local".to_string(), "demo".to_string(), "0.2.0".to_string()),
            ]
        );
    }

    #[test]
    fn load_bundle_install_reads_metadata_from_bundle_path() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let loaded = load_bundle_install(&install.path).expect("load bundle install");

        assert_eq!(loaded.path, install.path);
        assert_eq!(loaded.metadata.namespace, install.metadata.namespace);
        assert_eq!(loaded.metadata.id, install.metadata.id);
        assert_eq!(loaded.metadata.version, install.metadata.version);
        assert_eq!(loaded.metadata.digest, install.metadata.digest);
    }

    #[test]
    fn select_newer_bundle_install_prefers_higher_version() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        let older_root = temp.path().join("demo-0.1.0");
        fs::create_dir_all(&older_root).expect("create older project");
        write_bundle_project(
            &older_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );
        let older = store
            .build_and_install(&older_root)
            .expect("build older install");

        let newer_root = temp.path().join("demo-0.2.0");
        fs::create_dir_all(&newer_root).expect("create newer project");
        write_bundle_project(
            &newer_root,
            "demo",
            "0.2.0",
            "data/notes.txt",
            "hello world\n",
        );
        let newer = store
            .build_and_install(&newer_root)
            .expect("build newer install");

        let selected = select_newer_bundle_install(Some(older), newer.clone());

        assert_eq!(selected.path, newer.path);
        assert_eq!(selected.metadata.version, "0.2.0");
    }

    #[test]
    fn load_latest_installed_from_id_dir_returns_highest_version() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        for version in ["0.1.0", "0.10.0", "0.2.0"] {
            let project_root = temp.path().join(format!("demo-{version}"));
            fs::create_dir_all(&project_root).expect("create project");
            write_bundle_project(
                &project_root,
                "demo",
                version,
                "data/notes.txt",
                "hello world\n",
            );
            store
                .build_and_install(&project_root)
                .expect("build and install");
        }

        let id_dir = store.installs_root().join("local").join("demo");
        let latest = store
            .load_latest_installed_from_id_dir(&id_dir)
            .expect("load latest")
            .expect("latest install");

        assert_eq!(latest.metadata.version, "0.10.0");
        assert_eq!(latest.path, id_dir.join("0.10.0"));
    }

    #[test]
    fn build_and_install_with_namespace_uses_requested_namespace() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install_with_namespace(&project_root, "odyssey")
            .expect("build and install");

        assert_eq!(install.metadata.namespace, "odyssey");
        assert_eq!(
            install.path,
            temp.path()
                .join("store")
                .join("installs")
                .join("odyssey")
                .join("demo")
                .join("0.1.0")
        );
    }

    #[test]
    fn build_and_install_cleans_staging_root_after_success() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        store
            .build_and_install(&project_root)
            .expect("build and install");

        let staging_root = store.installs_root().join(".tmp");
        assert!(!staging_root.exists());
    }

    #[test]
    fn commit_staged_install_restores_previous_install_when_swap_fails() {
        let temp = tempdir().expect("tempdir");
        let install_root = temp
            .path()
            .join("installs")
            .join("local")
            .join("demo")
            .join("0.1.0");
        fs::create_dir_all(&install_root).expect("create install root");
        fs::write(install_root.join("agent.yaml"), "old bundle").expect("write bundle");

        let missing_staged_root = temp.path().join("missing-staged-root");
        let error = commit_staged_install(&missing_staged_root, &install_root)
            .expect_err("missing staged root should fail");

        assert!(matches!(
            error,
            BundleError::Io { path, .. } if path == install_root.display().to_string()
        ));
        assert_eq!(
            fs::read_to_string(install_root.join("agent.yaml")).expect("read restored bundle"),
            "old bundle"
        );
        assert!(
            !install_root
                .parent()
                .expect("id root")
                .join(".tmp")
                .exists()
        );
    }

    #[test]
    fn resolve_latest_prefers_higher_semver() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        for version in ["0.9.0", "0.10.0"] {
            let project_root = temp.path().join(format!("demo-{version}"));
            fs::create_dir_all(&project_root).expect("create project");
            write_bundle_project(
                &project_root,
                "demo",
                version,
                "data/notes.txt",
                "hello world\n",
            );
            store
                .build_and_install(&project_root)
                .expect("build and install");
        }

        let latest = store.resolve("demo").expect("resolve latest");
        assert_eq!(latest.metadata.version, "0.10.0");
    }

    #[test]
    fn export_and_import_preserve_metadata() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let export_dir = temp.path().join("exports");
        fs::create_dir_all(&export_dir).expect("create export dir");
        let export_path = store.export("demo", &export_dir).expect("export");

        let imported_store = BundleStore::new(temp.path().join("imported"));
        let imported = imported_store.import(&export_path).expect("import");

        assert_eq!(imported.metadata.id, install.metadata.id);
        assert_eq!(imported.metadata.version, install.metadata.version);
        assert_eq!(imported.metadata.digest, install.metadata.digest);
        assert_eq!(
            export_path.extension().and_then(|value| value.to_str()),
            Some("odyssey")
        );
    }

    #[test]
    fn resolve_rejects_bundle_archives_before_import() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let archive_path = temp.path().join("demo.odyssey");
        fs::write(&archive_path, "archive").expect("write archive placeholder");

        let error = store.resolve(archive_path.to_str().expect("archive path"));

        assert_eq!(
            error
                .expect_err("archive path should be rejected")
                .to_string(),
            format!(
                "invalid bundle: bundle archives must be imported before use: {}",
                archive_path.display()
            )
        );
    }

    #[test]
    fn import_rejects_archive_entries_that_escape_staging() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let archive_path = temp.path().join("evil.odyssey");
        let path = "../escape.txt";
        let mut archive = Vec::new();
        archive.extend_from_slice(ARCHIVE_MAGIC);
        archive.extend_from_slice(&1_u32.to_le_bytes());
        archive.extend_from_slice(&(path.len() as u32).to_le_bytes());
        archive.extend_from_slice(path.as_bytes());
        archive.extend_from_slice(&4_u64.to_le_bytes());
        archive.extend_from_slice(b"evil");
        fs::write(&archive_path, archive).expect("write archive");

        let error = store
            .import(&archive_path)
            .expect_err("reject escaping archive");

        assert_eq!(
            error.to_string(),
            "invalid bundle: bundle archive entry escapes destination: ../escape.txt"
        );
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn install_layout_payload_rejects_metadata_that_escapes_store_root() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let (_, manifest, _) =
            read_manifest(&install.path.join(".odyssey")).expect("read manifest");
        let mut metadata = install.metadata.clone();
        metadata.namespace = "../escape".to_string();

        let error = store
            .install_layout_payload(
                &install.path.join(".odyssey"),
                metadata,
                manifest.config.digest,
                manifest.layers,
            )
            .expect_err("reject escaping metadata");

        assert_eq!(
            error.to_string(),
            "invalid bundle: bundle namespace must not contain path separators or traversal segments"
        );
    }

    #[test]
    fn install_layout_payload_preserves_existing_install_on_unpack_failure() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let original_agent =
            fs::read_to_string(install.path.join("agent.yaml")).expect("read original agent");
        let source_layout_root = install.path.join(".odyssey");
        let (_, manifest, _) = read_manifest(&source_layout_root).expect("read manifest");
        let config_digest = manifest.config.digest.clone();
        let layers = manifest.layers.clone();

        let broken_layout_root = temp.path().join("broken-layout");
        fs::create_dir_all(&broken_layout_root).expect("create broken layout");
        for relative in ["oci-layout", "index.json"] {
            fs::copy(
                source_layout_root.join(relative),
                broken_layout_root.join(relative),
            )
            .expect("copy layout file");
        }
        for digest in std::iter::once(install.metadata.digest.clone())
            .chain(std::iter::once(config_digest.clone()))
            .chain(layers.iter().map(|layer| layer.digest.clone()))
        {
            let bytes = read_blob(&source_layout_root, &digest).expect("read source blob");
            let target = blob_path(&broken_layout_root, &digest).expect("target blob path");
            fs::create_dir_all(target.parent().expect("blob parent")).expect("create blob parent");
            fs::write(&target, bytes).expect("write blob copy");
        }
        let broken_layer_path =
            blob_path(&broken_layout_root, &layers[0].digest).expect("broken layer path");
        fs::write(&broken_layer_path, b"not-a-payload").expect("write invalid payload");

        let error = store
            .install_layout_payload(
                &broken_layout_root,
                install.metadata.clone(),
                config_digest,
                layers,
            )
            .expect_err("invalid payload should fail");

        assert_eq!(
            error.to_string(),
            "invalid bundle: invalid bundle payload header"
        );
        assert_eq!(
            fs::read_to_string(install.path.join("agent.yaml")).expect("read preserved agent"),
            original_agent
        );
        assert_eq!(
            store
                .resolve("demo@0.1.0")
                .expect("resolve preserved install")
                .metadata
                .digest,
            install.metadata.digest
        );
        assert!(!store.install_staging_parent().exists());
    }

    #[test]
    fn materialize_remote_layout_install_preserves_existing_install_on_unpack_failure() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(
            &project_root,
            "demo",
            "0.1.0",
            "data/notes.txt",
            "hello world\n",
        );

        let install = store
            .build_and_install(&project_root)
            .expect("build and install");
        let original_agent =
            fs::read_to_string(install.path.join("agent.yaml")).expect("read original agent");
        let layout_root = install.path.join(".odyssey");
        let (_, manifest, _) = read_manifest(&layout_root).expect("read manifest");
        let index_bytes = fs::read(layout_root.join("index.json")).expect("read index");

        let error = store
            .materialize_remote_layout_install(
                index_bytes,
                manifest.config.digest,
                install.metadata.clone(),
                vec![(manifest.layers[0].digest.clone(), b"not-a-payload".to_vec())],
            )
            .expect_err("invalid payload should fail");

        assert_eq!(
            error.to_string(),
            "invalid bundle: invalid bundle payload header"
        );
        assert_eq!(
            fs::read_to_string(install.path.join("agent.yaml")).expect("read preserved agent"),
            original_agent
        );
        assert_eq!(
            store
                .resolve("demo@0.1.0")
                .expect("resolve preserved install")
                .metadata
                .digest,
            install.metadata.digest
        );
        assert!(!store.install_staging_parent().exists());
    }
}
