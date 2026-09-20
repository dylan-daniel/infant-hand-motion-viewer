//! Shared test fixtures and synthetic data generators.
#![allow(dead_code)]

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Generates valid, self-contained synthetic `.hexport` file bytes in memory for testing.
pub fn generate_synthetic_hexport(subject: &str, trial: &str) -> Vec<u8> {
    let mut payload = Vec::new();

    let row_count = 1u32;
    let mut columns: Vec<(&str, u8)> = vec![
        ("subject", 3), // String
        ("trial", 3),
        ("frame", 1),    // Int32
        ("is_right", 2), // Int8
        ("hand_track_id", 1),
        ("label", 3),
        ("scaled_focal_length", 0), // Float32
        ("img_w", 1),
        ("img_h", 1),
        ("cam_t_x", 0),
        ("cam_t_y", 0),
        ("cam_t_z", 0),
    ];

    for i in 0..10 {
        columns.push((Box::leak(format!("beta_{i}").into_boxed_str()), 0));
    }
    for i in 0..3 {
        columns.push((Box::leak(format!("gorient_rotvec_{i}").into_boxed_str()), 0));
    }
    for i in 0..45 {
        columns.push((Box::leak(format!("pose_rotvec_{i}").into_boxed_str()), 0));
    }

    // Optional flags
    columns.push(("flag_same_side_infant_conflict", 2));
    columns.push(("flag_same_side_infant_unknown_conflict", 2));
    columns.push(("flag_translation_jump", 2));
    columns.push(("flag_pose_rotation_jump", 2));
    columns.push(("flag_scale_jump", 2));
    columns.push(("flag_track_contaminated", 2));
    columns.push(("flag_track_fragmented", 2));

    let col_count = columns.len() as u32;

    // Header of payload: row_count, column_count
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&col_count.to_le_bytes());

    // Schema definition
    for (name, dtype) in &columns {
        payload.push(name.len() as u8);
        payload.push(*dtype);
        payload.extend_from_slice(name.as_bytes());
    }

    // Column data: 1 row for each column
    for (name, dtype) in &columns {
        match *dtype {
            0 => {
                // Float32
                payload.extend_from_slice(&0.0f32.to_le_bytes());
            }
            1 => {
                // Int32
                let val = match *name {
                    "frame" => 1i32,
                    "hand_track_id" => 0i32,
                    "img_w" => 640i32,
                    "img_h" => 480i32,
                    _ => 0i32,
                };
                payload.extend_from_slice(&val.to_le_bytes());
            }
            2 => {
                // Int8
                let val: i8 = if *name == "is_right" { 1 } else { 0 };
                payload.push(val as u8);
            }
            3 => {
                // String (32 bytes)
                let text = match *name {
                    "subject" => subject,
                    "trial" => trial,
                    "label" => "infant",
                    _ => "",
                };
                let mut buf = [0u8; 32];
                let bytes = text.as_bytes();
                let len = bytes.len().min(31);
                buf[..len].copy_from_slice(&bytes[..len]);
                payload.extend_from_slice(&buf);
            }
            _ => unreachable!(),
        }
    }

    let payload_size = payload.len() as u32;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();
    let compressed_size = compressed.len() as u32;

    let mut file_bytes = Vec::new();
    file_bytes.extend_from_slice(b"HEXP");
    file_bytes.extend_from_slice(&1u32.to_le_bytes()); // Version 1
    file_bytes.extend_from_slice(&payload_size.to_le_bytes());
    file_bytes.extend_from_slice(&compressed_size.to_le_bytes());
    file_bytes.extend_from_slice(&compressed);

    file_bytes
}

/// Helper that creates a temporary synthetic `.hexport` file on disk for integration testing.
pub struct TempSyntheticExport {
    pub path: PathBuf,
}

impl TempSyntheticExport {
    pub fn new(subject: &str, trial: &str) -> Self {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("synthetic_test_{unique_id}.hexport"));
        let bytes = generate_synthetic_hexport(subject, trial);
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        Self { path }
    }
}

impl Drop for TempSyntheticExport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
