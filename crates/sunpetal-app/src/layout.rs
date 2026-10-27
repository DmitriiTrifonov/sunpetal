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
    /// Parts of a screen drawn elsewhere on the layout, over the screens.
    pub patches: Vec<Patch>,
}

/// A part of a screen's picture shown at another place: `src` in that screen's pixels (256x192),
/// `dst` in the layout's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub screen: usize,
    pub src: Rect,
    pub dst: Rect,
}

/// The counter in the lower screen's top left corner (the map's number) and where it goes on the
/// upper screen when only that is shown.
pub fn counter_patch() -> Patch {
    Patch { screen: 1, src: Rect { x: 4.0, y: 7.0, w: 60.0, h: 13.0 }, dst: Rect { x: 4.0, y: 4.0, w: 60.0, h: 13.0 } }
}

fn r(x: f32, y: f32) -> Rect {
    Rect { x, y, w: 256.0, h: 192.0 }
}

/// The DS as it is: the upper screen over the lower.
pub fn stacked() -> Layout {
    Layout { name: "stacked", size: (256.0, 384.0), screens: [Some(r(0.0, 0.0)), Some(r(0.0, 192.0))], patches: Vec::new() }
}

/// The screens side by side.
pub fn side_by_side() -> Layout {
    Layout { name: "side by side", size: (512.0, 192.0), screens: [Some(r(0.0, 0.0)), Some(r(256.0, 0.0))], patches: Vec::new() }
}

/// Only the upper screen.
pub fn top_only() -> Layout {
    Layout { name: "upper only", size: (256.0, 192.0), screens: [Some(r(0.0, 0.0)), None], patches: Vec::new() }
}

/// Only the lower screen.
pub fn bottom_only() -> Layout {
    Layout { name: "lower only", size: (256.0, 192.0), screens: [None, Some(r(0.0, 0.0))], patches: Vec::new() }
}

/// One screen, the one the player picks (`lower`); the hotkey, the button and the menu swap it.
pub fn single(lower: bool) -> Layout {
    Layout { name: "single", size: (256.0, 192.0), screens: if lower { [None, Some(r(0.0, 0.0))] } else { [Some(r(0.0, 0.0)), None] }, patches: Vec::new() }
}

/// The upper screen, and the lower one as a panel in the corner over it while `panel` is open (as
/// the PS2 version shows its device: docs/RECON-SCREENS.md). The lower screen is drawn after the
/// upper, so it is on top; touches land on the panel.
pub fn overlay(panel: bool) -> Layout {
    let (w, h) = (256.0 * PANEL, 192.0 * PANEL);
    let lower = Rect { x: 256.0 - w - PANEL_GAP, y: 192.0 - h - PANEL_GAP, w, h };
    Layout { name: "overlay", size: (256.0, 192.0), screens: [Some(r(0.0, 0.0)), panel.then_some(lower)], patches: Vec::new() }
}

/// The panel's size as a part of a screen, and its gap to the corner, in screen pixels.
const PANEL: f32 = 0.6;
const PANEL_GAP: f32 = 6.0;

pub fn all() -> Vec<Layout> {
    vec![stacked(), side_by_side(), single(false), overlay(false), top_only(), bottom_only()]
}

/// The layout a setting names; `single` shows the lower screen if `lower`, `overlay` the panel if
/// `panel`.
pub fn find(layouts: &[Layout], v: &crate::config::Video) -> Layout {
    let mut l = match v.layout.as_str() {
        "single" => single(v.lower),
        "overlay" => overlay(v.panel),
        name => layouts.iter().find(|l| l.name == name).unwrap_or(&layouts[0]).clone(),
    };
    // the number of the lower screen goes on the upper one when that is all there is
    if v.counter && l.screens[0].is_some() && l.screens[1].is_none() {
        l.patches.push(counter_patch());
    }
    l
}

impl Layout {
    /// The layout fitted into a window of `win` pixels, centred: each screen's rectangle in
    /// pixels.
    pub fn fit(&self, win: (f32, f32)) -> [Option<Rect>; 2] {
        let k = (win.0 / self.size.0).min(win.1 / self.size.1);
        let (ox, oy) = ((win.0 - self.size.0 * k) / 2.0, (win.1 - self.size.1 * k) / 2.0);
        self.screens.map(|s| s.map(|s| Rect { x: ox + s.x * k, y: oy + s.y * k, w: s.w * k, h: s.h * k }))
    }

    /// A rectangle of the layout's space fitted into the window like the screens.
    pub fn fit_rect(&self, win: (f32, f32), r: Rect) -> Rect {
        let k = (win.0 / self.size.0).min(win.1 / self.size.1);
        let (ox, oy) = ((win.0 - self.size.0 * k) / 2.0, (win.1 - self.size.1 * k) / 2.0);
        Rect { x: ox + r.x * k, y: oy + r.y * k, w: r.w * k, h: r.h * k }
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

    fn video(layout: &str, lower: bool, panel: bool, counter: bool) -> crate::config::Video {
        crate::config::Video { layout: layout.into(), lower, panel, counter, ..Default::default() }
    }

    #[test]
    fn the_single_layout_shows_the_screen_picked() {
        let all = all();
        assert_eq!(find(&all, &video("single", false, false, false)).screens.map(|s| s.is_some()), [true, false]);
        assert_eq!(find(&all, &video("single", true, false, false)).screens.map(|s| s.is_some()), [false, true]);
        assert_eq!(find(&all, &video("nonsense", true, true, false)).name, "stacked");
        // the picked lower screen takes touch
        assert!(single(true).touch((256.0, 192.0), (10.0, 10.0), false).is_some());
        assert!(single(false).touch((256.0, 192.0), (10.0, 10.0), false).is_none());
    }

    #[test]
    fn the_counter_goes_on_the_upper_screen_only_when_it_is_alone() {
        let all = all();
        assert_eq!(find(&all, &video("single", false, false, true)).patches, vec![counter_patch()]);
        assert_eq!(find(&all, &video("overlay", false, false, true)).patches.len(), 1);
        // not with the panel open, the lower screen showing, both screens, or the setting off
        assert!(find(&all, &video("overlay", false, true, true)).patches.is_empty());
        assert!(find(&all, &video("single", true, false, true)).patches.is_empty());
        assert!(find(&all, &video("stacked", false, false, true)).patches.is_empty());
        assert!(find(&all, &video("single", false, false, false)).patches.is_empty());
        // fitted like the screens: the corner of a window twice the size is at twice the offset
        let l = find(&all, &video("single", false, false, true));
        let d = l.fit_rect((512.0, 384.0), l.patches[0].dst);
        assert_eq!((d.x, d.y, d.w), (8.0, 8.0, 120.0));
    }

    #[test]
    fn the_panel_sits_in_the_corner_and_takes_the_touch() {
        assert_eq!(overlay(false).screens.map(|s| s.is_some()), [true, false]);
        let l = overlay(true);
        let win = (512.0, 384.0);
        let [up, panel] = l.fit(win);
        let (up, panel) = (up.unwrap(), panel.unwrap());
        assert_eq!((up.w, up.h), (512.0, 384.0));
        // in the lower right corner, inside the window, smaller than the upper screen
        assert!(panel.x + panel.w < 512.0 && panel.y + panel.h < 384.0 && panel.x > 512.0 * 0.3 && panel.y > 384.0 * 0.3);
        assert!(panel.w < up.w);
        // the panel's middle is the lower screen's middle; the rest of the window is no touch
        let mid = (panel.x + panel.w / 2.0, panel.y + panel.h / 2.0);
        let (x, y) = l.touch(win, mid, false).unwrap();
        assert!((127..=128).contains(&x) && (95..=96).contains(&y));
        assert_eq!(l.touch(win, (10.0, 10.0), false), None);
        assert_eq!(overlay(false).touch(win, mid, false), None);
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
