use crate::BundleError;
use crate::constants::{
    ARCHIVE_MAGIC, BUNDLE_CONFIG_SCHEMA_VERSION, LAYOUT_PAYLOAD_BUNDLE_MAGIC, REF_ANNOTATION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

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
    pub schema_version: usize,
    pub media_type: String,
    pub manifests: Vec<OciDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleConfig {
    pub schema_version: usize,
    pub id: String,
    pub version: String,
    pub namespace: String,
    pub readme: String,
    pub bundle_manifest: odyssey_rs_manifest::BundleManifest,
    pub agent_spec: odyssey_rs_manifest::AgentSpec,
}

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub path: String,
    pub bytes: Vec<u8>,
}

// Bound untrusted layout/archive parsing so malformed bundles cannot request
// arbitrarily large allocations during import or pull flows.
const MAX_LAYOUT_ENTRY_COUNT: u32 = 16_384;
const MAX_LAYOUT_PATH_BYTES: u32 = 4 * 1024;
const MAX_LAYOUT_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

pub fn pack_payload(root: &Path) -> Result<Vec<u8>, BundleError> {
    let entries = payload_entries(root)?;
    let entry_count = validate_entry_count(entries.len(), "payload file count")?;
    let mut bytes = Vec::new();
    //Add a Magic Header for the blob
    bytes.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
    bytes.extend_from_slice(&entry_count.to_le_bytes());
    for entry in entries {
        let path = normalize_relative(root, entry.path())?;
        let path_bytes = path.as_bytes();
        let contents = fs::read(entry.path()).map_err(|err| io_err(entry.path(), err))?;
        let path_len = validate_path_len(path_bytes.len(), "payload path length")?;
        let content_len = validate_data_len(contents.len(), "payload data length")?;
        bytes.extend_from_slice(&path_len.to_le_bytes());
        bytes.extend_from_slice(path_bytes);
        bytes.extend_from_slice(&content_len.to_le_bytes());
        bytes.extend_from_slice(&contents);
    }
    Ok(bytes)
}

pub fn unpack_payload(bytes: &[u8], dst: &Path) -> Result<(), BundleError> {
    let entries = parse_entry_ranges(
        bytes,
        LAYOUT_PAYLOAD_BUNDLE_MAGIC,
        "invalid bundle payload header",
        "payload file count",
        "payload path length",
        "payload data length",
        "payload bundle contains trailing data",
    )?;

    for (target, range) in entries
        .into_iter()
        .map(|(path, range)| Ok((payload_target(dst, &path)?, range)))
        .collect::<Result<Vec<_>, BundleError>>()?
    {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| io_err(parent, err))?;
        }
        fs::write(&target, &bytes[range]).map_err(|err| io_err(&target, err))?;
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
    parse_config_bytes(&bytes)
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
    let mut entry_count = 0_usize;
    for relative in ["oci-layout", "index.json"] {
        let path = root.join(relative);
        let bytes = fs::read(&path).map_err(|err| io_err(&path, err))?;
        entry_count += 1;
        entries.push(validated_archive_entry(
            entry_count,
            relative.to_string(),
            bytes,
        )?);
    }

    for entry in WalkDir::new(root.join("blobs")).sort_by_file_name() {
        let entry = entry.map_err(walkdir_err)?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = normalize_relative(root, entry.path())?;
        let bytes = fs::read(entry.path()).map_err(|err| io_err(entry.path(), err))?;
        entry_count += 1;
        entries.push(validated_archive_entry(entry_count, path, bytes)?);
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

#[cfg(test)]
fn read_archive_entries(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, BundleError> {
    archive_entry_ranges(bytes)?
        .into_iter()
        .map(|(path, range)| {
            Ok(ArchiveEntry {
                path,
                bytes: bytes[range].to_vec(),
            })
        })
        .collect()
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub(crate) fn archive_entry_ranges(
    bytes: &[u8],
) -> Result<Vec<(String, Range<usize>)>, BundleError> {
    parse_entry_ranges(
        bytes,
        ARCHIVE_MAGIC,
        "invalid bundle archive header",
        "archive entry count",
        "archive path length",
        "archive entry length",
        "bundle archive contains trailing data",
    )
}

fn payload_entries(root: &Path) -> Result<Vec<walkdir::DirEntry>, BundleError> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(walkdir_err)?;
        if entry.file_type().is_file() && is_payload_path(root, entry.path()) {
            entries.push(entry);
        }
    }
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

fn payload_target(dst: &Path, path: &str) -> Result<PathBuf, BundleError> {
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(BundleError::Invalid(format!(
            "payload entry path must be relative: {path}"
        )));
    }

    let mut normalized = PathBuf::default();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BundleError::Invalid(format!(
                    "payload entry path escapes destination: {path}"
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(BundleError::Invalid(
            "payload entry path cannot be empty".to_string(),
        ));
    }

    Ok(dst.join(normalized))
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

pub(crate) fn parse_config_bytes(bytes: &[u8]) -> Result<BundleConfig, BundleError> {
    let config: BundleConfig =
        serde_json::from_slice(bytes).map_err(|err| BundleError::Invalid(err.to_string()))?;
    if config.schema_version != BUNDLE_CONFIG_SCHEMA_VERSION {
        return Err(BundleError::Invalid(format!(
            "unsupported bundle config schema version {}",
            config.schema_version
        )));
    }
    Ok(config)
}

fn usize_len(value: u64, field: &str) -> Result<usize, BundleError> {
    usize::try_from(value)
        .map_err(|_| BundleError::Invalid(format!("{field} exceeds platform limits")))
}

fn validate_entry_count(value: usize, field: &str) -> Result<u32, BundleError> {
    let value = u32::try_from(value)
        .map_err(|_| BundleError::Invalid(format!("{field} exceeds archive format limits")))?;
    if value > MAX_LAYOUT_ENTRY_COUNT {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_ENTRY_COUNT}"
        )));
    }
    Ok(value)
}

fn validate_path_len(value: usize, field: &str) -> Result<u32, BundleError> {
    let value = u32::try_from(value)
        .map_err(|_| BundleError::Invalid(format!("{field} exceeds archive format limits")))?;
    if value > MAX_LAYOUT_PATH_BYTES {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_PATH_BYTES} bytes"
        )));
    }
    Ok(value)
}

fn validate_data_len(value: usize, field: &str) -> Result<u64, BundleError> {
    let value = u64::try_from(value)
        .map_err(|_| BundleError::Invalid(format!("{field} exceeds platform limits")))?;
    if value > MAX_LAYOUT_ENTRY_BYTES {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_ENTRY_BYTES} bytes"
        )));
    }
    Ok(value)
}

fn parse_entry_ranges(
    bytes: &[u8],
    magic: &[u8],
    invalid_magic_message: &str,
    count_field: &str,
    path_len_field: &str,
    data_len_field: &str,
    trailing_data_message: &str,
) -> Result<Vec<(String, Range<usize>)>, BundleError> {
    let mut cursor = Cursor::new(bytes);
    let mut actual_magic = vec![0_u8; magic.len()];
    cursor
        .read_exact(&mut actual_magic)
        .map_err(|err| BundleError::Invalid(err.to_string()))?;
    if actual_magic.as_slice() != magic {
        return Err(BundleError::Invalid(invalid_magic_message.to_string()));
    }

    let entry_count = read_bounded_entry_count(&mut cursor, count_field)?;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let path_len = read_bounded_path_len(&mut cursor, path_len_field)?;
        let path_range = read_entry_range(&cursor, bytes, path_len, path_len_field)?;
        let path = String::from_utf8(bytes[path_range.clone()].to_vec())
            .map_err(|err| BundleError::Invalid(err.to_string()))?;
        cursor.set_position(path_range.end as u64);

        let data_len = read_bounded_data_len(&mut cursor, data_len_field)?;
        let data_range = read_entry_range(&cursor, bytes, data_len, data_len_field)?;
        cursor.set_position(data_range.end as u64);
        entries.push((path, data_range));
    }
    if cursor.position() != bytes.len() as u64 {
        return Err(BundleError::Invalid(trailing_data_message.to_string()));
    }
    Ok(entries)
}

fn read_bounded_entry_count(cursor: &mut Cursor<&[u8]>, field: &str) -> Result<usize, BundleError> {
    let count = read_u32(cursor)?;
    if count > MAX_LAYOUT_ENTRY_COUNT {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_ENTRY_COUNT}"
        )));
    }
    Ok(count as usize)
}

fn read_bounded_path_len(cursor: &mut Cursor<&[u8]>, field: &str) -> Result<usize, BundleError> {
    let value = read_u32(cursor)?;
    if value > MAX_LAYOUT_PATH_BYTES {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_PATH_BYTES} bytes"
        )));
    }
    Ok(value as usize)
}

fn read_bounded_data_len(cursor: &mut Cursor<&[u8]>, field: &str) -> Result<usize, BundleError> {
    let value = read_u64(cursor)?;
    if value > MAX_LAYOUT_ENTRY_BYTES {
        return Err(BundleError::Invalid(format!(
            "{field} exceeds maximum {MAX_LAYOUT_ENTRY_BYTES} bytes"
        )));
    }
    usize_len(value, field)
}

fn read_entry_range(
    cursor: &Cursor<&[u8]>,
    bytes: &[u8],
    len: usize,
    field: &str,
) -> Result<Range<usize>, BundleError> {
    let start = usize_len(cursor.position(), field)?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| BundleError::Invalid(format!("{field} exceeds platform limits")))?;
    if end > bytes.len() {
        return Err(BundleError::Invalid(
            "bundle entry is truncated".to_string(),
        ));
    }
    Ok(start..end)
}

fn validated_archive_entry(
    entry_count: usize,
    path: String,
    bytes: Vec<u8>,
) -> Result<ArchiveEntry, BundleError> {
    validate_entry_count(entry_count, "archive entry count")?;
    validate_path_len(path.len(), "archive path length")?;
    validate_data_len(bytes.len(), "archive entry length")?;
    Ok(ArchiveEntry { path, bytes })
}

fn walkdir_err(err: walkdir::Error) -> BundleError {
    match (err.path(), err.io_error()) {
        (Some(path), Some(source_err)) => io_err(
            path,
            std::io::Error::new(source_err.kind(), source_err.to_string()),
        ),
        (Some(path), None) => {
            BundleError::Invalid(format!("unable to traverse {}: {err}", path.display()))
        }
        (None, _) => BundleError::Invalid(err.to_string()),
    }
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
        BundleConfig, MAX_LAYOUT_ENTRY_BYTES, MAX_LAYOUT_ENTRY_COUNT, MAX_LAYOUT_PATH_BYTES,
        OciImageIndex, OciImageManifest, annotated_descriptor, archive_entries, blob_path,
        collect_oci_entries, copy_blob_into_layout, descriptor, pack_payload, read_archive_entries,
        read_blob, read_config, read_manifest, sha256_digest, unpack_payload, write_blob,
    };
    use crate::BundleError;
    use crate::constants::{
        ARCHIVE_MAGIC, BUNDLE_CONFIG_MEDIA_TYPE, BUNDLE_LAYER_MEDIA_TYPE,
        LAYOUT_PAYLOAD_BUNDLE_MAGIC, OCI_INDEX_MEDIA_TYPE, OCI_LAYOUT_VERSION,
        OCI_MANIFEST_MEDIA_TYPE,
    };
    use odyssey_rs_manifest::{
        AgentSpec, AgentToolPolicy, BundleExecutor, BundleManifest, BundleMemory, BundleSandbox,
        ManifestVersion, ProviderKind,
    };
    use odyssey_rs_protocol::ModelSpec;
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
            readme: "# demo\n".to_string(),
            bundle_manifest: BundleManifest {
                id: "demo".to_string(),
                version: "0.1.0".to_string(),
                manifest_version: ManifestVersion::V1,
                readme: "README.md".to_string(),
                agent_spec: "agent.yaml".to_string(),
                executor: BundleExecutor {
                    kind: ProviderKind::Prebuilt,
                    id: "react".to_string(),
                    config: serde_json::Value::Null,
                },
                memory: BundleMemory::default(),
                skills: Vec::new(),
                tools: Vec::new(),
                sandbox: BundleSandbox::default(),
            },
            agent_spec: AgentSpec {
                id: "demo".to_string(),
                description: "demo bundle".to_string(),
                prompt: "be concise".to_string(),
                model: ModelSpec {
                    provider: "openai".to_string(),
                    name: "gpt-4.1-mini".to_string(),
                    config: None,
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
        assert!(!out.join("bundle.json").exists());
        assert!(!out.join("index.json").exists());
        assert!(!out.join("oci-layout").exists());
        assert!(!out.join("blobs").exists());
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
        assert!(decoded_paths.contains(&format!(
            "blobs/sha256/{}",
            layer_digest.trim_start_matches("sha256:")
        )));
        assert!(decoded_paths.contains(&format!(
            "blobs/sha256/{}",
            manifest_digest.trim_start_matches("sha256:")
        )));
        assert!(archived.starts_with(ARCHIVE_MAGIC));
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

    #[test]
    fn pack_payload_rejects_missing_root() {
        let temp = tempdir().expect("tempdir");
        let missing_root = temp.path().join("missing");

        let error = pack_payload(&missing_root).expect_err("missing root should fail");

        assert!(matches!(
            error,
            BundleError::Io { path, .. } if path == missing_root.display().to_string()
        ));
    }

    #[test]
    fn unpack_payload_rejects_paths_outside_destination() {
        let temp = tempdir().expect("tempdir");
        let mut payload = Vec::new();
        let path = "../escape.txt";

        payload.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(path.len() as u32).to_le_bytes());
        payload.extend_from_slice(path.as_bytes());
        payload.extend_from_slice(&4_u64.to_le_bytes());
        payload.extend_from_slice(b"evil");

        let error = unpack_payload(&payload, temp.path()).expect_err("reject traversal path");
        assert_eq!(
            error.to_string(),
            "invalid bundle: payload entry path escapes destination: ../escape.txt"
        );
        assert!(!temp.path().join("..").join("escape.txt").exists());
    }

    #[test]
    fn unpack_payload_rejects_trailing_bytes() {
        let temp = tempdir().expect("tempdir");
        let mut payload = Vec::new();
        let path = "agent.yaml";

        payload.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(path.len() as u32).to_le_bytes());
        payload.extend_from_slice(path.as_bytes());
        payload.extend_from_slice(&9_u64.to_le_bytes());
        payload.extend_from_slice(b"id: demo\n");
        payload.extend_from_slice(b"junk");

        let error = unpack_payload(&payload, temp.path()).expect_err("reject trailing bytes");
        assert_eq!(
            error.to_string(),
            "invalid bundle: payload bundle contains trailing data"
        );
        assert!(!temp.path().join("agent.yaml").exists());
    }

    #[test]
    fn payload_parser_rejects_oversized_counts_and_lengths() {
        let temp = tempdir().expect("tempdir");

        let mut payload = Vec::new();
        payload.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
        payload.extend_from_slice(&(MAX_LAYOUT_ENTRY_COUNT + 1).to_le_bytes());
        let error = unpack_payload(&payload, temp.path()).expect_err("reject oversized file count");
        assert_eq!(
            error.to_string(),
            format!("invalid bundle: payload file count exceeds maximum {MAX_LAYOUT_ENTRY_COUNT}")
        );

        let mut payload = Vec::new();
        payload.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(MAX_LAYOUT_PATH_BYTES + 1).to_le_bytes());
        let error =
            unpack_payload(&payload, temp.path()).expect_err("reject oversized path length");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid bundle: payload path length exceeds maximum {MAX_LAYOUT_PATH_BYTES} bytes"
            )
        );

        let path = "agent.yaml";
        let mut payload = Vec::new();
        payload.extend_from_slice(LAYOUT_PAYLOAD_BUNDLE_MAGIC);
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&(path.len() as u32).to_le_bytes());
        payload.extend_from_slice(path.as_bytes());
        payload.extend_from_slice(&(MAX_LAYOUT_ENTRY_BYTES + 1).to_le_bytes());
        let error =
            unpack_payload(&payload, temp.path()).expect_err("reject oversized data length");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid bundle: payload data length exceeds maximum {MAX_LAYOUT_ENTRY_BYTES} bytes"
            )
        );
    }

    #[test]
    fn collect_oci_entries_rejects_missing_blobs_directory() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        fs::write(root.join("oci-layout"), "{}").expect("write layout");
        fs::write(root.join("index.json"), "{}").expect("write index");

        let error = collect_oci_entries(root).expect_err("missing blobs dir should fail");

        assert!(matches!(
            error,
            BundleError::Io { path, .. } if path == root.join("blobs").display().to_string()
        ));
    }

    #[test]
    fn archive_parser_rejects_oversized_counts_and_lengths() {
        let mut archive = Vec::new();
        archive.extend_from_slice(ARCHIVE_MAGIC);
        archive.extend_from_slice(&(MAX_LAYOUT_ENTRY_COUNT + 1).to_le_bytes());
        let error = read_archive_entries(&archive).expect_err("reject oversized entry count");
        assert_eq!(
            error.to_string(),
            format!("invalid bundle: archive entry count exceeds maximum {MAX_LAYOUT_ENTRY_COUNT}")
        );

        let mut archive = Vec::new();
        archive.extend_from_slice(ARCHIVE_MAGIC);
        archive.extend_from_slice(&1_u32.to_le_bytes());
        archive.extend_from_slice(&(MAX_LAYOUT_PATH_BYTES + 1).to_le_bytes());
        let error = read_archive_entries(&archive).expect_err("reject oversized path length");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid bundle: archive path length exceeds maximum {MAX_LAYOUT_PATH_BYTES} bytes"
            )
        );

        let path = "agent.yaml";
        let mut archive = Vec::new();
        archive.extend_from_slice(ARCHIVE_MAGIC);
        archive.extend_from_slice(&1_u32.to_le_bytes());
        archive.extend_from_slice(&(path.len() as u32).to_le_bytes());
        archive.extend_from_slice(path.as_bytes());
        archive.extend_from_slice(&(MAX_LAYOUT_ENTRY_BYTES + 1).to_le_bytes());
        let error = read_archive_entries(&archive).expect_err("reject oversized entry length");
        assert_eq!(
            error.to_string(),
            format!(
                "invalid bundle: archive entry length exceeds maximum {MAX_LAYOUT_ENTRY_BYTES} bytes"
            )
        );
    }
}
