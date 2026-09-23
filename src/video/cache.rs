use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const CACHE_DIR_NAME: &str = "infant-hand-motion-cache";
const VIDEO_EXTENSION: &str = "mp4";
const PARTIAL_EXTENSION: &str = "part";

/// Where the cache lives unless configured otherwise: the OS temp directory.
pub fn default_location() -> PathBuf {
    std::env::temp_dir()
}

/// Whether `hash` is a full SHA-256 in hex, the only thing cache file names are built from.
pub fn is_video_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The video hash a pipeline export is named after: `<video_hash>__<fps>__<params>.hexport`.
pub fn video_hash_from_export_path(export_path: &str) -> Option<String> {
    let name = export_path.rsplit(['/', '\\']).next()?;
    let hash = name.split("__").next()?;
    is_video_hash(hash).then(|| hash.to_ascii_lowercase())
}

/// Downloaded source videos kept on disk as `<location>/infant-hand-motion-cache/<video_hash>.mp4`.
/// Only videos are cached: exports change between pipeline runs, videos do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCache {
    dir: PathBuf,
}

impl VideoCache {
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

    fn entries(&self) -> Vec<(PathBuf, u64)> {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        read.flatten()
            .filter(|entry| {
                let path = entry.path();
                matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some(VIDEO_EXTENSION | PARTIAL_EXTENSION)
                )
            })
            .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.len())))
            .collect()
    }

    /// Total size of the cached videos in bytes.
    pub fn size(&self) -> u64 {
        self.entries().iter().map(|(_, len)| len).sum()
    }

    /// Delete every cached video, leaving any other file in the directory alone; returns the bytes freed.
    pub fn clear(&self) -> u64 {
        let mut freed = 0;
        for (path, len) in self.entries() {
            if fs::remove_file(&path).is_ok() {
                freed += len;
            }
        }
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
