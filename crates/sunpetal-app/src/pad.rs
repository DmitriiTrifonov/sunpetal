//! The pad: what each button does (the game's, or the menus'), the menu's keys, the virtual stylus.
//! Pure functions and small states, so that the table can be tested without a pad; `lib.rs` feeds
//! them the events (`SUNPETAL_PAD_LOG=1` prints each one with what it was taken for).
use crate::pad_button;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// What a button press does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    /// Presses these bits of the DS's buttons (the release lets them go).
    Game(u32),
    /// SELECT: the game's own menu (the suitcase), as F8.
    GameMenu,
    /// SELECT with the game's menu already up (the lower screen is a menu, the case, a list): back, as B.
    GameBack,
    /// START: our menu.
    OpenMenu,
    /// START with our menu open: closes it.
    CloseMenu,
    /// The menu: the focus moves.
    Nav(Dir),
    /// The menu: A takes the widget in focus.
    Confirm,
    /// The menu: B closes a drop-down list, then the menu.
    Back,
    /// The right stick's click: the stylus touches the screen (the release lifts it).
    Pen,
    None,
}

/// What the answer depends on besides the button.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    pub menu_open: bool,
    /// The game waits for START (the title's "Press START"): the button is the game's.
    pub start_to_game: bool,
    /// The lower screen has the game's menu, the case or a list on it (not the map, not a puzzle): SELECT closes it.
    pub game_menu_up: bool,
}

/// The pad button that is the stylus' touch.
pub fn is_pen_button(b: gilrs::Button) -> bool {
    b == gilrs::Button::RightThumb
}

/// What pressing `b` does.
pub fn press(b: gilrs::Button, c: Ctx) -> Act {
    use gilrs::Button::*;
    if c.menu_open {
        return match b {
            DPadUp => Act::Nav(Dir::Up),
            DPadDown => Act::Nav(Dir::Down),
            DPadLeft => Act::Nav(Dir::Left),
            DPadRight => Act::Nav(Dir::Right),
            East => Act::Confirm,
            South => Act::Back,
            Start => Act::CloseMenu,
            _ => Act::None,
        };
    }
    match b {
        Start if !c.start_to_game => Act::OpenMenu,
        Select if c.game_menu_up => Act::GameBack,
        Select => Act::GameMenu,
        b if is_pen_button(b) => Act::Pen,
        b => match pad_button(b) {
            0 => Act::None,
            bits => Act::Game(bits),
        },
    }
}

/// The egui key a menu move or button stands for.
pub fn key_of(a: Act) -> Option<egui::Key> {
    match a {
        Act::Nav(Dir::Up) => Some(egui::Key::ArrowUp),
        Act::Nav(Dir::Down) => Some(egui::Key::ArrowDown),
        Act::Nav(Dir::Left) => Some(egui::Key::ArrowLeft),
        Act::Nav(Dir::Right) => Some(egui::Key::ArrowRight),
        Act::Confirm => Some(egui::Key::Enter),
        Act::Back => Some(egui::Key::Escape),
        _ => None,
    }
}

/// The direction a stick or an axis D-pad points, if it points past `threshold` (the larger axis wins).
pub fn stick_dir(x: f32, y: f32, threshold: f32) -> Option<Dir> {
    if x.abs().max(y.abs()) < threshold {
        return None;
    }
    Some(if x.abs() >= y.abs() { if x < 0.0 { Dir::Left } else { Dir::Right } } else if y < 0.0 { Dir::Down } else { Dir::Up })
}

/// A held direction repeating in the menu (a slider is moved by holding): after `FIRST` frames, each `EVERY`.
#[derive(Default, Debug)]
pub struct Repeat {
    dir: Option<Dir>,
    frames: u32,
}

const FIRST: u32 = 24;
const EVERY: u32 = 5;

impl Repeat {
    pub fn hold(&mut self, d: Option<Dir>) {
        if d != self.dir {
            self.dir = d;
            self.frames = 0;
        }
    }

    /// One frame: the direction to press again, if it is time.
    pub fn tick(&mut self) -> Option<Dir> {
        let d = self.dir?;
        self.frames += 1;
        (self.frames >= FIRST && (self.frames - FIRST) % EVERY == 0).then_some(d)
    }
}

/// The stick's push as a speed (a round dead zone, then 0 to 1 across the rest): the right stick moves the stylus.
pub fn stick_speed(x: f32, y: f32, dead: f32) -> (f32, f32) {
    let m = x.hypot(y);
    if m <= dead || m <= 0.0 {
        return (0.0, 0.0);
    }
    let k = ((m - dead) / (1.0 - dead).max(0.05)).min(1.0) / m;
    (x * k, y * k)
}

/// The stylus the right stick moves: a point in the window's pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stylus {
    pub pos: Option<(f32, f32)>,
}

impl Stylus {
    /// One step of `dt` seconds with the stick at (`x`, `y`; up is positive): the point moves `speed` window widths a
    /// second at full tilt and stays in the window (`size`); `home` is where it starts. True if it moved.
    pub fn step(&mut self, (x, y): (f32, f32), dt: f32, speed: f32, dead: f32, size: (f32, f32), home: (f32, f32)) -> bool {
        let (sx, sy) = stick_speed(x, y, dead);
        if sx == 0.0 && sy == 0.0 {
            return false;
        }
        let (px, py) = self.pos.unwrap_or(home);
        let d = speed * size.0 * dt;
        self.pos = Some(((px + sx * d).clamp(0.0, (size.0 - 1.0).max(0.0)), (py - sy * d).clamp(0.0, (size.1 - 1.0).max(0.0))));
        true
    }
}

/// The start of the log line for the axes the app does not read, to name them: only for `SUNPETAL_PAD_LOG`.
pub fn axis_use(a: gilrs::Axis) -> &'static str {
    match a {
        gilrs::Axis::LeftStickX | gilrs::Axis::LeftStickY => "left stick (D-pad, menu moves)",
        gilrs::Axis::RightStickX | gilrs::Axis::RightStickY => "right stick (stylus)",
        gilrs::Axis::DPadX | gilrs::Axis::DPadY => "D-pad as axes",
        _ => "not used",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bit;
    use gilrs::Button::*;

    const GAME: Ctx = Ctx { menu_open: false, start_to_game: false, game_menu_up: false };
    const MENU: Ctx = Ctx { menu_open: true, start_to_game: false, game_menu_up: false };
    const TITLE: Ctx = Ctx { menu_open: false, start_to_game: true, game_menu_up: false };

    #[test]
    fn the_faces_are_the_ds_buttons_and_select_is_not_the_games() {
        assert_eq!(press(East, GAME), Act::Game(bit("A")));
        assert_eq!(press(South, GAME), Act::Game(bit("B")));
        assert_eq!(press(North, GAME), Act::Game(bit("X")));
        assert_eq!(press(West, GAME), Act::Game(bit("Y")));
        assert_eq!(press(DPadLeft, GAME), Act::Game(bit("LEFT")));
        assert_eq!(press(LeftTrigger, GAME), Act::Game(bit("L")));
        assert_eq!(press(RightTrigger2, GAME), Act::Game(bit("R")));
        assert_eq!(press(Select, GAME), Act::GameMenu);
        assert_eq!(press(Mode, GAME), Act::None);
        assert_eq!(press(RightThumb, GAME), Act::Pen);
    }

    #[test]
    fn start_opens_our_menu_except_where_the_game_waits_for_it() {
        assert_eq!(press(Start, GAME), Act::OpenMenu);
        assert_eq!(press(Start, TITLE), Act::Game(bit("START")));
        // SELECT is the suitcase on the title too
        assert_eq!(press(Select, TITLE), Act::GameMenu);
        // with the game's menu up, SELECT is the way back (B), not the suitcase again
        assert_eq!(press(Select, Ctx { game_menu_up: true, ..GAME }), Act::GameBack);
        assert_eq!(press(Start, MENU), Act::CloseMenu);
    }

    #[test]
    fn with_the_menu_open_the_game_gets_nothing() {
        for b in [North, West, LeftTrigger, RightTrigger, Select, RightThumb, LeftThumb, Mode] {
            assert!(!matches!(press(b, MENU), Act::Game(_)), "{:?}", b);
        }
        assert_eq!(press(DPadDown, MENU), Act::Nav(Dir::Down));
        assert_eq!(press(East, MENU), Act::Confirm);
        assert_eq!(press(South, MENU), Act::Back);
        assert_eq!(key_of(Act::Nav(Dir::Left)), Some(egui::Key::ArrowLeft));
        assert_eq!(key_of(Act::Confirm), Some(egui::Key::Enter));
        assert_eq!(key_of(Act::Back), Some(egui::Key::Escape));
        assert_eq!(key_of(Act::OpenMenu), None);
    }

    #[test]
    fn a_stick_points_one_way_past_the_threshold() {
        assert_eq!(stick_dir(0.3, 0.2, 0.5), None);
        assert_eq!(stick_dir(-0.9, 0.3, 0.5), Some(Dir::Left));
        assert_eq!(stick_dir(0.2, -0.8, 0.5), Some(Dir::Down));
        assert_eq!(stick_dir(0.2, 0.8, 0.5), Some(Dir::Up));
    }

    #[test]
    fn a_held_direction_repeats_after_a_pause() {
        let mut r = Repeat::default();
        assert_eq!(r.tick(), None);
        r.hold(Some(Dir::Right));
        let mut n = Vec::new();
        for f in 1..=FIRST + 2 * EVERY {
            if r.tick().is_some() {
                n.push(f);
            }
        }
        assert_eq!(n, vec![FIRST, FIRST + EVERY, FIRST + 2 * EVERY]);
        r.hold(None);
        assert_eq!(r.tick(), None);
    }

    #[test]
    fn the_stick_has_a_dead_zone_and_a_full_tilt() {
        assert_eq!(stick_speed(0.1, 0.1, 0.2), (0.0, 0.0));
        let (x, y) = stick_speed(1.0, 0.0, 0.2);
        assert!((x - 1.0).abs() < 1e-5 && y == 0.0);
        let (x, _) = stick_speed(0.6, 0.0, 0.2);
        assert!((x - 0.5).abs() < 1e-5);
        // a diagonal is no faster than a straight push
        let (x, y) = stick_speed(1.0, 1.0, 0.2);
        assert!((x.hypot(y) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_stylus_moves_with_the_stick_and_stays_in_the_window() {
        let mut s = Stylus::default();
        let size = (800.0, 600.0);
        // untouched stick: no point yet
        assert!(!s.step((0.0, 0.0), 1.0 / 60.0, 1.0, 0.2, size, (400.0, 450.0)));
        assert_eq!(s.pos, None);
        // right and up: x grows, y shrinks (the stick's up is positive)
        assert!(s.step((1.0, 1.0), 0.1, 1.0, 0.2, size, (400.0, 450.0)));
        let (x, y) = s.pos.unwrap();
        assert!(x > 400.0 && y < 450.0);
        // far over the edge: held in
        for _ in 0..100 {
            s.step((1.0, -1.0), 0.5, 2.0, 0.2, size, (0.0, 0.0));
        }
        assert_eq!(s.pos, Some((799.0, 599.0)));
        for _ in 0..100 {
            s.step((-1.0, 1.0), 0.5, 2.0, 0.2, size, (0.0, 0.0));
        }
        assert_eq!(s.pos, Some((0.0, 0.0)));
    }
}
