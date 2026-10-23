use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::sha256::sha256_file;

const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "m4v", "mov", "mkv", "avi", "webm"];

/// What an export says about its source video; the video's location is deliberately not part of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoIdentity {
    pub hash: String,
    pub basename: Option<String>,
    pub size_bytes: Option<u64>,
}

/// A video file found under a root, with what is needed to tell whether a remembered hash is still valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ns: u64,
}

/// Remembered `path -> (size, mtime, sha256)` so a video is not rehashed unless it changed.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HashIndex {
    entries: HashMap<String, (u64, u64, String)>,
    #[serde(skip)]
    dirty: bool,
}

impl HashIndex {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(self)
            && fs::write(path, text).is_ok()
        {
            self.dirty = false;
        }
    }

    /// The remembered hash of `file`, if its size and modification time are unchanged.
    pub fn lookup(&self, file: &VideoFile) -> Option<&str> {
        let (size, mtime, hash) = self.entries.get(file.path.to_str()?)?;
        (*size == file.size && *mtime == file.mtime_ns).then_some(hash.as_str())
    }

    /// The hash of `file`, from memory when valid, otherwise computed and remembered.
    pub fn hash_of(&mut self, file: &VideoFile) -> std::io::Result<String> {
        if let Some(hash) = self.lookup(file) {
            return Ok(hash.to_string());
        }
        let hash = sha256_file(&file.path)?;
        if let Some(key) = file.path.to_str() {
            self.entries
                .insert(key.to_string(), (file.size, file.mtime_ns, hash.clone()));
            self.dirty = true;
        }
        Ok(hash)
    }
}

/// Every video file under `roots`, in root order.
pub fn list_video_files(roots: &[PathBuf]) -> Vec<VideoFile> {
    let mut files = Vec::new();
    let mut pending: Vec<PathBuf> = roots.iter().rev().cloned().collect();
    while let Some(dir) = pending.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(meta) = fs::metadata(&path) else { continue };
            if meta.is_dir() {
                pending.push(path);
            } else if meta.len() > 0
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            {
                let mtime_ns = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos() as u64);
                files.push(VideoFile {
                    path,
                    size: meta.len(),
                    mtime_ns,
                });
            }
        }
    }
    files
}

/// The files worth hashing, in order: those of the wanted size (all, when the size is unknown), same-named ones first.
pub fn hash_candidates<'a>(files: &'a [VideoFile], identity: &VideoIdentity) -> Vec<&'a VideoFile> {
    let mut candidates: Vec<&VideoFile> = files
        .iter()
        .filter(|f| identity.size_bytes.is_none_or(|size| f.size == size))
        .collect();
    candidates.sort_by_key(|f| {
        let name = f.path.file_name().and_then(|n| n.to_str());
        name.is_none() || name != identity.basename.as_deref()
    });
    candidates
}

/// Finds the video under `roots` whose SHA-256 is `identity.hash`. `progress(done, total)` reports hashing of
/// files not already remembered. Returns `None` when nothing matches or `cancelled` turns true.
pub fn find_video(
    roots: &[PathBuf],
    identity: &VideoIdentity,
    index: &mut HashIndex,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Option<PathBuf> {
    if identity.hash.is_empty() {
        return None;
    }
    let files = list_video_files(roots);
    if let Some(known) = files.iter().find(|f| index.lookup(f) == Some(identity.hash.as_str())) {
        return Some(known.path.clone());
    }
    let pending: Vec<&VideoFile> = hash_candidates(&files, identity)
        .into_iter()
        .filter(|f| index.lookup(f).is_none())
        .collect();
    let total = pending.len();
    for (done, file) in pending.into_iter().enumerate() {
        if cancelled() {
            return None;
        }
        progress(done, total);
        if index.hash_of(file).is_ok_and(|hash| hash == identity.hash) {
            return Some(file.path.clone());
        }
    }
    None
}
