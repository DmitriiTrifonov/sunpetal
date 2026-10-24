//! Sunpetal: the window. A recompiled DS game's two screens on the GPU, its sound on the host's
//! device, played with the keyboard, a pad and the mouse.
//!
//!     sunpetal-app ROM.nds [--out DIR] [--save FILE] [--mute] [--scale N]
//!
//! Keys: arrows = the D-pad, Z = A, X = B, S = X, A = Y, Q = L, W = R, Enter = START,
//! Backspace = SELECT; the mouse on the lower screen = the stylus. A pad: the D-pad or left stick,
//! the face buttons as the DS's (east A, south B, north X, west Y), shoulders L and R, Start, Select. F2 changes the layout, F3 the
//! filter, Tab (held) runs without waiting for the clock, Esc quits.
mod audio;
mod emu;
mod gfx;
mod layout;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use sunpetal_runtime::nds::Config;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

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

/// The button bit (`nds::KEY_NAMES`) a key presses.
fn button(k: KeyCode) -> Option<u32> {
    let name = match k {
        KeyCode::KeyZ => "A",
        KeyCode::KeyX => "B",
        KeyCode::KeyS => "X",
        KeyCode::KeyA => "Y",
        KeyCode::KeyQ => "L",
        KeyCode::KeyW => "R",
        KeyCode::Enter => "START",
        KeyCode::Backspace => "SELECT",
        KeyCode::ArrowUp => "UP",
        KeyCode::ArrowDown => "DOWN",
        KeyCode::ArrowLeft => "LEFT",
        KeyCode::ArrowRight => "RIGHT",
        _ => return None,
    };
    sunpetal_runtime::nds::KEY_NAMES.iter().position(|n| *n == name).map(|i| 1 << i)
}

struct App {
    shared: Arc<Shared>,
    window: Option<Arc<Window>>,
    gfx: Option<gfx::Gfx>,
    layouts: Vec<layout::Layout>,
    current: usize,
    mouse: (f32, f32),
    pen_down: bool,
    scale: f32,
    /// Buttons held on the keyboard and on the pad (both go to the machine).
    kb: u32,
    pad: u32,
    pads: Option<gilrs::Gilrs>,
}

impl App {
    fn send_keys(&self) {
        self.shared.input.lock().unwrap().keys = self.kb | self.pad;
    }

    /// The pad's events since the last frame: its buttons held, the left stick as the D-pad.
    fn poll_pads(&mut self) {
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
                    if v < -0.5 {
                        self.pad |= neg;
                    } else if v > 0.5 {
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
        let pen = if self.pen_down { self.layouts[self.current].touch(g.size(), self.mouse, true) } else { None };
        self.shared.input.lock().unwrap().pen = pen;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Sunpetal")
            .with_inner_size(LogicalSize::new(256.0 * self.scale, 384.0 * self.scale));
        let window = Arc::new(el.create_window(attrs).expect("a window"));
        match gfx::Gfx::new(window.clone()) {
            Ok(g) => self.gfx = Some(g),
            Err(e) => {
                eprintln!("{}", e);
                el.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = &mut self.gfx {
                    g.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let down = event.state == ElementState::Pressed;
                if let Some(b) = button(code) {
                    if down {
                        self.kb |= b;
                    } else {
                        self.kb &= !b;
                    }
                    self.send_keys();
                    return;
                }
                match code {
                    KeyCode::Tab => self.shared.fast.store(down, Ordering::Relaxed),
                    KeyCode::Escape if down => el.exit(),
                    KeyCode::F2 if down && !event.repeat => {
                        self.current = (self.current + 1) % self.layouts.len();
                        if let Some(w) = &self.window {
                            w.set_title(&format!("Sunpetal: {}", self.layouts[self.current].name));
                        }
                    }
                    KeyCode::F3 if down && !event.repeat => {
                        if let Some(g) = &mut self.gfx {
                            g.linear = !g.linear;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                self.update_pen();
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.pen_down = state == ElementState::Pressed;
                // a press must land on the screen; a drag may leave it
                if self.pen_down {
                    if let Some(g) = &self.gfx {
                        if self.layouts[self.current].touch(g.size(), self.mouse, false).is_none() {
                            self.pen_down = false;
                        }
                    }
                }
                self.update_pen();
            }
            WindowEvent::RedrawRequested => {
                self.poll_pads();
                if let Some(rgba) = self.shared.frame.lock().unwrap().take() {
                    if let Some(g) = &self.gfx {
                        g.upload(&rgba);
                    }
                }
                if let Some(g) = &mut self.gfx {
                    g.draw(&self.layouts[self.current]);
                }
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
    let (mut out, mut save, mut mute, mut scale) = (None, None, false, 2.0f32);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--save" => save = args.next().map(PathBuf::from),
            "--scale" => scale = args.next().and_then(|v| v.parse().ok()).unwrap_or(2.0),
            "--mute" => mute = true,
            _ if rom.is_none() => rom = Some(PathBuf::from(a)),
            _ => {
                eprintln!("unknown argument {}", a);
                std::process::exit(2);
            }
        }
    }
    let Some(rom) = rom else {
        eprintln!("usage: sunpetal-app ROM.nds [--out DIR] [--save FILE] [--mute] [--scale N]");
        std::process::exit(2);
    };
    let mut cfg = Config::new(&rom);
    if let Some(o) = out {
        cfg.out = o;
    } else {
        cfg.out = rom.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    }
    cfg.save = save;
    let shared = Shared::new();
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
    emu::spawn(cfg, shared.clone(), ring);
    let el = EventLoop::new().expect("an event loop");
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        shared: shared.clone(),
        window: None,
        gfx: None,
        layouts: layout::all(),
        current: 0,
        mouse: (0.0, 0.0),
        pen_down: false,
        scale,
        kb: 0,
        pad: 0,
        pads: gilrs::Gilrs::new().map_err(|e| eprintln!("no pads: {}", e)).ok(),
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
