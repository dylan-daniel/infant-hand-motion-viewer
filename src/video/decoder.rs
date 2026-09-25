use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use serde::Deserialize;

use super::frame_store::{FrameStore, plan_size};

/// Directories searched after `PATH`, because apps launched from a desktop shell often lack the user's shell `PATH`.
const EXTRA_TOOL_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

/// What the user should do when `ffmpeg` cannot be found.
pub fn missing_ffmpeg_message() -> String {
    let install = if cfg!(windows) {
        "winget install Gyan.FFmpeg"
    } else if cfg!(target_os = "macos") {
        "brew install ffmpeg"
    } else {
        "sudo apt install ffmpeg (or your distribution's ffmpeg package)"
    };
    format!(
        "FFmpeg was not found. It is needed to show video frames from remote trials.\n\n\
         Install it, make sure both ffmpeg and ffprobe are on your PATH, then restart the app:\n\n    {install}"
    )
}

/// The first `name` executable in `dirs`.
pub fn find_tool_in(dirs: impl IntoIterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.into_iter()
        .map(|dir| dir.join(&file_name))
        .find(|path| path.is_file())
}

/// Locate a tool such as `ffmpeg` on `PATH`, falling back to the usual install directories.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let path_dirs = env::var_os("PATH").map(|p| env::split_paths(&p).collect::<Vec<_>>());
    let extra = EXTRA_TOOL_DIRS.iter().map(PathBuf::from);
    find_tool_in(path_dirs.into_iter().flatten().chain(extra), name)
}

/// Whether both `ffmpeg` and `ffprobe` can be found.
pub fn ffmpeg_available() -> bool {
    find_tool("ffmpeg").is_some() && find_tool("ffprobe").is_some()
}

fn command(tool: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(tool);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

/// What `ffprobe` reports about a video stream.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    /// Best guess of the frame count; zero when the file does not say.
    pub frames: usize,
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeStream {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    nb_frames: Option<String>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (num, den) = rate.split_once('/')?;
    let (num, den) = (num.parse::<f64>().ok()?, den.parse::<f64>().ok()?);
    (den > 0.0).then_some(num / den)
}

/// Read the first video stream's size and frame count from `ffprobe -of json` output.
pub fn parse_probe(json: &str) -> Result<VideoInfo, String> {
    let output: ProbeOutput = serde_json::from_str(json).map_err(|e| format!("Unreadable ffprobe output: {e}"))?;
    let stream = output
        .streams
        .first()
        .filter(|s| s.width > 0 && s.height > 0)
        .ok_or_else(|| "Video has no picture".to_string())?;
    let counted = stream.nb_frames.as_deref().and_then(|n| n.parse::<usize>().ok());
    let estimated = || {
        let duration = stream.duration.as_deref()?.parse::<f64>().ok()?;
        let rate = parse_rate(stream.avg_frame_rate.as_deref()?)?;
        Some((duration * rate).round() as usize)
    };
    Ok(VideoInfo {
        width: stream.width,
        height: stream.height,
        frames: counted.or_else(estimated).unwrap_or(0),
    })
}

fn probe(ffprobe: &Path, video: &Path) -> Result<VideoInfo, String> {
    let output = command(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,nb_frames,avg_frame_rate,duration")
        .args(["-of", "json"])
        .arg(video)
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_probe(&String::from_utf8_lossy(&output.stdout))
}

fn spawn_decoder(ffmpeg: &Path, video: &Path, width: u32, height: u32, scale: bool) -> Result<Child, String> {
    let mut command = command(ffmpeg);
    command
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(video)
        .args(["-map", "0:v:0"]);
    if scale {
        command.arg("-vf").arg(format!("scale={width}:{height}:flags=area"));
    }
    command
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))
}

/// Decode every frame of the video at `path` into `store` as downscaled RGB, in presentation order, by piping raw
/// frames out of `ffmpeg`. Frames become readable as they are decoded; decoding stops early if the store is cancelled.
pub fn decode_into(path: &Path, store: &FrameStore) -> Result<(), String> {
    let (Some(ffmpeg), Some(ffprobe)) = (find_tool("ffmpeg"), find_tool("ffprobe")) else {
        return Err(missing_ffmpeg_message());
    };
    let info = probe(&ffprobe, path)?;
    let (width, height) = plan_size(info.width, info.height, info.frames);
    let scale = (width, height) != (info.width, info.height);

    let mut child = spawn_decoder(&ffmpeg, path, width, height, scale)?;
    let mut stdout = child.stdout.take().ok_or_else(|| "ffmpeg has no output".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ffmpeg has no error output".to_string())?;
    let errors = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    store.begin(width, height, info.frames);
    let frame_len = width as usize * height as usize * 3;
    loop {
        if store.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        let mut frame = vec![0u8; frame_len];
        if stdout.read_exact(&mut frame).is_err() {
            break;
        }
        store.push(frame);
    }
    let status = child.wait().map_err(|e| format!("ffmpeg failed: {e}"))?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() || store.is_empty() {
        let detail = errors.trim();
        return Err(if detail.is_empty() {
            "ffmpeg produced no frames".to_string()
        } else {
            format!("ffmpeg failed: {detail}")
        });
    }
    store.finish();
    Ok(())
}
