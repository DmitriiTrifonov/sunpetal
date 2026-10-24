//! Sunpetal: the window. A recompiled DS game's two screens on the GPU, its sound on the host's
//! device, played with the keyboard, a pad and the mouse, set up in a menu.
//!
//!     sunpetal-app ROM.nds [--out DIR] [--save FILE] [--mute] [--res N] [--menu]
//!                          [--dump-textures DIR] [--texture-pack DIR]
//!
//! F1 or Esc: the settings menu (video, sound, keys, profiles). Hotkeys: F2 changes the layout, F3
//! the filter, F4 and F5 the 3D's internal resolution (1-8 times the DS's), F11 full screen, Tab
//! (held) runs without waiting for the clock. The keys of the DS's buttons are set in the menu
//! (default: arrows = D-pad, Z = A, X = B, S = X, A = Y, Q = L, W = R, Enter = START, Backspace =
//! SELECT). A pad: the D-pad or left stick, the face buttons as the DS's (east A, south B, north
//! X, west Y), shoulders L and R, Start, Select. The mouse on the lower screen is the stylus. The
//! settings are kept in the configuration folder (`config::dir`) and saved when the menu closes
//! and at exit.
mod audio;
mod config;
mod effects;
mod emu;
mod gfx;
mod layout;
mod touchpad;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use sunpetal_runtime::nds::Config;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use config::Settings;
use emu::Shared;

fn bit(name: &str) -> u32 {
    sunpetal_runtime::nds::KEY_NAMES.iter().position(|n| *n == name).map(|i| 1 << i).unwrap_or(0)
}

/// The button bit a pad's button presses.
fn pad_button(b: gilrs::Button) -> u32 {
    use gilrs::Button::*;
    match b {
        East => bit("A"),
        South => bit("B"),
        North => bit("X"),
        West => bit("Y"),
        LeftTrigger | LeftTrigger2 => bit("L"),
        RightTrigger | RightTrigger2 => bit("R"),
        Start => bit("START"),
        Select => bit("SELECT"),
        DPadUp => bit("UP"),
        DPadDown => bit("DOWN"),
        DPadLeft => bit("LEFT"),
        DPadRight => bit("RIGHT"),
        _ => 0,
    }
}

struct App {
    shared: Arc<Shared>,
    ring: Option<Arc<audio::Ring>>,
    window: Option<Arc<Window>>,
    gfx: Option<gfx::Gfx>,
    layouts: Vec<layout::Layout>,
    settings: Settings,
    /// What was last put into effect (the menu compares).
    applied: Settings,
    menu: ui::Menu,
    /// Key code to button bit, from the settings.
    keymap: HashMap<KeyCode, u32>,
    mouse: (f32, f32),
    pen_down: bool,
    /// The finger that is the stylus.
    touch: Option<u64>,
    /// Fingers on the on-screen controls, by id.
    pad_touches: HashMap<u64, (f32, f32)>,
    /// The buttons those fingers hold.
    touch_held: Vec<&'static str>,
    /// Why the machine stopped, kept on screen on a phone (where there is no terminal).
    stopped_msg: Option<String>,
    /// Buttons held on the keyboard and on the pad (both go to the machine).
    kb: u32,
    pad: u32,
    pads: Option<gilrs::Gilrs>,
    win_size: (u32, u32),
}

impl App {
    fn layout(&self) -> &layout::Layout {
        self.layouts.iter().find(|l| l.name == self.settings.video.layout).unwrap_or(&self.layouts[0])
    }

    fn send_keys(&self) {
        let touch = self.touch_held.iter().fold(0, |a, n| a | bit(n));
        self.shared.input.lock().unwrap().keys = self.kb | self.pad | touch;
    }

    fn save(&mut self) {
        let k = self.window.as_ref().map_or(1.0, |w| w.scale_factor());
        self.settings.video.window = [(self.win_size.0 as f64 / k) as u32, (self.win_size.1 as f64 / k) as u32];
        config::save(&self.settings);
    }

    /// Puts the settings into effect (those that changed).
    fn apply(&mut self) {
        let s = self.settings.clone();
        let a = self.applied.clone();
        if s.video.res != a.video.res {
            self.shared.scale.store(s.video.res, Ordering::Relaxed);
        }
        if let Some(g) = &mut self.gfx {
            g.linear = s.video.linear;
        }
        if s.video.fullscreen != a.video.fullscreen {
            if let Some(w) = &self.window {
                w.set_fullscreen(s.video.fullscreen.then(|| Fullscreen::Borderless(None)));
            }
        }
        if let Some(r) = &self.ring {
            r.set_volume(if s.audio.mute { 0.0 } else { s.audio.volume });
        }
        if s.input != a.input {
            self.keymap = s
                .input
                .keys
                .iter()
                .filter_map(|(button, key)| Some((config::parse_key(key)?, bit(button))))
                .collect();
        }
        self.applied = s;
    }

    fn set_title(&self) {
        if let Some(w) = &self.window {
            let lang = config::Lang::resolve(&self.settings.interface.language);
            w.set_title(&format!("Sunpetal: {}, 3D {}x", ui::layout_label(self.layout().name, lang), self.settings.video.res));
        }
    }

    fn toggle_menu(&mut self) {
        self.menu.toggle();
        self.pen_down = false;
        self.shared.input.lock().unwrap().pen = None;
        if !self.menu.open {
            self.save();
        }
    }

    /// The pad's events since the last frame: its buttons held, the left stick as the D-pad.
    fn poll_pads(&mut self) {
        let threshold = self.settings.input.stick_threshold;
        let Some(g) = &mut self.pads else { return };
        let mut changed = false;
        while let Some(ev) = g.next_event() {
            use gilrs::EventType::*;
            match ev.event {
                ButtonPressed(b, _) => {
                    self.pad |= pad_button(b);
                    changed = true;
                }
                ButtonReleased(b, _) => {
                    self.pad &= !pad_button(b);
                    changed = true;
                }
                AxisChanged(a, v, _) => {
                    let (neg, pos) = match a {
                        gilrs::Axis::LeftStickX => (bit("LEFT"), bit("RIGHT")),
                        gilrs::Axis::LeftStickY => (bit("DOWN"), bit("UP")),
                        _ => continue,
                    };
                    self.pad &= !(neg | pos);
                    if v < -threshold {
                        self.pad |= neg;
                    } else if v > threshold {
                        self.pad |= pos;
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.send_keys();
        }
    }

    fn update_pen(&mut self) {
        let Some(g) = &self.gfx else { return };
        let pen = if self.pen_down && !self.menu.open { self.layout().touch(g.size(), self.mouse, true) } else { None };
        self.shared.input.lock().unwrap().pen = pen;
    }

    fn cycle_layout(&mut self) {
        let i = self.layouts.iter().position(|l| l.name == self.settings.video.layout).unwrap_or(0);
        self.settings.video.layout = self.layouts[(i + 1) % self.layouts.len()].name.to_string();
    }
}

impl ApplicationHandler for App {
    /// The app is no longer shown (on Android the window's surface goes with it): the picture's
    /// resources are let go, the machine waits; `resumed` makes them again.
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.save();
        self.gfx = None;
        self.window = None;
        self.touch = None;
        self.pen_down = false;
        self.pad_touches.clear();
        self.touch_held.clear();
        self.send_keys();
        self.shared.paused.store(true, Ordering::Relaxed);
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let [w, h] = self.settings.video.window;
        let attrs = Window::default_attributes().with_title("Sunpetal").with_inner_size(LogicalSize::new(w, h));
        let window = Arc::new(el.create_window(attrs).expect("a window"));
        match gfx::Gfx::new(window.clone()) {
            Ok(g) => self.gfx = Some(g),
            Err(e) => {
                eprintln!("{}", e);
                el.exit();
                return;
            }
        }
        self.win_size = (window.inner_size().width, window.inner_size().height);
        self.window = Some(window);
        self.shared.paused.store(self.menu.open && self.settings.emulation.pause_in_menu, Ordering::Relaxed);
        // everything that is not the first draw's business: the settings, in effect
        self.applied = Settings { video: config::Video { res: 0, fullscreen: !self.settings.video.fullscreen, ..self.settings.video.clone() }, ..self.settings.clone() };
        self.applied.input.keys.clear();
        self.apply();
        self.set_title();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        // the key a button is being set to: the next press, whatever it is (Esc cancels)
        if let WindowEvent::KeyboardInput { event, .. } = &ev {
            if event.state == ElementState::Pressed && !event.repeat {
                if let (Some(button), PhysicalKey::Code(code)) = (self.menu.rebinding.clone(), event.physical_key) {
                    self.menu.rebinding = None;
                    if code != KeyCode::Escape {
                        // a key belongs to one button
                        let name = format!("{:?}", code);
                        self.settings.input.keys.retain(|_, k| *k != name);
                        self.settings.input.keys.insert(button, name);
                        let defaults = config::Input::default().keys;
                        for (b, k) in defaults {
                            self.settings.input.keys.entry(b).or_insert(k);
                        }
                    }
                    return;
                }
            }
        }
        // the menu sees the events while it is open
        let mut consumed = false;
        if self.menu.open {
            if let Some(g) = &mut self.gfx {
                consumed = g.ui_event(&ev);
            }
        }
        match ev {
            WindowEvent::CloseRequested => {
                self.save();
                el.exit();
            }
            WindowEvent::Resized(s) => {
                if let Some(g) = &mut self.gfx {
                    g.resize(s.width, s.height);
                }
                if !self.settings.video.fullscreen && s.width > 0 && s.height > 0 {
                    self.win_size = (s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let down = event.state == ElementState::Pressed;
                // buttons are released whatever the menu is doing; pressed only in the game
                if let Some(&b) = self.keymap.get(&code) {
                    if !down {
                        self.kb &= !b;
                        self.send_keys();
                    } else if !self.menu.open {
                        self.kb |= b;
                        self.send_keys();
                    }
                    if !self.menu.open || !down {
                        return;
                    }
                }
                if consumed {
                    return;
                }
                if !down || event.repeat {
                    if code == KeyCode::Tab {
                        self.shared.fast.store(false, Ordering::Relaxed);
                    }
                    return;
                }
                match code {
                    KeyCode::F1 | KeyCode::Escape => self.toggle_menu(),
                    KeyCode::Tab => self.shared.fast.store(true, Ordering::Relaxed),
                    KeyCode::F2 => self.cycle_layout(),
                    KeyCode::F3 => self.settings.video.linear = !self.settings.video.linear,
                    KeyCode::F6 => self.settings.effects = self.settings.effects.next_preset(),
                    KeyCode::F4 => self.settings.video.res = self.settings.video.res.saturating_sub(1).max(1),
                    KeyCode::F5 => self.settings.video.res = (self.settings.video.res + 1).min(8),
                    KeyCode::F11 => self.settings.video.fullscreen = !self.settings.video.fullscreen,
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                self.update_pen();
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if !consumed => {
                self.pen_down = state == ElementState::Pressed;
                // a press must land on the screen; a drag may leave it
                if self.pen_down && !self.menu.open {
                    if let Some(g) = &self.gfx {
                        if self.layout().touch(g.size(), self.mouse, false).is_none() {
                            self.pen_down = false;
                        }
                    }
                }
                self.update_pen();
            }
            WindowEvent::Touch(t) => {
                use winit::event::TouchPhase::*;
                let p = (t.location.x as f32, t.location.y as f32);
                let band = self.gfx.as_ref().filter(|_| self.settings.touch_controls_on()).map(|g| {
                    let (w, h) = g.window_size();
                    touchpad::band(w, h)
                });
                let corner = self.gfx.as_ref().filter(|_| cfg!(target_os = "android") && !self.settings.touch_controls_on()).map(|g| g.window_size());
                match t.phase {
                    Started if !self.menu.open && !consumed && corner.map_or(false, |(w, h)| touchpad::on_menu_button(w, h, p)) => {
                        self.toggle_menu();
                    }
                    Started if !self.menu.open && !consumed && self.settings.emulation.fast_button && corner.map_or(false, |(w, h)| touchpad::on_fast_button(w, h, p)) => {
                        let on = !self.shared.fast.load(Ordering::Relaxed);
                        self.shared.fast.store(on, Ordering::Relaxed);
                    }
                    Started if !self.menu.open && !consumed => {
                        let on_screen = self.gfx.as_ref().map_or(false, |g| self.layout().touch(g.size(), p, false).is_some());
                        if on_screen && self.touch.is_none() {
                            self.touch = Some(t.id);
                            self.pen_down = true;
                            self.mouse = p;
                        } else if let Some(b) = band {
                            if touchpad::on_menu(b, p) {
                                self.toggle_menu();
                            } else {
                                self.pad_touches.insert(t.id, p);
                            }
                        }
                    }
                    Moved => {
                        if self.touch == Some(t.id) {
                            self.mouse = p;
                        } else if let Some(q) = self.pad_touches.get_mut(&t.id) {
                            *q = p;
                        }
                    }
                    Ended | Cancelled => {
                        if self.touch == Some(t.id) {
                            self.touch = None;
                            self.pen_down = false;
                        }
                        self.pad_touches.remove(&t.id);
                    }
                    _ => {}
                }
                self.touch_held = match band {
                    Some(b) if !self.pad_touches.is_empty() => touchpad::pressed(b, &self.pad_touches.values().copied().collect::<Vec<_>>()),
                    _ => Vec::new(),
                };
                self.send_keys();
                self.update_pen();
            }
            WindowEvent::RedrawRequested => {
                self.poll_pads();
                // the machine stopped (a failed boot, a fatal error): say so, do not sit black
                let stopped = self.shared.stopped.lock().unwrap().clone();
                if let Some(why) = stopped {
                    eprintln!("stopped: {}", why);
                    self.save();
                    if !self.settings.touch_controls_on() && !cfg!(target_os = "android") {
                        el.exit();
                        return;
                    }
                    // on a phone the window stays, with the reason
                    self.stopped_msg = Some(why);
                }
                let frame = self.shared.frame.lock().unwrap().take();
                if let (Some((scale, rgba)), Some(g)) = (frame, &mut self.gfx) {
                    g.upload(scale, &rgba);
                }
                let mut action = None;
                {
                    let App { gfx, menu, settings, layouts, touch_held, stopped_msg, shared, .. } = self;
                    if let Some(g) = gfx {
                        let layout = layouts.iter().find(|l| l.name == settings.video.layout).unwrap_or(&layouts[0]).clone();
                        let names: Vec<&str> = layouts.iter().map(|l| l.name).collect();
                        let fx = settings.effects.clone();
                        let internal = (256 * settings.video.res as u32, 192 * settings.video.res as u32);
                        let pad_on = settings.touch_controls_on();
                        let (ww, wh) = g.window_size();
                        let band = touchpad::band(ww, wh);
                        g.reserve = if pad_on { band.h } else { 0.0 };
                        let corner_on = cfg!(target_os = "android") && !pad_on;
                        if menu.open || pad_on || corner_on {
                            let held = touch_held.clone();
                            let mut f = |ctx: &egui::Context| {
                                if corner_on {
                                    let info = match stopped_msg {
                                        Some(why) => format!("stopped: {}", why),
                                        None if settings.video.show_stats => shared.status.lock().unwrap().clone(),
                                        None => String::new(),
                                    };
                                    let fast = settings.emulation.fast_button.then(|| shared.fast.load(Ordering::Relaxed));
                                    touchpad::paint_menu_button(ctx, ww, wh, &info, fast);
                                }
                                if pad_on {
                                    let info = match stopped_msg {
                                        Some(why) => format!("stopped: {}", why),
                                        None if settings.video.show_stats => shared.status.lock().unwrap().clone(),
                                        None => String::new(),
                                    };
                                    touchpad::paint(ctx, band, &held, &info);
                                }
                                if menu.open {
                                    action = menu.show(ctx, settings, &names, internal);
                                }
                            };
                            g.draw(&layout, &fx, Some(&mut f));
                        } else {
                            g.draw(&layout, &fx, None);
                        }
                    }
                }
                if action == Some(ui::Action::Quit) {
                    self.save();
                    el.exit();
                    return;
                }
                if self.settings != self.applied {
                    let layout_changed = self.settings.video.layout != self.applied.video.layout
                        || self.settings.video.res != self.applied.video.res
                        || self.settings.interface.language != self.applied.interface.language;
                    self.apply();
                    if layout_changed {
                        self.set_title();
                    }
                    if !self.menu.open {
                        self.save();
                    }
                }
                // the machine waits while the menu is open
                let pause = self.menu.open && self.settings.emulation.pause_in_menu;
                self.shared.paused.store(pause, Ordering::Relaxed);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// What the application starts with.
pub struct Options {
    pub rom: PathBuf,
    pub out: Option<PathBuf>,
    pub save: Option<PathBuf>,
    pub mute: bool,
    pub res: Option<usize>,
    pub menu: bool,
    pub dump: Option<PathBuf>,
    pub pack: Option<PathBuf>,
}

/// Runs the game in a window until it closes.
pub fn run(o: Options, el: EventLoop<()>) {
    let rom = o.rom;
    let mut settings = config::load();
    if let Some(r) = o.res {
        settings.video.res = r.clamp(1, 8);
    }
    let mut cfg = Config::new(&rom);
    cfg.out = o.out.unwrap_or_else(|| rom.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    cfg.save = o.save;
    cfg.dump_textures = o.dump;
    cfg.texture_pack = o.pack.or_else(|| {
        let p = PathBuf::from(&settings.video.texture_pack);
        if settings.video.texture_pack.is_empty() {
            None
        } else if p.is_dir() {
            Some(p)
        } else {
            eprintln!("texture pack {}: not a folder, ignored", p.display());
            None
        }
    });
    let shared = Shared::new();
    shared.scale.store(settings.video.res, Ordering::Relaxed);
    // the stream must outlive the machine's use of the ring
    let (ring, _stream) = if o.mute {
        (None, None)
    } else {
        match audio::open() {
            Ok((r, s)) => (Some(r), Some(s)),
            Err(e) => {
                eprintln!("no sound: {}", e);
                (None, None)
            }
        }
    };
    emu::spawn(cfg, shared.clone(), ring.clone());
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        shared: shared.clone(),
        ring,
        window: None,
        gfx: None,
        layouts: layout::all(),
        applied: settings.clone(),
        settings,
        menu: {
            let mut m = ui::Menu::new();
            m.open = o.menu;
            m
        },
        keymap: HashMap::new(),
        mouse: (0.0, 0.0),
        pen_down: false,
        touch: None,
        pad_touches: HashMap::new(),
        touch_held: Vec::new(),
        stopped_msg: None,
        kb: 0,
        pad: 0,
        pads: gilrs::Gilrs::new().map_err(|e| eprintln!("no pads: {}", e)).ok(),
        win_size: (512, 768),
    };
    el.run_app(&mut app).expect("the event loop");
    // the window is closed: stop the machine (its thread saves the backup memory and exits)
    shared.quit.store(true, Ordering::Relaxed);
    for _ in 0..100 {
        if shared.stopped.lock().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let why = shared.stopped.lock().unwrap().clone();
    if let Some(why) = why {
        eprintln!("stopped: {}", why);
    }
}

/// The command-line program.
pub fn desktop_main() {
    let mut args = std::env::args().skip(1);
    let mut rom: Option<PathBuf> = None;
    let mut o = Options { rom: PathBuf::new(), out: None, save: None, mute: false, res: None, menu: false, dump: None, pack: None };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => o.out = args.next().map(PathBuf::from),
            "--save" => o.save = args.next().map(PathBuf::from),
            "--res" => o.res = args.next().and_then(|v| v.parse().ok()),
            "--mute" => o.mute = true,
            "--menu" => o.menu = true,
            "--dump-textures" => o.dump = args.next().map(PathBuf::from),
            "--texture-pack" => o.pack = args.next().map(PathBuf::from),
            _ if rom.is_none() => rom = Some(PathBuf::from(a)),
            _ => {
                eprintln!("unknown argument {}", a);
                std::process::exit(2);
            }
        }
    }
    let Some(rom) = rom else {
        eprintln!("usage: sunpetal-app ROM.nds [--out DIR] [--save FILE] [--mute] [--res N] [--menu]");
        std::process::exit(2);
    };
    if !rom.is_file() {
        eprintln!("{}: not a file (the ROM's path: sunpetal-app ROM.nds)", rom.display());
        std::process::exit(2);
    }
    o.rom = rom;
    run(o, EventLoop::new().expect("an event loop"));
}

/// The Android entry point: the ROM is `rom.nds` in the app's folder on shared storage
/// (`Android/data/org.sunpetal.app/files`, where `adb push` and file managers reach), the saves
/// beside it; the settings are in the app's private folder.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android::redirect_stderr();
    let private = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let dir = app.external_data_path().unwrap_or_else(|| private.clone());
    std::env::set_var("SUNPETAL_CONFIG", private.join("config"));
    let rom = dir.join("rom.nds");
    eprintln!("sunpetal: files folder {}", dir.display());
    let el = EventLoop::builder().with_android_app(app).build().expect("an event loop");
    if !rom.is_file() {
        eprintln!("{}: no ROM there", rom.display());
        return;
    }
    run(Options { rom, out: Some(dir), save: None, mute: false, res: Some(1), menu: false, dump: None, pack: None }, el);
}

#[cfg(target_os = "android")]
mod android;
