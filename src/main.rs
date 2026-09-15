//! hypr-xp-magnifier — phase 4 (adds "follow keyboard focus")
//!
//! Adds live, cursor-following magnification on top of the phase 1 dock
//! scaffold:
//!   - A background thread polls Hyprland's own IPC socket for the global
//!     cursor position (Wayland hides this from ordinary clients on purpose,
//!     so on Hyprland specifically we go through its own socket instead).
//!   - Each display frame, we ask the compositor (via wlr-screencopy) to
//!     capture the region of the screen around the cursor, then scale that
//!     up to fill the docked strip.
//!   - Settings (zoom factor, thickness, position, refresh rate, sharpness,
//!     follow-keypress) live in a small JSON file, watched and reloaded a
//!     few times a second, so the separate `magnifier-settings` GUI can
//!     change them live without restarting this program.
//!   - Optionally (see `focus_tracker.rs`), it can follow keyboard focus
//!     (e.g. Tab-highlighted buttons) via AT-SPI2 instead of the mouse,
//!     whichever moved more recently.
//!
//! Known simplifications, worth knowing about:
//!   - The bar itself only ever lives on and captures from one monitor
//!     (whichever output is discovered first). Cursor coordinates are
//!     converted into that monitor's own local space and clamped to its
//!     bounds, so moving the cursor onto a second monitor doesn't leak that
//!     monitor's content in — the view just stays pinned to the nearest
//!     edge of the bar's own monitor instead.
//!   - When the tracked point (cursor or focused element) is over the bar
//!     itself, it shows an empty dark placeholder instead of capturing —
//!     capturing there would just show the bar magnifying its own output.
//!   - "Selection follow" (a second AT-SPI event type, for things like
//!     listbox/menu selection) is NOT in this version yet — only keyboard
//!     focus is. Planned as a follow-up once this is confirmed working.

mod focus_tracker;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};

use focus_tracker::FocusRect;

/// Widest monitor width we pre-allocate drawing memory for. If you're on
/// something wider than 4K and see a panic in `create_buffer`, raise this.
const MAX_ASSUMED_WIDTH: usize = 3840;

/// Tallest monitor height we pre-allocate drawing memory for. Also doubles
/// as the pixel-buffer budget in the other axis when docked left/right.
const MAX_ASSUMED_HEIGHT: usize = 2160;

/// How often the background thread polls Hyprland for the cursor position.
/// (This is independent of the "refresh rate" setting, which instead
/// controls how often we ask for a new *capture* — cursor tracking itself
/// stays fast so following the mouse feels responsive regardless.)
const CURSOR_POLL_INTERVAL: Duration = Duration::from_millis(8); // ~120Hz

/// How often the background thread re-reads the settings file.
const SETTINGS_RELOAD_INTERVAL: Duration = Duration::from_millis(300);

/// How often to print the measured frame rate to the terminal.
const FPS_REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// Which edge of the screen the bar is docked to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum DockPosition {
    Top,
    Bottom,
    Left,
    Right,
}

impl Default for DockPosition {
    fn default() -> Self {
        DockPosition::Top
    }
}

fn is_horizontal(position: DockPosition) -> bool {
    matches!(position, DockPosition::Top | DockPosition::Bottom)
}

fn anchor_for(position: DockPosition) -> Anchor {
    match position {
        DockPosition::Top => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        DockPosition::Bottom => Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        DockPosition::Left => Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
        DockPosition::Right => Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
    }
}

fn default_thickness() -> u32 {
    200
}

/// Live-adjustable settings, shared with the `magnifier-settings` GUI via a
/// small JSON file on disk.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Settings {
    zoom_factor: f64,
    /// How thick the bar is: its height when docked top/bottom, or its
    /// width when docked left/right. (Called `dock_height` in older
    /// settings files from before `position` existed — still read from
    /// there automatically.)
    #[serde(alias = "dock_height", default = "default_thickness")]
    thickness: u32,
    refresh_rate_hz: f64,
    /// 0.0 = smooth (bilinear), 1.0 = sharp/blocky (nearest-neighbor).
    sharpness: f64,
    #[serde(default)]
    position: DockPosition,
    /// Follow keyboard focus (e.g. Tab-highlighted buttons) via AT-SPI2
    /// instead of the mouse, whichever moved more recently. Requires the
    /// AT-SPI accessibility bus to be running.
    #[serde(default)]
    follow_keypress: bool,
    /// Added to the focused element's Y position before centering on it.
    /// Compensates for toolkits (GTK, etc.) whose window-relative
    /// coordinates don't include their own title bar height, which
    /// Hyprland's window bounds do — shows up as focus-follow consistently
    /// landing on the wrong row by a fixed amount. Tune by eye.
    #[serde(default)]
    focus_y_correction: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            zoom_factor: 2.5,
            thickness: 200,
            refresh_rate_hz: 60.0,
            sharpness: 1.0,
            position: DockPosition::Top,
            follow_keypress: false,
            focus_y_correction: 40,
        }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("hypr-xp-magnifier").join("settings.json")
}

fn load_settings() -> Settings {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings).unwrap_or_default();
    std::fs::write(path, text)
}

/// Reads `--height N` from the command line, if present. This only sets the
/// *starting* thickness on first run (before a settings file exists) — after
/// that, it's controlled by the settings file / GUI. (Still called
/// `--height` for now even though it also applies to left/right docking.)
fn parse_thickness_override() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--height" {
            if let Some(value) = args.get(i + 1) {
                match value.parse::<u32>() {
                    Ok(h) if h > 0 => return Some(h),
                    _ => eprintln!("Couldn't parse --height value '{value}', ignoring."),
                }
            } else {
                eprintln!("--height needs a number after it, ignoring.");
            }
        }
    }
    None
}

fn main() {
    env_logger::init();

    let mut initial_settings = load_settings();
    if let Some(t) = parse_thickness_override() {
        initial_settings.thickness = t;
    }
    if !config_path().exists() {
        let _ = save_settings(&initial_settings);
    }
    let thickness = initial_settings.thickness;
    let position = initial_settings.position;

    let conn = Connection::connect_to_env().expect(
        "Could not connect to a Wayland compositor. Run this from inside your Hyprland session.",
    );

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
    let layer_shell = LayerShell::bind(&globals, &qh)
        .expect("This compositor doesn't support wlr-layer-shell (needed to dock the bar).");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");
    let screencopy_manager = globals
        .bind::<ZwlrScreencopyManagerV1, Magnifier, ()>(&qh, 1..=3, ())
        .expect("This compositor doesn't support wlr-screencopy (needed to capture the screen).");

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some("hypr-xp-magnifier"),
        None,
    );

    layer.set_anchor(anchor_for(position));
    if is_horizontal(position) {
        layer.set_size(0, thickness);
    } else {
        layer.set_size(thickness, 0);
    }
    // Reserve this much space so other windows get pushed out of the way.
    layer.set_exclusive_zone(thickness as i32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    layer.commit();

    let pool_bytes = MAX_ASSUMED_WIDTH * MAX_ASSUMED_HEIGHT * 4;
    let display_pool =
        SlotPool::new(pool_bytes, &shm).expect("Failed to create display buffer pool");
    let capture_pool =
        SlotPool::new(pool_bytes, &shm).expect("Failed to create capture buffer pool");

    let follow_keypress_flag = Arc::new(Mutex::new(initial_settings.follow_keypress));
    let focus_rect = focus_tracker::spawn(Arc::clone(&follow_keypress_flag));
    let (cursor_pos, settings) =
        spawn_background_tracker(initial_settings, follow_keypress_flag);

    let mut app = Magnifier {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,

        exit: false,
        first_configure: true,
        display_pool,
        width: 0,
        height: 0,
        layer,
        keyboard: None,
        keyboard_focus: false,
        pointer: None,

        screencopy_manager,
        capture_pool,
        capture_buffer: None,
        output: None,
        cursor_pos,
        focus_rect,
        settings,
        capture_in_flight: false,
        latest_capture: None,
        last_requested_thickness: thickness,
        last_requested_position: position,
        last_capture_request: Instant::now(),

        frame_count: 0,
        fps_timer: Instant::now(),
    };

    println!("hypr-xp-magnifier (phase 4: follow keyboard focus)");
    println!("Settings file: {}", config_path().display());
    println!("Adjust it live with: cargo run --release --bin magnifier-settings");
    println!("A magnified live view of the area around your cursor should appear docked to your screen.");
    println!("Frame rate will be printed here every {} seconds.", FPS_REPORT_INTERVAL.as_secs());
    println!("To quit: click the bar once, then press Escape — or Ctrl+C here.");

    loop {
        event_queue.blocking_dispatch(&mut app).unwrap();
        if app.exit {
            println!("Exiting.");
            break;
        }
    }
}

/// Spawns a background thread that continuously polls Hyprland's own IPC
/// socket for the global cursor position (Wayland deliberately hides this
/// from ordinary clients, so on Hyprland we go through its own socket
/// instead of a normal Wayland input event), and periodically reloads the
/// settings file so the GUI's changes take effect without a restart.
fn spawn_background_tracker(
    initial_settings: Settings,
    follow_keypress_flag: Arc<Mutex<bool>>,
) -> (Arc<Mutex<(i32, i32, Instant)>>, Arc<Mutex<Settings>>) {
    let cursor_pos = Arc::new(Mutex::new((0, 0, Instant::now())));
    let settings = Arc::new(Mutex::new(initial_settings));

    let cursor_writer = Arc::clone(&cursor_pos);
    let settings_writer = Arc::clone(&settings);

    thread::spawn(move || {
        let socket_path = hyprland_socket_path();
        if socket_path.is_none() {
            eprintln!(
                "Warning: couldn't find Hyprland's IPC socket (is HYPRLAND_INSTANCE_SIGNATURE \
                 set?). Cursor tracking won't work outside a Hyprland session."
            );
        }

        let mut last_settings_check = Instant::now();
        let mut last_pos = (0, 0);

        loop {
            if let Some(path) = &socket_path {
                if let Some(pos) = query_cursor_pos(path) {
                    if pos != last_pos {
                        last_pos = pos;
                        if let Ok(mut guard) = cursor_writer.lock() {
                            *guard = (pos.0, pos.1, Instant::now());
                        }
                    }
                }
            }

            if last_settings_check.elapsed() >= SETTINGS_RELOAD_INTERVAL {
                let reloaded = load_settings();
                if let Ok(mut flag) = follow_keypress_flag.lock() {
                    *flag = reloaded.follow_keypress;
                }
                if let Ok(mut guard) = settings_writer.lock() {
                    *guard = reloaded;
                }
                last_settings_check = Instant::now();
            }

            thread::sleep(CURSOR_POLL_INTERVAL);
        }
    });

    (cursor_pos, settings)
}

pub(crate) fn hyprland_socket_path() -> Option<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    Some(PathBuf::from(format!(
        "{runtime_dir}/hypr/{signature}/.socket.sock"
    )))
}

fn query_cursor_pos(socket_path: &std::path::Path) -> Option<(i32, i32)> {
    // Hyprland's own docs ask that each connection be opened right before
    // use and closed right after, so we open a fresh one on every poll
    // rather than keeping one connection open.
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.write_all(b"cursorpos").ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok();
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    parse_cursor_pos(&response)
}

fn parse_cursor_pos(response: &str) -> Option<(i32, i32)> {
    let mut parts = response.trim().splitn(2, ',');
    let x: i32 = parts.next()?.trim().parse().ok()?;
    let y: i32 = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// A capture buffer we keep around and reuse for every capture, instead of
/// allocating a fresh one each frame. Only replaced if the compositor asks
/// for a different size/format than what we already have.
struct ReusableCaptureBuffer {
    buffer: Buffer,
    width: i32,
    height: i32,
    stride: i32,
    format: wl_shm::Format,
}

/// A completed capture, with its pixel data copied out into a plain `Vec`
/// so it keeps rendering smoothly even while the next capture is still in
/// flight (rather than flashing back to a placeholder between frames).
struct CapturedFrame {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    stride: i32,
}

struct Magnifier {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,

    exit: bool,
    first_configure: bool,
    display_pool: SlotPool,
    width: u32,
    height: u32,
    layer: LayerSurface,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: bool,
    pointer: Option<wl_pointer::WlPointer>,

    screencopy_manager: ZwlrScreencopyManagerV1,
    capture_pool: SlotPool,
    capture_buffer: Option<ReusableCaptureBuffer>,
    output: Option<wl_output::WlOutput>,
    cursor_pos: Arc<Mutex<(i32, i32, Instant)>>,
    focus_rect: Arc<Mutex<Option<(FocusRect, Instant)>>>,
    settings: Arc<Mutex<Settings>>,
    capture_in_flight: bool,
    latest_capture: Option<CapturedFrame>,
    last_requested_thickness: u32,
    last_requested_position: DockPosition,
    last_capture_request: Instant,

    frame_count: u32,
    fps_timer: Instant,
}

impl CompositorHandler for Magnifier {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.apply_pending_layout();
        self.maybe_request_capture(qh);
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        // This tells us, authoritatively, which output the bar itself is
        // actually displayed on — better than guessing, and needed so the
        // capture region and the "point is over the bar" check both agree
        // on the same monitor.
        if self.layer.wl_surface() == surface {
            self.output = Some(output.clone());
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Magnifier {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Magnifier {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = configure.new_size.0.max(1);
        self.height = configure.new_size.1.max(1);

        if self.first_configure {
            self.first_configure = false;
            self.draw(qh);
        }
    }
}

impl SeatHandler for Magnifier {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = Some(
                self.seat_state
                    .get_keyboard(qh, &seat, None)
                    .expect("Failed to create keyboard"),
            );
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = Some(
                self.seat_state
                    .get_pointer(qh, &seat)
                    .expect("Failed to create pointer"),
            );
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Magnifier {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = true;
        }
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = false;
        }
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape {
            self.exit = true;
        }
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for Magnifier {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            // Drag-to-resize logic hooks in here in a later version.
            let _ = event;
        }
    }
}

impl ShmHandler for Magnifier {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Magnifier {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrScreencopyManagerV1,
        _event: zwlr_screencopy_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The manager object itself doesn't send any events.
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Magnifier {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => match format {
                WEnum::Value(format) => {
                    let (w, h, s) = (width as i32, height as i32, stride as i32);
                    let needs_new = match &state.capture_buffer {
                        Some(existing) => {
                            existing.width != w
                                || existing.height != h
                                || existing.stride != s
                                || existing.format != format
                        }
                        None => true,
                    };

                    if needs_new {
                        match state.capture_pool.create_buffer(w, h, s, format) {
                            Ok((buffer, _canvas)) => {
                                state.capture_buffer = Some(ReusableCaptureBuffer {
                                    buffer,
                                    width: w,
                                    height: h,
                                    stride: s,
                                    format,
                                });
                            }
                            Err(err) => {
                                eprintln!("Failed to allocate capture buffer: {err}");
                                frame.destroy();
                                state.capture_in_flight = false;
                                return;
                            }
                        }
                    }

                    if let Some(cache) = &state.capture_buffer {
                        frame.copy(cache.buffer.wl_buffer());
                    }
                }
                WEnum::Unknown(_) => {
                    eprintln!("Compositor offered an unrecognized pixel format for capture.");
                    frame.destroy();
                    state.capture_in_flight = false;
                }
            },
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                if let Some(cache) = &state.capture_buffer {
                    if let Some(src) = cache.buffer.canvas(&mut state.capture_pool) {
                        state.latest_capture = Some(CapturedFrame {
                            pixels: src.to_vec(),
                            width: cache.width,
                            height: cache.height,
                            stride: cache.stride,
                        });
                    }
                }
                frame.destroy();
                state.capture_in_flight = false;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.destroy();
                state.capture_in_flight = false;
            }
            _ => {}
        }
    }
}

impl Magnifier {
    fn current_settings(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// The point we should currently be magnifying, in global compositor
    /// coordinates: the mouse cursor, or — if "follow keyboard focus" is on
    /// and the focused element changed more recently than the mouse last
    /// moved — the center of the focused element instead.
    fn aim_point(&self, follow_keypress: bool, focus_y_correction: i32) -> (i32, i32) {
        let (mouse_x, mouse_y, mouse_t) =
            self.cursor_pos.lock().map(|g| *g).unwrap_or((0, 0, Instant::now()));

        if follow_keypress {
            if let Ok(guard) = self.focus_rect.lock() {
                if let Some((rect, focus_t)) = *guard {
                    if focus_t > mouse_t {
                        let center_y = rect.y + rect.height / 2 + focus_y_correction;
                        return (rect.x + rect.width / 2, center_y);
                    }
                }
            }
        }

        (mouse_x, mouse_y)
    }

    /// If the settings file's thickness or position differ from what we
    /// last applied, request the change. This is what makes both genuinely
    /// live-adjustable from the GUI rather than startup-only.
    fn apply_pending_layout(&mut self) {
        let settings = self.current_settings();
        let changed = settings.thickness != self.last_requested_thickness
            || settings.position != self.last_requested_position;
        if changed && settings.thickness > 0 {
            self.layer.set_anchor(anchor_for(settings.position));
            if is_horizontal(settings.position) {
                self.layer.set_size(0, settings.thickness);
            } else {
                self.layer.set_size(settings.thickness, 0);
            }
            self.layer.set_exclusive_zone(settings.thickness as i32);
            self.layer.commit();
            self.last_requested_thickness = settings.thickness;
            self.last_requested_position = settings.position;
        }
    }

    /// Where this output sits in the compositor's global layout (top-left
    /// corner, in the same coordinate space Hyprland reports the cursor in).
    fn output_offset(&self, output: &wl_output::WlOutput) -> (i32, i32) {
        self.output_state
            .info(output)
            .map(|info| info.logical_position.unwrap_or(info.location))
            .unwrap_or((0, 0))
    }

    /// This output's own size in pixels.
    fn output_size(&self, output: &wl_output::WlOutput) -> (i32, i32) {
        self.output_state
            .info(output)
            .map(|info| {
                info.logical_size.unwrap_or_else(|| {
                    info.modes
                        .iter()
                        .find(|m| m.current)
                        .map(|m| m.dimensions)
                        .unwrap_or((MAX_ASSUMED_WIDTH as i32, MAX_ASSUMED_HEIGHT as i32))
                })
            })
            .unwrap_or((MAX_ASSUMED_WIDTH as i32, MAX_ASSUMED_HEIGHT as i32))
    }

    /// The bar's own on-screen rectangle (x, y, w, h), in output-local
    /// coordinates, given where it's docked. Only Bottom and Right actually
    /// sit away from the output's own (0,0) corner.
    fn bar_footprint(&self, output: &wl_output::WlOutput) -> (i32, i32, i32, i32) {
        let (out_w, out_h) = self.output_size(output);
        let t = self.last_requested_thickness as i32;
        match self.last_requested_position {
            DockPosition::Top => (0, 0, out_w, t),
            DockPosition::Bottom => (0, (out_h - t).max(0), out_w, t),
            DockPosition::Left => (0, 0, t, out_h),
            DockPosition::Right => ((out_w - t).max(0), 0, t, out_h),
        }
    }

    /// True if the given global-coordinate point currently falls inside the
    /// bar's own on-screen footprint. Capturing there would just show the
    /// bar magnifying itself — an infinite mirror — so we check this before
    /// ever asking the compositor for a capture.
    fn point_is_over_bar(&self, output: &wl_output::WlOutput, point: (i32, i32)) -> bool {
        let (off_x, off_y) = self.output_offset(output);
        let local_x = point.0 - off_x;
        let local_y = point.1 - off_y;
        let (bx, by, bw, bh) = self.bar_footprint(output);
        local_x >= bx && local_x < bx + bw && local_y >= by && local_y < by + bh
    }

    /// Works out which region of the screen to capture: a box centered on
    /// `point` (global coordinates — the mouse cursor, or a focused
    /// element's center), sized so that scaling it up to fill the dock
    /// strip gives the configured zoom level.
    ///
    /// `point` is in *global* compositor coordinates (spanning every
    /// monitor together), but `capture_output_region` wants coordinates
    /// *local* to the specific output we're capturing from. We convert
    /// using that output's own position in the layout, then clamp the box
    /// so it always stays fully inside that output's own bounds — this is
    /// what stops the view from bleeding into a second monitor. Once the
    /// point leaves this monitor, the view just stays pinned to the
    /// nearest edge of this monitor. This works the same regardless of
    /// which edge the bar itself is docked to, since it only cares about
    /// the bar's own (already orientation-correct) width/height.
    fn compute_capture_region(
        &self,
        output: &wl_output::WlOutput,
        zoom_factor: f64,
        point: (i32, i32),
    ) -> (i32, i32, i32, i32) {
        let zoom_factor = zoom_factor.max(0.1);
        let (off_x, off_y) = self.output_offset(output);
        let (out_w, out_h) = self.output_size(output);

        let local_x = point.0 - off_x;
        let local_y = point.1 - off_y;

        let capture_width = ((self.width.max(1) as f64) / zoom_factor).round().max(1.0) as i32;
        let capture_height = ((self.height.max(1) as f64) / zoom_factor).round().max(1.0) as i32;
        let capture_width = capture_width.min(out_w.max(1));
        let capture_height = capture_height.min(out_h.max(1));

        let x = (local_x - capture_width / 2).clamp(0, (out_w - capture_width).max(0));
        let y = (local_y - capture_height / 2).clamp(0, (out_h - capture_height).max(0));

        (x, y, capture_width, capture_height)
    }

    /// Kicks off a new screen capture if the previous one has finished and
    /// enough time has passed to respect the configured refresh rate.
    fn maybe_request_capture(&mut self, qh: &QueueHandle<Self>) {
        if self.capture_in_flight {
            return;
        }

        let settings = self.current_settings();
        let min_interval = Duration::from_secs_f64(1.0 / settings.refresh_rate_hz.max(1.0));
        if self.last_capture_request.elapsed() < min_interval {
            return; // not time for a new capture yet; keep showing the last one
        }

        let output = match self.output.clone() {
            Some(o) => o,
            None => match self.output_state.outputs().next() {
                Some(o) => {
                    self.output = Some(o.clone());
                    o
                }
                None => return, // outputs not discovered yet; try again next frame
            },
        };

        let point = self.aim_point(settings.follow_keypress, settings.focus_y_correction);

        if self.point_is_over_bar(&output, point) {
            // Show the empty placeholder instead of a self-magnifying loop,
            // and don't bother asking the compositor for a capture at all.
            self.latest_capture = None;
            return;
        }

        let (x, y, w, h) = self.compute_capture_region(&output, settings.zoom_factor, point);
        let overlay_cursor = 1; // show the cursor icon in the magnified view, like XP did
        let _frame = self
            .screencopy_manager
            .capture_output_region(overlay_cursor, &output, x, y, w, h, qh, ());
        self.capture_in_flight = true;
        self.last_capture_request = Instant::now();
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let stride = width as i32 * 4;
        let sharpness = self.current_settings().sharpness;

        let (buffer, canvas) = self
            .display_pool
            .create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888)
            .expect(
                "create display buffer (if this panics, raise MAX_ASSUMED_WIDTH/HEIGHT at the top of the file)",
            );

        match &self.latest_capture {
            Some(capture) => {
                blit_scaled(
                    &capture.pixels,
                    capture.width,
                    capture.height,
                    capture.stride,
                    canvas,
                    width as i32,
                    height as i32,
                    stride,
                    sharpness,
                );
            }
            None => {
                // Nothing captured yet — dark placeholder so the bar is still visible.
                for pixel in canvas.chunks_exact_mut(4) {
                    let color: u32 = 0xFF14141c;
                    let array: &mut [u8; 4] = pixel.try_into().unwrap();
                    *array = color.to_le_bytes();
                }
            }
        }

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        self.layer
            .wl_surface()
            .frame(qh, FrameCallbackData(self.layer.wl_surface().clone()));
        buffer
            .attach_to(self.layer.wl_surface())
            .expect("buffer attach");
        self.layer.commit();

        self.frame_count += 1;
        let elapsed = self.fps_timer.elapsed();
        if elapsed >= FPS_REPORT_INTERVAL {
            let fps = self.frame_count as f64 / elapsed.as_secs_f64();
            eprintln!("~{fps:.1} fps");
            self.frame_count = 0;
            self.fps_timer = Instant::now();
        }
    }
}

/// Upscales `src` into `dst`, blending between nearest-neighbor (blocky but
/// cheap) and bilinear (smoother but costs more per pixel) based on
/// `sharpness` (1.0 = fully nearest, 0.0 = fully bilinear). At sharpness
/// 1.0 this does exactly what the original nearest-only version did, at the
/// same cost — the extra math only runs when sharpness is turned down.
fn blit_scaled(
    src: &[u8],
    src_w: i32,
    src_h: i32,
    src_stride: i32,
    dst: &mut [u8],
    dst_w: i32,
    dst_h: i32,
    dst_stride: i32,
    sharpness: f64,
) {
    if src_w <= 0 || src_h <= 0 || dst_w <= 0 || dst_h <= 0 {
        return;
    }
    let sharpness = sharpness.clamp(0.0, 1.0);
    let scale_x = src_w as f64 / dst_w as f64;
    let scale_y = src_h as f64 / dst_h as f64;

    for dst_y in 0..dst_h {
        let src_yf = (dst_y as f64 + 0.5) * scale_y - 0.5;
        for dst_x in 0..dst_w {
            let src_xf = (dst_x as f64 + 0.5) * scale_x - 0.5;

            let pixel = if sharpness >= 0.999 {
                sample_nearest(src, src_w, src_h, src_stride, src_xf, src_yf)
            } else {
                let nearest = sample_nearest(src, src_w, src_h, src_stride, src_xf, src_yf);
                let bilinear = sample_bilinear(src, src_w, src_h, src_stride, src_xf, src_yf);
                blend(nearest, bilinear, sharpness)
            };

            let dst_off = (dst_y * dst_stride + dst_x * 4) as usize;
            if dst_off + 4 <= dst.len() {
                dst[dst_off..dst_off + 4].copy_from_slice(&pixel);
            }
        }
    }
}

fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

fn read_pixel(src: &[u8], stride: i32, x: i32, y: i32) -> [u8; 4] {
    let off = (y * stride + x * 4) as usize;
    if off + 4 <= src.len() {
        [src[off], src[off + 1], src[off + 2], src[off + 3]]
    } else {
        [0, 0, 0, 0xFF]
    }
}

fn sample_nearest(src: &[u8], src_w: i32, src_h: i32, src_stride: i32, xf: f64, yf: f64) -> [u8; 4] {
    let x = clampi(xf.round() as i32, 0, src_w - 1);
    let y = clampi(yf.round() as i32, 0, src_h - 1);
    read_pixel(src, src_stride, x, y)
}

fn sample_bilinear(
    src: &[u8],
    src_w: i32,
    src_h: i32,
    src_stride: i32,
    xf: f64,
    yf: f64,
) -> [u8; 4] {
    let x0 = clampi(xf.floor() as i32, 0, src_w - 1);
    let y0 = clampi(yf.floor() as i32, 0, src_h - 1);
    let x1 = clampi(x0 + 1, 0, src_w - 1);
    let y1 = clampi(y0 + 1, 0, src_h - 1);
    let tx = (xf - x0 as f64).clamp(0.0, 1.0);
    let ty = (yf - y0 as f64).clamp(0.0, 1.0);

    let p00 = read_pixel(src, src_stride, x0, y0);
    let p10 = read_pixel(src, src_stride, x1, y0);
    let p01 = read_pixel(src, src_stride, x0, y1);
    let p11 = read_pixel(src, src_stride, x1, y1);

    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] as f64 * (1.0 - tx) + p10[c] as f64 * tx;
        let bottom = p01[c] as f64 * (1.0 - tx) + p11[c] as f64 * tx;
        out[c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
    }
    out
}

fn blend(sharp: [u8; 4], smooth: [u8; 4], sharpness: f64) -> [u8; 4] {
    let mut out = [0u8; 4];
    for c in 0..4 {
        out[c] = (sharp[c] as f64 * sharpness + smooth[c] as f64 * (1.0 - sharpness)).round() as u8;
    }
    out
}

delegate_registry!(Magnifier);

impl ProvidesRegistryState for Magnifier {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(Magnifier);
