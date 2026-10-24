//! The on-screen controls for a touch screen: a D-pad, A/B/X/Y, L and R, Start and Select, and a
//! button for the menu, in a band under the screens. Several fingers at once; a finger that slides
//! from one button to another presses the second. The shapes are data (`controls`), the hit test a
//! plain function, so both are tested without a window.
use crate::layout::Rect;

/// What a control is for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// The four directions, by where in the circle the finger is.
    Dpad,
    /// A DS button, by its name in `nds::KEY_NAMES`.
    Button(&'static str),
    /// Opens the settings menu.
    Menu,
}

#[derive(Clone, Copy, Debug)]
pub struct Control {
    pub kind: Kind,
    pub label: &'static str,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Circle { x: f32, y: f32, r: f32 },
    Pill { rect: Rect },
}

impl Shape {
    fn contains(&self, p: (f32, f32)) -> bool {
        match *self {
            Shape::Circle { x, y, r } => (p.0 - x).powi(2) + (p.1 - y).powi(2) <= r * r,
            Shape::Pill { rect: r } => p.0 >= r.x && p.0 < r.x + r.w && p.1 >= r.y && p.1 < r.y + r.h,
        }
    }
}

/// The controls in the band `area` (window pixels).
pub fn controls(area: Rect) -> Vec<Control> {
    let (w, h) = (area.w, area.h);
    let at = |fx: f32, fy: f32| (area.x + fx * w, area.y + fy * h);
    let unit = w.min(h * 1.6);
    let mut v = Vec::new();
    let (dx, dy) = at(0.2, 0.56);
    v.push(Control { kind: Kind::Dpad, label: "", shape: Shape::Circle { x: dx, y: dy, r: unit * 0.15 } });
    let (cx, cy) = at(0.8, 0.56);
    let d = unit * 0.115;
    let rb = unit * 0.07;
    for (name, ox, oy) in [("A", d, 0.0), ("B", 0.0, d), ("X", 0.0, -d), ("Y", -d, 0.0)] {
        v.push(Control { kind: Kind::Button(name), label: name, shape: Shape::Circle { x: cx + ox, y: cy + oy, r: rb } });
    }
    let pill = |fx: f32, fy: f32, fw: f32, fh: f32| Shape::Pill { rect: Rect { x: area.x + fx * w, y: area.y + fy * h, w: fw * w, h: fh * h } };
    v.push(Control { kind: Kind::Button("L"), label: "L", shape: pill(0.03, 0.02, 0.3, 0.15) });
    v.push(Control { kind: Kind::Button("R"), label: "R", shape: pill(0.67, 0.02, 0.3, 0.15) });
    v.push(Control { kind: Kind::Button("SELECT"), label: "SELECT", shape: pill(0.4, 0.4, 0.2, 0.11) });
    v.push(Control { kind: Kind::Button("START"), label: "START", shape: pill(0.4, 0.56, 0.2, 0.11) });
    v.push(Control { kind: Kind::Menu, label: "MENU", shape: pill(0.4, 0.84, 0.2, 0.1) });
    v
}

/// The buttons the fingers at `points` hold: the DS buttons' names.
pub fn pressed(area: Rect, points: &[(f32, f32)]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut add = |n: &'static str| {
        if !out.contains(&n) {
            out.push(n)
        }
    };
    for c in controls(area) {
        for &p in points {
            if !c.shape.contains(p) {
                continue;
            }
            match (c.kind, c.shape) {
                (Kind::Button(n), _) => add(n),
                (Kind::Dpad, Shape::Circle { x, y, r }) => {
                    let (u, v) = ((p.0 - x) / r, (p.1 - y) / r);
                    if u > 0.35 {
                        add("RIGHT")
                    }
                    if u < -0.35 {
                        add("LEFT")
                    }
                    if v > 0.35 {
                        add("DOWN")
                    }
                    if v < -0.35 {
                        add("UP")
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Whether a finger landing at `p` is on the menu button.
pub fn on_menu(area: Rect, p: (f32, f32)) -> bool {
    controls(area).iter().any(|c| c.kind == Kind::Menu && c.shape.contains(p))
}

/// Draws the controls over the picture; `held` are the buttons now pressed. `ctx` works in points.
pub fn paint(ctx: &egui::Context, area: Rect, held: &[&'static str], info: &str) {
    use egui::{Align2, Color32, FontId, Pos2, Stroke};
    let k = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("touchpad")));
    let pt = |x: f32, y: f32| Pos2::new(x / k, y / k);
    let fill = |on: bool| if on { Color32::from_white_alpha(110) } else { Color32::from_white_alpha(38) };
    let line = Stroke::new(1.5, Color32::from_white_alpha(120));
    // a line for the screen's corner, under the status bar
    painter.text(Pos2::new(8.0, 40.0), Align2::LEFT_TOP, info, FontId::proportional(14.0), Color32::from_rgb(255, 80, 80));
    for c in controls(area) {
        let on = match c.kind {
            Kind::Button(n) => held.contains(&n),
            _ => false,
        };
        match c.shape {
            Shape::Circle { x, y, r } => {
                painter.circle(pt(x, y), r / k, fill(on), line);
                if c.kind == Kind::Dpad {
                    // the four arms
                    for (name, ox, oy) in [("UP", 0.0, -1.0), ("DOWN", 0.0, 1.0), ("LEFT", -1.0, 0.0), ("RIGHT", 1.0, 0.0)] {
                        let on = held.contains(&name);
                        let m = pt(x + ox * r * 0.62, y + oy * r * 0.62);
                        painter.circle_filled(m, r / k * 0.26, if on { Color32::from_white_alpha(160) } else { Color32::from_white_alpha(60) });
                    }
                } else {
                    painter.text(pt(x, y), Align2::CENTER_CENTER, c.label, FontId::proportional(r / k * 0.8), Color32::from_white_alpha(200));
                }
            }
            Shape::Pill { rect: r } => {
                let rect = egui::Rect::from_min_size(pt(r.x, r.y), egui::vec2(r.w / k, r.h / k));
                painter.rect(rect, egui::CornerRadius::same((r.h / k * 0.5) as u8), fill(on), line, egui::StrokeKind::Inside);
                painter.text(rect.center(), Align2::CENTER_CENTER, c.label, FontId::proportional(r.h / k * 0.45), Color32::from_white_alpha(200));
            }
        }
    }
}

/// The small menu button in the top-right corner, for a phone with the controls off.
pub fn menu_button(w: f32, h: f32) -> Shape {
    let r = w.min(h) * 0.045;
    Shape::Circle { x: w - 1.8 * r, y: 3.0 * r, r }
}

/// The fast-forward button, left of the menu button.
pub fn fast_button(w: f32, h: f32) -> Shape {
    let Shape::Circle { x, y, r } = menu_button(w, h) else { unreachable!() };
    Shape::Circle { x: x - 2.4 * r, y, r }
}

pub fn on_fast_button(w: f32, h: f32, p: (f32, f32)) -> bool {
    fast_button(w, h).contains(p)
}

/// The skip button (the DS's Start), left of the fast-forward button.
pub fn skip_button(w: f32, h: f32) -> Shape {
    let Shape::Circle { x, y, r } = fast_button(w, h) else { unreachable!() };
    Shape::Circle { x: x - 2.4 * r, y, r }
}

pub fn on_skip_button(w: f32, h: f32, p: (f32, f32)) -> bool {
    skip_button(w, h).contains(p)
}

/// Whether a finger landing at `p` is on that button.
pub fn on_menu_button(w: f32, h: f32, p: (f32, f32)) -> bool {
    menu_button(w, h).contains(p)
}

/// Draws that button; it stays faint so as not to be in the way.
pub fn paint_menu_button(ctx: &egui::Context, w: f32, h: f32, info: &str, fast: Option<bool>, skip: Option<bool>) {
    use egui::{Align2, Color32, FontId, Pos2, Stroke};
    let k = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("menubutton")));
    painter.text(Pos2::new(8.0, 40.0), Align2::LEFT_TOP, info, FontId::proportional(14.0), Color32::from_rgb(255, 80, 80));
    if let Shape::Circle { x, y, r } = menu_button(w, h) {
        let c = Pos2::new(x / k, y / k);
        painter.circle(c, r / k, Color32::from_black_alpha(90), Stroke::new(1.5, Color32::from_white_alpha(110)));
        // three bars, drawn: the bundled font has no such character
        let (hw, gap) = (r / k * 0.5, r / k * 0.32);
        for dy in [-gap, 0.0, gap] {
            painter.line_segment([Pos2::new(c.x - hw, c.y + dy), Pos2::new(c.x + hw, c.y + dy)], Stroke::new(r / k * 0.14, Color32::from_white_alpha(190)));
        }
    }
    // skip (Start): a triangle and a bar, lit while held
    if let (Some(on), Shape::Circle { x, y, r }) = (skip, skip_button(w, h)) {
        let c = Pos2::new(x / k, y / k);
        let fill = if on { Color32::from_rgba_unmultiplied(255, 255, 255, 120) } else { Color32::from_black_alpha(90) };
        painter.circle(c, r / k, fill, Stroke::new(1.5, Color32::from_white_alpha(110)));
        let u = r / k * 0.34;
        let ink = Stroke::new(r / k * 0.14, Color32::from_white_alpha(if on { 255 } else { 190 }));
        let (a, b, d) = (Pos2::new(c.x - u * 1.2, c.y - u), Pos2::new(c.x - u * 1.2, c.y + u), Pos2::new(c.x + u * 0.6, c.y));
        painter.line_segment([a, b], ink);
        painter.line_segment([b, d], ink);
        painter.line_segment([d, a], ink);
        painter.line_segment([Pos2::new(c.x + u * 1.3, c.y - u), Pos2::new(c.x + u * 1.3, c.y + u)], ink);
    }
    // fast-forward: lit while it is on
    if let (Some(on), Shape::Circle { x, y, r }) = (fast, fast_button(w, h)) {
        let c = Pos2::new(x / k, y / k);
        let fill = if on { Color32::from_rgba_unmultiplied(255, 255, 255, 120) } else { Color32::from_black_alpha(90) };
        painter.circle(c, r / k, fill, Stroke::new(1.5, Color32::from_white_alpha(110)));
        // two chevrons, drawn
        let (s, a) = (r / k * 0.3, r / k * 0.3);
        let ink = Stroke::new(r / k * 0.14, Color32::from_white_alpha(if on { 255 } else { 190 }));
        for dx in [-a, a * 0.6] {
            painter.line_segment([Pos2::new(c.x + dx - s * 0.5, c.y - s), Pos2::new(c.x + dx + s * 0.5, c.y)], ink);
            painter.line_segment([Pos2::new(c.x + dx + s * 0.5, c.y), Pos2::new(c.x + dx - s * 0.5, c.y + s)], ink);
        }
    }
}

/// The band under the screens for a window `w` x `h` pixels: tall enough for the DS's two screens
/// to fill the width of a phone, and no less than 28% nor more than 50% of the window.
pub fn band(w: f32, h: f32) -> Rect {
    let bh = (h - w * 1.5).clamp(h * 0.28, h * 0.5);
    Rect { x: 0.0, y: h - bh, w, h: bh }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        band(1080.0, 2400.0)
    }

    fn centre(kind: Kind) -> (f32, f32) {
        match controls(area()).iter().find(|c| c.kind == kind).unwrap().shape {
            Shape::Circle { x, y, .. } => (x, y),
            Shape::Pill { rect: r } => (r.x + r.w / 2.0, r.y + r.h / 2.0),
        }
    }

    #[test]
    fn the_band_leaves_a_phone_the_screens_at_full_width() {
        let b = band(1080.0, 2400.0);
        assert_eq!((b.y, b.h), (1620.0, 780.0));
        // a squat window gets at least 28% of its height
        assert_eq!(band(1000.0, 800.0).h, 800.0 * 0.28);
    }

    #[test]
    fn a_finger_on_a_button_presses_it() {
        let a = area();
        let (x, y) = match controls(a).iter().find(|c| c.kind == Kind::Button("A")).unwrap().shape {
            Shape::Circle { x, y, .. } => (x, y),
            _ => unreachable!(),
        };
        assert_eq!(pressed(a, &[(x, y)]), vec!["A"]);
        assert!(pressed(a, &[(x, a.y - 5.0)]).is_empty());
    }

    #[test]
    fn the_dpad_reads_directions_and_diagonals() {
        let a = area();
        let (x, y) = centre(Kind::Dpad);
        let r = match controls(a)[0].shape {
            Shape::Circle { r, .. } => r,
            _ => unreachable!(),
        };
        assert!(pressed(a, &[(x, y)]).is_empty());
        assert_eq!(pressed(a, &[(x + r * 0.8, y)]), vec!["RIGHT"]);
        assert_eq!(pressed(a, &[(x - r * 0.6, y - r * 0.6)]), vec!["LEFT", "UP"]);
    }

    #[test]
    fn several_fingers_press_several_buttons() {
        let a = area();
        let got = pressed(a, &[centre(Kind::Button("L")), centre(Kind::Button("START"))]);
        assert_eq!(got, vec!["L", "START"]);
    }

    #[test]
    fn the_corner_button_is_where_the_status_bar_is_not() {
        let Shape::Circle { x, y, r } = menu_button(1080.0, 2400.0) else { unreachable!() };
        assert!(x + r <= 1080.0 && y - r > 0.0);
        assert!(on_menu_button(1080.0, 2400.0, (x, y)));
        assert!(!on_menu_button(1080.0, 2400.0, (x - 3.0 * r, y)));
    }

    #[test]
    fn the_fast_button_sits_beside_the_menu_button_and_apart() {
        let (Shape::Circle { x: mx, y: my, r: mr }, Shape::Circle { x, y, r }) = (menu_button(1080.0, 2400.0), fast_button(1080.0, 2400.0)) else { unreachable!() };
        assert_eq!((y, r), (my, mr));
        assert!(mx - x > 2.0 * r, "the buttons must not touch");
        assert!(on_fast_button(1080.0, 2400.0, (x, y)));
        assert!(!on_fast_button(1080.0, 2400.0, (mx, my)));
        assert!(!on_menu_button(1080.0, 2400.0, (x, y)));
    }

    #[test]
    fn the_three_corner_buttons_are_apart_and_the_skip_one_is_found() {
        let (w, h) = (1080.0, 2400.0);
        let c = |s: Shape| match s { Shape::Circle { x, y, r } => (x, y, r), _ => unreachable!() };
        let (m, f, s) = (c(menu_button(w, h)), c(fast_button(w, h)), c(skip_button(w, h)));
        assert!(m.0 - f.0 > 2.0 * m.2 && f.0 - s.0 > 2.0 * f.2, "the buttons must not touch");
        assert!(s.0 - s.2 > 0.0, "all on the screen");
        assert!(on_skip_button(w, h, (s.0, s.1)));
        assert!(!on_skip_button(w, h, (f.0, f.1)) && !on_fast_button(w, h, (s.0, s.1)) && !on_menu_button(w, h, (s.0, s.1)));
    }

    #[test]
    fn only_the_menu_button_is_the_menu() {
        let a = area();
        assert!(on_menu(a, centre(Kind::Menu)));
        assert!(!on_menu(a, centre(Kind::Button("START"))));
    }
}

#[cfg(test)]
mod overlap {
    use super::*;

    /// No two controls may overlap on any phone-like or squat window: a finger would press two.
    #[test]
    fn no_controls_overlap() {
        for (w, h) in [(1080.0, 2400.0), (720.0, 1280.0), (1440.0, 3200.0), (1000.0, 800.0)] {
            let cs = controls(band(w, h));
            for (i, a) in cs.iter().enumerate() {
                for b in &cs[i + 1..] {
                    let apart = !touch(a.shape, b.shape);
                    assert!(apart, "{} and {} overlap at {}x{}", a.label, b.label, w, h);
                }
            }
        }
    }

    fn touch(a: Shape, b: Shape) -> bool {
        match (a, b) {
            (Shape::Circle { x: x1, y: y1, r: r1 }, Shape::Circle { x: x2, y: y2, r: r2 }) => (x1 - x2).hypot(y1 - y2) < r1 + r2,
            (Shape::Circle { x, y, r }, Shape::Pill { rect }) | (Shape::Pill { rect }, Shape::Circle { x, y, r }) => {
                let (cx, cy) = (x.clamp(rect.x, rect.x + rect.w), y.clamp(rect.y, rect.y + rect.h));
                (x - cx).hypot(y - cy) < r
            }
            (Shape::Pill { rect: p }, Shape::Pill { rect: q }) => p.x < q.x + q.w && q.x < p.x + p.w && p.y < q.y + q.h && q.y < p.y + p.h,
        }
    }
}
