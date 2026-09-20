pub mod explorer;
pub mod flags;
pub mod icons;
pub mod image_view;
pub mod menu_bar;
pub mod modals;
pub mod transport;
pub mod viewport;

// Pixel sizes the bundled font is baked at (baking each size keeps text crisp).
pub const UI_FONT_SIZE: f32 = 22.0;
pub const FPS_FONT_SIZE: f32 = 22.0;

// Height of the sequence playback bar pinned to the window bottom.
pub const PLAYBACK_BAR_HEIGHT: f32 = 56.0;

pub use explorer::{ExplorerResult, FileExplorer, SourceMode, natural_cmp};
pub use flags::draw_flags_window;
pub use icons::UiIcons;
pub use image_view::{ImageViewResult, draw_image_window};
pub use menu_bar::{MenuResult, MenuState, draw_menu_bar};
pub use modals::{draw_remote_modal, draw_storage_modal};
pub use transport::{FLAG_COLORS, FLAG_LAYERS, FLAG_NAMES, FlagLayer, Transport, TransportState, draw_transport_bar};
pub use viewport::{ViewportResult, draw_viewport_window};

pub(crate) fn rgba(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}
