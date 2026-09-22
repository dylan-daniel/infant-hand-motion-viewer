use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::data::geometry::{HandCamera, HandData};
use crate::data::hand_export::{HandExportError, HandExportRow, load_hand_export, parse_hand_export};
use crate::data::mano_model::mano_forward;

pub const FLAG_LAYER_COUNT: usize = 7;

pub type Frame = Vec<HandData>;

/// Resolves the sibling `frames/` directory containing source video images for an export file.
pub fn resolve_frames_dir<P: AsRef<Path>>(export_file: P) -> PathBuf {
    let export_path = export_file.as_ref();
    let stem = export_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let frames_key = match stem.find("__") {
        Some(pos1) => match stem[pos1 + 2..].find("__") {
            Some(pos2) => &stem[..pos1 + 2 + pos2],
            None => &stem,
        },
        None => &stem,
    };

    let parent = export_path.parent().unwrap_or_else(|| Path::new("."));
    let grandparent = parent.parent().unwrap_or_else(|| Path::new("."));

    let candidates = [
        grandparent.join("frames").join(frames_key),
        parent.join("frames").join(frames_key),
        parent.join(frames_key),
        parent.join("frames").join(&stem),
        grandparent.join("frames").join(&stem),
    ];

    for candidate in &candidates {
        if candidate.is_dir() {
            return candidate.clone();
        }
    }

    // Default candidate path even if not yet created on disk
    parent.join("frames").join(frames_key)
}

/// MeshSequence holds all per-frame hand meshes and flags loaded from a `.hexport` binary file.
#[derive(Debug, Clone)]
pub struct MeshSequence {
    path: String,
    /// The local `.hexport` this was read from; `None` for sequences built from bytes, which have no local frame images.
    export_path: Option<PathBuf>,
    frame_index: Arc<Mutex<Option<FrameIndex>>>,
    frames: Vec<Frame>,
    frame_numbers: Vec<i32>,
    frame_flags_all: Vec<[bool; FLAG_LAYER_COUNT]>,
    frame_flags_infant_only: Vec<[bool; FLAG_LAYER_COUNT]>,
}

impl MeshSequence {
    /// Loads a `.hexport` file, runs the MANO forward pass for all hands, and groups by frame.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, HandExportError> {
        let path_buf = path.as_ref().to_path_buf();
        let path_str = path_buf.to_string_lossy().to_string();

        let rows = load_hand_export(&path_buf)?;
        Ok(Self::from_rows(path_str, Some(path_buf), rows))
    }

    /// Builds a sequence from the bytes of a `.hexport` file held in memory. `label` identifies the source
    /// (for example the remote path) and no local frame images are looked up.
    pub fn from_bytes(label: &str, raw: &[u8]) -> Result<Self, HandExportError> {
        let rows = parse_hand_export(raw, label)?;
        Ok(Self::from_rows(label.to_string(), None, rows))
    }

    fn from_rows(path_str: String, export_path: Option<PathBuf>, rows: Vec<HandExportRow>) -> Self {
        let mut by_frame: BTreeMap<i32, Frame> = BTreeMap::new();
        let mut flags_all_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();
        let mut flags_infant_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();

        for row in rows {
            let hand = mano_forward(&row.params);
            let row_flags = [
                row.flag_same_side_infant_conflict != 0,
                row.flag_same_side_infant_unknown_conflict != 0,
                row.flag_translation_jump != 0,
                row.flag_pose_rotation_jump != 0,
                row.flag_scale_jump != 0,
                row.flag_track_contaminated != 0,
                row.flag_track_fragmented != 0,
            ];

            let data = HandData {
                verts: hand.verts,
                joints: hand.joints,
                is_right: row.params.is_right,
                hand_track_id: row.hand_track_id,
                label: row.label.clone(),
                camera: Some(HandCamera {
                    cam_t: row.params.cam_t,
                    focal_length: row.scaled_focal_length,
                    img_w: row.img_w.max(0) as u32,
                    img_h: row.img_h.max(0) as u32,
                }),
                flags: row_flags,
            };

            let frame_num = row.frame;
            by_frame.entry(frame_num).or_default().push(data);

            let flags_all = flags_all_by_frame.entry(frame_num).or_insert([false; FLAG_LAYER_COUNT]);
            let flags_infant = flags_infant_by_frame
                .entry(frame_num)
                .or_insert([false; FLAG_LAYER_COUNT]);
            let is_infant = row.label == "infant";

            for i in 0..FLAG_LAYER_COUNT {
                if row_flags[i] {
                    flags_all[i] = true;
                    if is_infant {
                        flags_infant[i] = true;
                    }
                }
            }
        }

        let mut frames = Vec::with_capacity(by_frame.len());
        let mut frame_numbers = Vec::with_capacity(by_frame.len());
        let mut frame_flags_all = Vec::with_capacity(by_frame.len());
        let mut frame_flags_infant_only = Vec::with_capacity(by_frame.len());

        for (frame_num, hands) in by_frame {
            frame_numbers.push(frame_num);
            frame_flags_all.push(flags_all_by_frame.remove(&frame_num).unwrap());
            frame_flags_infant_only.push(flags_infant_by_frame.remove(&frame_num).unwrap());
            frames.push(hands);
        }

        Self {
            path: path_str,
            export_path,
            frame_index: Arc::new(Mutex::new(None)),
            frames,
            frame_numbers,
            frame_flags_all,
            frame_flags_infant_only,
        }
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The directory holding this sequence's video frame images, looked up afresh so frames that appear
    /// after the sequence was opened are still found. `None` for sequences without a local export file.
    pub fn frames_dir(&self) -> Option<PathBuf> {
        self.export_path.as_deref().map(resolve_frames_dir)
    }

    pub fn frame_numbers(&self) -> &[i32] {
        &self.frame_numbers
    }

    pub fn frame_number(&self, index: usize) -> Option<i32> {
        self.frame_numbers.get(index).copied()
    }

    pub fn hand_count(&self, index: usize) -> usize {
        self.frames.get(index).map_or(0, |f| f.len())
    }

    pub fn load_frame(&self, index: usize) -> Option<&Frame> {
        self.frames.get(index)
    }

    pub fn is_flagged(&self, index: usize, flag_index: usize, per_track_coloring: bool) -> bool {
        if flag_index >= FLAG_LAYER_COUNT {
            return false;
        }
        let list = if per_track_coloring {
            &self.frame_flags_all
        } else {
            &self.frame_flags_infant_only
        };
        list.get(index).is_some_and(|flags| flags[flag_index])
    }

    /// Resolves the filesystem path to the video frame image (`frame_<n>.jpg|jpeg|png`, any zero padding)
    /// for a sequence playback index. The frames directory is indexed once and rescanned only on a miss,
    /// so images that appear later are still found.
    pub fn frame_image_path(&self, index: usize) -> Option<PathBuf> {
        let frame_num = self.frame_number(index)?;
        let frames_path = self.frames_dir()?;

        let mut cache = self.frame_index.lock().ok()?;
        for attempt in 0..2 {
            if attempt == 1 || cache.as_ref().is_none_or(|c| c.dir != frames_path) {
                *cache = Some(FrameIndex::scan(&frames_path));
            }
            if let Some(found) = cache.as_ref().and_then(|c| c.files.get(&frame_num))
                && found.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
            {
                return Some(found.clone());
            }
        }
        None
    }
}

/// Frame image files found in one directory, keyed by frame number.
#[derive(Debug)]
struct FrameIndex {
    dir: PathBuf,
    files: HashMap<i32, PathBuf>,
}

impl FrameIndex {
    fn scan(dir: &Path) -> Self {
        let mut files = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(number) = frame_number_of_image(&path) {
                    files.entry(number).or_insert(path);
                }
            }
        }
        Self {
            dir: dir.to_path_buf(),
            files,
        }
    }
}

/// The frame number of a `frame_<digits>.<jpg|jpeg|png>` file name, ignoring zero padding.
fn frame_number_of_image(path: &Path) -> Option<i32> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
        return None;
    }
    let digits = path.file_stem()?.to_str()?.strip_prefix("frame_")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Computes the mean camera-space depth (Z) across all hands in a frame.
pub fn reference_depth(hands: &[HandData]) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for hand in hands {
        for pos in &hand.verts {
            sum += pos.z as f64;
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { (sum / count as f64) as f32 }
}
