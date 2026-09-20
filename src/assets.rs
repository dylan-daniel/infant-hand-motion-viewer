//! Embedded application assets compiled directly into the binary.

/// Binary MANO face topology (uint16 triangle vertex indices, right-hand winding).
pub const MANO_FACES: &[u8] = include_bytes!("../assets/mano/mano_faces.bin");

/// Binary MANO model weights (template vertices, blendshapes, kinematics, joint regressors).
pub const MANO_MODEL: &[u8] = include_bytes!("../assets/mano/mano_model.bin");

/// Bundled UI and monospace font.
pub const UI_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMonoNL-Regular.ttf");

// ── Explorer icons ──────────────────────────────
pub const ICON_FOLDER_CLOSED: &[u8] = include_bytes!("../assets/icons/folder-blue.png");
pub const ICON_FOLDER_OPEN: &[u8] = include_bytes!("../assets/icons/folder-blue-open.png");
pub const ICON_FILE_HEXPORT: &[u8] = include_bytes!("../assets/icons/file-hexport.png");
pub const ICON_CHANGE_ROOT: &[u8] = include_bytes!("../assets/icons/folder-lucide.png");
pub const ICON_REFRESH: &[u8] = include_bytes!("../assets/icons/folder-sync.png");

// ── Transport bar icons ─────────────────────────
pub const ICON_PLAY: &[u8] = include_bytes!("../assets/icons/play.png");
pub const ICON_PAUSE: &[u8] = include_bytes!("../assets/icons/pause.png");
pub const ICON_SPEED: &[u8] = include_bytes!("../assets/icons/gauge.png");

/// Remote daemon script, installed to `~/.infant-hand-motion-viewer/` at connect time.
pub const VIEWER_DAEMON: &str = include_str!("../scripts/viewer_daemon.py");
