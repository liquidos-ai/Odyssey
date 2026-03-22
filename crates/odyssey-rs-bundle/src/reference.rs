use crate::build::{BundleArtifact, BundleMetadata};
use crate::distribution::{publish_layout, pull_layout};
use crate::layout::{
    BundleConfig, archive_entries, blob_path, collect_oci_entries, copy_blob_into_layout,
    read_archive_entries, read_blob, read_manifest, unpack_payload,
};
use crate::{BundleBuilder, BundleError, BundleProject};
use directories::BaseDirs;
use odyssey_rs_manifest::{BundleRef, BundleRefKind};
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
pub struct BundleStore {
    pub root: PathBuf,
}

impl BundleStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn from_default_location() -> Result<Self, BundleError> {
        let dirs = BaseDirs::new().ok_or_else(|| {
            BundleError::Invalid("unable to resolve odyssey home dir".to_string())
        })?;
        Ok(Self::new(dirs.home_dir().join(".odyssey").join("bundles")))
    }

    pub fn build_and_install(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<BundleInstall, BundleError> {
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
        let artifact = BundleBuilder::new(project)
            .with_namespace(namespace.as_ref())
            .build(self.installs_root())?;
        self.persist_layout_blobs(&artifact.path)?;
        Ok(BundleInstall {
            path: artifact.path,
            metadata: artifact.metadata,
        })
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
        publish_layout(hub_url, &install.path, &reference).await
    }

    pub fn list_installed(&self) -> Result<Vec<BundleInstallSummary>, BundleError> {
        let installs_root = self.installs_root();
        if !installs_root.exists() {
            return Ok(Vec::new());
        }
        let mut bundles = Vec::new();
        for namespace_entry in fs::read_dir(&installs_root).map_err(|err| BundleError::Io {
            path: installs_root.display().to_string(),
            message: err.to_string(),
        })? {
            let namespace_entry = namespace_entry.map_err(|err| BundleError::Io {
                path: installs_root.display().to_string(),
                message: err.to_string(),
            })?;
            let namespace_path = namespace_entry.path();
            if !namespace_path.is_dir() {
                continue;
            }
            let namespace = namespace_entry.file_name().to_string_lossy().to_string();
            for id_entry in fs::read_dir(&namespace_path).map_err(|err| BundleError::Io {
                path: namespace_path.display().to_string(),
                message: err.to_string(),
            })? {
                let id_entry = id_entry.map_err(|err| BundleError::Io {
                    path: namespace_path.display().to_string(),
                    message: err.to_string(),
                })?;
                let id_path = id_entry.path();
                if !id_path.is_dir() {
                    continue;
                }
                for version_entry in fs::read_dir(&id_path).map_err(|err| BundleError::Io {
                    path: id_path.display().to_string(),
                    message: err.to_string(),
                })? {
                    let version_entry = version_entry.map_err(|err| BundleError::Io {
                        path: id_path.display().to_string(),
                        message: err.to_string(),
                    })?;
                    let bundle_path = version_entry.path();
                    if !bundle_path.is_dir() || !bundle_path.join("bundle.json").exists() {
                        continue;
                    }
                    let metadata = read_metadata(&bundle_path)?;
                    bundles.push(BundleInstallSummary {
                        namespace: namespace.clone(),
                        id: metadata.id,
                        version: metadata.version,
                        path: bundle_path,
                    });
                }
            }
        }
        bundles.sort_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then(a.id.cmp(&b.id))
                .then(a.version.cmp(&b.version))
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
        let entries = collect_oci_entries(&install.path)?;
        let archive = archive_entries(&entries);
        let output = output.as_ref();
        let output_path = if output.is_dir() {
            output.join(format!(
                "{}-{}.odyssey",
                install.metadata.id, install.metadata.version
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
        let entries = read_archive_entries(&bytes)?;
        let temp_root = self.root.join("imports").join("staging");
        if temp_root.exists() {
            fs::remove_dir_all(&temp_root).map_err(|err| BundleError::Io {
                path: temp_root.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::create_dir_all(&temp_root).map_err(|err| BundleError::Io {
            path: temp_root.display().to_string(),
            message: err.to_string(),
        })?;
        for entry in entries {
            let target = temp_root.join(&entry.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|err| BundleError::Io {
                    path: parent.display().to_string(),
                    message: err.to_string(),
                })?;
            }
            fs::write(&target, entry.bytes).map_err(|err| BundleError::Io {
                path: target.display().to_string(),
                message: err.to_string(),
            })?;
        }
        let install = self.install_from_layout(&temp_root)?;
        fs::remove_dir_all(&temp_root).map_err(|err| BundleError::Io {
            path: temp_root.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(install)
    }

    fn load_from_path(&self, path: &Path) -> Result<BundleInstall, BundleError> {
        let bundle_path = if path.join("bundle.json").exists() {
            path.to_path_buf()
        } else {
            return Err(BundleError::NotFound(path.display().to_string()));
        };
        let metadata = read_metadata(&bundle_path)?;
        Ok(BundleInstall {
            path: bundle_path,
            metadata,
        })
    }

    fn load_installed(&self, reference: &BundleRef) -> Result<BundleInstall, BundleError> {
        let namespace = reference
            .namespace
            .clone()
            .unwrap_or_else(|| "local".to_string());
        let id = reference
            .id
            .as_ref()
            .ok_or_else(|| BundleError::Invalid("bundle id missing".to_string()))?;
        let id_dir = self.installs_root().join(&namespace).join(id);
        if !id_dir.exists() {
            return Err(BundleError::NotFound(reference.raw.clone()));
        }
        let version = reference
            .version
            .clone()
            .unwrap_or_else(|| "latest".to_string());
        if version == "latest" {
            let mut versions = fs::read_dir(&id_dir)
                .map_err(|err| BundleError::Io {
                    path: id_dir.display().to_string(),
                    message: err.to_string(),
                })?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            versions.sort();
            let bundle_path = versions
                .pop()
                .ok_or_else(|| BundleError::NotFound(reference.raw.clone()))?;
            let metadata = read_metadata(&bundle_path)?;
            return Ok(BundleInstall {
                path: bundle_path,
                metadata,
            });
        }
        let bundle_path = id_dir.join(version);
        let metadata = read_metadata(&bundle_path)?;
        Ok(BundleInstall {
            path: bundle_path,
            metadata,
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
        let id = reference
            .id
            .clone()
            .ok_or_else(|| BundleError::Invalid("bundle id missing".to_string()))?;
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
            if !path.is_dir() || !path.join("bundle.json").exists() {
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
        let config: BundleConfig =
            serde_json::from_slice(&read_blob(layout_root, &manifest.config.digest)?)
                .map_err(|err| BundleError::Invalid(err.to_string()))?;
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
        self.persist_blob_bytes(&metadata.digest, &manifest_bytes)?;
        let config_digest = crate::layout::sha256_digest(&config_bytes);
        self.persist_blob_bytes(&config_digest, &config_bytes)?;
        for (digest, bytes) in &layers {
            self.persist_blob_bytes(digest, bytes)?;
        }

        let install_root = self.install_path(&metadata.namespace, &metadata.id, &metadata.version);
        if install_root.exists() {
            fs::remove_dir_all(&install_root).map_err(|err| BundleError::Io {
                path: install_root.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::create_dir_all(&install_root).map_err(|err| BundleError::Io {
            path: install_root.display().to_string(),
            message: err.to_string(),
        })?;
        fs::write(
            install_root.join("oci-layout"),
            format!(
                "{{\"imageLayoutVersion\":\"{}\"}}\n",
                crate::layout::OCI_LAYOUT_VERSION
            ),
        )
        .map_err(|err| BundleError::Io {
            path: install_root.join("oci-layout").display().to_string(),
            message: err.to_string(),
        })?;
        fs::write(install_root.join("index.json"), index_bytes).map_err(|err| BundleError::Io {
            path: install_root.join("index.json").display().to_string(),
            message: err.to_string(),
        })?;
        copy_blob_into_layout(&self.root, &install_root, &metadata.digest)?;
        copy_blob_into_layout(&self.root, &install_root, &config_digest)?;
        for (digest, _) in &layers {
            copy_blob_into_layout(&self.root, &install_root, digest)?;
        }
        for (digest, bytes) in &layers {
            let _ = digest;
            unpack_payload(bytes, &install_root)?;
        }
        self.write_metadata(&install_root, &metadata)?;
        Ok(BundleInstall {
            path: install_root,
            metadata,
        })
    }

    fn install_layout_payload(
        &self,
        layout_root: &Path,
        metadata: BundleMetadata,
        config_digest: String,
        layers: Vec<crate::layout::OciDescriptor>,
    ) -> Result<BundleInstall, BundleError> {
        let install_root = self.install_path(&metadata.namespace, &metadata.id, &metadata.version);
        if install_root.exists() {
            fs::remove_dir_all(&install_root).map_err(|err| BundleError::Io {
                path: install_root.display().to_string(),
                message: err.to_string(),
            })?;
        }
        fs::create_dir_all(&install_root).map_err(|err| BundleError::Io {
            path: install_root.display().to_string(),
            message: err.to_string(),
        })?;
        for relative in ["oci-layout", "index.json"] {
            let src = layout_root.join(relative);
            let dst = install_root.join(relative);
            fs::copy(&src, &dst).map_err(|err| BundleError::Io {
                path: dst.display().to_string(),
                message: err.to_string(),
            })?;
        }
        copy_blob_into_layout(&self.root, &install_root, &metadata.digest)?;
        copy_blob_into_layout(&self.root, &install_root, &config_digest)?;
        for layer in &layers {
            copy_blob_into_layout(&self.root, &install_root, &layer.digest)?;
            let bytes = read_blob(layout_root, &layer.digest)?;
            unpack_payload(&bytes, &install_root)?;
        }
        self.write_metadata(&install_root, &metadata)?;
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
        fs::write(
            install_root.join("bundle.json"),
            serde_json::to_vec_pretty(metadata)
                .map_err(|err| BundleError::Invalid(err.to_string()))?,
        )
        .map_err(|err| BundleError::Io {
            path: install_root.join("bundle.json").display().to_string(),
            message: err.to_string(),
        })
    }

    fn installs_root(&self) -> PathBuf {
        self.root.join("installs")
    }

    fn install_path(&self, namespace: &str, id: &str, version: &str) -> PathBuf {
        self.installs_root().join(namespace).join(id).join(version)
    }
}

fn read_metadata(bundle_path: &Path) -> Result<BundleMetadata, BundleError> {
    let bytes = fs::read(bundle_path.join("bundle.json")).map_err(|err| BundleError::Io {
        path: bundle_path.join("bundle.json").display().to_string(),
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
    use super::{BundleInstallSummary, BundleStore};
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
    }

    #[test]
    fn list_installed_returns_sorted_summaries() {
        let temp = tempdir().expect("tempdir");
        let store = BundleStore::new(temp.path().join("store"));

        for (name, version) in [("zeta", "0.1.0"), ("alpha", "0.2.0"), ("alpha", "0.1.0")] {
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
                ("local".to_string(), "zeta".to_string(), "0.1.0".to_string()),
            ]
        );
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
}
