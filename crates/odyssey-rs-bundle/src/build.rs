use crate::BundleError;
use crate::constants::{
    AGENT_SPEC_FILE_NAME, BUNDLE_CONFIG_SCHEMA_VERSION, BUNDLE_INSTALL_LAYOUT_DIR_NAME,
    BUNDLE_LOCAL_NAMESPACE, RESOURCES_DIR_NAME, SKILLS_DIR_NAME,
};
use crate::constants::{
    BUNDLE_CONFIG_MEDIA_TYPE, BUNDLE_LAYER_MEDIA_TYPE, OCI_INDEX_MEDIA_TYPE, OCI_LAYOUT_VERSION,
    OCI_MANIFEST_MEDIA_TYPE,
};
use crate::layout::{
    BundleConfig, OciImageIndex, OciImageManifest, annotated_descriptor, descriptor, pack_payload,
    sha256_digest, write_blob,
};
use odyssey_rs_manifest::{AgentSpec, BundleLoader, BundleManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct BundleProject {
    pub root: PathBuf,
    pub manifest: BundleManifest,
    pub agent: AgentSpec,
    pub readme: String,
}

impl BundleProject {
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, BundleError> {
        let root = root.into();
        let loader = BundleLoader::new(&root);
        let (manifest, agent) = loader.load_project()?;
        let readme_path = root.join(&manifest.readme);
        let readme = fs::read_to_string(&readme_path).map_err(|err| BundleError::Io {
            path: readme_path.display().to_string(),
            message: err.to_string(),
        })?;
        Ok(Self {
            root,
            manifest,
            agent,
            readme,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleMetadata {
    pub namespace: String,
    pub id: String,
    pub version: String,
    pub digest: String,
    pub readme: String,
    pub bundle_manifest: BundleManifest,
    pub agent_spec: AgentSpec,
}

#[derive(Debug, Clone)]
pub struct BundleArtifact {
    pub path: PathBuf,
    pub metadata: BundleMetadata,
}

#[derive(Debug, Clone)]
pub struct BundleBuilder {
    project: BundleProject,
    namespace: String,
}

impl BundleBuilder {
    pub fn new(project: BundleProject) -> Self {
        Self {
            project,
            namespace: BUNDLE_LOCAL_NAMESPACE.to_string(),
        }
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    pub fn build(self, output_root: impl AsRef<Path>) -> Result<BundleArtifact, BundleError> {
        let output_root = output_root.as_ref();
        fs::create_dir_all(output_root).map_err(|err| io_err(output_root, err))?;
        validate_path_component(&self.namespace, "bundle namespace")?;
        validate_path_component(&self.project.manifest.id, "bundle id")?;
        validate_path_component(&self.project.manifest.version, "bundle version")?;
        //Create a bundle directory structure -> <namespace>/<manifest_id>/<manifest_version>/
        let bundle_dir = output_root
            .join(&self.namespace)
            .join(&self.project.manifest.id)
            .join(&self.project.manifest.version);

        if bundle_dir.exists() {
            fs::remove_dir_all(&bundle_dir).map_err(|err| io_err(&bundle_dir, err))?;
        }

        fs::create_dir_all(&bundle_dir).map_err(|err| io_err(&bundle_dir, err))?;

        //Create and copy files into the bundle dir
        materialize_payload(&self.project, &bundle_dir)?;
        let layout_dir = bundle_dir.join(BUNDLE_INSTALL_LAYOUT_DIR_NAME);
        fs::create_dir_all(&layout_dir).map_err(|err| io_err(&layout_dir, err))?;

        // The payload layer contains the runtime files a bundle install needs to unpack.
        let payload_bytes = pack_payload(&bundle_dir)?;
        let layer_digest = write_blob(&layout_dir, &payload_bytes)?;
        let layer_descriptor =
            descriptor(BUNDLE_LAYER_MEDIA_TYPE, &layer_digest, payload_bytes.len());

        // The config blob stores resolved bundle metadata alongside the validated source manifest.
        let config = BundleConfig {
            schema_version: BUNDLE_CONFIG_SCHEMA_VERSION,
            id: self.project.manifest.id.clone(),
            version: self.project.manifest.version.clone(),
            namespace: self.namespace.clone(),
            readme: self.project.readme.clone(),
            bundle_manifest: self.project.manifest.clone(),
            agent_spec: self.project.agent.clone(),
        };
        let config_bytes = serde_json::to_vec_pretty(&config)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let config_digest = write_blob(&layout_dir, &config_bytes)?;
        let config_descriptor =
            descriptor(BUNDLE_CONFIG_MEDIA_TYPE, &config_digest, config_bytes.len());

        let reference = format!(
            "{}/{id}:{version}",
            self.namespace,
            id = self.project.manifest.id,
            version = self.project.manifest.version
        );
        let mut annotations = BTreeMap::new();
        annotations.insert(
            "org.opencontainers.image.title".to_string(),
            reference.clone(),
        );
        // The OCI manifest ties the custom config and payload layer together under content digests.
        let manifest = OciImageManifest {
            schema_version: 2,
            media_type: OCI_MANIFEST_MEDIA_TYPE.to_string(),
            config: config_descriptor,
            layers: vec![layer_descriptor],
            annotations,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let manifest_digest = write_blob(&layout_dir, &manifest_bytes)?;

        // index.json is the OCI layout entrypoint; it points readers to the bundle manifest blob.
        let index = OciImageIndex {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.to_string(),
            manifests: vec![annotated_descriptor(
                OCI_MANIFEST_MEDIA_TYPE,
                &manifest_digest,
                manifest_bytes.len(),
                &reference,
            )],
        };
        let index_bytes = serde_json::to_vec_pretty(&index)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;

        fs::write(
            layout_dir.join("oci-layout"),
            format!("{{\"imageLayoutVersion\":\"{OCI_LAYOUT_VERSION}\"}}\n"),
        )
        .map_err(|err| io_err(&layout_dir.join("oci-layout"), err))?;
        fs::write(layout_dir.join("index.json"), index_bytes)
            .map_err(|err| io_err(&layout_dir.join("index.json"), err))?;

        let metadata = BundleMetadata {
            namespace: self.namespace,
            id: self.project.manifest.id.clone(),
            version: self.project.manifest.version.clone(),
            digest: manifest_digest,
            readme: self.project.readme,
            bundle_manifest: self.project.manifest,
            agent_spec: self.project.agent,
        };
        fs::write(
            layout_dir.join("bundle.json"),
            serde_json::to_vec_pretty(&metadata)
                .map_err(|err| BundleError::Invalid(err.to_string()))?,
        )
        .map_err(|err| io_err(&layout_dir.join("bundle.json"), err))?;

        Ok(BundleArtifact {
            path: bundle_dir,
            metadata,
        })
    }
}

/// Copy the runtime-visible bundle payload into the staging directory before packing it as a layer.
fn materialize_payload(project: &BundleProject, bundle_dir: &Path) -> Result<(), BundleError> {
    let agent_src = project.root.join(&project.manifest.agent_spec);
    let agent_dst = bundle_dir.join(AGENT_SPEC_FILE_NAME);
    fs::copy(&agent_src, &agent_dst).map_err(|err| io_err(&agent_dst, err))?;

    let readme_src = project.root.join(&project.manifest.readme);
    let readme_dst = bundle_dir.join(&project.manifest.readme);
    if let Some(parent) = readme_dst.parent() {
        fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
    }
    fs::copy(&readme_src, &readme_dst).map_err(|err| io_err(&readme_dst, err))?;

    let skills_dir = bundle_dir.join(SKILLS_DIR_NAME);
    let resources_dir = bundle_dir.join(RESOURCES_DIR_NAME);
    fs::create_dir_all(&skills_dir).map_err(|err| io_err(&skills_dir, err))?;
    fs::create_dir_all(&resources_dir).map_err(|err| io_err(&resources_dir, err))?;

    for skill in &project.manifest.skills {
        copy_dir_all(
            &project.root.join(&skill.path),
            &skills_dir.join(&skill.name),
        )?;
    }

    let project_resources = project.root.join(RESOURCES_DIR_NAME);
    if project_resources.exists() {
        if !project_resources.is_dir() {
            return Err(BundleError::Invalid(format!(
                "resources path must be a directory: {}",
                project_resources.display()
            )));
        }
        copy_dir_all(&project_resources, &resources_dir)?;
    }

    Ok(())
}

/// Utility to check if the value is a filename or directory and not a Path
fn validate_path_component(value: &str, label: &str) -> Result<(), BundleError> {
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

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), BundleError> {
    fs::create_dir_all(dst).map_err(|err| io_err(dst, err))?;
    for entry in WalkDir::new(src) {
        let entry = entry.map_err(|err| BundleError::Invalid(err.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(src)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|err| io_err(&target, err))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
            }
            fs::copy(entry.path(), &target).map_err(|err| io_err(&target, err))?;
        }
    }
    Ok(())
}

fn io_err(path: &Path, err: std::io::Error) -> BundleError {
    BundleError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

#[allow(dead_code)]
fn _payload_digest(root: &Path) -> Result<String, BundleError> {
    Ok(sha256_digest(&pack_payload(root)?))
}

#[cfg(test)]
mod tests {
    use super::{BundleBuilder, BundleProject};
    use crate::constants::BUNDLE_INSTALL_LAYOUT_DIR_NAME;
    use crate::layout::{read_config, read_manifest};
    use crate::test_support::write_bundle_project;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn builder_materializes_payload_and_metadata() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        let output_root = temp.path().join("output");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(&project_root, "demo", "0.1.0", "logo.txt", "liquidos");

        let project = BundleProject::load(&project_root).expect("load project");
        let artifact = BundleBuilder::new(project)
            .with_namespace("team")
            .build(&output_root)
            .expect("build bundle");

        let layout_root = artifact.path.join(BUNDLE_INSTALL_LAYOUT_DIR_NAME);
        let (_, manifest, manifest_digest) = read_manifest(&layout_root).expect("read manifest");
        let config = read_config(&layout_root, &manifest).expect("read config");

        assert_eq!(artifact.metadata.namespace, "team");
        assert_eq!(artifact.metadata.id, "demo");
        assert_eq!(artifact.metadata.version, "0.1.0");
        assert_eq!(artifact.metadata.digest, manifest_digest);
        assert_eq!(artifact.metadata.readme, "# demo\n");
        assert_eq!(config.namespace, "team");
        assert_eq!(config.readme, "# demo\n");
        assert_eq!(config.bundle_manifest.id, "demo");
        assert_eq!(
            fs::read_to_string(artifact.path.join("agent.yaml")).expect("read bundled agent"),
            "id: demo\ndescription: test bundle\nprompt: keep responses concise\nmodel:\n  provider: openai\n  name: gpt-4.1-mini\ntools:\n  allow: [\"Read\", \"Skill\"]\n"
        );
        assert_eq!(
            fs::read_to_string(
                artifact
                    .path
                    .join("skills")
                    .join("repo-hygiene")
                    .join("SKILL.md")
            )
            .expect("read bundled skill"),
            "# Repo Hygiene\n"
        );
        assert_eq!(
            fs::read_to_string(artifact.path.join("resources").join("logo.txt"))
                .expect("read bundled resource"),
            "liquidos"
        );
        assert!(!artifact.path.join("bundle.json").exists());
        assert!(!artifact.path.join("index.json").exists());
        assert!(!artifact.path.join("oci-layout").exists());
        assert!(!artifact.path.join("blobs").exists());
        assert!(layout_root.join("bundle.json").exists());
        assert!(layout_root.join("index.json").exists());
        assert!(layout_root.join("oci-layout").exists());
        assert!(layout_root.join("blobs").exists());
    }

    #[test]
    fn builder_rejects_namespace_with_path_traversal() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        let output_root = temp.path().join("output");
        fs::create_dir_all(&project_root).expect("create project");
        write_bundle_project(&project_root, "demo", "0.1.0", "logo.txt", "liquidos");

        let project = BundleProject::load(&project_root).expect("load project");
        let error = BundleBuilder::new(project)
            .with_namespace("../escape")
            .build(&output_root)
            .expect_err("reject path traversal namespace");

        assert_eq!(
            error.to_string(),
            "invalid bundle: bundle namespace must not contain path separators or traversal segments"
        );
    }
}
