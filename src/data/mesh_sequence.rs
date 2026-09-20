use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::data::geometry::HandData;
use crate::data::hand_export::{HandExportError, load_hand_export};
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
    frames_dir: PathBuf,
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
        let frames_dir = resolve_frames_dir(&path_buf);

        let rows = load_hand_export(&path_buf)?;

        let mut by_frame: BTreeMap<i32, Frame> = BTreeMap::new();
        let mut flags_all_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();
        let mut flags_infant_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();

        for row in rows {
            let hand = mano_forward(&row.params);
            let data = HandData {
                verts: hand.verts,
                joints: hand.joints,
                is_right: row.params.is_right,
                hand_track_id: row.hand_track_id,
                label: row.label.clone(),
            };

            let frame_num = row.frame;
            by_frame.entry(frame_num).or_default().push(data);

            let flags_all = flags_all_by_frame.entry(frame_num).or_insert([false; FLAG_LAYER_COUNT]);
            let flags_infant = flags_infant_by_frame
                .entry(frame_num)
                .or_insert([false; FLAG_LAYER_COUNT]);
            let is_infant = row.label == "infant";

            let row_flags = [
                row.flag_same_side_infant_conflict != 0,
                row.flag_same_side_infant_unknown_conflict != 0,
                row.flag_translation_jump != 0,
                row.flag_pose_rotation_jump != 0,
                row.flag_scale_jump != 0,
                row.flag_track_contaminated != 0,
                row.flag_track_fragmented != 0,
            ];

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

        Ok(Self {
            path: path_str,
            frames_dir,
            frames,
            frame_numbers,
            frame_flags_all,
            frame_flags_infant_only,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn frames_dir(&self) -> &Path {
        &self.frames_dir
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

    /// Resolves the filesystem path to the plain video frame image for a sequence playback index.
    pub fn frame_image_path(&self, index: usize) -> Option<PathBuf> {
        let frame_num = self.frame_number(index)?;
        let frames_path = &self.frames_dir;

        let formats = [
            format!("frame_{frame_num:05}.jpg"),
            format!("frame_{frame_num:05}.jpeg"),
            format!("frame_{frame_num:05}.png"),
            format!("frame_{frame_num:04}.jpg"),
            format!("frame_{frame_num:04}.jpeg"),
            format!("frame_{frame_num:04}.png"),
            format!("frame_{frame_num}.jpg"),
            format!("frame_{frame_num}.jpeg"),
            format!("frame_{frame_num}.png"),
            format!("{frame_num:05}.jpg"),
            format!("{frame_num:05}.jpeg"),
            format!("{frame_num:05}.png"),
            format!("{frame_num:04}.jpg"),
            format!("{frame_num:04}.jpeg"),
            format!("{frame_num:04}.png"),
        ];

        for name in &formats {
            let candidate = frames_path.join(name);
            if candidate.is_file() && candidate.metadata().is_ok_and(|m| m.len() > 0) {
                return Some(candidate);
            }
        }
        None
    }
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
