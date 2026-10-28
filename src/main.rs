#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use infant_hand_motion_viewer::data::{
    GraspMark, HandData, MeshSequence, Transform, clip_range, compute_transform, next_event_frame, reference_depth,
};
use infant_hand_motion_viewer::graphics::{
    Camera, FrameGpu, Framebuffer, FreeCamera, Gpu, HandOverlay, ImageTexture, OrbitCamera, Renderer, SceneRender,
    hand_display_color, overlay_focal_length, prepare_frame, prepare_overlay_hands, required_device_features,
    supported_sample_counts,
};
use infant_hand_motion_viewer::remote::{ConnectionState, RemoteClient, RemoteConfig, fetch_export};
use infant_hand_motion_viewer::ui::{
    FileExplorer, GraspPaneState, MeasuresView, MenuState, RootsTab, SourceMode, Transport, UiIcons, VideoNotice,
    VideoRootsDialog, draw_flags_window, draw_grasp_window, draw_image_window, draw_measures_window, draw_menu_bar,
    draw_remote_modal, draw_video_roots_modal, draw_viewport_window,
};
use infant_hand_motion_viewer::util::WorkerQueue;
use infant_hand_motion_viewer::util::window_placement::{MonitorRect, is_position_reachable};
use infant_hand_motion_viewer::video::decoder::{ffmpeg_available, missing_ffmpeg_message};
use infant_hand_motion_viewer::video::loader::{
    LocalVideoRequest, RemoteVideoRequest, load_local_video, load_remote_video,
};
use infant_hand_motion_viewer::video::{FrameStore, LoadStatus, VideoIdentity};

/// Longest the window stays hidden waiting for its first presented frame.
const FIRST_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const PLAYBACK_FPS: f64 = 30.0;
const SCRUB_INITIAL_DELAY: f64 = 0.25;
const SCRUB_REPEAT_INTERVAL: f64 = 0.03;

/// A sequence finished loading on the worker, waiting for the UI thread to swap it in.
struct OpenedSequence {
    generation: u64,
    path: String,
    remote: bool,
    start_frame: usize,
    result: Result<MeshSequence, String>,
}

const CACHE_SIZE_INTERVAL: Duration = Duration::from_secs(2);

struct DialogChannels {
    open_file_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    data_folder_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    cache_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    video_root_rx: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
}

impl DialogChannels {
    fn new() -> Self {
        Self {
            open_file_rx: None,
            data_folder_rx: None,
            cache_rx: None,
            video_root_rx: None,
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
    video_roots_dialog: VideoRootsDialog,
    /// (left, right) hidden in the scene and overlay as of the last prepared frame.
    applied_hidden_hands: (bool, bool),
    measures_view: MeasuresView,
    remote_connect_in_flight: bool,
    remote_config: RemoteConfig,
    remote_client: RemoteClient,
    remote_fetch_worker: Arc<WorkerQueue>,
    remote_open_worker: Arc<WorkerQueue>,
    opened_sequence: Arc<Mutex<Option<OpenedSequence>>>,
    /// Counts open requests, so only the newest one's result is installed.
    open_generation: Arc<AtomicU64>,
    video: Option<Arc<FrameStore>>,
    extra_views: [ExtraView; 2],
    video_worker: WorkerQueue,
    streamed_frame: Option<u32>,
    current_remote_path: String,
    active_sequence_id: Arc<AtomicU64>,

    // Native file dialogs
    dialogs: DialogChannels,
    /// Remote file (host, path, frame) to reopen once the saved connection is back up.
    pending_remote_restore: Option<(String, String, usize)>,

    // Timing
    last_frame_time: Instant,
    cache_size: u64,
    cache_size_checked: Instant,
    created_at: Instant,
    window_shown: bool,
}

impl AppState {
    /// The inclusive frames playback, scrubbing and the plots cover: the whole trial, or around E2..E4 when clipped.
    fn frame_range(&self) -> (usize, usize) {
        let Some(seq) = &self.sequence else { return (0, 0) };
        let last = seq.frame_count().saturating_sub(1);
        if !self.settings.clip_to_events {
            return (0, last);
        }
        clip_range(&open_trial_marks(seq, self.video.as_deref()), seq.frame_count())
    }

    /// Moves the current frame into the clip range.
    fn clamp_to_range(&mut self) {
        let (first, last) = self.frame_range();
        self.current_frame = self.current_frame.clamp(first, last);
    }

    /// Starts looking up and decoding the open trial's video, from the remote when the trial came from there.
    fn start_video_load(&mut self) {
        if let Some(video) = self.video.take() {
            video.cancel();
        }
        for view in &mut self.extra_views {
            view.reset();
        }
        self.streamed_frame = None;
        let Some(seq) = &self.sequence else { return };
        let meta = seq.meta().clone();
        if self.current_remote_path.is_empty() {
            self.video = Some(load_local_video(
                LocalVideoRequest {
                    identity: VideoIdentity {
                        hash: meta.video_hash,
                        basename: meta.video_basename,
                        size_bytes: meta.video_size_bytes,
                    },
                    roots: self.settings.local_video_roots.iter().map(PathBuf::from).collect(),
                    hash_index_path: self.config_path.with_file_name("video_hashes.json"),
                },
                &self.video_worker,
            ));
            return;
        }
        let request = |kind| RemoteVideoRequest {
            client: self.remote_client.clone(),
            export_path: self.current_remote_path.clone(),
            kind,
            video_hash: meta.video_hash.clone(),
            roots: self.settings.remote_video_roots.clone(),
        };
        self.video = Some(load_remote_video(
            request(None),
            self.settings.cache(),
            &self.video_worker,
        ));
        let kinds: Vec<_> = self.extra_views.iter().map(|view| view.kind).collect();
        for (view, kind) in self.extra_views.iter_mut().zip(kinds) {
            view.video = Some(load_remote_video(
                request(Some(kind)),
                self.settings.cache(),
                &self.video_worker,
            ));
        }
    }

    /// Swaps a sequence that finished loading on the worker in for the open one.
    fn install_sequence(&mut self, opened: OpenedSequence) {
        let OpenedSequence {
            generation: _,
            path,
            remote,
            start_frame,
            result,
        } = opened;
        self.frame_image.clear();
        self.current_remote_path = if remote { path.clone() } else { String::new() };
        // Freeing a large sequence takes long enough to hitch a frame, so it goes to another thread
        let previous = self.sequence.take();
        match result {
            Ok(seq) if seq.frame_count() > 0 => {
                self.current_frame = start_frame.min(seq.frame_count() - 1);
                self.sequence = Some(seq);
            }
            Ok(_) => {
                eprintln!("No frames found in {path}");
                self.current_frame = 0;
            }
            Err(err) => {
                eprintln!("Failed to open {path}: {err}");
                self.current_frame = 0;
            }
        }
        if let Some(previous) = previous {
            std::thread::spawn(move || drop(previous));
        }
        self.explorer.set_selected(&path);
        self.current_gpu = None;
        self.loaded_frame = None;
        self.transform = None;
        self.depth_reference = None;

        self.active_sequence_id.fetch_add(1, Ordering::SeqCst);
        self.remote_fetch_worker.clear();
        if let Some(video) = self.video.take() {
            video.cancel();
        }
        for view in &mut self.extra_views {
            view.reset();
        }
        self.streamed_frame = None;
        self.start_video_load();
    }

    /// Loads (and for remote files, downloads) and parses a `.hexport` on a worker, so the UI never waits for it. The
    /// open sequence stays on screen until the new one is ready; a newer request makes this one's result be dropped.
    fn request_open(&mut self, path: String, remote: bool, start_frame: usize) {
        self.explorer.set_selected(&path);
        let client = self.remote_client.clone();
        let generation = self.open_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let latest = Arc::clone(&self.open_generation);
        let ready = Arc::clone(&self.opened_sequence);
        let cache = self.settings.cache();
        self.remote_open_worker.submit(move || {
            let is_current = || latest.load(Ordering::SeqCst) == generation;
            if !is_current() {
                return;
            }
            let result = if remote {
                fetch_export(&client, cache.as_ref(), &path)
                    .and_then(|bytes| MeshSequence::from_bytes(&path, &bytes).map_err(|err| err.to_string()))
            } else {
                MeshSequence::open(&path).map_err(|err| err.to_string())
            };
            if is_current() {
                *ready.lock().unwrap() = Some(OpenedSequence {
                    generation,
                    path,
                    remote,
                    start_frame,
                    result,
                });
            }
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
        let ini_path = self.config_path.with_file_name("imgui.ini");
        if let Some(dir) = ini_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = imgui.set_ini_filename(Some(ini_path));
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
        explorer.set_filter_text(&self.settings.explorer_filter);
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
            video_roots_dialog: VideoRootsDialog::default(),
            applied_hidden_hands: (false, false),
            measures_view: MeasuresView::default(),
            remote_connect_in_flight: false,
            remote_config,
            remote_client,
            remote_fetch_worker,
            remote_open_worker,
            opened_sequence: Arc::new(Mutex::new(None)),
            open_generation: Arc::new(AtomicU64::new(0)),
            video: None,
            extra_views: [
                ExtraView::new("wilor", "WiLoR View"),
                ExtraView::new("sam3", "SAM3 View"),
            ],
            video_worker: WorkerQueue::new(),
            streamed_frame: None,
            current_remote_path: String::new(),
            active_sequence_id: Arc::new(AtomicU64::new(0)),
            dialogs: DialogChannels::new(),
            pending_remote_restore: None,
            last_frame_time: Instant::now(),
            cache_size: 0,
            cache_size_checked: Instant::now() - CACHE_SIZE_INTERVAL,
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
            app_state.request_open(last_folder, false, start_frame);
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

                let modal_open = state.show_remote_modal || state.video_roots_dialog.open;
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
                            state.current_frame = state.frame_range().0;
                        }
                        KeyCode::End if !want_text && !modal_open && state.sequence.is_some() => {
                            state.current_frame = state.frame_range().1;
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
                    let modal_open = state.show_remote_modal || state.video_roots_dialog.open;
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
    state.settings.explorer_filter = state.explorer.filter_text().to_string();
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
            state.request_open(path.to_string_lossy().into_owned(), false, 0);
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

    if let Some(ref rx) = state.dialogs.video_root_rx
        && let Ok(result) = rx.try_recv()
    {
        if let Some(folder) = result {
            let folder = folder.to_string_lossy().into_owned();
            if !state.settings.local_video_roots.contains(&folder) {
                state.settings.local_video_roots.push(folder);
                let _ = state.settings.save(&state.config_path);
                state.start_video_load();
            }
        }
        state.dialogs.video_root_rx = None;
    }

    if let Some(ref rx) = state.dialogs.cache_rx
        && let Ok(result) = rx.try_recv()
    {
        if let Some(folder) = result {
            state.settings.cache_location = Some(folder.to_string_lossy().into_owned());
            state.cache_size_checked = now - CACHE_SIZE_INTERVAL;
        }
        state.dialogs.cache_rx = None;
    }

    if state.cache_size_checked.elapsed() >= CACHE_SIZE_INTERVAL {
        state.cache_size = state.settings.cache_dir().size();
        state.cache_size_checked = now;
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
            state.request_open(path, true, frame);
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

    let opened = state.opened_sequence.lock().unwrap().take();
    if let Some(opened) = opened
        && opened.generation == state.open_generation.load(Ordering::SeqCst)
    {
        state.install_sequence(opened);
    }

    // Camera WASD / QE movement
    let modal_open = state.show_remote_modal || state.video_roots_dialog.open;
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

        state.clamp_to_range();
        let (range_first, range_last) = state.frame_range();
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
                if step_now && seq.frame_count() > 0 {
                    let next = (state.current_frame as i32 + scrub_dir).clamp(range_first as i32, range_last as i32);
                    state.current_frame = next as usize;
                }
            } else {
                state.scrub_timer = -1.0;
            }
        }
    }

    // Playback progression
    state.clamp_to_range();
    let (range_first, range_last) = state.frame_range();
    if let Some(ref seq) = state.sequence {
        let frame_count = seq.frame_count();
        if state.playing && !state.scrubbing && frame_count > 0 {
            state.playback_accumulator += dt;
            let step = (state.playback_accumulator * PLAYBACK_FPS * state.playback_speed as f64) as usize;
            if step > 0 {
                state.playback_accumulator -= (step as f64) / (PLAYBACK_FPS * state.playback_speed as f64);
                let from = state.current_frame;
                let stop = state
                    .settings
                    .pause_at_events
                    .then(|| {
                        let marks = open_trial_marks(seq, state.video.as_deref());
                        next_event_frame(&marks, from, (from + step).min(range_last))
                    })
                    .flatten();
                if let Some(frame) = stop {
                    state.current_frame = frame;
                    state.playing = false;
                } else {
                    let len = range_last - range_first + 1;
                    state.current_frame = range_first + (from - range_first + step) % len;
                }
            }
        } else {
            state.playback_accumulator = 0.0;
        }
    }

    if let Some(seq) = state.sequence.as_mut()
        && seq.is_smoothed() != state.settings.use_smoothed
    {
        seq.set_smoothed(state.settings.use_smoothed);
        state.loaded_frame = None;
        state.overlay_key = None;
    }

    // Prepare GPU mesh if frame changed
    let has_sequence = state.sequence.is_some();
    let frame_count = state.sequence.as_ref().map(|s| s.frame_count()).unwrap_or(0);

    if let Some(ref seq) = state.sequence {
        let hidden = hidden_hands(seq, state.settings.hide_unused_hands);
        if hidden != state.applied_hidden_hands {
            state.applied_hidden_hands = hidden;
            state.loaded_frame = None;
            state.overlay_key = None;
        }
        if state.transform.is_none()
            && frame_count > 0
            && let Some(f0) = seq.load_frame(0)
        {
            state.transform = Some(compute_transform(f0));
            state.depth_reference = Some(reference_depth(f0));
        }

        if state.loaded_frame != Some(state.current_frame)
            && frame_count > 0
            && let Some(frame) = seq.load_frame(state.current_frame)
        {
            let prepared = prepare_frame(&shown_hands(frame, hidden), state.settings.per_track_coloring);
            state.current_gpu = Some(FrameGpu::new(&state.gpu, &prepared));
            state.loaded_frame = Some(state.current_frame);
        }

        // Frames come from the decoded video, shown as soon as each one is decoded.
        // The previous image stays up meanwhile so scrubbing never flashes blank.
        if let Some(video) = &state.video
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

    if let Some(seq) = &state.sequence
        && let Some(f_num) = seq.frame_number(state.current_frame)
        && f_num > 0
    {
        for view in &mut state.extra_views {
            view.sync(&state.gpu, &mut state.imgui_renderer, f_num as u32);
        }
    }

    // Format status line
    let status = if let Some(ref seq) = state.sequence {
        let per_track = state.settings.per_track_coloring;
        let hands = seq.load_frame(state.current_frame).map_or(0, |frame| {
            frame
                .iter()
                .filter(|hand| hand_display_color(hand, per_track).is_some())
                .count()
        });
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
        show_measures: state.settings.show_measures,
        show_flags: state.settings.show_flags,
        show_grasp: state.settings.show_grasp,
        show_frame_view: state.settings.show_frame_view,
        show_wilor_view: state.settings.show_wilor_view,
        show_sam3_view: state.settings.show_sam3_view,
        use_smoothed: state.settings.use_smoothed,
        msaa_samples: state.renderer.sample_count(),
        msaa_options: state.gpu.msaa_counts.clone(),
        open_file: open_file_label,
        cache_enabled: state.settings.cache_enabled,
        cache_location: state.settings.cache_location.clone().unwrap_or_default(),
        cache_size: state.cache_size,
        remote_connected: state.remote_client.is_connected(),
    };

    state.clamp_to_range();
    let frame_range = state.frame_range();
    let grasp_row = state.sequence.as_ref().and_then(|seq| seq.meta().grasp_events.as_ref());
    let grasp_marks = match &state.sequence {
        Some(seq) => open_trial_marks(seq, state.video.as_deref()),
        None => Vec::new(),
    };

    let key_explorer = WindowKey::new("Explorer", "Explorer").expect("explorer key");
    let key_scene = WindowKey::new("Scene", "Scene").expect("scene key");
    let key_frame_view = WindowKey::new("Frame View", "Frame View").expect("frame view key");
    let key_flags = WindowKey::new("Flags", "Flags").expect("flags key");
    let key_measures = WindowKey::new("Hand Measures", "Hand Measures").expect("measures key");
    let key_grasp = WindowKey::new("Grasp", "Grasp").expect("grasp key");

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
                DockLayout::tabs([&key_flags, &key_measures, &key_grasp]),
                DockLayout::tabs([&key_frame_view]),
            ),
            DockLayout::tabs([&key_scene]),
        ),
    );

    let measures_menu_in = state.settings.show_measures;
    let (menu_result, explorer_result, viewport_result, image_result, extra_results, measures_scrub, roots_result) = {
        let ui = state.imgui.frame();

        let menu_result = draw_menu_bar(ui, menu_state);

        let _dock_id = ui
            .dockspace()
            .main_viewport()
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&full_layout, DockLayoutApply::IfMissing)
            .build();

        // Build transport props
        let image_windows_open =
            state.settings.show_frame_view || state.settings.show_wilor_view || state.settings.show_sam3_view;
        let transport_pane = if image_windows_open { state.active_pane } else { 0 };
        let transport_base = Transport {
            has_sequence,
            current_frame: state.current_frame,
            frame_count,
            range: frame_range,
            playing: state.playing,
            speed: state.playback_speed,
            show_transport: false,
            sequence: state.sequence.as_ref(),
            per_track_coloring: state.settings.per_track_coloring,
            flag_layers_enabled: state.settings.flag_layers_enabled,
            grasp_marks: &grasp_marks,
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

        let notice = video_notice(state.video.as_deref());
        let i_res = draw_image_window(
            ui,
            "Frame View###Frame View",
            frame_texture_id,
            state.frame_image.width(),
            state.frame_image.height(),
            &image_transport,
            &state.icons,
            state.settings.show_frame_view,
            None,
            notice.as_ref(),
        );

        let mut extra_results = Vec::new();
        let extra_dock = state.settings.show_frame_view.then_some(i_res.dock_id);
        for view in &state.extra_views {
            let open = match view.kind {
                "wilor" => state.settings.show_wilor_view,
                _ => state.settings.show_sam3_view,
            };
            extra_results.push(draw_image_window(
                ui,
                &format!("{}###{}", view.title, view.title),
                view.texture_id,
                view.image.width(),
                view.image.height(),
                &image_transport,
                &state.icons,
                open,
                extra_dock,
                None,
            ));
        }

        draw_flags_window(
            ui,
            "Flags###Flags",
            &mut state.settings.flag_layers_enabled,
            state.settings.show_flags,
            None,
        );

        let measures_scrub = draw_measures_window(
            ui,
            "Hand Measures###Hand Measures",
            state.settings.show_measures,
            &mut state.measures_view,
            state.sequence.as_ref(),
            state.settings.per_track_coloring,
            state.current_frame,
            &grasp_marks,
            state.applied_hidden_hands,
            frame_range,
            None,
        );

        let grasp_state = match (&state.sequence, grasp_row) {
            (None, _) => GraspPaneState::NoSequence,
            (Some(_), Some(row)) => GraspPaneState::Row(row),
            (Some(_), None) => GraspPaneState::NoEvents,
        };
        draw_grasp_window(
            ui,
            "Grasp###Grasp",
            grasp_state,
            &mut state.settings.hide_unused_hands,
            &mut state.settings.pause_at_events,
            &mut state.settings.clip_to_events,
            state.settings.show_grasp,
            None,
        );

        let explorer_result =
            state
                .explorer
                .draw_explorer_window(ui, &state.icons, state.settings.hide_unused_hands, None);

        let roots_result = draw_video_roots_modal(
            ui,
            &mut state.video_roots_dialog,
            &mut state.settings.local_video_roots,
            &mut state.settings.remote_video_roots,
            state.remote_client.is_connected(),
        );
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

        (
            menu_result,
            explorer_result,
            v_res,
            i_res,
            extra_results,
            measures_scrub,
            roots_result,
        )
    };

    if menu_result.state.show_measures != measures_menu_in {
        state.settings.show_measures = menu_result.state.show_measures;
    }
    state.settings.show_flags = menu_result.state.show_flags;
    state.settings.show_grasp = menu_result.state.show_grasp;
    state.settings.show_frame_view = menu_result.state.show_frame_view;
    state.settings.show_wilor_view = menu_result.state.show_wilor_view;
    state.settings.show_sam3_view = menu_result.state.show_sam3_view;

    state.settings.show_controls = viewport_result.show_controls;
    state.viewport_hovered = viewport_result.hovered;
    state.settings.hand_translucent = menu_result.state.hand_translucent;
    state.settings.show_camera_marker = menu_result.state.show_camera_marker;
    state.settings.show_hand_overlay = menu_result.state.hand_overlay;
    state.settings.use_smoothed = menu_result.state.use_smoothed;
    state.settings.cache_enabled = menu_result.state.cache_enabled;
    state.settings.cache_location =
        Some(menu_result.state.cache_location.clone()).filter(|location| !location.trim().is_empty());
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
    if image_result.locate_requested || menu_result.open_video_roots_requested {
        let tab = if state.explorer.mode() == SourceMode::Remote {
            RootsTab::Remote
        } else {
            RootsTab::Local
        };
        state.video_roots_dialog.show(tab);
    }
    if roots_result.changed {
        let _ = state.settings.save(&state.config_path);
        state.start_video_load();
    }
    if roots_result.browse_local_requested && state.dialogs.video_root_rx.is_none() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(rfd::FileDialog::new().pick_folder());
        });
        state.dialogs.video_root_rx = Some(rx);
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

    if menu_result.clear_cache_requested {
        state.settings.cache_dir().clear();
        state.cache_size_checked = now - CACHE_SIZE_INTERVAL;
    }
    if menu_result.choose_cache_location_requested && state.dialogs.cache_rx.is_none() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let res = rfd::FileDialog::new().pick_folder();
            let _ = tx.send(res);
        });
        state.dialogs.cache_rx = Some(rx);
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
        state.request_open(open_path, explorer_result.is_remote, 0);
    }

    // Echo transport state back from whichever window the user changed it in; unchanged windows just repeat it.
    let current = (state.current_frame, state.playing, state.playback_speed);
    let changed = |t: &&infant_hand_motion_viewer::ui::TransportState| {
        t.scrubbing || (t.current_frame, t.playing, t.speed) != current
    };
    let image_transports = std::iter::once(&image_result)
        .chain(&extra_results)
        .map(|r| &r.transport);
    let echo = image_transports
        .chain(std::iter::once(&viewport_result.transport))
        .find(changed)
        .unwrap_or(if state.active_pane == 1 {
            &image_result.transport
        } else {
            &viewport_result.transport
        });
    if has_sequence {
        state.current_frame = echo.current_frame;
        state.playing = echo.playing;
        state.scrubbing = echo.scrubbing;
        state.playback_speed = echo.speed;
    }
    // Dragging a measure plot wins over the transport echo above, and holds playback like the scrubber does
    if let Some(frame) = measures_scrub {
        state.current_frame = frame;
        state.playing = false;
        state.scrubbing = true;
    }
    if viewport_result.focused {
        state.active_pane = 0;
    } else if image_result.focused || extra_results.iter().any(|r| r.focused) {
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

/// Which sides, `(left, right)`, the Grasp pane's "hide unused hands" hides for the open trial: those its CSV row does
/// not mark as used. Nothing is hidden when the option is off or the trial has no row.
fn hidden_hands(seq: &MeshSequence, hide_unused: bool) -> (bool, bool) {
    match &seq.meta().grasp_events {
        Some(row) if hide_unused => row.unused_sides(),
        _ => (false, false),
    }
}

/// The open trial's grasp events placed on its frames. Uses the video's frame rate once it is probed and the
/// export's own `fps` until then, so the marks do not wait for the video; empty without events or a rate.
fn open_trial_marks(seq: &MeshSequence, video: Option<&FrameStore>) -> Vec<GraspMark> {
    let fps = video.and_then(|video| video.fps()).or(seq.meta().fps);
    match (&seq.meta().grasp_events, fps) {
        (Some(row), Some(fps)) => row.marks(fps, seq.frame_count()),
        _ => Vec::new(),
    }
}

/// The frame's hands minus the hidden sides.
fn shown_hands(hands: &[HandData], hidden: (bool, bool)) -> std::borrow::Cow<'_, [HandData]> {
    match hidden {
        (false, false) => std::borrow::Cow::Borrowed(hands),
        (left, right) => std::borrow::Cow::Owned(
            hands
                .iter()
                .filter(|hand| !(if hand.is_right { right } else { left }))
                .cloned()
                .collect(),
        ),
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

/// Debug: an extra tab beside Frame View showing a pre-rendered video of the trial.
struct ExtraView {
    kind: &'static str,
    title: &'static str,
    video: Option<Arc<FrameStore>>,
    image: ImageTexture,
    texture: Option<ExternalTextureId>,
    texture_id: Option<TextureId>,
    generation: u64,
    shown_frame: Option<u32>,
}

impl ExtraView {
    fn new(kind: &'static str, title: &'static str) -> Self {
        Self {
            kind,
            title,
            video: None,
            image: ImageTexture::new(),
            texture: None,
            texture_id: None,
            generation: 0,
            shown_frame: None,
        }
    }

    fn reset(&mut self) {
        if let Some(video) = self.video.take() {
            video.cancel();
        }
        self.image.clear();
        self.shown_frame = None;
    }

    fn sync(&mut self, gpu: &Gpu, renderer: &mut WgpuRenderer, f_num: u32) {
        if let Some(video) = &self.video
            && self.shown_frame != Some(f_num)
            && let Some(frame) = video.get(f_num as usize - 1)
        {
            self.image.load_rgb(gpu, &format!("{}#{f_num}", self.kind), frame);
            self.shown_frame = Some(f_num);
        }
        self.image.update(gpu);
        let Some(view) = self.image.view() else {
            self.texture_id = None;
            return;
        };
        let generation = self.image.generation();
        self.texture_id = match self.texture {
            Some(id) if self.generation == generation => Some(id.texture_id()),
            Some(id) => renderer.update_external_texture(id, view).ok().map(|_| id.texture_id()),
            None => renderer.register_external_texture(view).ok().map(|id| {
                self.texture = Some(id);
                id.texture_id()
            }),
        };
        self.generation = generation;
    }
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
        LoadStatus::Searching { done, total } if total > 0 => format!(" - indexing videos ({done}/{total})"),
        LoadStatus::Searching { .. } => " - searching for video".to_string(),
        LoadStatus::Missing(_) => " - video not found".to_string(),
        LoadStatus::Failed(err) => format!(" - video error: {err}"),
    }
}

/// What the Frame View says when the trial's video is not under any configured root.
fn video_notice(video: Option<&FrameStore>) -> Option<VideoNotice> {
    let LoadStatus::Missing(missing) = video?.status() else {
        return None;
    };
    let mut lines = vec!["Source video not found".to_string()];
    lines.extend(missing.basename.map(|name| format!("Looking for {name}")));
    if !missing.hash.is_empty() {
        lines.push(format!("sha256 {}", missing.hash));
    }
    Some(VideoNotice { lines })
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
        let meshes = prepare_overlay_hands(
            &state.gpu,
            &shown_hands(hands, state.applied_hidden_hands),
            key.per_track_coloring,
            demo_focal,
        );
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

    if !ffmpeg_available() {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("FFmpeg not found")
            .set_description(missing_ffmpeg_message())
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = AppRunner::new(settings, config_path);
    event_loop.run_app(&mut app).expect("application event loop failed");
}
