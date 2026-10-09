pub mod explorer;
pub mod flags;
pub mod icons;
pub mod image_view;
pub mod measures;
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
pub use measures::{MEASURE_TITLES, MeasuresView, draw_measures_window};
pub use menu_bar::{MenuResult, MenuState, draw_menu_bar};
pub use modals::draw_remote_modal;
pub use transport::{
    FLAG_COLORS, FLAG_LAYERS, FLAG_NAMES, FlagLayer, PLAYBACK_SPEEDS, SliderGeometry, Transport, TransportState,
    draw_transport_bar, next_playback_speed,
};
pub use viewport::{ViewportResult, draw_viewport_window};

pub(crate) fn rgba(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}

/// ImGui's own default, used until the app records its theme's padding.
const DEFAULT_WINDOW_PADDING: [f32; 2] = [8.0, 8.0];
static THEME_WINDOW_PADDING: std::sync::OnceLock<[f32; 2]> = std::sync::OnceLock::new();

/// Record the theme's window padding. Call once after the style is set up and before any window pushes its own
/// padding: ImGui keeps pushed style values in the live style, so reading it later would return the pushed value.
pub fn remember_theme_window_padding(padding: [f32; 2]) {
    let _ = THEME_WINDOW_PADDING.set(padding);
}

/// The theme's window padding recorded by [`remember_theme_window_padding`].
pub fn theme_window_padding() -> [f32; 2] {
    THEME_WINDOW_PADDING.get().copied().unwrap_or(DEFAULT_WINDOW_PADDING)
}

/// Show a tooltip with the theme's normal window padding. The Scene and Frame View windows set their own padding
/// to zero, and a tooltip opened inside them would otherwise inherit that and hug its text.
pub fn padded_tooltip(ui: &dear_imgui_rs::Ui, contents: impl FnOnce()) {
    let _padding = ui.push_style_var(dear_imgui_rs::StyleVar::WindowPadding(theme_window_padding()));
    ui.tooltip(contents);
}
