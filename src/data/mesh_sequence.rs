use std::collections::BTreeMap;
use std::path::Path;

use crate::data::geometry::{HandCamera, HandData};
use crate::data::hand_export::{
    HandExportError, HandExportRow, HexportMetadata, MEASURE_COUNT, load_hand_export_with_meta,
    parse_hand_export_with_meta,
};
use crate::data::mano_model::mano_forward;

pub const FLAG_LAYER_COUNT: usize = 11;

pub type Frame = Vec<HandData>;

/// One hand's hand-shape measures in one frame, in [`crate::data::MEASURE_COLUMNS`] order.
#[derive(Debug, Clone, PartialEq)]
pub struct HandMeasures {
    pub is_right: bool,
    pub is_infant: bool,
    pub hand_track_id: i32,
    pub values: [f32; MEASURE_COUNT],
}

impl HandMeasures {
    fn label_kind(&self) -> &'static str {
        if self.is_infant { "infant" } else { "other" }
    }
}

/// One line of a measure plot: a value per sequence frame, NaN where the hand is absent.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureSeries {
    pub label: String,
    pub is_right: bool,
    /// The tracking ID when the series is a single track; `None` for the per-side infant series.
    pub hand_track_id: Option<i32>,
    pub values: Vec<f32>,
}

/// MeshSequence holds all per-frame hand meshes and flags loaded from a `.hexport` binary file.
#[derive(Debug, Clone)]
pub struct MeshSequence {
    path: String,
    meta: HexportMetadata,
    /// Index 0 is raw, 1 is smoothed; `smoothed` picks which one the accessors serve.
    frames: [Vec<Frame>; 2],
    frame_numbers: Vec<i32>,
    frame_measures: [Vec<Vec<HandMeasures>>; 2],
    smoothed: bool,
    frame_flags_all: Vec<[bool; FLAG_LAYER_COUNT]>,
    frame_flags_infant_only: Vec<[bool; FLAG_LAYER_COUNT]>,
}

impl MeshSequence {
    /// Loads a `.hexport` file, runs the MANO forward pass for all hands, and groups by frame.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, HandExportError> {
        let path_buf = path.as_ref().to_path_buf();
        let path_str = path_buf.to_string_lossy().to_string();

        let (meta, rows) = load_hand_export_with_meta(&path_buf)?;
        Ok(Self::from_rows(path_str, meta, rows))
    }

    /// Builds a sequence from the bytes of a `.hexport` file held in memory. `label` identifies the source
    /// (for example the remote path).
    pub fn from_bytes(label: &str, raw: &[u8]) -> Result<Self, HandExportError> {
        let (meta, rows) = parse_hand_export_with_meta(raw, label)?;
        Ok(Self::from_rows(label.to_string(), meta, rows))
    }

    fn from_rows(path_str: String, meta: HexportMetadata, rows: Vec<HandExportRow>) -> Self {
        let mut by_frame: BTreeMap<i32, [Frame; 2]> = BTreeMap::new();
        let mut flags_all_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();
        let mut flags_infant_by_frame: BTreeMap<i32, [bool; FLAG_LAYER_COUNT]> = BTreeMap::new();
        let mut measures_by_frame: BTreeMap<i32, [Vec<HandMeasures>; 2]> = BTreeMap::new();

        for row in rows {
            let row_flags = [
                row.flag_same_side_infant_conflict != 0,
                row.flag_same_side_infant_unknown_conflict != 0,
                row.flag_translation_jump != 0,
                row.flag_pose_rotation_jump != 0,
                row.flag_scale_jump != 0,
                row.flag_track_contaminated != 0,
                row.flag_track_fragmented != 0,
                row.flag_competing_sam3_tracks != 0,
                row.flag_low_sam3_wilor_coverage != 0,
                row.flag_chirality_mismatch != 0,
                row.flag_persistent_adult_interference != 0,
            ];

            let datas = [&row.raw, &row.smooth].map(|variant| {
                let hand = mano_forward(&variant.params);
                HandData {
                    verts: hand.verts,
                    joints: hand.joints,
                    is_right: variant.params.is_right,
                    hand_track_id: row.hand_track_id,
                    label: row.label.clone(),
                    camera: Some(HandCamera {
                        cam_t: variant.params.cam_t,
                        focal_length: row.scaled_focal_length,
                        img_w: row.img_w.max(0) as u32,
                        img_h: row.img_h.max(0) as u32,
                    }),
                    flags: row_flags,
                }
            });
            let [raw_data, smooth_data] = datas;

            let frame_num = row.frame;
            let measures = measures_by_frame.entry(frame_num).or_default();
            for (slot, variant) in [&row.raw, &row.smooth].into_iter().enumerate() {
                measures[slot].push(HandMeasures {
                    is_right: variant.params.is_right,
                    is_infant: row.label == "infant",
                    hand_track_id: row.hand_track_id,
                    values: variant.measures,
                });
            }
            let hands = by_frame.entry(frame_num).or_default();
            hands[0].push(raw_data);
            hands[1].push(smooth_data);

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

        let mut frames: [Vec<Frame>; 2] = Default::default();
        let mut frame_numbers = Vec::with_capacity(by_frame.len());
        let mut frame_measures: [Vec<Vec<HandMeasures>>; 2] = Default::default();
        let mut frame_flags_all = Vec::with_capacity(by_frame.len());
        let mut frame_flags_infant_only = Vec::with_capacity(by_frame.len());

        for (frame_num, hands) in by_frame {
            frame_numbers.push(frame_num);
            let [raw_m, smooth_m] = measures_by_frame.remove(&frame_num).unwrap();
            frame_measures[0].push(raw_m);
            frame_measures[1].push(smooth_m);
            frame_flags_all.push(flags_all_by_frame.remove(&frame_num).unwrap());
            frame_flags_infant_only.push(flags_infant_by_frame.remove(&frame_num).unwrap());
            let [raw_h, smooth_h] = hands;
            frames[0].push(raw_h);
            frames[1].push(smooth_h);
        }

        Self {
            path: path_str,
            meta,
            frames,
            frame_numbers,
            frame_measures,
            smoothed: true,
            frame_flags_all,
            frame_flags_infant_only,
        }
    }

    pub fn frame_count(&self) -> usize {
        self.frame_numbers.len()
    }

    pub fn is_smoothed(&self) -> bool {
        self.smoothed
    }

    /// Chooses whether hands and measures come from the smoothed fit or WiLoR's raw one.
    pub fn set_smoothed(&mut self, smoothed: bool) {
        self.smoothed = smoothed;
    }

    fn active_frames(&self) -> &[Frame] {
        &self.frames[usize::from(self.smoothed)]
    }

    fn active_measures(&self) -> &[Vec<HandMeasures>] {
        &self.frame_measures[usize::from(self.smoothed)]
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The export's JSON header, naming the source video by hash.
    pub fn meta(&self) -> &HexportMetadata {
        &self.meta
    }

    pub fn frame_numbers(&self) -> &[i32] {
        &self.frame_numbers
    }

    pub fn frame_number(&self, index: usize) -> Option<i32> {
        self.frame_numbers.get(index).copied()
    }

    pub fn hand_count(&self, index: usize) -> usize {
        self.active_frames().get(index).map_or(0, |f| f.len())
    }

    pub fn load_frame(&self, index: usize) -> Option<&Frame> {
        self.active_frames().get(index)
    }

    /// Whether any hand in the export carries a hand-shape measure value.
    pub fn has_measure_data(&self) -> bool {
        self.active_measures()
            .iter()
            .flatten()
            .any(|hand| hand.values.iter().any(|v| v.is_finite()))
    }

    /// The lines to plot for one measure: the infant's left and right hand, or every tracked hand when
    /// `per_track` is set. Each line has one value per sequence frame, NaN where that hand is absent.
    pub fn measure_series(&self, measure: usize, per_track: bool) -> Vec<MeasureSeries> {
        if measure >= MEASURE_COUNT {
            return Vec::new();
        }
        let mut series: Vec<MeasureSeries> = Vec::new();
        for (index, hands) in self.active_measures().iter().enumerate() {
            for hand in hands.iter().filter(|h| per_track || h.is_infant) {
                let track = per_track.then_some(hand.hand_track_id);
                let slot = match series
                    .iter()
                    .position(|s| s.hand_track_id == track && (per_track || s.is_right == hand.is_right))
                {
                    Some(slot) => slot,
                    None => {
                        let side = if hand.is_right { "Right" } else { "Left" };
                        series.push(MeasureSeries {
                            label: match track {
                                Some(id) => format!("Track {id} ({}, {side})", hand.label_kind()),
                                None => format!("Infant {side}"),
                            },
                            is_right: hand.is_right,
                            hand_track_id: track,
                            values: vec![f32::NAN; self.frame_numbers.len()],
                        });
                        series.len() - 1
                    }
                };
                if series[slot].values[index].is_nan() {
                    series[slot].values[index] = hand.values[measure];
                }
            }
        }
        series.sort_by_key(|s| (s.hand_track_id, s.is_right));
        series
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
