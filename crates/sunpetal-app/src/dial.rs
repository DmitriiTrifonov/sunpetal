//! Turning a dial by swiping. One of the game's puzzles is a dial the stylus turns by dragging round
//! it; with a finger, a straight swipe passes near the middle and the angle jumps about. Here the
//! swipe's sideways travel turns the dial: the pen the game sees is kept on a circle round the
//! dial's centre, and moves along it as far as the finger goes across.
//!
//! The assist applies only while the lower screen shows that puzzle (`recognise`, from the colours
//! at a dozen points, so it holds with or without the screen effects) and only to a touch that
//! lands on the dial's rim, not its middle button or anything else.

/// The dial in the lower screen's pixels.
pub const CENTER: (f32, f32) = (153.0, 95.0);
pub const RADIUS: f32 = 79.0;
/// The middle button: a touch there stays as it is.
pub const BUTTON: f32 = 24.0;
/// Where the synthetic pen runs.
pub const PEN_RADIUS: f32 = 58.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Dark,
    Green,
    Metal,
    Brown,
}

/// Points of the lower screen and the kind of colour the dial puzzle has there.
const SIGNATURE: [((u32, u32), Class); 12] = [
    ((153, 95), Class::Dark),
    ((235, 172), Class::Dark),
    ((120, 14), Class::Green),
    ((200, 176), Class::Green),
    ((25, 20), Class::Green),
    ((235, 20), Class::Green),
    ((153, 45), Class::Metal),
    ((153, 145), Class::Metal),
    ((100, 95), Class::Metal),
    ((190, 120), Class::Metal),
    ((30, 70), Class::Brown),
    ((60, 168), Class::Metal),
];

/// The same puzzle drawn over by the 2D pack (stage 8е.7е: the silver case, the green plate; its band at the top is
/// black, the corners are not the game's): the pack's colours at points that the dial's art has in common.
const SIGNATURE_REDRAWN: [((u32, u32), Class); 12] = [
    ((153, 95), Class::Dark),
    ((60, 12), Class::Dark),
    ((125, 15), Class::Dark),
    ((15, 100), Class::Green),
    ((15, 150), Class::Green),
    ((20, 40), Class::Green),
    ((240, 100), Class::Green),
    ((240, 150), Class::Green),
    ((153, 45), Class::Metal),
    ((153, 145), Class::Metal),
    ((100, 95), Class::Metal),
    ((190, 120), Class::Metal),
];

fn is(c: Class, (r, g, b): (i32, i32, i32)) -> bool {
    match c {
        Class::Dark => r < 40 && g < 40 && b < 40,
        Class::Green => g > r + 8 && g > b + 15 && g > 50,
        Class::Metal => r + g + b > 200 && (r - g).abs() < 14 && b >= r - 6,
        Class::Brown => r > g + 4 && g > b + 15 && r > 30,
    }
}

/// Whether a frame (both screens, RGBA, the upper first, `scale` times the DS's size) shows the dial
/// puzzle on the lower screen.
pub fn recognise(scale: usize, rgba: &[u8]) -> bool {
    let (w, h) = (256 * scale, 192 * scale);
    let lower = w * h * 4;
    if rgba.len() < 2 * lower {
        return false;
    }
    let matches = |sig: &[((u32, u32), Class)]| {
        let hits = sig
            .iter()
            .filter(|((x, y), class)| {
                let (px, py) = (*x as usize * scale + scale / 2, *y as usize * scale + scale / 2);
                let o = lower + (py * w + px) * 4;
                is(*class, (rgba[o] as i32, rgba[o + 1] as i32, rgba[o + 2] as i32))
            })
            .count();
        hits >= sig.len() - 1
    };
    matches(&SIGNATURE) || matches(&SIGNATURE_REDRAWN)
}

/// A dial being turned by a finger.
#[derive(Clone, Copy, Debug)]
pub struct Dial {
    theta: f32,
    last_x: f32,
}

impl Dial {
    /// A touch at `p` (lower screen pixels) takes hold of the dial if it is on the rim.
    pub fn grab(p: (f32, f32)) -> Option<Dial> {
        let (dx, dy) = (p.0 - CENTER.0, p.1 - CENTER.1);
        let r = dx.hypot(dy);
        (r > BUTTON && r < RADIUS).then(|| Dial { theta: dy.atan2(dx), last_x: p.0 })
    }

    /// The finger is now at `x` across; `gain` is the turn in radians for a travel of one radius
    /// (negative turns the other way). Right is clockwise.
    pub fn moved(&mut self, x: f32, gain: f32) {
        self.theta += gain * (x - self.last_x) / RADIUS;
        self.last_x = x;
    }

    /// The pen the game sees.
    pub fn pen(&self) -> (i32, i32) {
        ((CENTER.0 + PEN_RADIUS * self.theta.cos()).round() as i32, (CENTER.1 + PEN_RADIUS * self.theta.sin()).round() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lower screen painted with the colours the signature expects.
    fn frame(scale: usize, tint: impl Fn(Class) -> (u8, u8, u8)) -> Vec<u8> {
        frame_of(scale, &SIGNATURE, tint)
    }

    fn frame_of(scale: usize, sig: &[((u32, u32), Class)], tint: impl Fn(Class) -> (u8, u8, u8)) -> Vec<u8> {
        let (w, h) = (256 * scale, 192 * scale);
        let mut v = vec![0u8; 2 * w * h * 4];
        for &((x, y), c) in sig {
            let (r, g, b) = tint(c);
            let (px, py) = (x as usize * scale + scale / 2, y as usize * scale + scale / 2);
            let o = w * h * 4 + (py * w + px) * 4;
            v[o..o + 4].copy_from_slice(&[r, g, b, 255]);
        }
        v
    }

    fn colours(c: Class) -> (u8, u8, u8) {
        match c {
            Class::Dark => (5, 5, 8),
            Class::Green => (60, 100, 50),
            Class::Metal => (110, 112, 130),
            Class::Brown => (80, 65, 30),
        }
    }

    #[test]
    fn the_puzzle_is_recognised_at_any_scale_and_under_a_tint() {
        for scale in [1, 2, 4] {
            assert!(recognise(scale, &frame(scale, colours)));
        }
        // a screen effect that darkens and shifts the colours a little
        assert!(recognise(2, &frame(2, |c| { let (r, g, b) = colours(c); (r.saturating_sub(4), g.saturating_sub(4), b) })));
    }

    #[test]
    fn the_puzzle_drawn_over_by_the_pack_is_recognised_too() {
        for scale in [1, 2, 4] {
            assert!(recognise(scale, &frame_of(scale, &SIGNATURE_REDRAWN, colours)));
        }
    }

    #[test]
    fn another_screen_is_not_taken_for_it() {
        // the map screen: blue and green, nothing dark
        let map = frame(1, |_| (90, 140, 190));
        assert!(!recognise(1, &map));
        assert!(!recognise(1, &frame(1, |_| (0, 0, 0))));
        assert!(!recognise(1, &[]));
    }

    #[test]
    fn only_the_rim_can_be_grabbed() {
        assert!(Dial::grab((CENTER.0 + 50.0, CENTER.1)).is_some());
        assert!(Dial::grab(CENTER).is_none(), "the middle button");
        assert!(Dial::grab((CENTER.0 + BUTTON - 1.0, CENTER.1)).is_none());
        assert!(Dial::grab((CENTER.0 + RADIUS + 5.0, CENTER.1)).is_none(), "outside the dial");
        assert!(Dial::grab((10.0, 10.0)).is_none());
    }

    #[test]
    fn a_swipe_across_turns_the_pen_round_the_circle() {
        let mut d = Dial::grab((CENTER.0, CENTER.1 - 50.0)).unwrap(); // at the top
        let start = d.pen();
        assert_eq!(start.0, CENTER.0 as i32);
        // every point of the path is on the circle
        let mut x = CENTER.0;
        for _ in 0..40 {
            x += 3.0;
            d.moved(x, 1.5);
            let (px, py) = d.pen();
            let r = ((px as f32 - CENTER.0).powi(2) + (py as f32 - CENTER.1).powi(2)).sqrt();
            assert!((r - PEN_RADIUS).abs() < 1.0, "the pen left the circle: {}", r);
        }
        // right is clockwise: from the top it went round to the right side and on
        assert_ne!(d.pen(), start);
        // the other way with a negative gain: a swipe right turns anticlockwise
        let mut a = Dial::grab((CENTER.0, CENTER.1 - 50.0)).unwrap();
        let mut b = a;
        a.moved(CENTER.0 + 10.0, 1.5);
        b.moved(CENTER.0 + 10.0, -1.5);
        assert!(a.pen().0 > CENTER.0 as i32 && b.pen().0 < CENTER.0 as i32);
    }
}
