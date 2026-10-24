//! An SDL2 window with GLES2 drawing, for machines without a window system.
//! Handheld firmware (muOS and the like) has no X11 or Wayland; its own SDL2
//! build knows how to reach the framebuffer and the Mali driver, so this
//! host links against that SDL2 and lets it open the screen. Controllers
//! come through SDL too, mapped by the firmware's game controller database.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::MouseButton;

use crate::actions::Action;
use crate::app::{App, Options, Shot};
use crate::backend::{Backend, Waker};
use crate::gamepad::{Gamepad, PadButton, Stick, Triggers, button_action};
use crate::images::CoverLoader;

pub struct Window {
    pub size: (f32, f32),
    pub fullscreen: bool,
    /// Log how often the loop woke and drew, and what asked for it.
    pub frame_stats: bool,
}

/// How often the frame stats are logged.
const STATS_EVERY: Duration = Duration::from_secs(5);

/// What the loop did since the last report, for finding what keeps it
/// awake on a battery.
struct FrameStats {
    since: Instant,
    /// Passes through the interface, drawn or not.
    passes: u32,
    /// Frames put on screen.
    drawn: u32,
    /// Looks at SDL's queue while waiting.
    polls: u32,
    /// Passes that had SDL input.
    input: u32,
    /// Who asked for each pass, by where the request was made.
    causes: std::collections::BTreeMap<String, u32>,
}

impl FrameStats {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            passes: 0,
            drawn: 0,
            polls: 0,
            input: 0,
            causes: Default::default(),
        }
    }

    fn report(&mut self) {
        if self.since.elapsed() < STATS_EVERY {
            return;
        }
        let secs = self.since.elapsed().as_secs_f32();
        let causes: Vec<String> = self
            .causes
            .iter()
            .map(|(cause, count)| format!("{cause} x{count}"))
            .collect();
        log::info!(
            "frames: {:.1}/s drawn, {:.1}/s passes, {:.1}/s polls, {} with input; asked by: {}",
            self.drawn as f32 / secs,
            self.passes as f32 / secs,
            self.polls as f32 / secs,
            self.input,
            if causes.is_empty() {
                "nothing".to_string()
            } else {
                causes.join(", ")
            },
        );
        *self = Self::new();
    }
}

/// SDL treats a trigger as an axis; past this it counts as pressed.
const TRIGGER_PRESSED: i16 = 16_000;
/// egui asks for a repaint "never" as a very long delay; keep the
/// arithmetic on it finite.
const LONGEST_WAIT: Duration = Duration::from_secs(3600);
/// How often the idle loop pumps SDL for input. SDL's own timed wait can
/// only block on drivers with a window system; on the framebuffer drivers
/// this host exists for it falls back to polling every millisecond, which
/// cost 5% of a core doing nothing. A frame's worth of latency on input
/// is not felt in a menu, and brings idle down to 0.6%.
const POLL: Duration = Duration::from_millis(16);
/// While a game has the screen its input is thrown away, so the pump only
/// has to keep SDL's queue from filling. The backend's wake ends the wait
/// when the game exits.
const HIDDEN_POLL: Duration = Duration::from_secs(1);

/// Dark enough not to flash, and unlike any tile the interface draws.
const SCRUB_COLOR: [f32; 4] = [0.02, 0.0, 0.03, 1.0];

/// Wakes the frame loop from another thread, between input polls.
#[derive(Default)]
struct Wake {
    flag: Mutex<bool>,
    ready: Condvar,
}

impl Wake {
    fn signal(&self) {
        *self.flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.ready.notify_one();
    }

    /// Sleeps until signalled or `timeout` passes. Whether it was signalled,
    /// now or since the last call.
    fn wait(&self, timeout: Duration) -> bool {
        let mut flag = self.flag.lock().unwrap_or_else(|p| p.into_inner());
        if !*flag {
            flag = self
                .ready
                .wait_timeout(flag, timeout)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        std::mem::take(&mut *flag)
    }
}

/// Runs the interface until the window closes.
pub fn run(
    backend: Backend,
    covers: CoverLoader,
    waker: Waker,
    mut options: Options,
    shot: Option<Shot>,
    window: Window,
) -> Result<()> {
    let sdl = sdl2::init().map_err(|e| anyhow!("initialising SDL: {e}"))?;
    let video = sdl.video().map_err(|e| anyhow!("SDL video: {e}"))?;
    let controllers = sdl
        .game_controller()
        .map_err(|e| anyhow!("SDL controllers: {e}"))?;
    log::info!("SDL video driver: {}", video.current_video_driver());
    {
        let attr = video.gl_attr();
        attr.set_context_profile(sdl2::video::GLProfile::GLES);
        attr.set_context_version(2, 0);
        attr.set_double_buffer(true);
        attr.set_depth_size(0);
    }
    let mut builder = video.window("zitch", window.size.0 as u32, window.size.1 as u32);
    builder.opengl().position_centered();
    if window.fullscreen {
        builder.fullscreen_desktop();
    }
    let sdl_window = builder.build().context("opening the window")?;
    let gl_context = sdl_window
        .gl_create_context()
        .map_err(|e| anyhow!("creating a GLES2 context: {e}"))?;
    sdl_window
        .gl_make_current(&gl_context)
        .map_err(|e| anyhow!("making the GL context current: {e}"))?;
    if let Err(error) = video.gl_set_swap_interval(1) {
        log::warn!("no vsync: {error}");
    }
    let gl = unsafe {
        egui_glow::glow::Context::from_loader_function(|name| {
            gl_fix::wrap(name, video.gl_get_proc_address(name) as *const std::ffi::c_void)
        })
    };
    let mut painter = egui_glow::Painter::new(Arc::new(gl), "", None, false)
        .map_err(|e| anyhow!("setting up the egui painter: {e}"))?;
    video.text_input().start();

    let ctx = egui::Context::default();
    // The backend and the cover loader wake the frame loop through `wake`;
    // a delayed request is served by the loop's own timeout.
    let wake = Arc::new(Wake::default());
    let waker_wake = Arc::clone(&wake);
    ctx.set_request_repaint_callback(move |info| {
        if info.delay.is_zero() {
            waker_wake.signal();
        }
    });
    waker.attach(&ctx);

    let (gamepad, pad_tx) = Gamepad::external();
    options.gamepad = Some(gamepad);
    // One screen, one program: a game takes it whole.
    options.handoff = true;
    let mut app = App::new(backend, covers, &ctx, options, shot);

    let mut pads = Pads::new();
    // Keypad buttons are there from the start; no controller announces them.
    if keypad_enabled() {
        let _ = pad_tx.connected();
    }
    let mut event_pump = sdl
        .event_pump()
        .map_err(|e| anyhow!("SDL event pump: {e}"))?;
    let mut input = Input::default();
    let started = Instant::now();
    let mut wait = Duration::ZERO;
    let mut shots: Vec<egui::UserData> = Vec::new();
    // Set while a game has the screen: the interface keeps running but
    // draws nothing and lets the controller through to the game. Minimize
    // is the closest thing egui has for a window that owns the screen.
    let mut hidden = false;
    // Set when the window comes back from behind a game. The game drew to
    // the same screen memory, and Mali's transaction elimination skips
    // tiles it thinks are unchanged since its last frame there, so the
    // first frame back only lands where the interface itself changed.
    // Two frames of a colour nothing else uses, one per buffer, make every
    // tile different from what the GPU remembers.
    let mut scrub = false;
    let mut stats = window.frame_stats.then(FrameStats::new);

    'frames: loop {
        let deadline = [
            pads.stick.deadline(),
            pads.dpad.deadline(),
            pads.triggers.deadline(),
        ]
        .into_iter()
        .flatten()
        .min();
        if let Some(deadline) = deadline {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        let due = Instant::now() + wait.min(LONGEST_WAIT);
        let mut sdl_events: Vec<Event> = event_pump.poll_iter().collect();
        while sdl_events.is_empty() {
            let now = Instant::now();
            let poll = if hidden { HIDDEN_POLL } else { POLL };
            if now >= due || wake.wait(poll.min(due - now)) {
                break;
            }
            sdl_events = event_pump.poll_iter().collect();
            if let Some(stats) = &mut stats {
                stats.polls += 1;
                stats.report();
            }
        }
        if let Some(stats) = &mut stats {
            stats.passes += 1;
            stats.input += u32::from(!sdl_events.is_empty());
        }
        let (w, _) = sdl_window.size();
        let (dw, dh) = sdl_window.drawable_size();
        let native_ppp = if w > 0 { dw as f32 / w as f32 } else { 1.0 };
        // A turned screen is laid out upright; gl_fix turns the frame.
        let (pw, ph) = if gl_fix::quarter_turn() { (dh, dw) } else { (dw, dh) };
        let ppp = ctx.pixels_per_point();
        // SDL reports the pointer in window units; egui wants points.
        let to_points = native_ppp / ppp;
        for event in sdl_events {
            match event {
                Event::Quit { .. } => break 'frames,
                Event::ControllerDeviceAdded { which, .. } => {
                    if pads.open(&controllers, which) {
                        let _ = pad_tx.connected();
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => pads.close(which),
                _ if hidden => {}
                Event::ControllerButtonDown { button, .. } => {
                    if let Some(action) = pads.press(button) {
                        let _ = pad_tx.send(action);
                    }
                }
                Event::ControllerButtonUp { button, .. } => pads.release(button),
                Event::ControllerAxisMotion { axis, value, .. } => {
                    if let Some(action) = pads.axis(axis, value) {
                        let _ = pad_tx.send(action);
                    }
                }
                Event::KeyDown {
                    keycode: Some(key),
                    repeat,
                    ..
                } if keypad(key).is_some() => {
                    if !repeat
                        && let Some(action) = keypad(key).and_then(|k| pads.key(k, true))
                    {
                        let _ = pad_tx.send(action);
                    }
                }
                Event::KeyUp {
                    keycode: Some(key), ..
                } if keypad(key).is_some() => {
                    if let Some(action) = keypad(key).and_then(|k| pads.key(k, false)) {
                        let _ = pad_tx.send(action);
                    }
                }
                other => input.translate(other, to_points),
            }
        }
        for direction in pads.repeat() {
            if !hidden {
                let _ = pad_tx.send(Action::MoveFocus(direction));
            }
        }

        let screen =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pw as f32, ph as f32) / ppp);
        let viewport = egui::ViewportInfo {
            native_pixels_per_point: Some(native_ppp),
            inner_rect: Some(screen),
            outer_rect: Some(screen),
            focused: Some(true),
            fullscreen: Some(window.fullscreen),
            ..Default::default()
        };
        let raw = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            viewports: std::iter::once((egui::ViewportId::ROOT, viewport)).collect(),
            screen_rect: Some(screen),
            max_texture_side: Some(painter.max_texture_side()),
            time: Some(started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut input.events),
            focused: true,
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            app.update_logic(ui.ctx());
            app.update_ui(ui);
        });
        if let Some(stats) = &mut stats {
            for cause in ctx.repaint_causes() {
                let file = cause.file.rsplit('/').next().unwrap_or(cause.file);
                *stats
                    .causes
                    .entry(format!("{file}:{}", cause.line))
                    .or_default() += 1;
            }
            stats.report();
        }
        let egui::FullOutput {
            platform_output,
            mut textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = output;
        wait = Duration::MAX;
        let mut close = false;
        if let Some(root) = viewport_output.get(&egui::ViewportId::ROOT) {
            wait = root.repaint_delay;
            for command in &root.commands {
                match command {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::Screenshot(data) => shots.push(data.clone()),
                    egui::ViewportCommand::Minimized(minimized) => {
                        if hidden && !*minimized {
                            scrub = true;
                            wait = Duration::ZERO;
                            // Input queued up to now was for the game, such
                            // as the press that quit it; the queue is only
                            // read once a second while hidden.
                            let stale = event_pump.poll_iter().count();
                            if stale > 0 {
                                log::debug!("dropped {stale} events queued while hidden");
                            }
                        }
                        hidden = *minimized;
                    }
                    // Focus and the rest mean nothing on a single-window
                    // screen.
                    _ => {}
                }
            }
        }
        // spruce runs the game after we exit; see muos::spruce.
        if crate::muos::exit_requested() {
            close = true;
        }
        for command in platform_output.commands {
            if let egui::OutputCommand::OpenUrl(url) = command
                && let Err(error) = open::that_detached(&url.url)
            {
                log::warn!("opening {}: {error}", url.url);
            }
        }
        if hidden && !close {
            // Nothing is drawn, but egui's textures move on (the font
            // atlas grows, covers arrive) and the painter has to follow or
            // the next visible frame draws with stale ones.
            for (id, deltas) in &textures_delta.set {
                for delta in deltas {
                    painter.set_texture(*id, delta);
                }
            }
            for id in &textures_delta.free {
                painter.free_texture(*id);
            }
            continue;
        }
        let primitives = ctx.tessellate(shapes, pixels_per_point);
        if std::mem::take(&mut scrub) {
            for _ in 0..2 {
                painter.clear([pw, ph], SCRUB_COLOR);
                sdl_window.gl_swap_window();
            }
        }
        painter.clear([pw, ph], [0.0, 0.0, 0.0, 1.0]);
        painter.paint_and_update_textures(
            [pw, ph],
            pixels_per_point,
            &primitives,
            &mut textures_delta,
        );
        for data in shots.drain(..) {
            let image = painter.read_screen_rgba([pw, ph]);
            input.events.push(egui::Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data: data,
                image: Arc::new(image),
            });
            wait = Duration::ZERO;
        }
        sdl_window.gl_swap_window();
        if let Some(stats) = &mut stats {
            stats.drawn += 1;
        }
        if close {
            break;
        }
    }
    app.on_close();
    painter.destroy();
    Ok(())
}

/// A handheld button that its SDL sends as a key.
#[derive(Clone, Copy)]
enum Keypad {
    Button(Button),
    Trigger(Axis),
}

fn keypad_enabled() -> bool {
    static KEYPAD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *KEYPAD.get_or_init(|| std::env::var_os("ZITCH_KEYPAD").is_some_and(|v| v == "miyoo"))
}

/// The pad button a key stands for where the buttons arrive only as keys,
/// as the Miyoo Mini's mmiyoo SDL2 sends them (ZITCH_KEYPAD=miyoo). SDL's
/// buttons are by position; the face buttons are labelled the Nintendo way.
fn keypad(key: Keycode) -> Option<Keypad> {
    if !keypad_enabled() {
        return None;
    }
    let button = match key {
        Keycode::Up => Button::DPadUp,
        Keycode::Down => Button::DPadDown,
        Keycode::Left => Button::DPadLeft,
        Keycode::Right => Button::DPadRight,
        Keycode::Space => Button::B,
        Keycode::LCtrl => Button::A,
        Keycode::LShift => Button::Y,
        Keycode::LAlt => Button::X,
        Keycode::E => Button::LeftShoulder,
        Keycode::T => Button::RightShoulder,
        Keycode::RCtrl => Button::Back,
        Keycode::Return => Button::Start,
        Keycode::Escape => Button::Guide,
        Keycode::Tab => return Some(Keypad::Trigger(Axis::TriggerLeft)),
        Keycode::Backspace => return Some(Keypad::Trigger(Axis::TriggerRight)),
        _ => return None,
    };
    Some(Keypad::Button(button))
}

/// Whether to drop keyboard events: where the buttons also arrive as keys
/// (the A30's), each press would act twice (ZITCH_IGNORE_KEYBOARD=1).
fn ignore_keyboard() -> bool {
    static IGNORE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IGNORE.get_or_init(|| std::env::var_os("ZITCH_IGNORE_KEYBOARD").is_some_and(|v| v == "1"))
}

/// Keyboard and mouse state between frames, as egui events.
#[derive(Default)]
struct Input {
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    pointer: egui::Pos2,
}

impl Input {
    fn translate(&mut self, event: Event, to_points: f32) {
        let keyboard =
            matches!(event, Event::KeyDown { .. } | Event::KeyUp { .. } | Event::TextInput { .. });
        if keyboard && ignore_keyboard() {
            return;
        }
        match event {
            Event::KeyDown {
                keycode: Some(keycode),
                keymod,
                repeat,
                ..
            } => {
                self.modifiers = modifiers(keymod);
                if let Some(key) = key(keycode) {
                    self.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat,
                        modifiers: self.modifiers,
                    });
                }
            }
            Event::KeyUp {
                keycode: Some(keycode),
                keymod,
                ..
            } => {
                self.modifiers = modifiers(keymod);
                if let Some(key) = key(keycode) {
                    self.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                }
            }
            Event::TextInput { text, .. } => {
                if !text.is_empty() {
                    self.events.push(egui::Event::Text(text));
                }
            }
            Event::MouseMotion { x, y, .. } => {
                self.pointer = egui::pos2(x as f32, y as f32) * to_points;
                self.events.push(egui::Event::PointerMoved(self.pointer));
            }
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            }
            | Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } => {
                let pressed = matches!(event, Event::MouseButtonDown { .. });
                let button = match mouse_btn {
                    MouseButton::Left => egui::PointerButton::Primary,
                    MouseButton::Right => egui::PointerButton::Secondary,
                    MouseButton::Middle => egui::PointerButton::Middle,
                    MouseButton::X1 => egui::PointerButton::Extra1,
                    MouseButton::X2 => egui::PointerButton::Extra2,
                    MouseButton::Unknown => return,
                };
                self.pointer = egui::pos2(x as f32, y as f32) * to_points;
                self.events.push(egui::Event::PointerButton {
                    pos: self.pointer,
                    button,
                    pressed,
                    modifiers: self.modifiers,
                });
            }
            Event::MouseWheel {
                precise_x,
                precise_y,
                ..
            } => self.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(precise_x, precise_y),
                phase: egui::TouchPhase::Move,
                modifiers: self.modifiers,
            }),
            Event::Window {
                win_event: WindowEvent::Leave,
                ..
            } => self.events.push(egui::Event::PointerGone),
            _ => {}
        }
    }
}

fn modifiers(keymod: Mod) -> egui::Modifiers {
    let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    egui::Modifiers {
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
        ctrl,
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        mac_cmd: false,
        command: ctrl,
    }
}

fn key(keycode: Keycode) -> Option<egui::Key> {
    use egui::Key;
    Some(match keycode {
        Keycode::Up => Key::ArrowUp,
        Keycode::Down => Key::ArrowDown,
        Keycode::Left => Key::ArrowLeft,
        Keycode::Right => Key::ArrowRight,
        Keycode::Return | Keycode::KpEnter => Key::Enter,
        Keycode::Escape => Key::Escape,
        Keycode::Tab => Key::Tab,
        Keycode::Backspace => Key::Backspace,
        Keycode::Delete => Key::Delete,
        Keycode::Space => Key::Space,
        Keycode::Home => Key::Home,
        Keycode::End => Key::End,
        Keycode::PageUp => Key::PageUp,
        Keycode::PageDown => Key::PageDown,
        // Letters and digits share egui's names.
        other => Key::from_name(&other.name())?,
    })
}

fn nintendo_buttons() -> bool {
    static NINTENDO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NINTENDO.get_or_init(|| std::env::var_os("ZITCH_NINTENDO_BUTTONS").is_some_and(|v| v == "1"))
}

/// Open controllers and their held state. Buttons that mean a direction
/// repeat while held, the same as a stick.
struct Pads {
    open: Vec<GameController>,
    stick: Stick,
    dpad: Stick,
    stick_pos: (f32, f32),
    dpad_pos: (f32, f32),
    left_trigger: bool,
    right_trigger: bool,
    triggers: Triggers,
}

impl Pads {
    /// SDL announces the controllers already attached with device-added
    /// events at startup, so they are all opened from the event loop.
    fn new() -> Self {
        Self {
            open: Vec::new(),
            stick: Stick::default(),
            dpad: Stick::default(),
            stick_pos: (0.0, 0.0),
            dpad_pos: (0.0, 0.0),
            left_trigger: false,
            right_trigger: false,
            triggers: Triggers::default(),
        }
    }

    /// Opens a newly seen controller; true when it is one and is new.
    fn open(&mut self, controllers: &sdl2::GameControllerSubsystem, index: u32) -> bool {
        if !controllers.is_game_controller(index) {
            log::info!("joystick {index} has no controller mapping");
            return false;
        }
        match controllers.open(index) {
            Ok(pad)
                if self
                    .open
                    .iter()
                    .any(|p| p.instance_id() == pad.instance_id()) =>
            {
                false
            }
            Ok(pad) => {
                log::info!("gamepad: {}", pad.name());
                self.open.push(pad);
                true
            }
            Err(error) => {
                log::warn!("opening controller {index}: {error}");
                false
            }
        }
    }

    fn close(&mut self, id: u32) {
        self.open.retain(|pad| pad.instance_id() != id);
        log::info!("gamepad disconnected");
    }

    fn press(&mut self, button: Button) -> Option<Action> {
        let pad = match button {
            Button::DPadUp => return self.dpad_set(0.0, 1.0),
            Button::DPadDown => return self.dpad_set(0.0, -1.0),
            Button::DPadLeft => return self.dpad_set(-1.0, 0.0),
            Button::DPadRight => return self.dpad_set(1.0, 0.0),
            // Nintendo-labelled pads (spruce sets this): the A label is
            // SDL's B, the Y label SDL's X, and the hints name labels.
            Button::B if nintendo_buttons() => PadButton::South,
            Button::A if nintendo_buttons() => PadButton::East,
            Button::X if nintendo_buttons() => PadButton::North,
            Button::A => PadButton::South,
            Button::B => PadButton::East,
            Button::Y => PadButton::North,
            Button::LeftShoulder => PadButton::LeftBumper,
            Button::RightShoulder => PadButton::RightBumper,
            Button::Guide => PadButton::Guide,
            Button::Start => PadButton::Start,
            _ => return None,
        };
        button_action(pad)
    }

    /// A keypad button, pressed or let go.
    fn key(&mut self, key: Keypad, down: bool) -> Option<Action> {
        match key {
            Keypad::Button(button) if down => self.press(button),
            Keypad::Button(button) => {
                self.release(button);
                None
            }
            Keypad::Trigger(axis) => self.axis(axis, if down { i16::MAX } else { 0 }),
        }
    }

    fn release(&mut self, button: Button) {
        if matches!(
            button,
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight
        ) {
            self.dpad_set(0.0, 0.0);
        }
    }

    fn dpad_set(&mut self, x: f32, y: f32) -> Option<Action> {
        self.dpad_pos = (x, y);
        self.dpad.update(x, y).map(Action::MoveFocus)
    }

    fn axis(&mut self, axis: Axis, value: i16) -> Option<Action> {
        let unit = value as f32 / i16::MAX as f32;
        match axis {
            Axis::LeftX => self.stick_pos.0 = unit,
            // SDL's y grows downward; the stick logic wants up positive.
            Axis::LeftY => self.stick_pos.1 = -unit,
            Axis::TriggerLeft | Axis::TriggerRight => {
                let held = if axis == Axis::TriggerLeft {
                    &mut self.left_trigger
                } else {
                    &mut self.right_trigger
                };
                let pressed = value > TRIGGER_PRESSED;
                if pressed == *held {
                    return None;
                }
                *held = pressed;
                return self
                    .triggers
                    .update(self.left_trigger, self.right_trigger)
                    .map(Action::MoveFocus);
            }
            _ => return None,
        }
        self.stick
            .update(self.stick_pos.0, self.stick_pos.1)
            .map(Action::MoveFocus)
    }

    /// Repeats due now for anything still held.
    fn repeat(&mut self) -> Vec<crate::actions::Direction> {
        let mut moves = Vec::new();
        if self.stick.deadline().is_some() {
            moves.extend(self.stick.update(self.stick_pos.0, self.stick_pos.1));
        }
        if self.dpad.deadline().is_some() {
            moves.extend(self.dpad.update(self.dpad_pos.0, self.dpad_pos.1));
        }
        if self.triggers.deadline().is_some() {
            moves.extend(self.triggers.update(self.left_trigger, self.right_trigger));
        }
        moves
    }
}

/// What egui asks of GLES2 that the A30's Mali-400 driver lacks, fixed on
/// the way to the driver, and the frame turned for a panel mounted on its side
/// (ZITCH_SCREEN_ROTATION, degrees clockwise as spruce's DISPLAY_ROTATION).
/// zitch's only GL user is egui.
mod gl_fix {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    type ShaderSource = unsafe extern "C" fn(u32, c_int, *const *const c_char, *const c_int);
    type BufferData = unsafe extern "C" fn(u32, isize, *const c_void, u32);
    type DrawElements = unsafe extern "C" fn(u32, c_int, u32, *const c_void);
    type Rect = unsafe extern "C" fn(c_int, c_int, c_int, c_int);

    static SHADER_SOURCE: OnceLock<ShaderSource> = OnceLock::new();
    static BUFFER_DATA: OnceLock<BufferData> = OnceLock::new();
    static DRAW_ELEMENTS: OnceLock<DrawElements> = OnceLock::new();
    static VIEWPORT: OnceLock<Rect> = OnceLock::new();
    static SCISSOR: OnceLock<Rect> = OnceLock::new();
    /// The upright frame's size, from egui's full-screen viewport.
    static FRAME_W: AtomicI32 = AtomicI32::new(0);
    static FRAME_H: AtomicI32 = AtomicI32::new(0);
    /// The element buffer last filled holds u16 indices egui sent as u32.
    static SHORT_INDICES: AtomicBool = AtomicBool::new(false);

    const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
    const UNSIGNED_SHORT: u32 = 0x1403;
    const UNSIGNED_INT: u32 = 0x1405;

    /// egui picks its precision with `#if defined(X) && X == 1`. The Mali-400's
    /// preprocessor evaluates `X == 1` even when X is undefined and fails the
    /// compile; `#if defined(X)` means the same where X is defined.
    const PRECISION_TEST: &str =
        "defined(GL_FRAGMENT_PRECISION_HIGH) && GL_FRAGMENT_PRECISION_HIGH == 1";

    fn rotation() -> u32 {
        static ROTATION: OnceLock<u32> = OnceLock::new();
        *ROTATION.get_or_init(|| {
            match std::env::var("ZITCH_SCREEN_ROTATION").ok().and_then(|r| r.parse().ok()) {
                Some(degrees @ (90 | 180 | 270)) => degrees,
                _ => 0,
            }
        })
    }

    /// Whether the frame is laid out with the screen's width and height swapped.
    pub fn quarter_turn() -> bool {
        matches!(rotation(), 90 | 270)
    }

    pub fn wrap(name: &str, real: *const c_void) -> *const c_void {
        if real.is_null() {
            return real;
        }
        unsafe {
            match name {
                "glShaderSource" => {
                    let real: ShaderSource = std::mem::transmute(real);
                    let _ = SHADER_SOURCE.set(real);
                    shader_source as *const c_void
                }
                "glBufferData" => {
                    let real: BufferData = std::mem::transmute(real);
                    let _ = BUFFER_DATA.set(real);
                    buffer_data as *const c_void
                }
                "glDrawElements" => {
                    let real: DrawElements = std::mem::transmute(real);
                    let _ = DRAW_ELEMENTS.set(real);
                    draw_elements as *const c_void
                }
                "glViewport" if rotation() != 0 => {
                    let real: Rect = std::mem::transmute(real);
                    let _ = VIEWPORT.set(real);
                    viewport as *const c_void
                }
                "glScissor" if rotation() != 0 => {
                    let real: Rect = std::mem::transmute(real);
                    let _ = SCISSOR.set(real);
                    scissor as *const c_void
                }
                _ => real,
            }
        }
    }

    unsafe extern "C" fn shader_source(
        shader: u32,
        count: c_int,
        strings: *const *const c_char,
        lengths: *const c_int,
    ) {
        let Some(real) = SHADER_SOURCE.get() else {
            return;
        };
        let mut source = Vec::new();
        for i in 0..count.max(0) as usize {
            let (string, length) = unsafe {
                let string = *strings.add(i);
                let length = if lengths.is_null() { -1 } else { *lengths.add(i) };
                (string, length)
            };
            let bytes = if length < 0 {
                unsafe { CStr::from_ptr(string) }.to_bytes()
            } else {
                unsafe { std::slice::from_raw_parts(string.cast::<u8>(), length as usize) }
            };
            source.extend_from_slice(bytes);
        }
        let mut fixed = String::from_utf8_lossy(&source)
            .replace(PRECISION_TEST, "defined(GL_FRAGMENT_PRECISION_HIGH)");
        if let Some(turned) = turned_position()
            && fixed.contains("gl_Position")
        {
            fixed = fixed.replacen("void main()", "void egui_main()", 1)
                + "\nvoid main() {\n    egui_main();\n    gl_Position = "
                + turned
                + ";\n}\n";
        }
        let fixed = CString::new(fixed).unwrap_or_default();
        let pointer = fixed.as_ptr();
        unsafe { real(shader, 1, &pointer, std::ptr::null()) };
    }

    /// GLES2 only draws u32 indices with OES_element_index_uint, which the
    /// Mali-400 does not have. egui's meshes stay far below 65536 vertices, so
    /// their indices go to the driver as u16.
    unsafe extern "C" fn buffer_data(target: u32, size: isize, data: *const c_void, usage: u32) {
        let Some(real) = BUFFER_DATA.get() else {
            return;
        };
        if target == ELEMENT_ARRAY_BUFFER {
            let narrowed = (!data.is_null() && size > 0 && size % 4 == 0)
                .then(|| unsafe {
                    std::slice::from_raw_parts(data.cast::<u32>(), size as usize / 4)
                })
                .and_then(|indices| {
                    indices.iter().map(|&i| u16::try_from(i).ok()).collect::<Option<Vec<u16>>>()
                });
            SHORT_INDICES.store(narrowed.is_some(), Ordering::Relaxed);
            if let Some(short) = narrowed {
                let bytes = (short.len() * 2) as isize;
                unsafe { real(target, bytes, short.as_ptr().cast(), usage) };
                return;
            }
        }
        unsafe { real(target, size, data, usage) };
    }

    unsafe extern "C" fn draw_elements(mode: u32, count: c_int, kind: u32, offset: *const c_void) {
        let Some(real) = DRAW_ELEMENTS.get() else {
            return;
        };
        if kind == UNSIGNED_INT && SHORT_INDICES.load(Ordering::Relaxed) {
            let offset = (offset as usize / 2) as *const c_void;
            unsafe { real(mode, count, UNSIGNED_SHORT, offset) };
        } else {
            unsafe { real(mode, count, kind, offset) };
        }
    }

    /// The vertex position turned clockwise by the rotation, in clip space.
    fn turned_position() -> Option<&'static str> {
        match rotation() {
            90 => Some("vec4(gl_Position.y, -gl_Position.x, gl_Position.zw)"),
            180 => Some("vec4(-gl_Position.xy, gl_Position.zw)"),
            270 => Some("vec4(-gl_Position.y, gl_Position.x, gl_Position.zw)"),
            _ => None,
        }
    }

    /// A rectangle of the upright frame, in GL's bottom-left pixels, as it
    /// lands in the turned framebuffer.
    fn turn_rect(x: c_int, y: c_int, w: c_int, h: c_int) -> (c_int, c_int, c_int, c_int) {
        let (fw, fh) = (FRAME_W.load(Ordering::Relaxed), FRAME_H.load(Ordering::Relaxed));
        match rotation() {
            90 => (y, fw - (x + w), h, w),
            180 => (fw - (x + w), fh - (y + h), w, h),
            270 => (fh - (y + h), x, h, w),
            _ => (x, y, w, h),
        }
    }

    unsafe extern "C" fn viewport(x: c_int, y: c_int, w: c_int, h: c_int) {
        let Some(real) = VIEWPORT.get() else {
            return;
        };
        if x == 0 && y == 0 {
            FRAME_W.store(w, Ordering::Relaxed);
            FRAME_H.store(h, Ordering::Relaxed);
        }
        let (x, y, w, h) = turn_rect(x, y, w, h);
        unsafe { real(x, y, w, h) };
    }

    unsafe extern "C" fn scissor(x: c_int, y: c_int, w: c_int, h: c_int) {
        let Some(real) = SCISSOR.get() else {
            return;
        };
        let (x, y, w, h) = turn_rect(x, y, w, h);
        unsafe { real(x, y, w, h) };
    }
}
