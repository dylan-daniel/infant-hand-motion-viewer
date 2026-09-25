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
    columns.push(("flag_competing_sam3_tracks", 2));
    columns.push(("flag_low_sam3_wilor_coverage", 2));
    columns.push(("flag_chirality_mismatch", 2));
    columns.push(("flag_persistent_adult_interference", 2));

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
                let val: i8 = match *name {
                    "is_right" | "flag_chirality_mismatch" | "flag_persistent_adult_interference" => 1,
                    _ => 0,
                };
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

/// A GPU device on whatever adapter the machine has (a software one is fine), or `None` when there is none.
pub fn headless_gpu() -> Option<infant_hand_motion_viewer::graphics::Gpu> {
    use dear_imgui_wgpu::wgpu;
    use infant_hand_motion_viewer::graphics::{Gpu, required_device_features, supported_sample_counts};

    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(&adapter),
        ..Default::default()
    }))
    .ok()?;
    Some(Gpu {
        device,
        queue,
        msaa_counts: supported_sample_counts(&adapter),
    })
}

/// Copies an RGBA8 texture (whose width times four is a multiple of 256 bytes) back to the CPU.
pub fn read_rgba(
    gpu: &infant_hand_motion_viewer::graphics::Gpu,
    texture: &dear_imgui_wgpu::wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    use dear_imgui_wgpu::wgpu;

    assert_eq!((width * 4) % 256, 0, "row size must be a multiple of 256 bytes");
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width * height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = readback.slice(..).get_mapped_range().expect("mapped range").to_vec();
    readback.unmap();
    pixels
}
