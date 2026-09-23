use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use dear_imgui_rs::{
    ConfigFlags, Context as ImGuiContext, DockLayout, DockLayoutApply, DockNodeFlags, DockSplit, TextureId, WindowKey,
};
use dear_imgui_wgpu::{ExternalTextureId, FramebufferExtent, GammaMode, WgpuInitInfo, WgpuRenderer, wgpu};
use dear_imgui_winit::{HiDpiMode, WinitPlatform};

use infant_hand_motion_viewer::assets;
use infant_hand_motion_viewer::config::Config;
use infant_hand_motion_viewer::data::{MeshSequence, Transform, compute_transform, reference_depth};
use infant_hand_motion_viewer::graphics::{
    Camera, FrameGpu, Framebuffer, FreeCamera, Gpu, HandOverlay, ImageTexture, OrbitCamera, Renderer, SceneRender,
    overlay_focal_length, prepare_frame, prepare_overlay_hands, required_device_features, supported_sample_counts,
};
use infant_hand_motion_viewer::remote::{ConnectionState, RemoteClient, RemoteConfig};
use infant_hand_motion_viewer::ui::{
    FileExplorer, MenuState, SourceMode, Transport, UiIcons, draw_flags_window, draw_image_window, draw_menu_bar,
    draw_remote_modal, draw_viewport_window,
};
use infant_hand_motion_viewer::util::WorkerQueue;
use infant_hand_motion_viewer::util::window_placement::{MonitorRect, is_position_reachable};
use infant_hand_motion_viewer::video::loader::{MISSING_VIDEO_MESSAGE, load_remote_video};
use infant_hand_motion_viewer::video::{FrameStore, LoadStatus};

/// Longest the window stays hidden waiting for its first presented frame.
const FIRST_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const PLAYBACK_FPS: f64 = 30.0;
const SCRUB_INITIAL_DELAY: f64 = 0.25;
const SCRUB_REPEAT_INTERVAL: f64 = 0.03;

/// Where a sequence to open comes from: a local file, or the bytes of a remote file held in memory.
enum OpenSource {
    Local(String),
    Remote { path: String, bytes: Vec<u8> },
}

struct PendingOpen {
    source: OpenSource,
    start_frame: usize,
}

struct DialogChannels {
    open_file_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    data_folder_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
}

impl DialogChannels {
    fn new() -> Self {
        Self {
            open_file_rx: None,
            data_folder_rx: None,
        }
    }
}

struct AppState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    imgui: ImGuiContext,
    platform: WinitPlatform,
    imgui_renderer: WgpuRenderer,
    scene_texture: ExternalTextureId,
    scene_generation: u64,
    frame_texture: Option<ExternalTextureId>,
    frame_generation: u64,
    hand_overlay: HandOverlay,
    overlay_texture: Option<ExternalTextureId>,
    overlay_generation: u64,
    overlay_key: Option<OverlayKey>,
    fps_font: dear_imgui_rs::FontId,

    renderer: Renderer,
    framebuffer: Framebuffer,
    frame_image: ImageTexture,
    icons: UiIcons,

    // Persistence
    settings: Config,
    config_path: PathBuf,

    // Cameras
    orbit_cam: OrbitCamera,
    free_cam: FreeCamera,
    camera_is_free: bool,

    // Input states
    right_mouse_down: bool,
    cursor_pos: PhysicalPosition<f64>,
    relative_mouse_anchor: Option<PhysicalPosition<f64>>,
    shift_down: bool,
    orbiting: bool,
    panning: bool,
    viewport_hovered: bool,
    keys_down: HashSet<KeyCode>,

    // Sequence & Playback
    sequence: Option<MeshSequence>,
    current_gpu: Option<FrameGpu>,
    loaded_frame: Option<usize>,
    transform: Option<Transform>,
    depth_reference: Option<f32>,
    current_frame: usize,
    playing: bool,
    playback_speed: f32,
    playback_accumulator: f64,
    scrubbing: bool,
    scrub_timer: f64,

    // UI state
    active_pane: i32, // 0 = Scene, 1 = Frame View
    restore_focus_frames: i32,
    explorer: FileExplorer,
    show_remote_modal: bool,
    remote_connect_in_flight: bool,
    remote_config: RemoteConfig,
    remote_client: RemoteClient,
    remote_fetch_worker: Arc<WorkerQueue>,
    remote_open_worker: Arc<WorkerQueue>,
    pending_remote_open: Arc<Mutex<Option<PendingOpen>>>,
    video: Option<Arc<FrameStore>>,
    video_worker: WorkerQueue,
    streamed_frame: Option<u32>,
    current_remote_path: String,
    active_sequence_id: Arc<AtomicU64>,

    // Native file dialogs
    dialogs: DialogChannels,
    pending_open_file: Option<PendingOpen>,
    /// Remote file (host, path, frame) to reopen once the saved connection is back up.
    pending_remote_restore: Option<(String, String, usize)>,

    // Timing
    last_frame_time: Instant,
    created_at: Instant,
    window_shown: bool,
}

impl AppState {
    fn open_sequence(&mut self, source: &OpenSource, start_frame: usize) {
        self.frame_image.clear();
        let (label, opened) = match source {
            OpenSource::Local(path) => (path.as_str(), MeshSequence::open(path)),
            OpenSource::Remote { path, bytes } => (path.as_str(), MeshSequence::from_bytes(path, bytes)),
        };
        self.current_remote_path = match source {
            OpenSource::Local(_) => String::new(),
            OpenSource::Remote { path, .. } => path.clone(),
        };
        match opened {
            Ok(seq) => {
                let frame_count = seq.frame_count();
                if frame_count > 0 {
                    self.current_frame = start_frame.min(frame_count - 1);
                    self.sequence = Some(seq);
                } else {
                    eprintln!("No frames found in {label}");
                    self.sequence = None;
                    self.current_frame = 0;
                }
            }
            Err(err) => {
                eprintln!("Failed to open {label}: {err}");
                self.sequence = None;
                self.current_frame = 0;
            }
        }
        self.explorer.set_selected(label);
        self.current_gpu = None;
        self.loaded_frame = None;
        self.transform = None;
        self.depth_reference = None;
    }

    /// Download a remote `.hexport` into memory on a worker and queue it to open once it arrives.
    fn request_remote_open(&mut self, remote_path: String, start_frame: usize) {
        let client = self.remote_client.clone();
        let seq_generation = self.active_sequence_id.fetch_add(1, Ordering::SeqCst) + 1;
        let active_id = Arc::clone(&self.active_sequence_id);
        let ready = Arc::clone(&self.pending_remote_open);
        self.remote_open_worker
            .submit(move || match client.fetch_file_bytes(&remote_path) {
                Ok(bytes) if active_id.load(Ordering::SeqCst) == seq_generation => {
                    *ready.lock().unwrap() = Some(PendingOpen {
                        source: OpenSource::Remote {
                            path: remote_path,
                            bytes,
                        },
                        start_frame,
                    });
                }
                Ok(_) => {}
                Err(err) => eprintln!("Failed to fetch {remote_path}: {err}"),
            });
    }

    /// Hide and pin the cursor while dragging the camera, restoring it where it was afterwards.
    fn set_relative_mouse(&mut self, enabled: bool) {
        if enabled {
            if self.relative_mouse_anchor.is_some() {
                return;
            }
            self.relative_mouse_anchor = Some(self.cursor_pos);
            self.window.set_cursor_visible(false);
            if self.window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                let _ = self.window.set_cursor_grab(CursorGrabMode::Confined);
            }
        } else if let Some(anchor) = self.relative_mouse_anchor.take() {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            let _ = self.window.set_cursor_position(anchor);
            self.window.set_cursor_visible(true);
        }
    }

    fn active_camera_mut(&mut self) -> &mut dyn Camera {
        if self.camera_is_free {
            &mut self.free_cam
        } else {
            &mut self.orbit_cam
        }
    }

    fn update_camera_mode(&mut self, free: bool) {
        if self.camera_is_free != free {
            if free {
                self.free_cam.set_from_orbit(&self.orbit_cam);
            } else {
                self.orbit_cam.set_from_free(&self.free_cam);
            }
            self.camera_is_free = free;
            self.settings.free_camera = free;
        }
    }
}

struct AppRunner {
    settings: Config,
    config_path: PathBuf,
    state: Option<AppState>,
}

impl AppRunner {
    fn new(settings: Config, config_path: PathBuf) -> Self {
        Self {
            settings,
            config_path,
            state: None,
        }
    }
}

impl ApplicationHandler for AppRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let mut window_attributes = Window::default_attributes()
            .with_title("Infant Hand Motion Viewer")
            // Stay hidden until the first frame has been presented so the window never flashes an empty surface
            .with_visible(false)
            .with_inner_size(LogicalSize::new(
                self.settings.window_width as f64,
                self.settings.window_height as f64,
            ))
            .with_resizable(true);
        if let (Some(x), Some(y)) = (self.settings.window_x, self.settings.window_y) {
            let monitors: Vec<MonitorRect> = event_loop
                .available_monitors()
                .map(|m| (m.position().x, m.position().y, m.size().width, m.size().height))
                .collect();
            // A monitor that was unplugged since last run would leave the window unreachable, so let the OS place it.
            if monitors.is_empty() || is_position_reachable((x, y), &monitors) {
                window_attributes = window_attributes.with_position(PhysicalPosition::new(x, y));
            }
        }
        if self.settings.window_fullscreen {
            window_attributes = window_attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        let window = Arc::new(
            event_loop
                .create_window(window_attributes)
                .expect("failed to create window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("failed to create window surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("failed to find a suitable GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("infant hand motion viewer"),
            required_features: required_device_features(&adapter),
            ..Default::default()
        }))
        .expect("failed to create GPU device");
        let gpu = Gpu {
            device,
            queue,
            msaa_counts: supported_sample_counts(&adapter),
        };
        let sample_count = gpu.resolve_sample_count(self.settings.msaa_samples);
        self.settings.msaa_samples = sample_count;

        // Prefer a non-sRGB surface so UI colors are written raw, matching the C++ viewer's default framebuffer.
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let phys_size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: phys_size.width.max(1),
            height: phys_size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &surface_config);

        let mut imgui = ImGuiContext::create();
        let config_flags = imgui.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
        imgui.io_mut().set_config_flags(config_flags);

        // Remove the docking/collapse menu button (reclaims tab-bar offset everywhere)
        imgui
            .style_mut()
            .set_window_menu_button_position(dear_imgui_rs::Direction::None);

        // Windows push their own padding; keep the theme's so tooltips can restore it
        infant_hand_motion_viewer::ui::remember_theme_window_padding(imgui.style().window_padding());

        // Rasterize glyphs with FreeType using light hinting (grid-fit stems to pixel grid for crisp UI text)
        imgui
            .font_atlas()
            .set_font_loader_flags(dear_imgui_rs::fonts::FontLoaderFlags::LIGHT_HINTING);

        // Add embedded UI font at UI_FONT_SIZE (22.0)
        let _ui_font = imgui.font_atlas().add_font(&[unsafe {
            dear_imgui_rs::FontSource::ttf_data_with_size(assets::UI_FONT, infant_hand_motion_viewer::ui::UI_FONT_SIZE)
        }]);

        // Add secondary FPS overlay font at FPS_FONT_SIZE (22.0) with thickened stroke weight
        let fps_font = imgui.font_atlas().add_font(&[unsafe {
            dear_imgui_rs::FontSource::ttf_data_with_size(assets::UI_FONT, infant_hand_motion_viewer::ui::FPS_FONT_SIZE)
                .with_config(dear_imgui_rs::FontConfig::new().rasterizer_multiply(1.15))
        }]);

        let mut platform = WinitPlatform::new(&mut imgui).expect("failed to create winit platform");
        platform
            .attach_window(Arc::clone(&window), HiDpiMode::Default, &mut imgui)
            .expect("failed to attach window to platform");

        let mut imgui_renderer = WgpuRenderer::new(
            WgpuInitInfo::new(gpu.device.clone(), gpu.queue.clone(), surface_format),
            &mut imgui,
        )
        .expect("failed to create imgui renderer");
        imgui_renderer.set_gamma_mode(GammaMode::Linear);

        let renderer = Renderer::new(&gpu, sample_count);
        let hand_overlay = HandOverlay::new(&gpu, sample_count);
        let framebuffer = Framebuffer::new(&gpu, phys_size.width.max(1), phys_size.height.max(1), sample_count);
        let scene_texture = imgui_renderer
            .register_external_texture(framebuffer.color_view())
            .expect("failed to register scene texture");
        let scene_generation = framebuffer.generation();
        let frame_image = ImageTexture::new();
        let icons = UiIcons::new(&gpu, &mut imgui_renderer);

        // Orbit & Free Cameras
        let mut orbit_cam = OrbitCamera::new(self.settings.camera_distance);
        orbit_cam.set_state(
            self.settings.camera_azimuth,
            self.settings.camera_elevation,
            self.settings.camera_distance,
            self.settings.camera_target,
        );
        let mut free_cam = FreeCamera::new();
        if self.settings.free_camera {
            free_cam.set_from_orbit(&orbit_cam);
        }

        let remote_config = RemoteConfig {
            host: self.settings.remote_host.clone(),
            port: self.settings.remote_port,
            python_bin: self.settings.remote_python.clone(),
            root_folder: self.settings.remote_data_folder.clone(),
        };

        let remote_client = RemoteClient::new();
        let remote_fetch_worker = Arc::new(WorkerQueue::new());
        let remote_open_worker = Arc::new(WorkerQueue::new());

        let mut explorer = FileExplorer::new();
        explorer.set_remote_client(remote_client.clone());
        explorer.set_remote_root(&self.settings.remote_data_folder);
        explorer.set_saved_open(self.settings.expanded_folders.clone());
        explorer.set_saved_collapsed(self.settings.collapsed_folders.clone());

        if self.settings.remote_mode && !self.settings.remote_host.is_empty() {
            explorer.set_mode(SourceMode::Remote);
            remote_client.connect_async(remote_config.clone());
        } else if let Some(ref data_folder) = self.settings.data_folder {
            explorer.set_root(data_folder);
        }

        let mut app_state = AppState {
            window,
            surface,
            surface_config,
            gpu,
            imgui,
            platform,
            imgui_renderer,
            scene_texture,
            scene_generation,
            frame_texture: None,
            frame_generation: 0,
            hand_overlay,
            overlay_texture: None,
            overlay_generation: 0,
            overlay_key: None,
            fps_font,
            renderer,
            framebuffer,
            frame_image,
            icons,
            settings: self.settings.clone(),
            config_path: self.config_path.clone(),
            orbit_cam,
            free_cam,
            camera_is_free: self.settings.free_camera,
            right_mouse_down: false,
            cursor_pos: PhysicalPosition::new(0.0, 0.0),
            relative_mouse_anchor: None,
            shift_down: false,
            orbiting: false,
            panning: false,
            viewport_hovered: false,
            keys_down: HashSet::new(),
            sequence: None,
            current_gpu: None,
            loaded_frame: None,
            transform: None,
            depth_reference: None,
            current_frame: self.settings.last_frame,
            playing: false,
            playback_speed: self.settings.playback_speed,
            playback_accumulator: 0.0,
            scrubbing: false,
            scrub_timer: -1.0,
            active_pane: self.settings.active_pane,
            restore_focus_frames: 3,
            explorer,
            show_remote_modal: false,
            remote_connect_in_flight: false,
            remote_config,
            remote_client,
            remote_fetch_worker,
            remote_open_worker,
            pending_remote_open: Arc::new(Mutex::new(None)),
            video: None,
            video_worker: WorkerQueue::new(),
            streamed_frame: None,
            current_remote_path: String::new(),
            active_sequence_id: Arc::new(AtomicU64::new(0)),
            dialogs: DialogChannels::new(),
            pending_open_file: None,
            pending_remote_restore: None,
            last_frame_time: Instant::now(),
            created_at: Instant::now(),
            window_shown: false,
        };

        if let Some(last_folder) = app_state.settings.last_folder.clone()
            && app_state.settings.last_folder_remote
        {
            if app_state.settings.remote_mode {
                app_state.pending_remote_restore = Some((
                    app_state.settings.remote_host.clone(),
                    last_folder,
                    app_state.settings.last_frame,
                ));
            }
        } else if let Some(last_folder) = app_state.settings.last_folder.clone()
            && Path::new(&last_folder).is_file()
        {
            let start_frame = app_state.settings.last_frame;
            app_state.open_sequence(&OpenSource::Local(last_folder), start_frame);
        }

        // Render the first frame while the window is still hidden; it is shown once that frame is presented
        render_app_frame(&mut app_state, event_loop);

        self.state = Some(app_state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let Some(ref mut state) = self.state else {
            return;
        };

        // Forward event to dear-imgui-winit
        let _ = state
            .platform
            .handle_window_event(&mut state.imgui, &state.window, &event);

        match event {
            WindowEvent::CloseRequested => {
                persist_settings(state);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    state.surface_config.width = size.width;
                    state.surface_config.height = size.height;
                    state.surface.configure(&state.gpu.device, &state.surface_config);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                state.shift_down = modifiers.state().shift_key();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state: el_state,
                        repeat,
                        ..
                    },
                ..
            } => {
                let pressed = el_state == ElementState::Pressed;
                if pressed {
                    state.keys_down.insert(key);
                } else {
                    state.keys_down.remove(&key);
                }

                let modal_open = state.show_remote_modal;
                let want_text = state.imgui.io().want_text_input();

                if pressed && !repeat {
                    match key {
                        KeyCode::Escape => {
                            if modal_open {
                                state.show_remote_modal = false;
                            } else if !want_text {
                                persist_settings(state);
                                event_loop.exit();
                            }
                        }
                        KeyCode::F11 => {
                            if state.window.fullscreen().is_some() {
                                state.window.set_fullscreen(None);
                                state.settings.window_fullscreen = false;
                            } else {
                                state.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                                state.settings.window_fullscreen = true;
                            }
                        }
                        KeyCode::Space if !want_text && !modal_open => {
                            if state.sequence.is_some() {
                                state.playing = !state.playing;
                            }
                        }
                        KeyCode::KeyH if !want_text && !modal_open => {
                            state.settings.hand_translucent = !state.settings.hand_translucent;
                        }
                        KeyCode::KeyR if !want_text && !modal_open => {
                            state.active_camera_mut().reset();
                        }
                        KeyCode::Home if !want_text && !modal_open => {
                            state.current_frame = 0;
                        }
                        KeyCode::End if !want_text && !modal_open => {
                            if let Some(ref seq) = state.sequence {
                                state.current_frame = seq.frame_count().saturating_sub(1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseInput {
                button,
                state: el_state,
                ..
            } => {
                if button == MouseButton::Right {
                    let pressed = el_state == ElementState::Pressed;
                    state.right_mouse_down = pressed;
                    let modal_open = state.show_remote_modal;
                    if pressed && state.viewport_hovered && !modal_open {
                        if state.shift_down {
                            state.panning = true;
                        } else {
                            state.orbiting = true;
                        }
                        state.set_relative_mouse(true);
                    } else {
                        state.orbiting = false;
                        state.panning = false;
                        state.set_relative_mouse(false);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if state.relative_mouse_anchor.is_none() {
                    state.cursor_pos = position;
                }
            }
            WindowEvent::Focused(false) => {
                state.keys_down.clear();
                state.shift_down = false;
                state.orbiting = false;
                state.panning = false;
                state.set_relative_mouse(false);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if state.viewport_hovered {
                    let y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(pos) => (pos.y / 20.0) as f32,
                    };
                    if y != 0.0 {
                        state.active_camera_mut().zoom(y);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                render_app_frame(state, event_loop);
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        let Some(ref mut state) = self.state else {
            return;
        };

        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            let dx = dx as f32;
            let dy = dy as f32;
            if state.orbiting {
                state.active_camera_mut().orbit(dx, dy);
            } else if state.panning {
                state.active_camera_mut().pan(dx, dy);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(ref mut state) = self.state {
            // If no frame could be presented (e.g. the surface keeps being lost), still show the window eventually
            if !state.window_shown && state.created_at.elapsed() > FIRST_FRAME_TIMEOUT {
                state.window_shown = true;
                state.window.set_visible(true);
            }
            state.window.request_redraw();
        }
    }
}

/// Gather the live app state into the settings and write them to disk.
fn persist_settings(state: &mut AppState) {
    if let Some(ref seq) = state.sequence {
        state.settings.last_folder = Some(seq.path().to_string());
        state.settings.last_folder_remote = !state.current_remote_path.is_empty();
        state.settings.last_frame = state.current_frame;
    }
    state.settings.active_pane = state.active_pane;
    state.settings.playback_speed = state.playback_speed;
    state.settings.expanded_folders = state.explorer.expanded_paths();
    state.settings.collapsed_folders = state.explorer.collapsed_paths();
    if state.camera_is_free {
        state.orbit_cam.set_from_free(&state.free_cam);
    }
    let (az, el, dist, target) = state.orbit_cam.get_state();
    state.settings.camera_azimuth = az;
    state.settings.camera_elevation = el;
    state.settings.camera_distance = dist;
    state.settings.camera_target = target;

    let remote = state.remote_client.config();
    if !remote.host.is_empty() {
        state.settings.remote_host = remote.host;
        state.settings.remote_port = remote.port;
        state.settings.remote_python = remote.python_bin;
        state.settings.remote_data_folder = remote.root_folder;
    }

    state.settings.window_fullscreen = state.window.fullscreen().is_some();
    if !state.settings.window_fullscreen && state.window.is_minimized() != Some(true) {
        let scale = state.window.scale_factor();
        let size = state.window.inner_size().to_logical::<f64>(scale);
        state.settings.window_width = size.width.round().max(1.0) as u32;
        state.settings.window_height = size.height.round().max(1.0) as u32;
        if let Ok(pos) = state.window.outer_position() {
            state.settings.window_x = Some(pos.x);
            state.settings.window_y = Some(pos.y);
        }
    }
    let _ = state.settings.save(&state.config_path);
}

fn render_app_frame(state: &mut AppState, event_loop: &ActiveEventLoop) {
    let now = Instant::now();
    let dt = (now - state.last_frame_time).as_secs_f64();
    state.last_frame_time = now;

    if let Some(ref rx) = state.dialogs.open_file_rx
        && let Ok(result) = rx.try_recv()
    {
        if let Some(path) = result {
            state.pending_open_file = Some(PendingOpen {
                source: OpenSource::Local(path.to_string_lossy().into_owned()),
                start_frame: 0,
            });
        }
        state.dialogs.open_file_rx = None;
    }

    if let Some(ref rx) = state.dialogs.data_folder_rx
        && let Ok(result) = rx.try_recv()
    {
        if let Some(folder) = result {
            let folder_str = folder.to_string_lossy().into_owned();
            state.explorer.set_root(&folder_str);
            state.settings.data_folder = Some(folder_str);
        }
        state.dialogs.data_folder_rx = None;
    }

    // Track the remote connection: point the explorer at the remote tree once connected, fall back to local otherwise
    state.remote_client.poll();
    if state.remote_client.consume_just_connected() {
        let remote = state.remote_client.config();
        state.remote_config = remote.clone();
        state.settings.remote_mode = true;
        state.settings.remote_host = remote.host.clone();
        state.settings.remote_port = remote.port;
        state.settings.remote_python = remote.python_bin;
        state.settings.remote_data_folder = remote.root_folder.clone();
        let _ = state.settings.save(&state.config_path);

        state.explorer.set_remote_root(&remote.root_folder);
        state.explorer.set_mode(SourceMode::Remote);
        state.explorer.refresh();

        if let Some((host, path, frame)) = state.pending_remote_restore.take()
            && host == remote.host
        {
            state.request_remote_open(path, frame);
        }
    }
    if matches!(
        state.remote_client.state(),
        ConnectionState::Disconnected | ConnectionState::Error
    ) && state.explorer.mode() == SourceMode::Remote
    {
        state.explorer.set_mode(SourceMode::Local);
        if let Some(ref df) = state.settings.data_folder {
            state.explorer.set_root(df);
        }
    }

    if let Some(ready) = state.pending_remote_open.lock().unwrap().take() {
        state.pending_open_file = Some(ready);
    }

    // Apply deferred open
    if let Some(pending) = state.pending_open_file.take() {
        state.open_sequence(&pending.source, pending.start_frame);

        state.active_sequence_id.fetch_add(1, Ordering::SeqCst);
        state.remote_fetch_worker.clear();
        if let Some(video) = state.video.take() {
            video.cancel();
        }
        state.streamed_frame = None;
        if !state.current_remote_path.is_empty() && state.sequence.is_some() {
            state.video = Some(load_remote_video(
                state.remote_client.clone(),
                state.current_remote_path.clone(),
                None,
                &state.video_worker,
            ));
        }
    }

    // Camera WASD / QE movement
    let modal_open = state.show_remote_modal;
    let want_text = state.imgui.io().want_text_input();
    if !want_text && !modal_open {
        let mut forward = 0.0f32;
        let mut right = 0.0f32;
        let mut up = 0.0f32;

        if state.keys_down.contains(&KeyCode::KeyW) {
            forward += 1.0;
        }
        if state.keys_down.contains(&KeyCode::KeyS) {
            forward -= 1.0;
        }
        if state.keys_down.contains(&KeyCode::KeyD) {
            right += 1.0;
        }
        if state.keys_down.contains(&KeyCode::KeyA) {
            right -= 1.0;
        }
        if state.keys_down.contains(&KeyCode::KeyE) {
            up += 1.0;
        }
        if state.keys_down.contains(&KeyCode::KeyQ) {
            up -= 1.0;
        }

        if forward != 0.0 || right != 0.0 || up != 0.0 {
            state.active_camera_mut().move_camera(forward, right, up, dt as f32);
        }

        // Left / Right arrow scrubbing
        let mut scrub_dir = 0i32;
        if state.keys_down.contains(&KeyCode::ArrowRight) {
            scrub_dir += 1;
        }
        if state.keys_down.contains(&KeyCode::ArrowLeft) {
            scrub_dir -= 1;
        }

        if let Some(ref seq) = state.sequence {
            if !state.playing && scrub_dir != 0 {
                let mut step_now = false;
                if state.scrub_timer < 0.0 {
                    step_now = true;
                    state.scrub_timer = SCRUB_INITIAL_DELAY;
                } else {
                    state.scrub_timer -= dt;
                    if state.scrub_timer <= 0.0 {
                        step_now = true;
                        state.scrub_timer = SCRUB_REPEAT_INTERVAL;
                    }
                }
                if step_now {
                    let total = seq.frame_count();
                    if total > 0 {
                        let next = (state.current_frame as i32 + scrub_dir).clamp(0, total as i32 - 1);
                        state.current_frame = next as usize;
                    }
                }
            } else {
                state.scrub_timer = -1.0;
            }
        }
    }

    // Playback progression
    if let Some(ref seq) = state.sequence {
        let frame_count = seq.frame_count();
        if state.playing && !state.scrubbing && frame_count > 0 {
            state.playback_accumulator += dt;
            let step = (state.playback_accumulator * PLAYBACK_FPS * state.playback_speed as f64) as usize;
            if step > 0 {
                state.playback_accumulator -= (step as f64) / (PLAYBACK_FPS * state.playback_speed as f64);
                state.current_frame = (state.current_frame + step) % frame_count;
            }
        } else {
            state.playback_accumulator = 0.0;
        }
    }

    // Prepare GPU mesh if frame changed
    let has_sequence = state.sequence.is_some();
    let frame_count = state.sequence.as_ref().map(|s| s.frame_count()).unwrap_or(0);

    if let Some(ref seq) = state.sequence {
        if state.transform.is_none()
            && frame_count > 0
            && let Some(f0) = seq.load_frame(0)
        {
            state.transform = Some(compute_transform(f0));
            state.depth_reference = Some(reference_depth(f0));
        }

        if state.loaded_frame != Some(state.current_frame) && frame_count > 0 {
            if let Some(frame) = seq.load_frame(state.current_frame) {
                let prepared = prepare_frame(frame, state.settings.per_track_coloring);
                state.current_gpu = Some(FrameGpu::new(&state.gpu, &prepared));
                state.loaded_frame = Some(state.current_frame);
            }

            if let Some(ref img_path) = seq.frame_image_path(state.current_frame) {
                state.frame_image.load(&state.gpu, &img_path.to_string_lossy());
                state.streamed_frame = None;
            }
        }

        // Remote frames come from the decoded video, shown as soon as each one is decoded.
        // The previous image stays up meanwhile so scrubbing never flashes blank.
        if let Some(video) = &state.video
            && seq.frame_image_path(state.current_frame).is_none()
            && let Some(f_num) = seq.frame_number(state.current_frame)
            && f_num > 0
        {
            let f_num = f_num as u32;
            if state.streamed_frame != Some(f_num)
                && let Some(frame) = video.get(f_num as usize - 1)
            {
                let key = format!("{}#{f_num}", state.current_remote_path);
                state.frame_image.load_rgb(&state.gpu, &key, frame);
                state.streamed_frame = Some(f_num);
            }
        }
    }

    // Format status line
    let status = if let Some(ref seq) = state.sequence {
        let hands = seq.hand_count(state.current_frame);
        let s_suffix = if hands == 1 { "" } else { "s" };
        format!(
            "frame {} / {} - {} hand{}{}",
            state.current_frame + 1,
            frame_count,
            hands,
            s_suffix,
            video_status_suffix(state.video.as_deref())
        )
    } else {
        String::new()
    };

    // Prepare ImGui Frame
    state
        .platform
        .prepare_frame(&mut state.imgui, &state.window)
        .expect("failed to prepare imgui frame");

    state.frame_image.update(&state.gpu);
    let frame_texture_id = sync_frame_texture(state);
    // With the hand overlay on, the Frame View shows the frame with the hands drawn over it instead of the bare image
    let frame_texture_id = sync_hand_overlay(state).or(frame_texture_id);

    let open_file_label = state
        .sequence
        .as_ref()
        .map(|s| s.path().to_string())
        .unwrap_or_default();

    let menu_state = MenuState {
        hand_translucent: state.settings.hand_translucent,
        show_camera_marker: state.settings.show_camera_marker,
        free_camera: state.settings.free_camera,
        per_track_coloring: state.settings.per_track_coloring,
        hand_overlay: state.settings.show_hand_overlay,
        msaa_samples: state.renderer.sample_count(),
        msaa_options: state.gpu.msaa_counts.clone(),
        open_file: open_file_label,
        remote_connected: state.remote_client.is_connected(),
    };

    let key_explorer = WindowKey::new("Explorer", "Explorer").expect("explorer key");
    let key_scene = WindowKey::new("Scene", "Scene").expect("scene key");
    let key_frame_view = WindowKey::new("Frame View", "Frame View").expect("frame view key");
    let key_flags = WindowKey::new("Flags", "Flags").expect("flags key");

    let full_layout = DockLayout::split(
        DockSplit::Left,
        0.25,
        DockLayout::tabs([&key_explorer]),
        DockLayout::split(
            DockSplit::Right,
            0.333,
            DockLayout::split(
                DockSplit::Down,
                0.30,
                DockLayout::tabs([&key_flags]),
                DockLayout::tabs([&key_frame_view]),
            ),
            DockLayout::tabs([&key_scene]),
        ),
    );

    let (menu_result, explorer_result, viewport_result, image_result) = {
        let ui = state.imgui.frame();

        let menu_result = draw_menu_bar(ui, menu_state);

        let _dock_id = ui
            .dockspace()
            .main_viewport()
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&full_layout, DockLayoutApply::IfMissing)
            .build();

        // Build transport props
        let transport_pane = state.active_pane;
        let transport_base = Transport {
            has_sequence,
            current_frame: state.current_frame,
            frame_count,
            playing: state.playing,
            speed: state.playback_speed,
            show_transport: false,
            sequence: state.sequence.as_ref(),
            per_track_coloring: state.settings.per_track_coloring,
            flag_layers_enabled: state.settings.flag_layers_enabled,
        };

        let viewport_transport = Transport {
            show_transport: transport_pane == 0,
            ..transport_base
        };
        let image_transport = Transport {
            show_transport: transport_pane == 1,
            ..transport_base
        };

        let cam: &dyn Camera = if state.camera_is_free {
            &state.free_cam
        } else {
            &state.orbit_cam
        };

        let v_res = draw_viewport_window(
            ui,
            Some(state.scene_texture.texture_id()),
            [state.framebuffer.width(), state.framebuffer.height()],
            ui.io().framerate(),
            Some(state.fps_font),
            &status,
            state.settings.show_controls,
            &viewport_transport,
            &state.icons,
            cam,
            state.transform.as_ref(),
            state.depth_reference,
            state.orbiting || state.panning,
            None,
        );

        let i_res = draw_image_window(
            ui,
            "Frame View###Frame View",
            frame_texture_id,
            state.frame_image.width(),
            state.frame_image.height(),
            &image_transport,
            &state.icons,
            None,
        );

        draw_flags_window(ui, "Flags###Flags", &mut state.settings.flag_layers_enabled, None);

        let explorer_result = state.explorer.draw_explorer_window(ui, &state.icons, None);

        draw_remote_modal(
            ui,
            &mut state.show_remote_modal,
            &mut state.remote_config,
            &state.remote_client,
            &mut state.remote_connect_in_flight,
        );

        if state.restore_focus_frames > 0 {
            state.restore_focus_frames -= 1;
            ui.set_window_focus(Some(if state.active_pane == 1 {
                "Frame View###Frame View"
            } else {
                "Scene###Scene"
            }));
        }

        state
            .platform
            .prepare_render(ui, &state.window)
            .expect("failed to prepare imgui render");

        (menu_result, explorer_result, v_res, i_res)
    };

    state.settings.show_controls = viewport_result.show_controls;
    state.viewport_hovered = viewport_result.hovered;
    state.settings.hand_translucent = menu_result.state.hand_translucent;
    state.settings.show_camera_marker = menu_result.state.show_camera_marker;
    state.settings.show_hand_overlay = menu_result.state.hand_overlay;
    if menu_result.state.msaa_samples != state.renderer.sample_count() {
        // Switch live: rebuild the pipelines and the multisampled targets together so they always agree
        let samples = state.gpu.resolve_sample_count(menu_result.state.msaa_samples);
        state.renderer.set_sample_count(&state.gpu, samples);
        state.framebuffer.set_sample_count(&state.gpu, samples);
        state.hand_overlay.set_sample_count(&state.gpu, samples);
        state.overlay_key = None;
        state.settings.msaa_samples = samples;
    }
    if menu_result.state.free_camera != state.camera_is_free {
        state.update_camera_mode(menu_result.state.free_camera);
    }
    if menu_result.state.per_track_coloring != state.settings.per_track_coloring {
        state.settings.per_track_coloring = menu_result.state.per_track_coloring;
        state.loaded_frame = None; // Trigger re-prepare
    }

    if menu_result.exit_requested {
        persist_settings(state);
        event_loop.exit();
        return;
    }
    if menu_result.open_remote_modal_requested || explorer_result.change_remote_requested {
        state.show_remote_modal = true;
    }
    if menu_result.disconnect_remote_requested {
        state.remote_client.disconnect();
        state.settings.remote_mode = false;
        let _ = state.settings.save(&state.config_path);
        state.explorer.set_mode(SourceMode::Local);
        if let Some(ref df) = state.settings.data_folder {
            state.explorer.set_root(df);
        }
    }

    // Native dialog requests
    if menu_result.export_file_requested && state.dialogs.open_file_rx.is_none() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let res = rfd::FileDialog::new()
                .add_filter("Hand export files", &["hexport"])
                .pick_file();
            let _ = tx.send(res);
        });
        state.dialogs.open_file_rx = Some(rx);
    }

    if explorer_result.choose_root_requested && state.dialogs.data_folder_rx.is_none() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let res = rfd::FileDialog::new().pick_folder();
            let _ = tx.send(res);
        });
        state.dialogs.data_folder_rx = Some(rx);
    }

    if let Some(open_path) = explorer_result.open_file {
        if explorer_result.is_remote {
            state.request_remote_open(open_path, 0);
        } else {
            state.pending_open_file = Some(PendingOpen {
                source: OpenSource::Local(open_path),
                start_frame: 0,
            });
        }
    }

    // Echo transport state back
    let echo = if state.active_pane == 1 {
        &image_result.transport
    } else {
        &viewport_result.transport
    };
    if has_sequence {
        state.current_frame = echo.current_frame;
        state.playing = echo.playing;
        state.scrubbing = echo.scrubbing;
        state.playback_speed = echo.speed;
    }
    if viewport_result.focused {
        state.active_pane = 0;
    } else if image_result.focused {
        state.active_pane = 1;
    }

    // Resize the offscreen target to the scene window's size and repoint the UI texture at it
    if state
        .framebuffer
        .resize(&state.gpu, viewport_result.width, viewport_result.height)
        && state.framebuffer.generation() != state.scene_generation
    {
        state.scene_generation = state.framebuffer.generation();
        state
            .imgui_renderer
            .update_external_texture(state.scene_texture, state.framebuffer.color_view())
            .expect("failed to update scene texture");
    }

    // Render 3D Scene into offscreen Framebuffer
    let cam: &dyn Camera = if state.camera_is_free {
        &state.free_cam
    } else {
        &state.orbit_cam
    };
    let scene_req = SceneRender {
        frame: state.current_gpu.as_ref(),
        translucent: state.settings.hand_translucent,
        transform: state.transform.as_ref(),
        reference_depth: state.depth_reference,
        show_camera_marker: state.settings.show_camera_marker,
        hovered_hand: viewport_result.hovered_hand_index,
    };
    state
        .renderer
        .render_scene(&state.gpu, &state.framebuffer, cam, &scene_req);

    // Render ImGui onto the window surface
    let surface_texture = match state.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture) => texture,
        wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            state.surface.configure(&state.gpu.device, &state.surface_config);
            texture
        }
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            state.surface.configure(&state.gpu.device, &state.surface_config);
            return;
        }
        _ => return,
    };
    let surface_view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let consumer = state.imgui_renderer.renderer_consumer().expect("renderer consumer");
    let pending_frame = state.imgui.render(consumer);

    let mut encoder = state
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui") });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.08,
                        g: 0.08,
                        b: 0.10,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        state
            .imgui_renderer
            .render(
                pending_frame,
                &mut pass,
                FramebufferExtent::from_texture(&surface_texture.texture),
            )
            .expect("failed to render imgui");
    }
    state.gpu.queue.submit([encoder.finish()]);
    state.window.pre_present_notify();
    state.gpu.queue.present(surface_texture);

    if !state.window_shown {
        state.window_shown = true;
        state.window.set_visible(true);
        state.window.focus_window();
    }
}

/// What the hand overlay's last render was built from; the overlay is redrawn only when one of these changes.
#[derive(Clone, Copy, PartialEq)]
struct OverlayKey {
    sequence_id: u64,
    frame: usize,
    per_track_coloring: bool,
    image_generation: u64,
    image_version: u64,
    sample_count: u32,
}

fn video_status_suffix(video: Option<&FrameStore>) -> String {
    let Some(video) = video else {
        return String::new();
    };
    match video.status() {
        LoadStatus::Downloading => " - downloading video".to_string(),
        LoadStatus::Decoding => match video.expected() {
            0 => format!(" - decoding video ({})", video.len()),
            total => format!(" - decoding video ({}/{total})", video.len()),
        },
        LoadStatus::Ready => String::new(),
        LoadStatus::Missing => format!(" - {MISSING_VIDEO_MESSAGE}"),
        LoadStatus::Failed(err) => format!(" - video error: {err}"),
    }
}

/// Draws the current frame's hands over the frame image with the WiLoR demo camera, when the hand overlay is on and
/// there is a frame image to draw them on, and returns the UI texture showing the result.
fn sync_hand_overlay(state: &mut AppState) -> Option<TextureId> {
    if !state.settings.show_hand_overlay {
        return None;
    }
    let seq = state.sequence.as_ref()?;
    let hands = seq.load_frame(state.current_frame)?;
    let image_view = state.frame_image.view()?;
    let size = [state.frame_image.width(), state.frame_image.height()];
    if size[0] == 0 || size[1] == 0 {
        return None;
    }

    let key = OverlayKey {
        sequence_id: state.active_sequence_id.load(Ordering::SeqCst),
        frame: state.current_frame,
        per_track_coloring: state.settings.per_track_coloring,
        image_generation: state.frame_image.generation(),
        image_version: state.frame_image.version(),
        sample_count: state.hand_overlay.sample_count(),
    };
    if state.overlay_key != Some(key) {
        // The demo focal length is defined for the image the hands were fitted on; the overlay is drawn at the frame
        // image's resolution, so the camera scales with it.
        let (fit_w, fit_h) = hands
            .iter()
            .find_map(|hand| hand.camera)
            .map_or((size[0], size[1]), |camera| (camera.img_w.max(1), camera.img_h.max(1)));
        let demo_focal = overlay_focal_length(fit_w, fit_h);
        let render_focal = demo_focal * size[0] as f32 / fit_w as f32;
        let meshes = prepare_overlay_hands(&state.gpu, hands, key.per_track_coloring, demo_focal);
        state
            .hand_overlay
            .render(&state.gpu, image_view, size, render_focal, &meshes);
        state.overlay_key = Some(key);
    }

    let view = state.hand_overlay.color_view()?;
    let generation = state.hand_overlay.generation();
    match state.overlay_texture {
        Some(id) if state.overlay_generation == generation => Some(id.texture_id()),
        Some(id) => {
            state.imgui_renderer.update_external_texture(id, view).ok()?;
            state.overlay_generation = generation;
            Some(id.texture_id())
        }
        None => {
            let id = state.imgui_renderer.register_external_texture(view).ok()?;
            state.overlay_texture = Some(id);
            state.overlay_generation = generation;
            Some(id.texture_id())
        }
    }
}

/// Keeps the UI's registration of the frame image texture pointed at its current GPU texture.
fn sync_frame_texture(state: &mut AppState) -> Option<TextureId> {
    let view = state.frame_image.view()?;
    let generation = state.frame_image.generation();
    match state.frame_texture {
        Some(id) if state.frame_generation == generation => Some(id.texture_id()),
        Some(id) => {
            state.imgui_renderer.update_external_texture(id, view).ok()?;
            state.frame_generation = generation;
            Some(id.texture_id())
        }
        None => {
            let id = state.imgui_renderer.register_external_texture(view).ok()?;
            state.frame_texture = Some(id);
            state.frame_generation = generation;
            Some(id.texture_id())
        }
    }
}

fn main() {
    let config_path = Config::default_config_path();
    let settings = Config::load(&config_path);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = AppRunner::new(settings, config_path);
    event_loop.run_app(&mut app).expect("application event loop failed");
}
