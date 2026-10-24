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
        self.shared.input.lock().unwrap().keys = self.kb | self.pad;
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
            WindowEvent::RedrawRequested => {
                self.poll_pads();
                // the machine stopped (a failed boot, a fatal error): say so, do not sit black
                let stopped = self.shared.stopped.lock().unwrap().clone();
                if let Some(why) = stopped {
                    eprintln!("stopped: {}", why);
                    self.save();
                    el.exit();
                    return;
                }
                let frame = self.shared.frame.lock().unwrap().take();
                if let (Some((scale, rgba)), Some(g)) = (frame, &mut self.gfx) {
                    g.upload(scale, &rgba);
                }
                let mut action = None;
                {
                    let App { gfx, menu, settings, layouts, .. } = self;
                    if let Some(g) = gfx {
                        let layout = layouts.iter().find(|l| l.name == settings.video.layout).unwrap_or(&layouts[0]).clone();
                        let names: Vec<&str> = layouts.iter().map(|l| l.name).collect();
                        let fx = settings.effects.clone();
                        let internal = (256 * settings.video.res as u32, 192 * settings.video.res as u32);
                        if menu.open {
                            let mut f = |ctx: &egui::Context| action = menu.show(ctx, settings, &names, internal);
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

fn main() {
    let mut args = std::env::args().skip(1);
    let mut rom: Option<PathBuf> = None;
    let (mut out, mut save, mut mute, mut res, mut menu) = (None, None, false, None::<usize>, false);
    let (mut dump, mut pack) = (None::<PathBuf>, None::<PathBuf>);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--save" => save = args.next().map(PathBuf::from),
            "--res" => res = args.next().and_then(|v| v.parse().ok()),
            "--mute" => mute = true,
            "--menu" => menu = true,
            "--dump-textures" => dump = args.next().map(PathBuf::from),
            "--texture-pack" => pack = args.next().map(PathBuf::from),
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
    let mut settings = config::load();
    if let Some(r) = res {
        settings.video.res = r.clamp(1, 8);
    }
    let mut cfg = Config::new(&rom);
    cfg.out = out.unwrap_or_else(|| rom.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    cfg.save = save;
    cfg.dump_textures = dump;
    cfg.texture_pack = pack.or_else(|| {
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
    let (ring, _stream) = if mute {
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
    let el = EventLoop::new().expect("an event loop");
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
            m.open = menu;
            m
        },
        keymap: HashMap::new(),
        mouse: (0.0, 0.0),
        pen_down: false,
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
