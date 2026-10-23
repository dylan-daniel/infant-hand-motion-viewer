use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::video::sha256::sha256_hex;

pub const CACHE_DIR_NAME: &str = "infant-hand-motion-cache";
const VIDEO_EXTENSION: &str = "mp4";
const PARTIAL_EXTENSION: &str = "part";
const EXPORT_EXTENSION: &str = "hexport";
const EXPORTS_DIR_NAME: &str = "exports";
const EXPORT_INDEX_NAME: &str = "index.json";

/// Where the cache lives unless configured otherwise: the OS temp directory.
pub fn default_location() -> PathBuf {
    std::env::temp_dir()
}

/// Whether `hash` is a full SHA-256 in hex, the only thing cache file names are built from.
pub fn is_video_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Remote files kept on disk under `<location>/infant-hand-motion-cache/`:
/// - source videos as `<video_hash>.mp4`, which never need checking against the server because the hash is the
///   content's identity;
/// - hand exports as `exports/<sha256>.hexport`, found through `exports/index.json` (`host:path -> sha256`) and
///   kept current by asking the server whether the hash still matches, since an export can change in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn new(location: &Path) -> Self {
        Self {
            dir: location.join(CACHE_DIR_NAME),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, hash: &str) -> Option<PathBuf> {
        is_video_hash(hash).then(|| {
            self.dir
                .join(format!("{}.{VIDEO_EXTENSION}", hash.to_ascii_lowercase()))
        })
    }

    /// The cached video for `hash`, if there is one.
    pub fn get(&self, hash: &str) -> Option<PathBuf> {
        let path = self.path_for(hash)?;
        fs::metadata(&path).ok().filter(|m| m.len() > 0).map(|_| path)
    }

    /// Save `bytes` as the video for `hash` and return where it went; a half-written file is never visible.
    pub fn store(&self, hash: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let path = self
            .path_for(hash)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a video hash"))?;
        fs::create_dir_all(&self.dir)?;
        let partial = path.with_extension(PARTIAL_EXTENSION);
        fs::write(&partial, bytes)?;
        fs::rename(&partial, &path)?;
        Ok(path)
    }

    pub fn remove(&self, hash: &str) {
        if let Some(path) = self.path_for(hash) {
            let _ = fs::remove_file(path);
        }
    }

    fn exports_dir(&self) -> PathBuf {
        self.dir.join(EXPORTS_DIR_NAME)
    }

    fn export_path_for(&self, hash: &str) -> Option<PathBuf> {
        is_video_hash(hash).then(|| {
            self.exports_dir()
                .join(format!("{}.{EXPORT_EXTENSION}", hash.to_ascii_lowercase()))
        })
    }

    fn read_index(&self) -> HashMap<String, String> {
        fs::read_to_string(self.exports_dir().join(EXPORT_INDEX_NAME))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The hash of the export last cached for `key` (`host:path`), if its file is still there.
    pub fn known_export(&self, key: &str) -> Option<String> {
        let hash = self.read_index().remove(key)?;
        self.export_path_for(&hash)?.is_file().then_some(hash)
    }

    /// The cached export with this hash. A file whose contents no longer hash to it is deleted and not returned.
    pub fn read_export(&self, hash: &str) -> Option<Vec<u8>> {
        let path = self.export_path_for(hash)?;
        let bytes = fs::read(&path).ok()?;
        if sha256_hex(&bytes) == hash.to_ascii_lowercase() {
            Some(bytes)
        } else {
            let _ = fs::remove_file(path);
            None
        }
    }

    /// Save `bytes` as the export with `hash` and point `key` at it, dropping the file `key` pointed at before
    /// unless another key still uses it. A half-written file is never visible.
    pub fn store_export(&self, key: &str, hash: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self
            .export_path_for(hash)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a sha256"))?;
        fs::create_dir_all(self.exports_dir())?;
        let partial = path.with_extension(PARTIAL_EXTENSION);
        fs::write(&partial, bytes)?;
        fs::rename(&partial, &path)?;

        let mut index = self.read_index();
        let previous = index.insert(key.to_string(), hash.to_ascii_lowercase());
        if let Some(previous) = previous.filter(|old| *old != hash.to_ascii_lowercase())
            && !index.values().any(|used| *used == previous)
            && let Some(old_path) = self.export_path_for(&previous)
        {
            let _ = fs::remove_file(old_path);
        }
        let index_path = self.exports_dir().join(EXPORT_INDEX_NAME);
        let partial = index_path.with_extension(PARTIAL_EXTENSION);
        let text = serde_json::to_string(&index).map_err(io::Error::other)?;
        fs::write(&partial, text)?;
        fs::rename(&partial, &index_path)
    }

    fn entries(&self) -> Vec<(PathBuf, u64)> {
        let listed = |dir: PathBuf, extensions: &[&str]| -> Vec<(PathBuf, u64)> {
            let Ok(read) = fs::read_dir(dir) else {
                return Vec::new();
            };
            read.flatten()
                .filter(|entry| {
                    let path = entry.path();
                    path.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| extensions.contains(&e))
                })
                .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.len())))
                .collect()
        };
        let mut entries = listed(self.dir.clone(), &[VIDEO_EXTENSION, PARTIAL_EXTENSION]);
        entries.extend(listed(
            self.exports_dir(),
            &[EXPORT_EXTENSION, PARTIAL_EXTENSION, "json"],
        ));
        entries
    }

    /// Total size of the cached videos and exports in bytes.
    pub fn size(&self) -> u64 {
        self.entries().iter().map(|(_, len)| len).sum()
    }

    /// Delete every cached video and export, leaving any other file in the directory alone; returns the bytes freed.
    pub fn clear(&self) -> u64 {
        let mut freed = 0;
        for (path, len) in self.entries() {
            if fs::remove_file(&path).is_ok() {
                freed += len;
            }
        }
        let _ = fs::remove_dir(self.exports_dir());
        let _ = fs::remove_dir(&self.dir);
        freed
    }
}

/// A byte count as a short human-readable size.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
