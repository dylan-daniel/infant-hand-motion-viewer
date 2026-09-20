use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

static CUSTOM_CACHE_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Manages local disk caching for downloaded .hexport files and frame images.
pub struct CacheManager;

impl CacheManager {
    /// Initialize cache manager (ensures cache directories exist).
    pub fn init() -> std::io::Result<()> {
        let root = Self::get_cache_root();
        fs::create_dir_all(&root)
    }

    /// Default cache directory: `<temp_dir>/infant_hand_motion_viewer_cache`
    pub fn get_default_cache_root() -> PathBuf {
        std::env::temp_dir().join("infant_hand_motion_viewer_cache")
    }

    /// Set custom cache root directory. Pass `None` to revert to the default path.
    pub fn set_custom_cache_root(custom_root: Option<PathBuf>) {
        if let Ok(mut lock) = CUSTOM_CACHE_ROOT.write() {
            *lock = custom_root;
        }
        let _ = Self::init();
    }

    /// Get custom cache root if set.
    pub fn get_custom_cache_root() -> Option<PathBuf> {
        CUSTOM_CACHE_ROOT.read().ok().and_then(|lock| lock.clone())
    }

    /// Current effective cache root directory.
    pub fn get_cache_root() -> PathBuf {
        if let Some(custom) = Self::get_custom_cache_root()
            && !custom.as_os_str().is_empty()
        {
            return custom;
        }
        Self::get_default_cache_root()
    }

    /// Sanitize identifier for host directory: only alphanumeric, '-' and '_', else '_'.
    pub fn sanitize_identifier(input: &str) -> String {
        let mut result = String::new();
        for c in input.chars() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                result.push(c);
            } else {
                result.push('_');
            }
        }
        if result.is_empty() {
            "default".to_string()
        } else {
            result
        }
    }

    /// Convert a remote host and export file path to a local cached `.hexport` path.
    pub fn get_local_export_path(host: &str, remote_export_path: &Path) -> PathBuf {
        let safe_host = Self::sanitize_identifier(host);
        let filename = remote_export_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent_name = remote_export_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut local_p = Self::get_cache_root().join(safe_host);
        if !parent_name.is_empty() {
            local_p.push(parent_name);
        }
        local_p.push(filename);
        local_p
    }

    /// Get local frames directory for a cached export path:
    /// returns `<parent_dir>/frames/<frames_key>` where `frames_key` is parsed from stem up to 2nd `__`.
    pub fn get_local_frames_dir(local_export_path: &Path) -> PathBuf {
        let stem = local_export_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let frames_key = if let Some(pos1) = stem.find("__") {
            if let Some(pos2) = stem[pos1 + 2..].find("__") {
                stem[..pos1 + 2 + pos2].to_string()
            } else {
                stem
            }
        } else {
            stem
        };

        let parent = local_export_path.parent().unwrap_or(Path::new("."));
        parent.join("frames").join(frames_key)
    }

    /// Check if the `.hexport` file is already cached locally and valid.
    pub fn is_export_cached(local_export_path: &Path) -> bool {
        if let Ok(meta) = fs::metadata(local_export_path) {
            meta.is_file() && meta.len() > 0
        } else {
            false
        }
    }

    /// Check if frames folder is already populated locally with at least one image (`.jpg`, `.png`, `.jpeg`).
    pub fn are_frames_cached(local_frames_dir: &Path) -> bool {
        if let Ok(entries) = fs::read_dir(local_frames_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type()
                    && file_type.is_file()
                    && let Some(ext) = entry.path().extension().and_then(|e| e.to_str())
                {
                    let ext_lower = ext.to_ascii_lowercase();
                    if ext_lower == "jpg" || ext_lower == "png" || ext_lower == "jpeg" {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Calculate total disk space currently used by the cache in bytes.
    pub fn calculate_cache_size_bytes() -> u64 {
        let root = Self::get_cache_root();
        Self::dir_size_recursive(&root)
    }

    fn dir_size_recursive(dir: &Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total += meta.len();
                    } else if meta.is_dir() {
                        total += Self::dir_size_recursive(&path);
                    }
                }
            }
        }
        total
    }

    /// Clear all files in the current cache directory.
    pub fn clear_cache() -> std::io::Result<()> {
        let root = Self::get_cache_root();
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        Self::init()
    }
}
