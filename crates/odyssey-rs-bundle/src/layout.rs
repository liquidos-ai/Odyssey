use crate::BundleError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const OCI_LAYOUT_VERSION: &str = "1.0.0";
pub const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const BUNDLE_CONFIG_MEDIA_TYPE: &str = "application/vnd.odyssey.bundle.config.v1+json";
pub const BUNDLE_LAYER_MEDIA_TYPE: &str = "application/vnd.odyssey.bundle.layer.v1";
pub const REF_ANNOTATION: &str = "org.opencontainers.image.ref.name";
pub const ARCHIVE_MAGIC: &[u8; 6] = b"ODYB1\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciImageManifest {
    pub schema_version: u32,
    pub media_type: String,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciImageIndex {
    pub schema_version: u32,
    pub media_type: String,
    pub manifests: Vec<OciDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleConfig {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub namespace: String,
    pub bundle_manifest: odyssey_rs_manifest::BundleManifest,
    pub agent_spec: odyssey_rs_manifest::AgentSpec,
}

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn pack_payload(root: &Path) -> Result<Vec<u8>, BundleError> {
    let entries = payload_entries(root)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ODLP1\n");
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        let path = normalize_relative(root, entry.path())?;
        let path_bytes = path.as_bytes();
        let contents = fs::read(entry.path()).map_err(|err| io_err(entry.path(), err))?;
        bytes.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(path_bytes);
        bytes.extend_from_slice(&(contents.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&contents);
    }
    Ok(bytes)
}

pub fn unpack_payload(bytes: &[u8], dst: &Path) -> Result<(), BundleError> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0_u8; 6];
    cursor
        .read_exact(&mut magic)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    if &magic != b"ODLP1\n" {
        return Err(BundleError::Invalid(
            "invalid bundle payload header".to_string(),
        ));
    }

    let file_count = read_u32(&mut cursor)?;
    for _ in 0..file_count {
        let path_len = read_u32(&mut cursor)? as usize;
        let mut path_bytes = vec![0_u8; path_len];
        cursor
            .read_exact(&mut path_bytes)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let path =
            String::from_utf8(path_bytes).map_err(|err| BundleError::Invalid(err.to_string()))?;
        let data_len = read_u64(&mut cursor)? as usize;
        let mut contents = vec![0_u8; data_len];
        cursor
            .read_exact(&mut contents)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let target = dst.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
        }
        fs::write(&target, contents).map_err(|err| io_err(&target, err))?;
    }
    Ok(())
}

pub fn write_blob(root: &Path, bytes: &[u8]) -> Result<String, BundleError> {
    let digest = sha256_digest(bytes);
    let target = blob_path(root, &digest)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
    }
    if !target.exists() {
        fs::write(&target, bytes).map_err(|err| io_err(&target, err))?;
    }
    Ok(digest)
}

pub fn descriptor(media_type: &str, digest: &str, size: usize) -> OciDescriptor {
    OciDescriptor {
        media_type: media_type.to_string(),
        digest: digest.to_string(),
        size: size as u64,
        annotations: None,
    }
}

pub fn annotated_descriptor(
    media_type: &str,
    digest: &str,
    size: usize,
    reference: &str,
) -> OciDescriptor {
    let mut annotations = BTreeMap::new();
    annotations.insert(REF_ANNOTATION.to_string(), reference.to_string());
    OciDescriptor {
        media_type: media_type.to_string(),
        digest: digest.to_string(),
        size: size as u64,
        annotations: Some(annotations),
    }
}

pub fn blob_path(root: &Path, digest: &str) -> Result<PathBuf, BundleError> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(BundleError::Invalid(format!(
            "unsupported digest format {digest}"
        )));
    };
    Ok(root.join("blobs").join("sha256").join(hex))
}

pub fn read_blob(root: &Path, digest: &str) -> Result<Vec<u8>, BundleError> {
    let path = blob_path(root, digest)?;
    fs::read(&path).map_err(|err| io_err(&path, err))
}

pub fn read_config(root: &Path, manifest: &OciImageManifest) -> Result<BundleConfig, BundleError> {
    let bytes = read_blob(root, &manifest.config.digest)?;
    serde_json::from_slice(&bytes).map_err(|err| BundleError::Invalid(err.to_string()))
}

pub fn read_manifest(
    root: &Path,
) -> Result<(OciImageIndex, OciImageManifest, String), BundleError> {
    let index_path = root.join("index.json");
    let index_bytes = fs::read(&index_path).map_err(|err| io_err(&index_path, err))?;
    let index: OciImageIndex = serde_json::from_slice(&index_bytes)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    let descriptor = index
        .manifests
        .first()
        .ok_or_else(|| BundleError::Invalid("oci index does not contain a manifest".to_string()))?;
    let manifest_digest = descriptor.digest.clone();
    let manifest_bytes = read_blob(root, &manifest_digest)?;
    let manifest: OciImageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    Ok((index, manifest, manifest_digest))
}

pub fn copy_blob_into_layout(
    store_root: &Path,
    layout_root: &Path,
    digest: &str,
) -> Result<(), BundleError> {
    let src = blob_path(store_root, digest)?;
    let dst = blob_path(layout_root, digest)?;
    if dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
    }
    match fs::hard_link(&src, &dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(&src, &dst).map_err(|err| io_err(&dst, err))?;
            Ok(())
        }
    }
}

pub fn collect_oci_entries(root: &Path) -> Result<Vec<ArchiveEntry>, BundleError> {
    let mut entries = Vec::new();
    for relative in ["oci-layout", "index.json"] {
        let path = root.join(relative);
        let bytes = fs::read(&path).map_err(|err| io_err(&path, err))?;
        entries.push(ArchiveEntry {
            path: relative.to_string(),
            bytes,
        });
    }

    for entry in WalkDir::new(root.join("blobs"))
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = normalize_relative(root, entry.path())?;
        let bytes = fs::read(entry.path()).map_err(|err| io_err(entry.path(), err))?;
        entries.push(ArchiveEntry { path, bytes });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

pub fn archive_entries(entries: &[ArchiveEntry]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(ARCHIVE_MAGIC);
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        let path_bytes = entry.path.as_bytes();
        bytes.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(path_bytes);
        bytes.extend_from_slice(&(entry.bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&entry.bytes);
    }
    bytes
}

pub fn read_archive_entries(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, BundleError> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0_u8; 6];
    cursor
        .read_exact(&mut magic)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    if &magic != ARCHIVE_MAGIC {
        return Err(BundleError::Invalid(
            "invalid bundle archive header".to_string(),
        ));
    }
    let count = read_u32(&mut cursor)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let path_len = read_u32(&mut cursor)? as usize;
        let mut path_bytes = vec![0_u8; path_len];
        cursor
            .read_exact(&mut path_bytes)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        let path =
            String::from_utf8(path_bytes).map_err(|err| BundleError::Invalid(err.to_string()))?;
        let data_len = read_u64(&mut cursor)? as usize;
        let mut data = vec![0_u8; data_len];
        cursor
            .read_exact(&mut data)
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        entries.push(ArchiveEntry { path, bytes: data });
    }
    Ok(entries)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn payload_entries(root: &Path) -> Result<Vec<walkdir::DirEntry>, BundleError> {
    let mut entries = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_payload_path(root, entry.path()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path().to_path_buf());
    Ok(entries)
}

fn is_payload_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let value = relative.to_string_lossy();
    !(value == "bundle.json"
        || value == "index.json"
        || value == "oci-layout"
        || value.starts_with("blobs/"))
}

fn normalize_relative(root: &Path, path: &Path) -> Result<String, BundleError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, BundleError> {
    let mut buffer = [0_u8; 4];
    cursor
        .read_exact(&mut buffer)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64, BundleError> {
    let mut buffer = [0_u8; 8];
    cursor
        .read_exact(&mut buffer)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    Ok(u64::from_le_bytes(buffer))
}

fn io_err(path: &Path, err: std::io::Error) -> BundleError {
    BundleError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ARCHIVE_MAGIC, BUNDLE_CONFIG_MEDIA_TYPE, BUNDLE_LAYER_MEDIA_TYPE, BundleConfig,
        OCI_INDEX_MEDIA_TYPE, OCI_LAYOUT_VERSION, OCI_MANIFEST_MEDIA_TYPE, OciImageIndex,
        OciImageManifest, annotated_descriptor, archive_entries, blob_path, collect_oci_entries,
        copy_blob_into_layout, descriptor, pack_payload, read_archive_entries, read_blob,
        read_config, read_manifest, sha256_digest, unpack_payload, write_blob,
    };
    use odyssey_rs_manifest::{
        AgentModel, AgentSpec, AgentToolPolicy, BundleExecutor, BundleManifest, BundleMemory,
        BundleSandbox, BundleServer,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    fn sample_config() -> BundleConfig {
        BundleConfig {
            schema_version: 1,
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            namespace: "local".to_string(),
            bundle_manifest: BundleManifest {
                id: "demo".to_string(),
                version: "0.1.0".to_string(),
                agent_spec: "agent.yaml".to_string(),
                executor: BundleExecutor {
                    kind: "prebuilt".to_string(),
                    id: "react".to_string(),
                    config: serde_json::Value::Null,
                },
                memory: BundleMemory::default(),
                resources: Vec::new(),
                skills: Vec::new(),
                tools: Vec::new(),
                server: BundleServer::default(),
                sandbox: BundleSandbox::default(),
            },
            agent_spec: AgentSpec {
                id: "demo".to_string(),
                description: "demo bundle".to_string(),
                prompt: "be concise".to_string(),
                model: AgentModel {
                    provider: "openai".to_string(),
                    name: "gpt-4.1-mini".to_string(),
                },
                tools: AgentToolPolicy::default(),
            },
        }
    }

    #[test]
    fn payload_round_trip_excludes_layout_metadata() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();

        fs::create_dir_all(root.join("skills").join("repo")).expect("create skill dir");
        fs::create_dir_all(root.join("blobs").join("sha256")).expect("create blob dir");
        fs::write(root.join("agent.yaml"), "id: demo\n").expect("write agent");
        fs::write(
            root.join("skills").join("repo").join("SKILL.md"),
            "# Repo\n",
        )
        .expect("write skill");
        fs::write(root.join("bundle.json"), "{}").expect("write bundle metadata");
        fs::write(root.join("index.json"), "{}").expect("write index");
        fs::write(root.join("oci-layout"), "{}").expect("write layout");
        fs::write(root.join("blobs").join("sha256").join("deadbeef"), "blob").expect("write blob");

        let payload = pack_payload(root).expect("pack payload");
        let out = temp.path().join("out");
        unpack_payload(&payload, &out).expect("unpack payload");

        assert_eq!(
            fs::read_to_string(out.join("agent.yaml")).expect("read unpacked agent"),
            "id: demo\n"
        );
        assert_eq!(
            fs::read_to_string(out.join("skills").join("repo").join("SKILL.md"))
                .expect("read unpacked skill"),
            "# Repo\n"
        );
        assert_eq!(out.join("bundle.json").exists(), false);
        assert_eq!(out.join("index.json").exists(), false);
        assert_eq!(out.join("oci-layout").exists(), false);
        assert_eq!(out.join("blobs").exists(), false);
    }

    #[test]
    fn manifest_and_archive_round_trip_reads_back_expected_data() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();

        let payload_root = root.join("payload");
        fs::create_dir_all(&payload_root).expect("create payload root");
        fs::write(payload_root.join("agent.yaml"), "id: demo\n").expect("write payload file");
        let layer_bytes = pack_payload(&payload_root).expect("pack layer");
        let layer_digest = write_blob(root, &layer_bytes).expect("write layer blob");

        let config = sample_config();
        let config_bytes = serde_json::to_vec_pretty(&config).expect("serialize config");
        let config_digest = write_blob(root, &config_bytes).expect("write config blob");

        let manifest = OciImageManifest {
            schema_version: 2,
            media_type: OCI_MANIFEST_MEDIA_TYPE.to_string(),
            config: descriptor(BUNDLE_CONFIG_MEDIA_TYPE, &config_digest, config_bytes.len()),
            layers: vec![descriptor(
                BUNDLE_LAYER_MEDIA_TYPE,
                &layer_digest,
                layer_bytes.len(),
            )],
            annotations: BTreeMap::new(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("serialize manifest");
        let manifest_digest = write_blob(root, &manifest_bytes).expect("write manifest blob");

        let index = OciImageIndex {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.to_string(),
            manifests: vec![annotated_descriptor(
                OCI_MANIFEST_MEDIA_TYPE,
                &manifest_digest,
                manifest_bytes.len(),
                "local/demo:0.1.0",
            )],
        };
        fs::write(
            root.join("oci-layout"),
            format!("{{\"imageLayoutVersion\":\"{OCI_LAYOUT_VERSION}\"}}\n"),
        )
        .expect("write oci-layout");
        fs::write(
            root.join("index.json"),
            serde_json::to_vec_pretty(&index).expect("serialize index"),
        )
        .expect("write index");

        let (_, loaded_manifest, loaded_digest) = read_manifest(root).expect("read manifest");
        let loaded_config = read_config(root, &loaded_manifest).expect("read config");
        let entries = collect_oci_entries(root).expect("collect oci entries");
        let archived = archive_entries(&entries);
        let decoded = read_archive_entries(&archived).expect("decode archive");

        let decoded_paths = decoded
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();

        assert_eq!(loaded_digest, manifest_digest);
        assert_eq!(loaded_config.id, "demo");
        assert_eq!(loaded_config.version, "0.1.0");
        assert_eq!(loaded_manifest.layers[0].digest, layer_digest);
        assert_eq!(
            decoded_paths.contains(&format!(
                "blobs/sha256/{}",
                layer_digest.trim_start_matches("sha256:")
            )),
            true
        );
        assert_eq!(
            decoded_paths.contains(&format!(
                "blobs/sha256/{}",
                manifest_digest.trim_start_matches("sha256:")
            )),
            true
        );
        assert_eq!(archived.starts_with(ARCHIVE_MAGIC), true);
    }

    #[test]
    fn copy_blob_into_layout_copies_store_content() {
        let temp = tempdir().expect("tempdir");
        let store = temp.path().join("store");
        let layout = temp.path().join("layout");
        fs::create_dir_all(&layout).expect("create layout");

        let digest = write_blob(&store, b"hello bundle").expect("write source blob");
        copy_blob_into_layout(&store, &layout, &digest).expect("copy blob");

        assert_eq!(
            read_blob(&layout, &digest).expect("read copied blob"),
            b"hello bundle"
        );
        assert_eq!(sha256_digest(b"hello bundle"), digest);
    }

    #[test]
    fn invalid_payload_and_digest_inputs_fail_cleanly() {
        let temp = tempdir().expect("tempdir");
        let error = unpack_payload(b"not-a-payload", temp.path()).expect_err("invalid payload");
        assert_eq!(
            error.to_string(),
            "invalid bundle: invalid bundle payload header"
        );

        let error = blob_path(temp.path(), "md5:abc").expect_err("invalid digest");
        assert_eq!(
            error.to_string(),
            "invalid bundle: unsupported digest format md5:abc"
        );
    }
}
