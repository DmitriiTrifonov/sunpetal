//! Sunpetal: the window. A recompiled DS game's two screens on the GPU, its sound on the host's
//! device, played with the keyboard, a pad and the mouse, set up in a menu.
//!
//!     sunpetal-app ROM.nds [--out DIR] [--save FILE] [--mute] [--res N] [--menu]
//!                          [--dump-textures DIR] [--texture-pack DIR]
//!
//! F1 or Esc: the settings menu (video, sound, keys, profiles). Hotkeys: F2 changes the layout, F8 the game's menu in the single layout while the screen follows the game (START and the lower screen, B and back), else the screen shown (or the panel of the overlay layout), F3
//! the filter, / (the key beside the right Shift), P or F10 writes the picture and its parts to `artifacts/shots` (for bug reports), F4 and F5 the 3D's internal resolution (1-8 times the DS's), F11 full screen, Tab
//! (held) runs without waiting for the clock. The keys of the DS's buttons are set in the menu
//! (default: arrows = D-pad, Z = A, X = B, S = X, A = Y, Q = L, W = R, Enter = START, Backspace =
//! SELECT). A pad: the D-pad or left stick, the face buttons as the DS's (east A, south B, north
//! X, west Y), shoulders L and R, Start, Select. The mouse on the lower screen is the stylus. The
//! settings are kept in the configuration folder (`config::dir`) and saved when the menu closes
//! and at exit.
mod audio;
mod config;
mod dial;
mod effects;
mod emu;
mod fonts;
mod gfx;
mod layout;
mod layout_file;
mod locate;
mod post;
mod touchpad;
mod ui;
mod uitext;

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

/// Where the single layout is while the player is in the game's menu, opened by the button.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hold {
    /// Held on the lower screen, with the frames passed, until the menu shows something.
    Waiting(u32),
    /// The menu has shown something: held until the picture is the map again.
    Seen,
}

/// The hold after a picture that wants the lower screen (`wants`) or not, `settled` when it has
/// wanted so for the moment a change of screen takes.
/// The dialogue line over the upper screen's `r` (widened by `wide` DS pixels each side): the lines the game broke, in the
/// window's resolution, in the text area beside the portrait, centred on the band (DS pixels: x 58..243 with the portrait on
/// the left, 10..195 on the right; the band is y 144..191).
fn paint_dialogue(ctx: &egui::Context, r: layout::Rect, wide: f32, left: bool, text: &str, middle: Option<f32>, font: &str) {
    const PITCH: f32 = 14.0;
    const SIZE: f32 = 13.0;
    // the layout's rectangles are in the window's pixels, egui's coordinates in points
    let ppp = ctx.pixels_per_point();
    let r = layout::Rect { x: r.x / ppp, y: r.y / ppp, w: r.w / ppp, h: r.h / ppp };
    let (kx, ky) = (r.w / (256.0 + 2.0 * wide), r.h / 192.0);
    // the text starts where the game's letters do, beside the portrait; it is centred on the panel
    let (ds_x, ds_cy) = (if left { 58.0 } else { 10.0 }, middle.unwrap_or(168.0));
    let x0 = r.x + (wide + ds_x) * kx;
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let top = ds_cy - lines.len() as f32 * PITCH / 2.0;
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("dialogue")));
    for (i, l) in lines.iter().enumerate() {
        let at = egui::pos2(x0, r.y + (top + i as f32 * PITCH) * ky);
        p.text(at, egui::Align2::LEFT_TOP, l, egui::FontId::new(SIZE * ky, fonts::dialogue_family(font)), egui::Color32::from_rgb(16, 16, 24));
    }
}

fn next_hold(hold: Option<Hold>, wants: bool, settled: bool) -> Option<Hold> {
    match hold {
        Some(Hold::Waiting(_)) if wants => Some(Hold::Seen),
        Some(Hold::Waiting(n)) if n < HOLD_FRAMES => Some(Hold::Waiting(n + 1)),
        Some(Hold::Waiting(_)) => None,
        Some(Hold::Seen) if !wants && settled => None,
        h => h,
    }
}

/// Frames a button pressed by the app stays down, and the frames the menu may take to show.
/// Frames Edo's screen may go unseen before its hint patch is dropped (B-05; about 0.5 s at 60 frames a second).
const EDO_HOLD: u32 = 30;
const TAP_FRAMES: u32 = 8;
const HOLD_FRAMES: u32 = 1800;

/// Frames both screens stay up after the dial puzzle was last seen.
const PUZZLE_FRAMES: u32 = 240;
/// Frames the picture must ask for other bars before they change.
const BAR_FRAMES: u32 = 3;

/// Frames the picture must want the other screen before the single layout moves to it.
const AUTO_FRAMES: u32 = 20;

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
    /// The lower screen shows the dial puzzle (from the last frame).
    dial_scene: bool,
    /// A finger turning the dial.
    dial: Option<dial::Dial>,
    /// How black the screen is for a skip (0 none, 1 all): it goes dark while one runs and back after.
    skip_fade: f32,
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
    /// The picture's shape (width over height) the window was last given (`video.follow_shape`); 0: none yet.
    shape_last: f32,
    /// The lower screen has the map's counter (`layout::counter_in`, from the last picture).
    counter_seen: bool,
    /// The lower screen has the action icon (`layout::hint_in`).
    hint_seen: bool,
    /// Frames left of showing both screens for the dial puzzle (its answer's animation is on the upper one).
    puzzle_hold: u32,
    /// For this puzzle the panel would cover the LED digits: both screens side by side (`layout::led_under_panel`).
    puzzle_side: bool,
    /// The collection's list is on (`layout::collection_in`), and whether its card (the upper screen) is the one shown.
    coll_seen: bool,
    coll_card: std::cell::Cell<bool>,
    /// F10 asks the next frame for the window's picture (the file to write it to).
    pending_capture: std::cell::Cell<Option<PathBuf>>,
    /// The buttons last sent, to find the press of A or B; and a B that closed the card, held back from the game.
    keys_before: std::cell::Cell<u32>,
    swallow_b: std::cell::Cell<bool>,
    /// The upper screen is Edo's (`layout::edo_in`).
    edo_seen: bool,
    /// Frames since Edo's screen was last seen: the flag goes off only after `EDO_HOLD` of them (B-05).
    edo_off: u32,
    /// The side of the portrait last seen (`layout::portrait_left`).
    portrait_left: bool,
    /// The title's "Press START" box (`layout::prompt_box`).
    prompt: Option<layout::Rect>,
    /// A page of paper over both screens is shown (`layout::paper_in`, settled), and how far it is scrolled.
    paper: bool,
    scroll: f32,
    /// How many pixels the upper screen of the last picture is wider by each side (0: not wide).
    wide_now: usize,
    /// The colour for the bars (`layout::edge_color`), from the last picture.
    bar_color: Option<[f32; 3]>,
    /// What the picture asks of the bars now and for how many frames it has (a change waits, so the
    /// bars do not flicker between a colour and the glow on a picture whose edge keeps changing).
    bar_ask: (Option<[f32; 3]>, u32),
    /// The screen the picture wants (`layout::lower_wanted`) and for how many frames it has wanted it.
    want: (bool, u32),
    auto_lower: bool,
    hold: Option<Hold>,
    /// A button the app presses (bit) and the frames it has left.
    tap: (u32, u32),
    /// The restart was asked for: once the machine has stopped, the program starts again.
    restart: bool,
}

impl App {
    fn layout(&self) -> layout::Layout {
        layout::find(&self.layouts, &self.settings.video, &self.live())
    }

    fn live(&self) -> layout::Live {
        layout::Live { vblank: self.shared.vblank.load(Ordering::Relaxed), counter_seen: self.counter_seen, hint_seen: self.hint_seen, both: self.puzzle_hold > 0, both_side: self.puzzle_side, collection: self.coll_seen.then(|| self.coll_card.get()), lower: self.auto_lower || self.hold.is_some(), edo: self.edo_seen, prompt: self.prompt, wide: self.wide_now as f32, scroll: (self.paper && self.auto_lower).then_some(self.scroll) }
    }

    /// F10: the emulation writes the picture and its parts to `artifacts/shots`; this is what the window knows, for the
    /// file beside them (commit, layout, window, widening, what the picture was taken for, settings).
    fn dump_shot(&self) {
        let v = &self.settings.video;
        let commit = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let dirty = std::process::Command::new("git").args(["status", "--porcelain"]).output().ok().is_some_and(|o| !o.stdout.is_empty());
        let l = self.layout();
        let (ww, wh) = self.gfx.as_ref().map_or((0, 0), |g| (g.size().0 as u32, g.size().1 as u32));
        let meta = format!(
            "\"commit\":{:?},\"dirty\":{},\"layout\":{:?},\"layout_setting\":{:?},\"window\":[{},{}],\"wide_each_side\":{},\"want_wide\":{},\"auto_lower\":{},\"hold\":{},\"counter_seen\":{},\"edo_seen\":{},\"paper\":{},\"scroll\":{},\"bar_color\":{},\"fit\":{:?},\"pixel_aspect\":{},\"res\":{},\"language\":{:?},\"patches\":{}",
            commit,
            dirty,
            l.name,
            v.layout,
            ww,
            wh,
            self.wide_now,
            v.wide,
            self.auto_lower,
            self.hold.is_some(),
            self.counter_seen,
            self.edo_seen,
            self.paper,
            self.scroll,
            self.bar_color.map_or("null".to_string(), |c| format!("{:?}", c)),
            v.fit,
            v.pixel_aspect,
            v.res,
            self.settings.game.language,
            l.patches.len()
        );
        let e = &self.settings.effects;
        let window_png = format!("artifacts/shots/window_{}.png", self.shared.vblank.load(Ordering::Relaxed));
        let meta = format!(
            "{},\"window_png\":{:?},\"effects\":{{\"preset\":{:?},\"smooth_2d\":{:?},\"smooth_strength\":{},\"texture_filter\":{:?},\"outline\":{},\"shadow\":{},\"fxaa\":{},\"bloom\":{},\"sharp\":{}}},\"background\":{:?},\"texture_pack\":{:?},\"renderer\":{:?},\"msaa\":{}",
            meta, window_png, e.preset, e.smooth_2d, e.smooth_strength, e.texture_filter, e.outline, e.shadow, e.fxaa, e.bloom, e.sharp, v.background, v.texture_pack, v.renderer, e.msaa
        );
        self.pending_capture.set(Some(std::path::PathBuf::from(window_png)));
        *self.shared.dump_meta.lock().unwrap() = meta;
        self.shared.dump.store(true, Ordering::Relaxed);
    }

    /// Swaps the screen shown in the single layout; from another layout, goes into it.
    fn swap_screen(&mut self) {
        if self.settings.video.layout == "single" && self.settings.video.auto {
            self.game_menu();
            return;
        }
        if self.settings.video.layout == "overlay" {
            self.settings.video.panel = !self.settings.video.panel;
        } else if self.settings.video.layout == "single" {
            // the player's own choice: from the screen now shown, and the game stops deciding
            self.settings.video.lower = if self.settings.video.auto { !self.auto_lower } else { !self.settings.video.lower };
            self.settings.video.auto = false;
        } else {
            self.settings.video.layout = "single".into();
        }
        self.pen_down = false;
        self.update_pen();
    }

    fn send_keys(&self) {
        let touch = self.touch_held.iter().fold(0, |a, n| a | bit(n));
        let raw = self.kb | self.pad | touch | self.tap.0;
        let pressed = raw & !self.keys_before.replace(raw);
        let mut keys = raw;
        // the collection on one screen: A takes the item on the list to its card (the upper screen), B closes the card
        // back to the list and is not the game's (it would leave the collection)
        if self.coll_seen && self.settings.video.auto && self.settings.video.layout == "single" {
            if pressed & bit("A") != 0 && !self.coll_card.get() {
                self.coll_card.set(true);
            } else if pressed & bit("B") != 0 && self.coll_card.get() {
                self.coll_card.set(false);
                self.swallow_b.set(true);
            }
            if self.swallow_b.get() {
                keys &= !bit("B");
                if raw & bit("B") == 0 {
                    self.swallow_b.set(false);
                }
            }
        }
        self.shared.input.lock().unwrap().keys = keys;
    }

    /// The game's menu on one screen: from the scene, START (which brings up the game's menu button on
    /// the lower screen) and the lower screen held until the menu has been used; from the menu, B
    /// (back) and the upper screen.
    fn game_menu(&mut self) {
        if self.hold.is_some() || self.auto_lower {
            self.hold = None;
            self.press("B");
        } else {
            self.hold = Some(Hold::Waiting(0));
            self.press("START");
        }
        self.pen_down = false;
        self.update_pen();
    }

    /// A button pressed for a moment, as by a finger.
    fn press(&mut self, name: &str) {
        self.tap = (bit(name), TAP_FRAMES);
        self.send_keys();
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
        if s.video.renderer != a.video.renderer {
            self.shared.renderer.store((config::parse_renderer(&s.video.renderer) == Some(sunpetal_runtime::nds::Renderer::Gpu)) as u8, Ordering::Relaxed);
        }
        // another texture set chosen in the menu: swapped in before the next frame
        if s.video.texture_pack != a.video.texture_pack {
            let p = PathBuf::from(&s.video.texture_pack);
            *self.shared.pack_swap.lock().unwrap() = Some(if s.video.texture_pack.is_empty() || !p.exists() { None } else { Some(p) });
        }
        if s.effects.extras() != a.effects.extras() {
            *self.shared.extras.lock().unwrap() = s.effects.extras();
        }
        if let Some(g) = &mut self.gfx {
            g.linear = s.video.linear;
            g.background = s.video.background == "glow";
            g.edge_bars = s.video.background == "edge";
        }
        // a window size picked in the menu (logical pixels); the saved size follows the window itself
        if s.video.window != a.video.window && !s.video.fullscreen && self.menu.open {
            if let Some(w) = &self.window {
                let _ = w.request_inner_size(LogicalSize::new(s.video.window[0], s.video.window[1]));
            }
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
        let pen = match self.dial {
            Some(d) if self.pen_down && !self.menu.open => Some(d.pen()),
            _ if self.pen_down && !self.menu.open => self.layout().touch(g.size(), self.mouse, true),
            _ => None,
        };
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
        self.dial = None;
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
                // a page of paper scrolled over both screens: up and down move it, the game does not get them
                if self.live().scroll.is_some() && !self.menu.open && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown) {
                    if down {
                        let d = if code == KeyCode::ArrowUp { -24.0 } else { 24.0 };
                        self.scroll = (self.scroll + d).clamp(0.0, 192.0);
                    }
                    return;
                }
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
                    KeyCode::Tab if self.settings.emulation.fast_key => self.shared.fast.store(true, Ordering::Relaxed),
                    KeyCode::F2 => self.cycle_layout(),
                    KeyCode::F8 => self.swap_screen(),
                    KeyCode::F3 => self.settings.video.linear = !self.settings.video.linear,
                    KeyCode::F10 | KeyCode::KeyP | KeyCode::Slash => self.dump_shot(),
                    KeyCode::F7 => {
                        self.shared.toggle_skip();
                    }
                    KeyCode::F6 => self.settings.effects = self.settings.effects.next_preset(),
                    KeyCode::F4 => self.settings.video.res = self.settings.video.res.saturating_sub(1).max(1),
                    KeyCode::F5 => self.settings.video.res = (self.settings.video.res + 1).min(8),
                    KeyCode::PageUp => self.scroll = (self.scroll - 96.0).max(0.0),
                    KeyCode::PageDown => self.scroll = (self.scroll + 96.0).min(192.0),
                    KeyCode::F11 => self.settings.video.fullscreen = !self.settings.video.fullscreen,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 24.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 3.0,
                };
                self.scroll = (self.scroll - d).clamp(0.0, 192.0);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                self.update_pen();
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if !consumed => {
                self.pen_down = state == ElementState::Pressed;
                if self.pen_down && !self.menu.open && self.settings.emulation.click_skips_logos {
                    self.shared.logo_click.store(true, Ordering::Relaxed);
                }
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
                if t.phase == Started && !consumed && !self.menu.open && self.settings.emulation.click_skips_logos {
                    self.shared.logo_click.store(true, Ordering::Relaxed);
                }
                let band = self.gfx.as_ref().filter(|_| self.settings.touch_controls_on()).map(|g| {
                    let (w, h) = g.window_size();
                    touchpad::band(w, h)
                });
                let corner = self.gfx.as_ref().filter(|_| cfg!(target_os = "android") && !self.settings.touch_controls_on()).map(|g| g.window_size());
                match t.phase {
                    Started if !self.menu.open && !consumed && corner.map_or(false, |(w, h)| touchpad::on_menu_button(w, h, p)) => {
                        self.toggle_menu();
                    }
                    Started if !self.menu.open && !consumed && corner.map_or(false, |(w, h)| touchpad::on_swap_button(w, h, p)) => {
                        self.swap_screen();
                    }
                    Started if !self.menu.open && !consumed && self.settings.emulation.skip_button && corner.map_or(false, |(w, h)| touchpad::on_skip_button(w, h, p)) => {
                        // a tap starts a skip, another cancels it
                        self.shared.toggle_skip();
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
                            self.dial = None;
                            if self.settings.input.dial_swipe && self.dial_scene {
                                let at = self.gfx.as_ref().and_then(|g| self.layout().touch(g.size(), p, false));
                                self.dial = at.and_then(|(x, y)| dial::Dial::grab((x as f32, y as f32)));
                            }
                        } else if let Some(b) = band {
                            if touchpad::on_menu(b, p) {
                                self.toggle_menu();
                            } else if touchpad::on_screen_button(b, p) {
                                self.swap_screen();
                            } else {
                                self.pad_touches.insert(t.id, p);
                            }
                        }
                    }
                    Moved => {
                        if self.touch == Some(t.id) {
                            self.mouse = p;
                            if self.dial.is_some() {
                                let at = self.gfx.as_ref().and_then(|g| self.layout().touch(g.size(), p, true));
                                let gain = self.settings.input.dial_gain;
                                if let (Some((x, _)), Some(d)) = (at, self.dial.as_mut()) {
                                    d.moved(x as f32, gain);
                                }
                            }
                        } else if let Some(q) = self.pad_touches.get_mut(&t.id) {
                            *q = p;
                        }
                    }
                    Ended | Cancelled => {
                        if self.touch == Some(t.id) {
                            self.touch = None;
                            self.pen_down = false;
                            self.dial = None;
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
                if self.tap.1 > 0 {
                    self.tap.1 -= 1;
                    if self.tap.1 == 0 {
                        self.tap.0 = 0;
                        self.send_keys();
                    }
                }
                // the picture and its wide partner, taken together (the emulation writes them so)
                let (frame, wide_pair) = {
                    let mut slot = self.shared.frame.lock().unwrap();
                    let f = slot.take();
                    let w = if f.is_some() { self.shared.wide_frame.lock().unwrap().take() } else { None };
                    (f, w)
                };
                if let Some((scale, rgba)) = &frame {
                    self.dial_scene = dial::recognise(*scale, rgba);
                    // the dial's answer plays out on the upper screen a while after the dial is gone
                    let was = self.puzzle_hold;
                    self.puzzle_hold = if self.dial_scene {
                        PUZZLE_FRAMES
                    } else if layout::counter_in(*scale, rgba) {
                        0 // the map is back: the walk goes on
                    } else {
                        self.puzzle_hold.saturating_sub(1)
                    };
                    if was > 0 && self.puzzle_hold == 0 && layout::counter_in(*scale, rgba) {
                        // the puzzle is done: straight to the upper screen, the lower's map is not shown after it
                        self.auto_lower = false;
                        self.hold = None;
                        self.want = (false, AUTO_FRAMES);
                    }
                    // the digits' place is known once they are there; the layout then stays till the puzzle is done
                    self.puzzle_side = self.puzzle_hold > 0 && (self.puzzle_side || layout::led_under_panel(*scale, rgba));
                    self.counter_seen = layout::counter_in(*scale, rgba);
                    self.hint_seen = layout::hint_in(*scale, rgba);
                    let coll = layout::collection_in(*scale, rgba);
                    if coll != self.coll_seen {
                        // on the list again when the collection is entered or left
                        self.coll_card.set(false);
                        self.coll_seen = coll;
                    }
                    // a pink flash of the screen's own art can take the sign off for a few frames (the hint blinked on the
                    // option screen, B-05): it goes off only when it has been missing for `EDO_HOLD` frames
                    if layout::edo_in(*scale, rgba) && !layout::edo_title_in_hint(*scale, rgba) {
                        self.edo_seen = true;
                        self.edo_off = 0;
                    } else if self.edo_seen {
                        self.edo_off += 1;
                        self.edo_seen = self.edo_off < EDO_HOLD;
                    }
                    self.prompt = layout::prompt_box(*scale, rgba);
                    if let Some(left) = layout::portrait_left(*scale, rgba) {
                        self.portrait_left = left;
                    }
                    let paper = layout::paper_in(*scale, rgba);
                    if paper && !self.paper {
                        self.scroll = 0.0;
                    }
                    self.paper = paper;
                    // the bars of the window: the picture's edge colour if that is one
                    let layout = self.layout();
                    let ask = if self.settings.video.bars_match && matches!(self.settings.video.background.as_str(), "glow" | "edge") { layout::edge_color(&layout, *scale, rgba) } else { None };
                    let same = match (ask, self.bar_ask.0) {
                        (Some(a), Some(b)) => (0..3).all(|c| (a[c] - b[c]).abs() < 0.04),
                        (None, None) => true,
                        _ => false,
                    };
                    self.bar_ask = if same { (self.bar_ask.0, self.bar_ask.1 + 1) } else { (ask, 0) };
                    if self.bar_ask.1 >= BAR_FRAMES {
                        self.bar_color = self.bar_ask.0;
                    }
                    // a flat edge that only changes its shade (a logo fading in or out) is followed at once: waiting for
                    // it to hold leaves the old colour round a picture gone dark
                    if let (Some(a), Some(_)) = (ask, self.bar_color) {
                        self.bar_color = Some(a);
                    }
                    // a change of screen waits until the picture has held for a moment (a fade, a
                    // flash between scenes would flicker it)
                    if let Some(w) = layout::lower_wanted(*scale, rgba) {
                        self.want = if w == self.want.0 { (w, self.want.1 + 1) } else { (w, 0) };
                        if self.want.1 >= AUTO_FRAMES {
                            self.auto_lower = w;
                        }
                        self.hold = next_hold(self.hold, w, self.want.1 >= AUTO_FRAMES);
                    }
                }
                if let (Some((scale, rgba)), Some(g)) = (frame, &mut self.gfx) {
                    g.upload(scale, &rgba);
                    // the 3D scene widened, when the emulation made one for this picture
                    match wide_pair {
                        Some((sc, e, px)) => {
                            g.upload_wide(sc, e, &px);
                            self.wide_now = e;
                        }
                        None => self.wide_now = 0,
                    }
                }
                // the picture goes dark for a skip and comes back after it
                let target = if self.shared.skip.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
                self.skip_fade += (target - self.skip_fade).clamp(-0.07, 0.07);
                let fade = self.skip_fade;
                let mut action = None;
                let mut want_shape: Option<f32> = None;
                {
                    let App { gfx, menu, settings, layouts, touch_held, stopped_msg, shared, counter_seen, hint_seen, puzzle_hold, puzzle_side, coll_seen, coll_card, pending_capture, edo_seen, prompt, bar_color, auto_lower, hold, wide_now, paper, scroll, portrait_left, .. } = self;
                    let live = layout::Live { vblank: shared.vblank.load(Ordering::Relaxed), counter_seen: *counter_seen, hint_seen: *hint_seen, both: *puzzle_hold > 0, both_side: *puzzle_side, collection: coll_seen.then(|| coll_card.get()), lower: *auto_lower || hold.is_some(), edo: *edo_seen, prompt: *prompt, wide: *wide_now as f32, scroll: (*paper && *auto_lower).then_some(*scroll) };
                    if let Some(g) = gfx {
                        let layout = layout::find(layouts, &settings.video, &live);
                        g.bar_color = *bar_color;
                        if let Some(p) = pending_capture.take() {
                            g.capture = Some(p);
                        }
                        // the 3D view is widened to the window's shape when the upper screen is alone
                        let base = layout::find(layouts, &settings.video, &layout::Live { wide: 0.0, ..live });
                        // the layout the setting names, not what the game moves to for a moment (a puzzle, a card)
                        if settings.video.follow_shape && !settings.video.fullscreen {
                            let named = layouts.iter().find(|l| l.name == settings.video.layout).unwrap_or(&base);
                            want_shape = Some(named.size.0 * named.fit.par.clamp(0.25, 4.0) / named.size.1);
                        }
                        let upper_alone = base.screens[0].is_some() && (base.screens[1].is_none() || base.name == "overlay");
                        let want = if settings.video.wide && upper_alone {
                            layout::wide_each_side(g.size(), settings.video.pixel_aspect, sunpetal_runtime::nds::WIDE_MAX)
                        } else {
                            0
                        };
                        shared.wide_px.store(want, Ordering::Relaxed);
                        let names: Vec<&str> = layouts.iter().map(|l| l.name).collect();
                        let fx = settings.effects.clone();
                        let internal = (256 * settings.video.res as u32, 192 * settings.video.res as u32);
                        let pad_on = settings.touch_controls_on();
                        let (ww, wh) = g.window_size();
                        let band = touchpad::band(ww, wh);
                        g.reserve = if pad_on { band.h } else { 0.0 };
                        let corner_on = cfg!(target_os = "android") && !pad_on;
                        // the touch panel and the Android corner show the speed figures themselves; elsewhere they go over the picture
                        let stats_on = settings.video.show_stats && !pad_on && !corner_on;
                        // the dialogue line, drawn here in the window's resolution (the game's raster text is hidden)
                        let vrect = layout.fit(g.size())[0];
                        let crisp = match settings.video.dialogue_text.as_str() {
                            "pixel" => false,
                            "crisp" => vrect.is_some(),
                            _ => vrect.is_some_and(|r| r.h / 192.0 >= 2.0),
                        };
                        shared.crisp_text.store(crisp, Ordering::Relaxed);
                        // the line goes with the picture it was made for (empty when that picture has the game's own letters)
                        let vline = shared.dialogue_text.lock().unwrap().clone();
                        let vwide = *wide_now as f32;
                        if menu.open || pad_on || corner_on || stats_on || fade > 0.0 || vline.is_some() {
                            let held = touch_held.clone();
                            let mut f = |ctx: &egui::Context| {
                                if fade > 0.0 {
                                    // first, so that the buttons stay on top of it and can cancel the skip
                                    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("skipfade")));
                                    p.rect_filled(ctx.content_rect(), 0.0, egui::Color32::from_black_alpha((fade * 255.0) as u8));
                                }
                                if let (Some(text), Some(r)) = (&vline, vrect) {
                                    paint_dialogue(ctx, r, vwide, *portrait_left, &text.0, text.1, &settings.video.dialogue_font);
                                }
                                if corner_on {
                                    let info = match stopped_msg {
                                        Some(why) => format!("stopped: {}", why),
                                        None if settings.video.show_stats => shared.status.lock().unwrap().clone(),
                                        None => String::new(),
                                    };
                                    let fast = settings.emulation.fast_button.then(|| shared.fast.load(Ordering::Relaxed));
                                    let skip = settings.emulation.skip_button.then(|| shared.skip.load(Ordering::Relaxed));
                                    touchpad::paint_menu_button(ctx, ww, wh, &info, fast, skip);
                                }
                                if stats_on {
                                    // one line, in the top right corner (the game's own counter is at the left)
                                    let info = match stopped_msg {
                                        Some(why) => format!("stopped: {}", why),
                                        None => shared.status.lock().unwrap().lines().take(2).collect::<Vec<_>>().join("   "),
                                    };
                                    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("stats")));
                                    let galley = p.layout_no_wrap(info, egui::FontId::monospace(13.0), egui::Color32::WHITE);
                                    let at = ctx.content_rect().right_top() + egui::vec2(-8.0 - galley.size().x, 8.0);
                                    p.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(4.0), 3.0, egui::Color32::from_black_alpha(160));
                                    p.galley(at, galley, egui::Color32::WHITE);
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
                // the window takes the picture's shape, keeping its area (once per change of shape, so a window the player drags stays)
                match want_shape {
                    Some(a) if (a - self.shape_last).abs() > 0.004 => {
                        let area = self.win_size.0 as f32 * self.win_size.1 as f32;
                        let w = (area * a).sqrt();
                        if let Some(win) = &self.window {
                            let _ = win.request_inner_size(winit::dpi::PhysicalSize::new(w as u32, (w / a) as u32));
                        }
                        self.shape_last = a;
                    }
                    Some(_) => {}
                    None => self.shape_last = 0.0,
                }
                if action == Some(ui::Action::Quit) {
                    self.save();
                    el.exit();
                    return;
                }
                // a save to export or import: the emulation does it (paused or not); what it did comes back below
                match &action {
                    Some(ui::Action::ExportSave(to)) => *self.shared.save_job.lock().unwrap() = Some(emu::SaveJob::Export(to.clone())),
                    Some(ui::Action::ImportSave(from)) => *self.shared.save_job.lock().unwrap() = Some(emu::SaveJob::Import(from.clone())),
                    _ => {}
                }
                let done = self.shared.save_done.lock().unwrap().take();
                match done {
                    Some((false, said)) => self.menu.say(said.unwrap_or_else(|e| e)),
                    // the file is in place: the game starts anew and reads it (the machine writes nothing more)
                    Some((true, Ok(said))) => {
                        eprintln!("{}", said);
                        action = Some(ui::Action::Restart);
                    }
                    Some((true, Err(e))) => self.menu.say(e),
                    None => {}
                }
                if action == Some(ui::Action::Restart) {
                    self.save();
                    self.restart = true;
                    *self.shared.restart.lock().unwrap() = Some(restart_args(std::env::args_os().skip(1).collect(), &self.settings.video.texture_pack));
                    el.exit();
                    return;
                }
                if self.settings != self.applied {
                    let layout_changed = self.settings.video.layout != self.applied.video.layout
                        || self.settings.video.lower != self.applied.video.lower
                        || self.settings.video.panel != self.applied.video.panel
                        || self.settings.video.auto != self.applied.video.auto
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
    /// The game language for this run (`--lang ru`; `--lang none`: the ROM's own text), over the setting.
    pub lang: Option<String>,
    /// The widened 3D view for this run (`--wide on|off`), over the setting.
    pub wide: Option<bool>,
    /// The 3D engine for this run (`--renderer software|gpu`), over the setting.
    pub renderer: Option<String>,
}

/// Runs the game in a window until it closes.
pub fn run(o: Options, el: EventLoop<()>) {
    let rom = o.rom;
    let mut settings = config::load();
    if let Some(r) = o.res {
        settings.video.res = r.clamp(1, 8);
    }
    if let Some(w) = o.wide {
        settings.video.wide = w;
    }
    if let Some(r) = o.renderer {
        settings.video.renderer = r;
        settings.sanitize(); // an unknown name becomes `software`
    }
    if let Some(l) = o.lang {
        settings.game.language = if l == "none" { String::new() } else { l };
    }
    let mut cfg = Config::new(&rom);
    // saves go beside the ROM; with no ROM file, beside the data file
    let beside = if rom.as_os_str().is_empty() { o.pack.as_deref() } else { Some(rom.as_path()) };
    cfg.out = o.out.unwrap_or_else(|| beside.and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_default());
    cfg.save = o.save;
    cfg.dump_textures = o.dump;
    cfg.texture_pack = o.pack.or_else(|| {
        let p = PathBuf::from(&settings.video.texture_pack);
        if settings.video.texture_pack.is_empty() {
            let found = locate::find_data(&locate::search_dirs());
            if let Some(f) = &found {
                eprintln!("data file: {}", f.display());
            }
            found
        } else if p.exists() {
            Some(p)
        } else {
            eprintln!("texture pack {}: no such folder or file, ignored", p.display());
            None
        }
    });
    // a data file is checked whole before the game starts from it: a damaged one is no game, and
    // beside a ROM it is only left out
    if let Some(p) = cfg.texture_pack.clone().filter(|p| p.is_file()) {
        if let Err(e) = locate::check_data(&p) {
            eprintln!("{}", e);
            if rom.as_os_str().is_empty() {
                std::process::exit(2);
            }
            eprintln!("the game starts from the ROM without it");
            cfg.texture_pack = None;
        }
    }
    // the pack in use is the texture setting (the menu's list of sets starts from its folder)
    if let Some(p) = cfg.texture_pack.as_ref().filter(|p| p.is_file()) {
        settings.video.texture_pack = p.display().to_string();
    }
    // the data file (`sunpetal.data`) is the texture setting's file; its translation is put in
    if cfg.texture_pack.as_ref().is_some_and(|p| p.is_file()) && !settings.game.language.is_empty() {
        cfg.data = cfg.texture_pack.clone();
        cfg.language = Some(settings.game.language.clone());
    }
    if rom.as_os_str().is_empty() {
        cfg.data = cfg.texture_pack.clone();
    } else if cfg.data.is_none() && !settings.game.language.is_empty() {
        eprintln!("the game language `{}` is set, but there is no data file to take it from (the texture setting or --texture-pack); the ROM's own text is used", settings.game.language);
    }
    let shared = Shared::new();
    shared.scale.store(settings.video.res, Ordering::Relaxed);
    shared.renderer.store((config::parse_renderer(&settings.video.renderer) == Some(sunpetal_runtime::nds::Renderer::Gpu)) as u8, Ordering::Relaxed);
    *shared.extras.lock().unwrap() = settings.effects.extras();
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
        layouts: {
            // the player's layout files go after the built-in ones; a broken file is named and left out
            let mut all = layout::all();
            let (mine, bad) = layout_file::load_dir(&config::dir().join("layouts"));
            for b in bad {
                eprintln!("layout file: {}", b);
            }
            for l in mine {
                if !all.iter().any(|a| a.name == l.name) {
                    all.push(l);
                }
            }
            all
        },
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
        dial_scene: false,
        dial: None,
        skip_fade: 0.0,
        pad_touches: HashMap::new(),
        touch_held: Vec::new(),
        stopped_msg: None,
        kb: 0,
        pad: 0,
        pads: gilrs::Gilrs::new().map_err(|e| eprintln!("no pads: {}", e)).ok(),
        win_size: (512, 768),
        shape_last: 0.0,
        counter_seen: false,
        hint_seen: false,
        puzzle_hold: 0,
        puzzle_side: false,
        coll_seen: false,
        coll_card: std::cell::Cell::new(false),
        pending_capture: std::cell::Cell::new(None),
        keys_before: std::cell::Cell::new(0),
        swallow_b: std::cell::Cell::new(false),
        edo_seen: false,
        edo_off: 0,
        portrait_left: true,
        paper: false,
        bar_ask: (None, 0),
        scroll: 0.0,
        prompt: None,
        wide_now: 0,
        bar_color: None,
        want: (false, 0),
        auto_lower: false,
        hold: None,
        tap: (0, 0),
        restart: false,
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

/// The command line of the restarted program: the same, except that what the settings now hold wins over the
/// options that override them (the texture pack the menu may have changed, the width, the resolution, the language).
fn restart_args(mut args: Vec<std::ffi::OsString>, pack: &str) -> Vec<std::ffi::OsString> {
    let mut out = Vec::new();
    while !args.is_empty() {
        let a = args.remove(0);
        if matches!(a.to_str(), Some("--texture-pack" | "--wide" | "--res" | "--lang" | "--renderer")) {
            if !args.is_empty() {
                args.remove(0);
            }
            continue;
        }
        out.push(a);
    }
    if !pack.is_empty() {
        out.push("--texture-pack".into());
        out.push(pack.into());
    }
    out
}

#[cfg(test)]
mod save_command_tests {
    use super::{save_command, Options};
    use std::fs;
    use std::path::PathBuf;

    fn options() -> Options {
        Options { rom: PathBuf::new(), out: None, save: None, mute: false, res: None, menu: false, dump: None, pack: None, lang: None, wide: None, renderer: None }
    }

    #[test]
    fn the_save_goes_out_and_comes_back_with_no_window() {
        let dir = std::env::temp_dir().join(format!("sunpetal-savecmd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // the save beside the ROM, named by it
        let rom = dir.join("game.nds");
        let mut bytes = vec![0xFF; 65536];
        bytes[..5].copy_from_slice(&[9, 8, 7, 6, 5]);
        fs::write(dir.join("game.sav"), &bytes).unwrap();
        let o = options();
        let copy = dir.join("copy.sav");
        assert_eq!(save_command(Some(&copy), None, &o, Some(&rom)), 0);
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        // another save comes in, the first is kept
        let mut other = bytes.clone();
        other[0] = 1;
        fs::write(dir.join("other.sav"), &other).unwrap();
        assert_eq!(save_command(None, Some(&dir.join("other.sav")), &o, Some(&rom)), 0);
        assert_eq!(fs::read(dir.join("game.sav")).unwrap(), other);
        let kept: Vec<_> = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("game.sav.bak-")).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(fs::read(kept[0].path()).unwrap(), bytes);
        // a wrong file, a missing save and a missing game are refused (the save stays)
        fs::write(dir.join("bad.sav"), [1u8; 100]).unwrap();
        assert_eq!(save_command(None, Some(&dir.join("bad.sav")), &o, Some(&rom)), 1);
        assert_eq!(fs::read(dir.join("game.sav")).unwrap(), other);
        assert_eq!(save_command(Some(&copy), None, &o, Some(&dir.join("none.nds"))), 1);
        assert_eq!(save_command(Some(&copy), None, &o, None), 2);
        // --save names the file and --out the folder
        let o = Options { save: Some(dir.join("named.sav")), ..options() };
        assert_eq!(save_command(None, Some(&copy), &o, Some(&rom)), 0);
        assert_eq!(fs::read(dir.join("named.sav")).unwrap(), bytes);
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod restart_tests {
    use super::restart_args;
    use std::ffi::OsString;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_restart_keeps_the_line_but_the_settings_win_over_the_overrides() {
        let line = os(&["rom.nds", "--out", "dir", "--texture-pack", "old.data", "--wide", "on", "--mute", "--lang", "ru"]);
        assert_eq!(restart_args(line.clone(), "new.data"), os(&["rom.nds", "--out", "dir", "--mute", "--texture-pack", "new.data"]));
        assert_eq!(restart_args(line, ""), os(&["rom.nds", "--out", "dir", "--mute"]));
    }
}

/// The save file of a run, as the machine names it: `--save`, else `<out>/<name of the ROM or data file>.sav`
/// (the folder `--out`, else the one beside the ROM, or beside the data file when there is no ROM).
fn save_file_of(o: &Options, source: &std::path::Path) -> PathBuf {
    let out = o.out.clone().or_else(|| source.parent().map(|p| p.to_path_buf())).unwrap_or_default();
    sunpetal_runtime::savefile::default_path(o.save.as_deref(), &out, source)
}

/// `--export-save` / `--import-save`: the save file copied out, or a copy put in (the old one kept beside it
/// as `.bak-STAMP`). With no window and no machine: the game must not be running, which would write its own
/// memory over an import. The exit code of the program.
fn save_command(export: Option<&std::path::Path>, import: Option<&std::path::Path>, o: &Options, source: Option<&std::path::Path>) -> i32 {
    let Some(source) = source else {
        eprintln!("no game to name the save by: give the ROM (sunpetal-app ROM.nds --export-save FILE) or --save FILE");
        return 2;
    };
    let path = save_file_of(o, source);
    if let Some(to) = export {
        let said = std::fs::read(&path)
            .map_err(|e| format!("no save to export at {}: {}", path.display(), e))
            .and_then(|bytes| sunpetal_runtime::savefile::export(&bytes, to));
        match said {
            Ok(s) => println!("{}", s),
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    }
    if let Some(from) = import {
        match sunpetal_runtime::savefile::import(from, &path) {
            Ok(s) => println!("{}", s),
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    }
    0
}

/// The command-line program.
pub fn desktop_main() {
    let mut args = std::env::args().skip(1);
    let mut rom: Option<PathBuf> = None;
    let mut o = Options { rom: PathBuf::new(), out: None, save: None, mute: false, res: None, menu: false, dump: None, pack: None, lang: None, wide: None, renderer: None };
    // the save file out to a copy, or a copy in (`--export-save FILE`, `--import-save FILE`): done with no window
    let (mut export_save, mut import_save): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--export-save" => export_save = args.next().map(PathBuf::from),
            "--import-save" => import_save = args.next().map(PathBuf::from),
            "--out" => o.out = args.next().map(PathBuf::from),
            "--save" => o.save = args.next().map(PathBuf::from),
            "--res" => o.res = args.next().and_then(|v| v.parse().ok()),
            "--mute" => o.mute = true,
            // the speed figures and the skip's marks on the terminal each two seconds (as SUNPETAL_STATS=1)
            "--stats" => std::env::set_var("SUNPETAL_STATS", "1"),
            "--menu" => o.menu = true,
            "--dump-ui-strings" => {
                print!("{}", uitext::dump());
                std::process::exit(0);
            }
            "--dump-textures" => o.dump = args.next().map(PathBuf::from),
            "--texture-pack" => o.pack = args.next().map(PathBuf::from),
            "--lang" => o.lang = args.next(),
            "--wide" => o.wide = args.next().map(|v| !matches!(v.as_str(), "off" | "0" | "no")),
            "--renderer" => match args.next() {
                Some(v) if config::parse_renderer(&v).is_some() => o.renderer = Some(v),
                v => {
                    eprintln!("unknown renderer {} (software or gpu)", v.unwrap_or_default());
                    std::process::exit(2);
                }
            },
            _ if rom.is_none() => rom = Some(PathBuf::from(a)),
            _ => {
                eprintln!("unknown argument {}", a);
                std::process::exit(2);
            }
        }
    }
    // no ROM named: the one in the settings, then one found in the usual folders
    let dirs = locate::search_dirs();
    let rom = rom
        .or_else(|| Some(PathBuf::from(config::load().game.rom)).filter(|p| !p.as_os_str().is_empty()))
        .or_else(|| locate::find_rom(&dirs));
    if export_save.is_some() || import_save.is_some() {
        let named_data = o.pack.clone().or_else(|| Some(PathBuf::from(config::load().video.texture_pack)).filter(|p| !p.as_os_str().is_empty()));
        let data = named_data.or_else(|| locate::find_data(&dirs)).filter(|p| sunpetal_data::dsrom::has_rom(p));
        std::process::exit(save_command(export_save.as_deref(), import_save.as_deref(), &o, rom.as_deref().or(data.as_deref())));
    }
    let rom = match rom {
        Some(rom) => {
            if !rom.is_file() {
                eprintln!("{}: not a file (the ROM's path: sunpetal-app ROM.nds)", rom.display());
                std::process::exit(2);
            }
            if let Err(e) = locate::check_rom(&rom) {
                eprintln!("{}", e);
                std::process::exit(2);
            }
            rom
        }
        // no ROM file: the image kept in the data file (the one named, the settings', or one found)
        None => {
            let named = o.pack.clone().or_else(|| Some(PathBuf::from(config::load().video.texture_pack)).filter(|p| !p.as_os_str().is_empty()));
            match named.or_else(|| locate::find_data(&dirs)).filter(|p| sunpetal_data::dsrom::has_rom(p)) {
                Some(data) => {
                    o.pack = Some(data);
                    PathBuf::new()
                }
                None => {
                    // desktop: ask the player for the game (a ROM or a data file) and remember it
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    if let Some((picked, path)) = pick_game() {
                        let mut s = config::load();
                        match picked {
                            locate::Picked::Rom => s.game.rom = path.display().to_string(),
                            locate::Picked::Data => {
                                s.video.texture_pack = path.display().to_string();
                                // a translation in the file that is the system's language is the game's, unless one is set
                                if s.game.language.is_empty() {
                                    if let Some(l) = sys_locale::get_locale().and_then(|loc| locate::language_for(&path, &loc)) {
                                        s.game.language = l;
                                    }
                                }
                            }
                        }
                        config::save(&s);
                        match picked {
                            locate::Picked::Rom => {
                                o.rom = path;
                                run(o, EventLoop::new().expect("an event loop"));
                            }
                            locate::Picked::Data => {
                                o.pack = Some(path);
                                o.rom = PathBuf::new();
                                run(o, EventLoop::new().expect("an event loop"));
                            }
                        }
                        return;
                    }
                    eprintln!("usage: sunpetal-app [ROM.nds] [--out DIR] [--save FILE] [--export-save FILE] [--import-save FILE] [--mute] [--res N] [--menu] [--texture-pack FILE] [--lang ru|none] [--renderer software|gpu]");
                    eprintln!("{}", locate::no_game(&dirs));
                    std::process::exit(2);
                }
            }
        }
    };
    o.rom = rom;
    run(o, EventLoop::new().expect("an event loop"));
}

/// No game to play: the file dialog asks for a ROM or a data file until a good one is picked (the
/// message says why a bad one is not) or the player cancels. The one picked is checked as the
/// start checks it.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn pick_game() -> Option<(locate::Picked, PathBuf)> {
    loop {
        let path = rfd::FileDialog::new()
            .set_title("Sunpetal: pick the game's ROM (.nds) or sunpetal.data")
            .pick_file()?;
        match locate::classify(&path) {
            Ok(p) => return Some((p, path)),
            Err(e) => {
                rfd::MessageDialog::new().set_title("Sunpetal").set_description(e).set_level(rfd::MessageLevel::Error).show();
            }
        }
    }
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
    eprintln!("sunpetal: files folder {}", dir.display());
    let el = EventLoop::builder().with_android_app(app).build().expect("an event loop");
    // the game in the app's folder. A data file that holds the ROM is the whole game (the ROM in it
    // is the one its pictures and translations were made for) and goes first; else `rom.nds`, or any
    // other ROM of the supported dump, with a data file beside it if there is one
    let dirs = [dir.clone()];
    let data = locate::find_data(&dirs);
    let whole = data.clone().filter(|p| sunpetal_data::dsrom::has_rom(p));
    let (rom, pack) = if let Some(data) = whole {
        match locate::check_data(&data) {
            Ok(()) => (PathBuf::new(), Some(data)),
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        }
    } else {
        let named = dir.join("rom.nds");
        match Some(named).filter(|r| r.is_file() && locate::check_rom(r).is_ok()).or_else(|| locate::find_rom(&dirs)) {
            Some(rom) => (rom, data.filter(|p| locate::check_data(p).is_ok())),
            None => {
                eprintln!("{}", locate::no_game(&dirs));
                return;
            }
        }
    };
    // once, the first time a data file starts the game: its translation in the system's language
    // is the game's language (as the desktop's file dialog does); the menu changes it after
    if let Some(data) = &pack {
        let marker = private.join("config").join("language-chosen");
        if !marker.exists() {
            let mut s = config::load();
            if s.game.language.is_empty() {
                if let Some(l) = sys_locale::get_locale().and_then(|loc| locate::language_for(data, &loc)) {
                    s.game.language = l;
                    config::save(&s);
                }
            }
            let _ = std::fs::create_dir_all(marker.parent().unwrap());
            let _ = std::fs::write(&marker, b"");
        }
    }
    run(Options { rom, out: Some(dir), save: None, mute: false, res: Some(1), menu: false, dump: None, pack, lang: None, wide: None, renderer: None }, el);
}

#[cfg(target_os = "android")]
mod android;

#[cfg(test)]
mod hold_tests {
    use super::*;

    #[test]
    fn the_menu_is_held_until_it_has_been_used_and_the_map_is_back() {
        let mut h = Some(Hold::Waiting(0));
        // the game's menu button on the map: still the map, held
        for _ in 0..100 {
            h = next_hold(h, false, true);
        }
        assert_eq!(h, Some(Hold::Waiting(100)));
        // the menu shows something: seen; a moment of map in between does not let go
        h = next_hold(h, true, false);
        assert_eq!(h, Some(Hold::Seen));
        h = next_hold(h, false, false);
        assert_eq!(h, Some(Hold::Seen));
        // the map for good: let go
        assert_eq!(next_hold(h, false, true), None);
    }

    #[test]
    fn a_menu_never_opened_is_let_go_after_a_while() {
        let mut h = Some(Hold::Waiting(0));
        for _ in 0..HOLD_FRAMES + 1 {
            h = next_hold(h, false, true);
        }
        assert_eq!(h, None);
        assert_eq!(next_hold(None, true, true), None);
    }
}
