//! Where the two screens go in the window, as data: a layout is a size in its own units and a
//! rectangle for each screen (or none). The window fits the layout's size into itself, keeping
//! the proportions. Mods (docs/PLAN.md, stage 12) will bring their own.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub name: &'static str,
    /// The size of the layout's own space.
    pub size: (f32, f32),
    /// The upper screen, then the lower, in that space; `None`: not shown.
    pub screens: [Option<Rect>; 2],
}

fn r(x: f32, y: f32) -> Rect {
    Rect { x, y, w: 256.0, h: 192.0 }
}

/// The DS as it is: the upper screen over the lower.
pub fn stacked() -> Layout {
    Layout { name: "stacked", size: (256.0, 384.0), screens: [Some(r(0.0, 0.0)), Some(r(0.0, 192.0))] }
}

/// The screens side by side.
pub fn side_by_side() -> Layout {
    Layout { name: "side by side", size: (512.0, 192.0), screens: [Some(r(0.0, 0.0)), Some(r(256.0, 0.0))] }
}

/// Only the upper screen.
pub fn top_only() -> Layout {
    Layout { name: "upper only", size: (256.0, 192.0), screens: [Some(r(0.0, 0.0)), None] }
}

/// Only the lower screen.
pub fn bottom_only() -> Layout {
    Layout { name: "lower only", size: (256.0, 192.0), screens: [None, Some(r(0.0, 0.0))] }
}

pub fn all() -> Vec<Layout> {
    vec![stacked(), side_by_side(), top_only(), bottom_only()]
}

impl Layout {
    /// The layout fitted into a window of `win` pixels, centred: each screen's rectangle in
    /// pixels.
    pub fn fit(&self, win: (f32, f32)) -> [Option<Rect>; 2] {
        let k = (win.0 / self.size.0).min(win.1 / self.size.1);
        let (ox, oy) = ((win.0 - self.size.0 * k) / 2.0, (win.1 - self.size.1 * k) / 2.0);
        self.screens.map(|s| s.map(|s| Rect { x: ox + s.x * k, y: oy + s.y * k, w: s.w * k, h: s.h * k }))
    }

    /// A window position as a point of the lower screen (x 0-255, y 0-191), if it is on it.
    pub fn touch(&self, win: (f32, f32), p: (f32, f32), clamp: bool) -> Option<(i32, i32)> {
        let b = self.fit(win)[1]?;
        let (mut u, mut v) = ((p.0 - b.x) / b.w, (p.1 - b.y) / b.h);
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            if !clamp {
                return None;
            }
            u = u.clamp(0.0, 0.999);
            v = v.clamp(0.0, 0.999);
        }
        Some(((u * 256.0) as i32, (v * 192.0) as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_fits_the_window_keeping_proportions() {
        // a window twice as wide as the DS is tall: bars at the sides
        let r = stacked().fit((1000.0, 768.0));
        let (up, down) = (r[0].unwrap(), r[1].unwrap());
        assert_eq!((up.w, up.h), (512.0, 384.0));
        assert_eq!(up.x, 244.0);
        assert_eq!(down.y, 384.0);
    }

    #[test]
    fn touch_maps_the_lower_screen_to_ds_pixels() {
        let l = stacked();
        let win = (512.0, 768.0);
        assert_eq!(l.touch(win, (0.0, 384.0), false), Some((0, 0)));
        assert_eq!(l.touch(win, (511.0, 767.0), false), Some((255, 191)));
        assert_eq!(l.touch(win, (256.0, 576.0), false), Some((128, 96)));
        // on the upper screen: nothing, unless a drag is clamped
        assert_eq!(l.touch(win, (256.0, 100.0), false), None);
        assert_eq!(l.touch(win, (256.0, 100.0), true), Some((128, 0)));
    }

    #[test]
    fn a_layout_without_the_lower_screen_has_no_touch() {
        assert_eq!(top_only().touch((256.0, 192.0), (10.0, 10.0), true), None);
    }
}
