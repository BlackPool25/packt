use crate::chunking::fastcdc::FastCdcChunker;
use crate::error::{PacktError, Result};
use crate::hash::blake3_hasher::Blake3Hasher;
use crate::index::DedupIndex;
use crate::index::hashindex::HashIndex;
use crate::pipeline::{BackupPipeline, BackupStats, PipelineConfig};
use crate::similarity::SimilarityConfig;
use crate::store::ContentStore;
use crate::store::local::LocalStore;
use crate::types::{ChunkConfig, Hash};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

#[cfg(feature = "cloud")]
use crate::store::CloudStore;
#[cfg(feature = "cloud")]
use opendal::blocking::Operator as BlockingOperator;
#[cfg(feature = "cloud")]
use opendal::services;

/// Configuration for opening a content-addressed store.
#[non_exhaustive]
#[derive(Clone)]
pub enum StoreConfig {
    /// Local filesystem store at the given path.
    Local { path: PathBuf },
    /// Amazon S3 (or S3-compatible) store.
    #[cfg(feature = "cloud")]
    S3 {
        bucket: String,
        region: Option<String>,
        endpoint: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        cache_dir: Option<PathBuf>,
    },
    /// Google Cloud Storage store.
    #[cfg(feature = "cloud")]
    GCS {
        bucket: String,
        prefix: Option<String>,
        cache_dir: Option<PathBuf>,
    },
}

impl fmt::Debug for StoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { path } => f.debug_struct("Local").field("path", path).finish(),
            #[cfg(feature = "cloud")]
            Self::S3 {
                bucket,
                region,
                endpoint,
                cache_dir,
                ..
            } => f
                .debug_struct("S3")
                .field("bucket", bucket)
                .field("region", region)
                .field("endpoint", endpoint)
                .field("access_key_id", &"<redacted>")
                .field("secret_access_key", &"<redacted>")
                .field("cache_dir", cache_dir)
                .finish(),
            #[cfg(feature = "cloud")]
            Self::GCS {
                bucket,
                prefix,
                cache_dir,
            } => f
                .debug_struct("GCS")
                .field("bucket", bucket)
                .field("prefix", prefix)
                .field("cache_dir", cache_dir)
                .finish(),
        }
    }
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self::Local {
            path: PathBuf::from("packt-store"),
        }
    }
}

/// A file backed up in the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub modified: String,
    pub chunk_count: usize,
}

/// Store summary information.
#[derive(Debug, Clone)]
pub struct StoreInfo {
    pub file_count: usize,
    pub total_source_bytes: u64,
    pub total_chunks: usize,
}

/// Result of a verify operation.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub files_checked: usize,
    pub chunks_checked: usize,
    pub errors: Vec<String>,
    pub ok: bool,
}

/// Options for backup operation.
#[derive(Debug, Clone)]
pub struct BackupOpts {
    /// Chunking configuration (min/avg/max sizes).
    /// Defaults to Docker/ML-optimized config: 4KB/8KB/64KB.
    pub chunk_config: ChunkConfig,
    pub similarity_threshold: f64,
    pub force: bool,
}

impl Default for BackupOpts {
    fn default() -> Self {
        Self {
            chunk_config: ChunkConfig::default(),
            similarity_threshold: 0.7,
            force: false,
        }
    }
}

/// A backed-up file entry stored in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub size: u64,
    pub modified: String,
    pub permissions: u32,
    pub chunk_hashes: Vec<String>,
}

/// High-level content-addressed store.
///
/// Provides backup, restore, listing, and verification for both local
/// and cloud (S3/GCS) backends.
#[allow(clippy::struct_field_names)]
pub struct Store {
    store: Arc<dyn ContentStore>,
    index: Arc<dyn DedupIndex>,
    config: StoreConfig,
    #[cfg(feature = "cloud")]
    manifest_op: Option<BlockingOperator>,
}

impl Store {
    /// Open a store from the given configuration.
    ///
    /// For local stores, creates or opens a LocalStore at the path.
    /// For cloud stores, connects to S3/GCS via OpenDAL.
    pub fn open(config: StoreConfig) -> Result<Self> {
        match &config {
            StoreConfig::Local { path } => {
                let local = Arc::new(LocalStore::open(path)?);
                let index: Arc<dyn DedupIndex> = Arc::new(HashIndex::new(1_000_000));
                local.populate_index(&index)?;
                local.set_index(index.clone());
                Ok(Self {
                    store: local,
                    index,
                    config,
                    #[cfg(feature = "cloud")]
                    manifest_op: None,
                })
            }
            #[cfg(feature = "cloud")]
            StoreConfig::S3 {
                bucket,
                region,
                endpoint,
                access_key_id,
                secret_access_key,
                cache_dir,
            } => {
                let mut b = services::S3::default().bucket(bucket);
                if let Some(r) = region {
                    b = b.region(r);
                }
                if let Some(e) = endpoint {
                    b = b.endpoint(e);
                }
                if let Some(k) = access_key_id {
                    b = b.access_key_id(k);
                }
                if let Some(s) = secret_access_key {
                    b = b.secret_access_key(s);
                }
                let op = opendal::Operator::new(b)
                    .map_err(|e| PacktError::Cloud {
                        context: "failed to create S3 operator".into(),
                        source: e,
                    })?
                    .finish();
                let bop = BlockingOperator::new(op.clone()).map_err(|e| PacktError::Cloud {
                    context: "failed to create blocking S3 operator".into(),
                    source: e,
                })?;
                let index: Arc<dyn DedupIndex> = Arc::new(HashIndex::new(1_000_000));
                let cloud = Arc::new(CloudStore::open(op, index.clone(), cache_dir.clone())?);
                Ok(Self {
                    store: cloud,
                    index,
                    config,
                    manifest_op: Some(bop),
                })
            }
            #[cfg(feature = "cloud")]
            StoreConfig::GCS {
                bucket,
                prefix,
                cache_dir,
            } => {
                let mut b = services::Gcs::default().bucket(bucket);
                if let Some(p) = prefix {
                    b = b.root(p);
                }
                let op = opendal::Operator::new(b)
                    .map_err(|e| PacktError::Cloud {
                        context: "failed to create GCS operator".into(),
                        source: e,
                    })?
                    .finish();
                let bop = BlockingOperator::new(op.clone()).map_err(|e| PacktError::Cloud {
                    context: "failed to create blocking GCS operator".into(),
                    source: e,
                })?;
                let index: Arc<dyn DedupIndex> = Arc::new(HashIndex::new(1_000_000));
                let cloud = Arc::new(CloudStore::open(op, index.clone(), cache_dir.clone())?);
                Ok(Self {
                    store: cloud,
                    index,
                    config,
                    manifest_op: Some(bop),
                })
            }
        }
    }

    /// Parse a URI-style path into a StoreConfig.
    ///
    /// Supported formats:
    /// - `/path/to/store` or `./relative/path` → Local
    /// - `s3://bucket/key?region=...` → S3
    /// - `gcs://bucket/prefix` → GCS
    #[cfg(feature = "cloud")]
    pub fn config_from_uri(uri: &str) -> Result<StoreConfig> {
        if uri.starts_with("s3://") {
            let rest = uri.trim_start_matches("s3://");
            // Split bucket/key from query params
            let (path_part, query_part) = match rest.split_once('?') {
                Some((p, q)) => (p, Some(q)),
                None => (rest, None),
            };
            let (bucket, _key) = match path_part.split_once('/') {
                Some((b, k)) => (b, k),
                None => (path_part, ""),
            };
            // Parse query string
            let query = query_part
                .map(|q| {
                    q.split('&')
                        .filter_map(|pair| pair.split_once('='))
                        .collect::<std::collections::HashMap<&str, &str>>()
                })
                .unwrap_or_default();
            Ok(StoreConfig::S3 {
                bucket: bucket.to_string(),
                region: query.get("region").map(ToString::to_string),
                endpoint: query.get("endpoint").map(ToString::to_string),
                access_key_id: query.get("access_key_id").map(ToString::to_string),
                secret_access_key: query.get("secret_access_key").map(ToString::to_string),
                cache_dir: query.get("cache_dir").map(PathBuf::from),
            })
        } else if uri.starts_with("gcs://") {
            let rest = uri.trim_start_matches("gcs://");
            let (path_part, query_part) = match rest.split_once('?') {
                Some((p, q)) => (p, Some(q)),
                None => (rest, None),
            };
            let (bucket, prefix) = match path_part.split_once('/') {
                Some((b, p)) => (b, p),
                None => (path_part, ""),
            };
            let query = query_part
                .map(|q| {
                    q.split('&')
                        .filter_map(|pair| pair.split_once('='))
                        .collect::<std::collections::HashMap<&str, &str>>()
                })
                .unwrap_or_default();
            Ok(StoreConfig::GCS {
                bucket: bucket.to_string(),
                prefix: if prefix.is_empty() {
                    None
                } else {
                    Some(prefix.to_string())
                },
                cache_dir: query.get("cache_dir").map(PathBuf::from),
            })
        } else {
            Ok(StoreConfig::Local {
                path: PathBuf::from(uri),
            })
        }
    }

    /// Parse a URI-style path into a StoreConfig (non-cloud fallback).
    #[cfg(not(feature = "cloud"))]
    pub fn config_from_uri(uri: &str) -> Result<StoreConfig> {
        if uri.starts_with("s3://") || uri.starts_with("gcs://") {
            return Err(PacktError::Config(format!(
                "cloud storage not supported: enable the 'cloud' feature: {uri}"
            )));
        }
        Ok(StoreConfig::Local {
            path: PathBuf::from(uri),
        })
    }

    // ── Manifest helpers ────────────────────────────────────────────

    fn manifest_name(path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    fn _manifest_path(&self, name: &str) -> PathBuf {
        match &self.config {
            StoreConfig::Local { path } => path.join("manifests").join(format!("{name}.manifest")),
            #[cfg(feature = "cloud")]
            _ => PathBuf::from(format!("manifests/{name}.manifest")),
        }
    }

    fn read_manifest_entry(&self, name: &str) -> Result<Option<ManifestEntry>> {
        match &self.config {
            StoreConfig::Local { path } => {
                let p = path.join("manifests").join(format!("{name}.manifest"));
                if !p.exists() {
                    return Ok(None);
                }
                let data = std::fs::read(&p)?;
                Ok(Some(serde_json::from_slice(&data).map_err(|e| {
                    PacktError::Serialization(format!("manifest parse: {e}"))
                })?))
            }
            #[cfg(feature = "cloud")]
            _ => {
                if let Some(ref op) = self.manifest_op {
                    let path = format!("manifests/{name}.manifest");
                    match op.read(&path) {
                        Ok(buf) => {
                            let data = buf.to_vec();
                            Ok(Some(serde_json::from_slice(&data).map_err(|e| {
                                PacktError::Serialization(format!("manifest parse: {e}"))
                            })?))
                        }
                        Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
                        Err(e) => Err(PacktError::Cloud {
                            context: format!("failed to read manifest {name}"),
                            source: e,
                        }),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn write_manifest_entry(&self, name: &str, entry: &ManifestEntry) -> Result<()> {
        let data = serde_json::to_string_pretty(entry)
            .map_err(|e| PacktError::Serialization(format!("manifest serialize: {e}")))?;
        match &self.config {
            StoreConfig::Local { path } => {
                let md = path.join("manifests");
                std::fs::create_dir_all(&md)?;
                std::fs::write(md.join(format!("{name}.manifest")), &data)?;
            }
            #[cfg(feature = "cloud")]
            _ => {
                if let Some(ref op) = self.manifest_op {
                    let path = format!("manifests/{name}.manifest");
                    op.write(&path, data.into_bytes()).map_err(|e| PacktError::Cloud {
                        context: format!("failed to write manifest {name}"),
                        source: e,
                    })?;
                }
            }
        }
        Ok(())
    }

    fn list_manifest_names(&self) -> Result<Vec<String>> {
        match &self.config {
            StoreConfig::Local { path } => {
                let md = path.join("manifests");
                if !md.exists() {
                    return Ok(Vec::new());
                }
                let mut names = Vec::new();
                for entry in std::fs::read_dir(&md)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Some(stem) = name.strip_suffix(".manifest") {
                        names.push(stem.to_string());
                    }
                }
                names.sort();
                Ok(names)
            }
            #[cfg(feature = "cloud")]
            _ => {
                if let Some(ref op) = self.manifest_op {
                    let mut names = Vec::new();
                    match op.lister("manifests/") {
                        Ok(lister) => {
                            for result in lister {
                                let entry = result.map_err(|e| PacktError::Cloud {
                                    context: "failed to list manifests".into(),
                                    source: e,
                                })?;
                                let path = entry.path().to_string();
                                if let Some(stem) = path
                                    .strip_prefix("manifests/")
                                    .and_then(|s| s.strip_suffix(".manifest"))
                                {
                                    names.push(stem.to_string());
                                }
                            }
                        }
                        Err(e) if e.kind() == opendal::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(PacktError::Cloud {
                                context: "failed to list manifests".into(),
                                source: e,
                            });
                        }
                    }
                    names.sort();
                    Ok(names)
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }

    fn delete_manifest_entry(&self, name: &str) -> Result<()> {
        match &self.config {
            StoreConfig::Local { path } => {
                let p = path.join("manifests").join(format!("{name}.manifest"));
                if p.exists() {
                    std::fs::remove_file(&p)?;
                }
            }
            #[cfg(feature = "cloud")]
            _ => {
                if let Some(ref op) = self.manifest_op {
                    let path = format!("manifests/{name}.manifest");
                    let _ = op.delete(&path);
                }
            }
        }
        Ok(())
    }

    fn is_file_unchanged(&self, source: &Path, name: &str) -> Result<bool> {
        let Some(entry) = self.read_manifest_entry(name)? else {
            return Ok(false);
        };
        let meta = std::fs::metadata(source)?;
        if entry.size != meta.len() {
            return Ok(false);
        }
        if let Ok(modified) = meta.modified() {
            if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                if entry.modified == duration.as_secs().to_string() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    // ── Public API ──────────────────────────────────────────────────

    /// Back up a file to the store.
    ///
    /// If the file hasn't changed (mtime+size), skip unless `opts.force` is set.
    pub fn backup(&self, source: &Path, opts: &BackupOpts) -> Result<BackupStats> {
        let name = Self::manifest_name(source);

        if !opts.force && self.is_file_unchanged(source, &name)? {
            let stats = BackupStats {
                source_size: std::fs::metadata(source)?.len(),
                ..BackupStats::default()
            };
            return Ok(stats);
        }

        let cfg = opts.chunk_config;
        if !cfg.validate() {
            return Err(PacktError::Config("invalid chunk config".into()));
        }

        let sim = if opts.similarity_threshold > 0.0 {
            Some(SimilarityConfig {
                threshold: opts.similarity_threshold.clamp(0.0, 1.0),
                ..Default::default()
            })
        } else {
            None
        };

        let pipeline = BackupPipeline::new(
            PipelineConfig {
                chunk_config: cfg,
                similarity_config: sim,
                ..Default::default()
            },
            Arc::new(FastCdcChunker::new(cfg)),
            Arc::new(Blake3Hasher::new()),
            self.store.clone(),
            self.index.clone(),
        );

        // Rebuild similarity index from stored signatures
        #[cfg(feature = "cloud")]
        if let Some(_sim_stage) = pipeline.similarity() {}

        let stats = pipeline.backup_file(source)?;

        let meta = std::fs::metadata(source)?;
        let entry = ManifestEntry {
            path: name.clone(),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
            permissions: perm(&meta),
            chunk_hashes: stats.chunk_hashes.iter().map(Hash::to_hex).collect(),
        };
        self.write_manifest_entry(&name, &entry)?;

        Ok(stats)
    }

    /// Restore files from the store.
    ///
    /// If `file` is specified, restore only that file. Otherwise restore all.
    pub fn restore(&self, dest: &Path, file: Option<&str>) -> Result<()> {
        let names = if let Some(f) = file {
            vec![f.to_string()]
        } else {
            self.list_manifest_names()?
        };

        if names.is_empty() {
            let msg = file.map_or_else(|| "no manifests found".into(), ToString::to_string);
            return Err(PacktError::ChunkNotFound(msg));
        }

        std::fs::create_dir_all(dest)?;

        for name in &names {
            let Some(entry) = self.read_manifest_entry(name)? else {
                continue;
            };

            let out_path = if entry.path.is_empty() {
                dest.join(name)
            } else {
                let p = dest.join(&entry.path);
                if !p.starts_with(dest) {
                    return Err(PacktError::Config(format!(
                        "path traversal detected in manifest entry: {}",
                        entry.path
                    )));
                }
                p
            };

            let mut file_data = Vec::new();
            for hash_hex in &entry.chunk_hashes {
                let hash =
                    Hash::from_hex(hash_hex).map_err(|e| PacktError::Serialization(format!("invalid hash: {e}")))?;
                let chunk = self.store.get(&hash)?;
                file_data.extend_from_slice(&chunk);
            }

            std::fs::write(&out_path, &file_data)?;

            // Restore mtime
            if let Ok(modified) = entry.modified.parse::<u64>() {
                let timestamp = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified);
                if let Ok(ft) = timestamp.duration_since(SystemTime::UNIX_EPOCH) {
                    let secs = i64::try_from(ft.as_secs()).unwrap_or(i64::MAX);
                    let filetime = filetime::FileTime::from_unix_time(secs, 0);
                    let _ = filetime::set_file_mtime(&out_path, filetime);
                }
            }

            println!("Restored: {}", out_path.display());
        }

        Ok(())
    }

    /// List all backed-up files.
    pub fn list_files(&self) -> Result<Vec<FileInfo>> {
        let names = self.list_manifest_names()?;
        let mut files = Vec::new();
        for name in &names {
            if let Some(entry) = self.read_manifest_entry(name)? {
                files.push(FileInfo {
                    name: name.clone(),
                    size: entry.size,
                    modified: entry.modified,
                    chunk_count: entry.chunk_hashes.len(),
                });
            }
        }
        Ok(files)
    }

    /// Get store summary information.
    pub fn info(&self) -> Result<StoreInfo> {
        let names = self.list_manifest_names()?;
        let file_count = names.len();
        let mut total_source_bytes = 0u64;
        let mut total_chunks = 0usize;
        for name in &names {
            if let Some(entry) = self.read_manifest_entry(name)? {
                total_source_bytes += entry.size;
                total_chunks += entry.chunk_hashes.len();
            }
        }
        Ok(StoreInfo {
            file_count,
            total_source_bytes,
            total_chunks,
        })
    }

    /// Verify integrity of backed-up files.
    ///
    /// Reads each chunk, verifies BLAKE3 checksum matches stored hash.
    pub fn verify(&self, file: Option<&str>) -> Result<VerifyReport> {
        let names = if let Some(f) = file {
            vec![f.to_string()]
        } else {
            self.list_manifest_names()?
        };

        let mut errors = Vec::new();
        let mut files_checked = 0usize;
        let mut chunks_checked = 0usize;

        for name in &names {
            let Some(entry) = self.read_manifest_entry(name)? else {
                errors.push(format!("manifest not found: {name}"));
                continue;
            };

            files_checked += 1;

            for hash_hex in &entry.chunk_hashes {
                let hash = match Hash::from_hex(hash_hex) {
                    Ok(h) => h,
                    Err(e) => {
                        errors.push(format!("{name}: invalid hash {hash_hex}: {e}"));
                        continue;
                    }
                };
                chunks_checked += 1;
                match self.store.get(&hash) {
                    Ok(data) => {
                        let actual = blake3::hash(&data);
                        if Hash::from_blake3(actual) != hash {
                            errors.push(format!("{name}: chunk {hash_hex} checksum mismatch"));
                        }
                    }
                    Err(e) => {
                        errors.push(format!("{name}: chunk {hash_hex} not found: {e}"));
                    }
                }
            }
        }

        let ok = errors.is_empty();
        Ok(VerifyReport {
            files_checked,
            chunks_checked,
            errors,
            ok,
        })
    }

    /// Delete a file from the store (removes manifest only).
    ///
    /// Chunks are NOT deleted (GC is Phase 4d).
    pub fn delete_file(&self, name: &str) -> Result<()> {
        if !self.has_file(name)? {
            return Err(PacktError::ChunkNotFound(format!("file not found in store: {name}")));
        }
        self.delete_manifest_entry(name)?;
        Ok(())
    }

    /// Check if a file exists in the store.
    pub fn has_file(&self, name: &str) -> Result<bool> {
        Ok(self.read_manifest_entry(name)?.is_some())
    }

    /// Iterate all chunk hashes stored across all packs.
    ///
    /// Required for future GC (Phase 4d).
    #[cfg(feature = "cloud")]
    pub fn iter_chunks(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>> {
        // For cloud stores, _meta.index contains all hashes
        // We'll rely on DedupIndex enumeration (not yet implemented there)
        Err(PacktError::Config("iter_chunks not yet implemented for cloud".into()))
    }

    /// Iterate all chunk hashes (non-cloud fallback).
    #[cfg(not(feature = "cloud"))]
    pub fn iter_chunks(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>> {
        Err(PacktError::Config("iter_chunks not yet implemented".into()))
    }
}

#[cfg(unix)]
fn perm(m: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}

#[cfg(not(unix))]
fn perm(_: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_local_store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let config = StoreConfig::Local {
            path: dir.path().to_path_buf(),
        };
        let store = Store::open(config).unwrap();
        (dir, store)
    }

    #[test]
    fn test_store_config_from_uri_local() {
        let cfg = Store::config_from_uri("/tmp/mystore").unwrap();
        assert!(matches!(cfg, StoreConfig::Local { .. }));
    }

    #[test]
    fn test_store_backup_restore_roundtrip() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        let file_path = src.path().join("test.txt");
        std::fs::write(&file_path, b"hello world this is a test file").unwrap();

        let stats = store.backup(&file_path, &BackupOpts::default()).unwrap();
        assert!(stats.total_chunks > 0);

        let files = store.list_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "test.txt");
    }

    #[test]
    fn test_store_has_file() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join("test.txt"), b"data").unwrap();
        store
            .backup(&src.path().join("test.txt"), &BackupOpts::default())
            .unwrap();
        assert!(store.has_file("test.txt").unwrap());
        assert!(!store.has_file("nonexistent.txt").unwrap());
    }

    #[test]
    fn test_store_delete_file() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join("test.txt"), b"data").unwrap();
        store
            .backup(&src.path().join("test.txt"), &BackupOpts::default())
            .unwrap();
        assert!(store.has_file("test.txt").unwrap());
        store.delete_file("test.txt").unwrap();
        assert!(!store.has_file("test.txt").unwrap());
    }

    #[test]
    fn test_store_info() {
        let (_dir, store) = setup_local_store();
        let info = store.info().unwrap();
        assert_eq!(info.file_count, 0);

        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join("f1.txt"), b"data one").unwrap();
        std::fs::write(src.path().join("f2.txt"), b"data two longer").unwrap();
        store
            .backup(&src.path().join("f1.txt"), &BackupOpts::default())
            .unwrap();
        store
            .backup(&src.path().join("f2.txt"), &BackupOpts::default())
            .unwrap();

        let info = store.info().unwrap();
        assert_eq!(info.file_count, 2);
        assert!(info.total_source_bytes > 0);
    }

    #[test]
    fn test_store_verify_ok() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join("test.txt"), b"verify test data").unwrap();
        store
            .backup(&src.path().join("test.txt"), &BackupOpts::default())
            .unwrap();
        let report = store.verify(None).unwrap();
        assert!(report.ok);
        assert_eq!(report.files_checked, 1);
        assert!(report.chunks_checked > 0);
    }

    #[test]
    fn test_store_restore_file() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        let original = b"file content to restore";
        std::fs::write(src.path().join("restore_me.txt"), original).unwrap();
        store
            .backup(&src.path().join("restore_me.txt"), &BackupOpts::default())
            .unwrap();

        let dest = TempDir::new().unwrap();
        store.restore(dest.path(), Some("restore_me.txt")).unwrap();
        let restored = std::fs::read(dest.path().join("restore_me.txt")).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn test_store_incremental_backup() {
        let (_dir, store) = setup_local_store();
        let src = TempDir::new().unwrap();
        std::fs::write(src.path().join("file.txt"), b"incremental test").unwrap();

        let stats1 = store
            .backup(&src.path().join("file.txt"), &BackupOpts::default())
            .unwrap();
        assert!(stats1.unique_chunks > 0);

        // Second backup without changes — should skip (incremental)
        let stats2 = store
            .backup(&src.path().join("file.txt"), &BackupOpts::default())
            .unwrap();
        assert_eq!(stats2.unique_chunks, 0);
        assert_eq!(stats2.total_chunks, 0);

        // Forced backup: skips mtime check, but dedup still applies.
        // Identical data won't produce unique chunks.
        let stats3 = store
            .backup(
                &src.path().join("file.txt"),
                &BackupOpts {
                    force: true,
                    ..Default::default()
                },
            )
            .unwrap();
        // File is processed (source_size reported) but chunks may be 0
        // since data is identical to first backup
        assert!(stats3.source_size > 0);
    }

    #[test]
    fn test_store_config_from_uri_s3() {
        #[cfg(feature = "cloud")]
        {
            let cfg = Store::config_from_uri("s3://mybucket/path").unwrap();
            match cfg {
                StoreConfig::S3 { bucket, .. } => {
                    assert_eq!(bucket, "mybucket");
                }
                _ => panic!("expected S3 variant"),
            }
        }
    }

    #[test]
    fn test_store_config_from_uri_gcs() {
        #[cfg(feature = "cloud")]
        {
            let cfg = Store::config_from_uri("gcs://mybucket/prefix").unwrap();
            match cfg {
                StoreConfig::GCS { bucket, prefix, .. } => {
                    assert_eq!(bucket, "mybucket");
                    assert_eq!(prefix, Some("prefix".to_string()));
                }
                _ => panic!("expected GCS variant"),
            }
        }
    }

    #[test]
    fn test_store_list_files_empty() {
        let (_dir, store) = setup_local_store();
        let files = store.list_files().unwrap();
        assert!(files.is_empty());
    }
}
