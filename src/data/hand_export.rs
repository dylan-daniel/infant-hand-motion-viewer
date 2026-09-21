use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;
use glam::Vec3;
use thiserror::Error;

const MAGIC: [u8; 4] = *b"HEXP";
const VERSION: u32 = 1;
const HEADER_SIZE: usize = 16;
const STRING_FIELD_WIDTH: usize = 32;

#[derive(Debug, Error)]
pub enum HandExportError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Not a hand export binary file (bad magic): {0}")]
    InvalidMagic(String),
    #[error("Unsupported hand export binary version {version} (expected {expected}): {path}")]
    UnsupportedVersion { version: u32, expected: u32, path: String },
    #[error("Truncated hand export file: {0}")]
    Truncated(String),
    #[error("Failed to decompress hand export payload: {path} ({reason})")]
    DecompressionFailed { path: String, reason: String },
    #[error("Hand export file is missing expected column '{column}': {path}")]
    MissingColumn { column: String, path: String },
    #[error("Malformed schema or payload: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DType {
    Float32 = 0,
    Int32 = 1,
    Int8 = 2,
    String = 3,
}

impl DType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Float32),
            1 => Some(Self::Int32),
            2 => Some(Self::Int8),
            3 => Some(Self::String),
            _ => None,
        }
    }

    fn item_size(self) -> usize {
        match self {
            Self::Float32 | Self::Int32 => 4,
            Self::Int8 => 1,
            Self::String => STRING_FIELD_WIDTH,
        }
    }
}

#[derive(Debug, Clone)]
struct ColumnInfo {
    #[allow(dead_code)]
    dtype: DType,
    offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManoParams {
    pub betas: [f32; 10],
    pub global_orient: [f32; 9],
    pub hand_pose: [f32; 135],
    pub is_right: bool,
    pub cam_t: Vec3,
}

impl Default for ManoParams {
    fn default() -> Self {
        let mut hand_pose = [0.0f32; 135];
        for j in 0..15 {
            hand_pose[j * 9] = 1.0;
            hand_pose[j * 9 + 4] = 1.0;
            hand_pose[j * 9 + 8] = 1.0;
        }
        Self {
            betas: [0.0; 10],
            global_orient: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            hand_pose,
            is_right: true,
            cam_t: Vec3::ZERO,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandExportRow {
    pub subject: String,
    pub trial: String,
    pub frame: i32,
    pub is_right: i8,
    pub hand_track_id: i32,
    pub label: String,
    pub params: ManoParams,
    pub scaled_focal_length: f32,
    pub img_w: i32,
    pub img_h: i32,
    pub flag_same_side_infant_conflict: i8,
    pub flag_same_side_infant_unknown_conflict: i8,
    pub flag_translation_jump: i8,
    pub flag_pose_rotation_jump: i8,
    pub flag_scale_jump: i8,
    pub flag_track_contaminated: i8,
    pub flag_track_fragmented: i8,
}

/// Converts an axis-angle rotation vector (x, y, z) into a row-major 3x3 rotation matrix using Rodrigues' formula.
pub fn axis_angle_to_matrix(x: f32, y: f32, z: f32) -> [f32; 9] {
    let theta = (x * x + y * y + z * z).sqrt();
    if theta < 1e-12 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let kx = x / theta;
    let ky = y / theta;
    let kz = z / theta;
    let s = theta.sin();
    let c = theta.cos();
    let one_c = 1.0 - c;

    [
        c + kx * kx * one_c,
        kx * ky * one_c - kz * s,
        kx * kz * one_c + ky * s,
        ky * kx * one_c + kz * s,
        c + ky * ky * one_c,
        ky * kz * one_c - kx * s,
        kz * kx * one_c - ky * s,
        kz * ky * one_c + kx * s,
        c + kz * kz * one_c,
    ]
}

/// Reads all rows from a `.hexport` hand motion export file.
pub fn load_hand_export<P: AsRef<Path>>(export_path: P) -> Result<Vec<HandExportRow>, HandExportError> {
    let path_str = export_path.as_ref().to_string_lossy().to_string();
    let mut file = File::open(&export_path).map_err(|e| HandExportError::Io {
        path: path_str.clone(),
        source: e,
    })?;

    let mut raw = Vec::new();
    file.read_to_end(&mut raw).map_err(|e| HandExportError::Io {
        path: path_str.clone(),
        source: e,
    })?;

    parse_hand_export(&raw, &path_str)
}

/// Parses the bytes of a `.hexport` file; `source` names where they came from, for error messages.
pub fn parse_hand_export(raw: &[u8], source: &str) -> Result<Vec<HandExportRow>, HandExportError> {
    let path_str = source.to_string();
    if raw.len() < HEADER_SIZE || raw[0..4] != MAGIC {
        return Err(HandExportError::InvalidMagic(path_str));
    }

    let version = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    let payload_size = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
    let compressed_size = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;

    if version != VERSION {
        return Err(HandExportError::UnsupportedVersion {
            version,
            expected: VERSION,
            path: path_str,
        });
    }

    if raw.len() < HEADER_SIZE + compressed_size {
        return Err(HandExportError::Truncated(path_str));
    }

    let compressed_bytes = &raw[HEADER_SIZE..HEADER_SIZE + compressed_size];
    let mut decoder = ZlibDecoder::new(compressed_bytes);
    let mut payload = Vec::with_capacity(payload_size);
    decoder
        .read_to_end(&mut payload)
        .map_err(|e| HandExportError::DecompressionFailed {
            path: path_str.clone(),
            reason: e.to_string(),
        })?;

    if payload.len() != payload_size {
        return Err(HandExportError::DecompressionFailed {
            path: path_str,
            reason: format!("Decompressed size {} != expected {}", payload.len(), payload_size),
        });
    }

    if payload.len() < 8 {
        return Err(HandExportError::Malformed("Payload too short for header".into()));
    }

    let mut offset = 0;
    let row_count = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    let column_count = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;

    struct PendingColumn {
        name: String,
        dtype: DType,
    }

    let mut ordered = Vec::with_capacity(column_count);
    for _ in 0..column_count {
        if offset + 2 > payload.len() {
            return Err(HandExportError::Malformed("Truncated column descriptor".into()));
        }
        let name_len = payload[offset] as usize;
        offset += 1;
        let dtype_byte = payload[offset];
        offset += 1;
        let dtype = DType::from_u8(dtype_byte)
            .ok_or_else(|| HandExportError::Malformed(format!("Unknown dtype tag {dtype_byte}")))?;

        if offset + name_len > payload.len() {
            return Err(HandExportError::Malformed("Truncated column name".into()));
        }
        let name = String::from_utf8_lossy(&payload[offset..offset + name_len]).to_string();
        offset += name_len;
        ordered.push(PendingColumn { name, dtype });
    }

    let mut columns = HashMap::with_capacity(ordered.len());
    for col in ordered {
        columns.insert(
            col.name,
            ColumnInfo {
                dtype: col.dtype,
                offset,
            },
        );
        offset += col.dtype.item_size() * row_count;
    }

    let require = |name: &str| -> Result<&ColumnInfo, HandExportError> {
        columns.get(name).ok_or_else(|| HandExportError::MissingColumn {
            column: name.to_string(),
            path: path_str.clone(),
        })
    };

    let get_float = |col: &ColumnInfo, r: usize| -> f32 {
        let start = col.offset + r * 4;
        f32::from_le_bytes(payload[start..start + 4].try_into().unwrap())
    };

    let get_int32 = |col: &ColumnInfo, r: usize| -> i32 {
        let start = col.offset + r * 4;
        i32::from_le_bytes(payload[start..start + 4].try_into().unwrap())
    };

    let get_int8 = |col: &ColumnInfo, r: usize| -> i8 { payload[col.offset + r] as i8 };

    let get_string = |col: &ColumnInfo, r: usize| -> String {
        let start = col.offset + r * STRING_FIELD_WIDTH;
        let slice = &payload[start..start + STRING_FIELD_WIDTH];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(STRING_FIELD_WIDTH);
        String::from_utf8_lossy(&slice[..end]).to_string()
    };

    let find_col = |name: &str| -> Option<&ColumnInfo> { columns.get(name) };

    let get_int8_opt = |col: Option<&ColumnInfo>, r: usize| -> i8 {
        match col {
            Some(c) => payload[c.offset + r] as i8,
            None => 0,
        }
    };

    let subject_col = require("subject")?;
    let trial_col = require("trial")?;
    let frame_col = require("frame")?;
    let is_right_col = require("is_right")?;
    let hand_track_id_col = require("hand_track_id")?;
    let label_col = require("label")?;
    let scaled_focal_length_col = require("scaled_focal_length")?;
    let img_w_col = require("img_w")?;
    let img_h_col = require("img_h")?;

    let flag_same_side_infant_conflict_col = find_col("flag_same_side_infant_conflict");
    let flag_same_side_infant_unknown_conflict_col = find_col("flag_same_side_infant_unknown_conflict");
    let flag_translation_jump_col = find_col("flag_translation_jump");
    let flag_pose_rotation_jump_col = find_col("flag_pose_rotation_jump");
    let flag_scale_jump_col = find_col("flag_scale_jump");
    let flag_track_contaminated_col = find_col("flag_track_contaminated");
    let flag_track_fragmented_col = find_col("flag_track_fragmented");

    let cam_t_x_col = require("cam_t_x")?;
    let cam_t_y_col = require("cam_t_y")?;
    let cam_t_z_col = require("cam_t_z")?;

    let mut beta_cols = Vec::with_capacity(10);
    for i in 0..10 {
        beta_cols.push(require(&format!("beta_{i}"))?);
    }

    let mut gorient_rotvec_cols = Vec::with_capacity(3);
    for i in 0..3 {
        gorient_rotvec_cols.push(require(&format!("gorient_rotvec_{i}"))?);
    }

    let mut pose_rotvec_cols = Vec::with_capacity(45);
    for i in 0..45 {
        pose_rotvec_cols.push(require(&format!("pose_rotvec_{i}"))?);
    }

    let mut rows = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let is_right_val = get_int8(is_right_col, r);
        let mut betas = [0.0f32; 10];
        for i in 0..10 {
            betas[i] = get_float(beta_cols[i], r);
        }

        let global_orient = axis_angle_to_matrix(
            get_float(gorient_rotvec_cols[0], r),
            get_float(gorient_rotvec_cols[1], r),
            get_float(gorient_rotvec_cols[2], r),
        );

        let mut hand_pose = [0.0f32; 135];
        for j in 0..15 {
            let base = j * 3;
            let mat = axis_angle_to_matrix(
                get_float(pose_rotvec_cols[base], r),
                get_float(pose_rotvec_cols[base + 1], r),
                get_float(pose_rotvec_cols[base + 2], r),
            );
            let matrix_base = j * 9;
            hand_pose[matrix_base..matrix_base + 9].copy_from_slice(&mat);
        }

        let cam_t = Vec3::new(
            get_float(cam_t_x_col, r),
            get_float(cam_t_y_col, r),
            get_float(cam_t_z_col, r),
        );

        let params = ManoParams {
            betas,
            global_orient,
            hand_pose,
            is_right: is_right_val != 0,
            cam_t,
        };

        rows.push(HandExportRow {
            subject: get_string(subject_col, r),
            trial: get_string(trial_col, r),
            frame: get_int32(frame_col, r),
            is_right: is_right_val,
            hand_track_id: get_int32(hand_track_id_col, r),
            label: get_string(label_col, r),
            params,
            scaled_focal_length: get_float(scaled_focal_length_col, r),
            img_w: get_int32(img_w_col, r),
            img_h: get_int32(img_h_col, r),
            flag_same_side_infant_conflict: get_int8_opt(flag_same_side_infant_conflict_col, r),
            flag_same_side_infant_unknown_conflict: get_int8_opt(flag_same_side_infant_unknown_conflict_col, r),
            flag_translation_jump: get_int8_opt(flag_translation_jump_col, r),
            flag_pose_rotation_jump: get_int8_opt(flag_pose_rotation_jump_col, r),
            flag_scale_jump: get_int8_opt(flag_scale_jump_col, r),
            flag_track_contaminated: get_int8_opt(flag_track_contaminated_col, r),
            flag_track_fragmented: get_int8_opt(flag_track_fragmented_col, r),
        });
    }

    Ok(rows)
}

/// Quickly reads only the subject and trial identifiers from a `.hexport` file without full frame decompression.
pub fn read_hexport_metadata<P: AsRef<Path>>(path: P) -> Option<(String, String)> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; HEADER_SIZE];
    file.read_exact(&mut header).ok()?;

    if header[0..4] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if version != VERSION {
        return None;
    }
    let payload_size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let compressed_size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;

    if payload_size == 0 || compressed_size == 0 {
        return None;
    }

    let mut compressed = vec![0u8; compressed_size];
    file.read_exact(&mut compressed).ok()?;

    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut payload = Vec::with_capacity(payload_size);
    decoder.read_to_end(&mut payload).ok()?;

    if payload.len() != payload_size || payload.len() < 8 {
        return None;
    }

    let mut offset = 0;
    let row_count = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;
    let column_count = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4;

    if row_count == 0 {
        return None;
    }

    struct ColDesc {
        name: String,
        dtype: DType,
    }

    let mut ordered = Vec::with_capacity(column_count);
    for _ in 0..column_count {
        if offset + 2 > payload.len() {
            return None;
        }
        let name_len = payload[offset] as usize;
        offset += 1;
        let dtype_byte = payload[offset];
        offset += 1;
        let dtype = DType::from_u8(dtype_byte)?;
        if offset + name_len > payload.len() {
            return None;
        }
        let name = String::from_utf8_lossy(&payload[offset..offset + name_len]).to_string();
        offset += name_len;
        ordered.push(ColDesc { name, dtype });
    }

    let mut columns = HashMap::with_capacity(ordered.len());
    for col in ordered {
        columns.insert(
            col.name,
            ColumnInfo {
                dtype: col.dtype,
                offset,
            },
        );
        offset += col.dtype.item_size() * row_count;
    }

    let subj_col = columns.get("subject")?;
    let trial_col = columns.get("trial")?;

    let get_first_string = |col: &ColumnInfo| -> Option<String> {
        if col.offset + STRING_FIELD_WIDTH > payload.len() {
            return None;
        }
        let slice = &payload[col.offset..col.offset + STRING_FIELD_WIDTH];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(STRING_FIELD_WIDTH);
        let s = String::from_utf8_lossy(&slice[..end]).to_string();
        if s.is_empty() { None } else { Some(s) }
    };

    let subject = get_first_string(subj_col)?;
    let trial = get_first_string(trial_col)?;
    Some((subject, trial))
}
