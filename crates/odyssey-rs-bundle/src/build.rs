use crate::BundleError;
use crate::layout::{
    BUNDLE_CONFIG_MEDIA_TYPE, BUNDLE_LAYER_MEDIA_TYPE, BundleConfig, OCI_INDEX_MEDIA_TYPE,
    OCI_LAYOUT_VERSION, OCI_MANIFEST_MEDIA_TYPE, OciImageIndex, OciImageManifest,
    annotated_descriptor, descriptor, pack_payload, sha256_digest, write_blob,
};
use odyssey_rs_manifest::{AgentSpec, BundleLoader, BundleManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
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
            namespace: "local".to_string(),
        }
    }

    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    pub fn build(self, output_root: impl AsRef<Path>) -> Result<BundleArtifact, BundleError> {
        let output_root = output_root.as_ref();
        fs::create_dir_all(output_root).map_err(|err| io_err(output_root, err))?;
        let bundle_dir = output_root
            .join(&self.namespace)
            .join(&self.project.manifest.id)
            .join(&self.project.manifest.version);
        if bundle_dir.exists() {
            fs::remove_dir_all(&bundle_dir).map_err(|err| io_err(&bundle_dir, err))?;
        }
        fs::create_dir_all(&bundle_dir).map_err(|err| io_err(&bundle_dir, err))?;

        materialize_payload(&self.project, &bundle_dir)?;
        let payload_bytes = pack_payload(&bundle_dir)?;
        let layer_digest = write_blob(&bundle_dir, &payload_bytes)?;
        let layer_descriptor =
            descriptor(BUNDLE_LAYER_MEDIA_TYPE, &layer_digest, payload_bytes.len());

        let config = BundleConfig {
            schema_version: 1,
            id: self.project.manifest.id.clone(),
            version: self.project.manifest.version.clone(),
            namespace: self.namespace.clone(),
            readme: self.project.readme.clone(),
            bundle_manifest: self.project.manifest.clone(),
            agent_spec: self.project.agent.clone(),
        };
        let config_bytes = serde_json::to_vec_pretty(&config)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let config_digest = write_blob(&bundle_dir, &config_bytes)?;
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
        let manifest = OciImageManifest {
            schema_version: 2,
            media_type: OCI_MANIFEST_MEDIA_TYPE.to_string(),
            config: config_descriptor,
            layers: vec![layer_descriptor],
            annotations,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let manifest_digest = write_blob(&bundle_dir, &manifest_bytes)?;

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
            bundle_dir.join("oci-layout"),
            format!("{{\"imageLayoutVersion\":\"{OCI_LAYOUT_VERSION}\"}}\n"),
        )
        .map_err(|err| io_err(&bundle_dir.join("oci-layout"), err))?;
        fs::write(bundle_dir.join("index.json"), index_bytes)
            .map_err(|err| io_err(&bundle_dir.join("index.json"), err))?;

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
            bundle_dir.join("bundle.json"),
            serde_json::to_vec_pretty(&metadata)
                .map_err(|err| BundleError::Invalid(err.to_string()))?,
        )
        .map_err(|err| io_err(&bundle_dir.join("bundle.json"), err))?;

        Ok(BundleArtifact {
            path: bundle_dir,
            metadata,
        })
    }
}

fn materialize_payload(project: &BundleProject, bundle_dir: &Path) -> Result<(), BundleError> {
    let agent_src = project.root.join(&project.manifest.agent_spec);
    let agent_dst = bundle_dir.join("agent.yaml");
    fs::copy(&agent_src, &agent_dst).map_err(|err| io_err(&agent_dst, err))?;
    let readme_src = project.root.join(&project.manifest.readme);
    let readme_dst = bundle_dir.join(&project.manifest.readme);
    if let Some(parent) = readme_dst.parent() {
        fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
    }
    fs::copy(&readme_src, &readme_dst).map_err(|err| io_err(&readme_dst, err))?;

    let skills_dir = bundle_dir.join("skills");
    let resources_dir = bundle_dir.join("resources");
    fs::create_dir_all(&skills_dir).map_err(|err| io_err(&skills_dir, err))?;
    fs::create_dir_all(&resources_dir).map_err(|err| io_err(&resources_dir, err))?;

    for skill in &project.manifest.skills {
        copy_dir_all(
            &project.root.join(&skill.path),
            &skills_dir.join(&skill.name),
        )?;
    }
    let project_resources = project.root.join("resources");
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

        let (_, manifest, manifest_digest) = read_manifest(&artifact.path).expect("read manifest");
        let config = read_config(&artifact.path, &manifest).expect("read config");

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
    }
}
