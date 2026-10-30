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
    /// How the layout fills the window.
    pub fit: Fit,
}

/// How the layout is put into a window that is not its own shape.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Mode {
    /// All of it, in proportion, with bars where the window is longer.
    #[default]
    Fit,
    /// As `Fit`, at a whole number of times the DS's pixels when there is room (sharp pixels).
    Integer,
    /// In proportion, the window filled and what does not fit cut off.
    Fill,
    /// The window filled, out of proportion.
    Stretch,
}

impl Mode {
    /// The mode a setting names; `Fit` for anything else.
    pub fn from_name(name: &str) -> Mode {
        match name {
            "integer" => Mode::Integer,
            "fill" => Mode::Fill,
            "stretch" => Mode::Stretch,
            _ => Mode::Fit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub mode: Mode,
    /// The width of a pixel over its height: 1 for square ones; the DS's own screen is a little
    /// narrower, and a wide window may want more.
    pub par: f32,
}

impl Default for Fit {
    fn default() -> Fit {
        Fit { mode: Mode::Fit, par: 1.0 }
    }
}

/// A part of a screen's picture shown at another place: `src` in that screen's pixels (256x192),
/// `dst` in the layout's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub screen: usize,
    pub src: Rect,
    pub dst: Rect,
    /// Stays at the layout's own edge (the left, or the right if it lies in the right half) when the upper
    /// screen is widened, instead of keeping its place on the 256 in the middle.
    pub pinned: bool,
    /// Only the circle in the rectangle (an icon).
    pub round: bool,
}

/// The studio logos at the start are on both screens at once; with one screen shown they come in
/// turn: which screen to show at this DS frame (`Some(true)`: the lower), or none of the logos.
/// The frames are those of the European ROM's start (measured: upper Rising Star Games, lower
/// the Grasshopper crest then h.a.n.d., then Marvelous above Act Imagine).
pub fn logo_screen(vblank: u64) -> Option<bool> {
    const TURNS: [(u64, u64, bool); 5] = [(230, 320, false), (320, 560, true), (560, 700, false), (700, 800, true), (800, 800, false)];
    TURNS.iter().find(|(from, to, _)| (*from..*to).contains(&vblank)).map(|t| t.2)
}

/// Whether the lower screen's picture has the counter where it is cut from: a dark box with bright
/// digits (`rgba`: both screens, the upper first, `scale` times 256x192 each). A black corner (the
/// title) or a plain one (a menu) has none, and nothing is cut from it. The box has a lit rim (two
/// rows lit across it): the header band of a list (dark, light letters) has none and is no counter.
pub fn counter_in(scale: usize, rgba: &[u8]) -> bool {
    let s = counter_patch().src;
    let (w, top) = (256 * scale, 192 * scale);
    let (mut n, mut dark, mut bright, mut rim) = (0usize, 0usize, 0usize, 0usize);
    for y in (s.y as usize * scale..(s.y + s.h) as usize * scale).step_by(scale.max(1)) {
        let mut row = 0usize;
        for x in (s.x as usize * scale..(s.x + s.w) as usize * scale).step_by(scale.max(1)) {
            let o = ((top + y) * w + x) * 4;
            let Some(p) = rgba.get(o..o + 3) else { return false };
            let sum = p[0] as u32 + p[1] as u32 + p[2] as u32;
            n += 1;
            dark += (sum < 150) as usize;
            bright += (sum > 600) as usize;
            row += (sum > 600) as usize;
        }
        rim += (row * 10 >= s.w as usize * 7) as usize;
    }
    n > 0 && dark * 10 >= n * 5 && bright * 100 >= n * 8 && rim >= 2
}

/// The counter in the lower screen's top left corner (the map's number) and where it goes on the
/// upper screen when only that is shown.
pub fn counter_patch() -> Patch {
    Patch { screen: 1, src: Rect { x: 4.0, y: 7.0, w: 59.0, h: 13.0 }, dst: Rect { x: 4.0, y: 4.0, w: 59.0, h: 13.0 }, pinned: true, round: false }
}

/// The icon in the lower screen's bottom right corner that shows when something can be acted on (a
/// black disc in a red ring), and where it goes on the upper screen when only that is shown.
pub fn hint_patch() -> Patch {
    Patch { screen: 1, src: Rect { x: 163.0, y: 137.0, w: 39.0, h: 39.0 }, dst: Rect { x: 224.0, y: 4.0, w: 28.0, h: 28.0 }, pinned: true, round: true }
}

/// Whether the lower screen shows that icon: its red ring (15% of the square on the screen measured, none
/// without it) is there.
pub fn hint_in(scale: usize, rgba: &[u8]) -> bool {
    let s = hint_patch().src;
    let (w, top) = (256 * scale, 192 * scale);
    let (mut n, mut red) = (0usize, 0usize);
    for y in (s.y as usize * scale..(s.y + s.h) as usize * scale).step_by(scale.max(1)) {
        for x in (s.x as usize * scale..(s.x + s.w) as usize * scale).step_by(scale.max(1)) {
            let o = ((top + y) * w + x) * 4;
            let Some(p) = rgba.get(o..o + 3) else { return false };
            n += 1;
            red += (p[0] > 190 && p[1] < 80 && p[2] < 120) as usize;
        }
    }
    n > 0 && red * 100 >= n * 8
}

/// Which side the dialogue window's portrait is on (the game's setting): the window's red divider is a column
/// of the upper screen's band (x 48-49 with the portrait on the left, 206-207 on the right); `None` if neither is there.
pub fn portrait_left(scale: usize, rgba: &[u8]) -> Option<bool> {
    let w = 256 * scale;
    let red_at = |x: usize| {
        let mut n = 0;
        let mut y = 150 * scale;
        while y < 186 * scale {
            let o = (y * w + x * scale) * 4;
            if let Some(p) = rgba.get(o..o + 3) {
                n += (p[0] > 170 && p[1] < 90 && p[2] < 90) as usize;
            }
            y += scale.max(1);
        }
        n >= 30
    };
    match (red_at(48), red_at(206)) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// The dialogue panel's vertical middle (DS pixels): the panel's top edge is a red line across the upper screen (the
/// line is at 144 in the usual box, lower in the narrow one for two lines) and the panel runs to the screen's foot.
pub fn dialogue_panel_middle(scale: usize, rgba: &[u8]) -> Option<f32> {
    let w = 256 * scale;
    for y in 140..190usize {
        let (mut n, mut red) = (0, 0);
        for x in (20..236usize).step_by(4) {
            let o = (y * scale * w + x * scale) * 4;
            let Some(p) = rgba.get(o..o + 3) else { return None };
            n += 1;
            red += (p[0] > 200 && p[1] < 60 && p[2] < 60) as usize;
        }
        if red * 10 >= n * 8 {
            return Some((y as f32 + 2.0 + 192.0) / 2.0);
        }
    }
    None
}

/// Whether the red wipe that opens and closes the dialogue panel covers the row of text (DS pixel row `y`): the wipe is
/// a solid red band across it, the panel's own red lines are a column or two (under a tenth of the row).
pub fn wipe_over_row(scale: usize, rgba: &[u8], y: f32) -> bool {
    let w = 256 * scale;
    let row = (y as usize).min(191) * scale;
    let (mut n, mut red) = (0, 0);
    for x in (0..256usize).step_by(2) {
        let o = (row * w + x * scale) * 4;
        let Some(p) = rgba.get(o..o + 3) else { return false };
        n += 1;
        red += (p[0] > 200 && p[1] < 60 && p[2] < 60) as usize;
    }
    red * 10 >= n
}

/// Whether the picture is the collection's list: a black header band on the upper screen (the card above) and the list's
/// yellow squares down the lower screen's left side (56 rows lit on the game's frames, none on any other screen).
pub fn collection_in(scale: usize, rgba: &[u8]) -> bool {
    let (w, top) = (256 * scale, 192 * scale);
    if rgba.len() < w * top * 2 * 4 {
        return false;
    }
    let (mut n, mut dark) = (0usize, 0usize);
    for y in 0..10 {
        for x in (0..256).step_by(2) {
            let o = (y * scale * w + x * scale) * 4;
            n += 1;
            dark += ((rgba[o] as u32 + rgba[o + 1] as u32 + rgba[o + 2] as u32) < 90) as usize;
        }
    }
    let rows = (80..190)
        .filter(|&y| {
            (8..18).filter(|&x| {
                let o = ((top + y * scale) * w + x * scale) * 4;
                rgba[o] > 200 && rgba[o + 1] > 170 && rgba[o + 2] < 110
            }).count() >= 3
        })
        .count();
    dark * 2 >= n && rows >= 20
}

fn r(x: f32, y: f32) -> Rect {
    Rect { x, y, w: 256.0, h: 192.0 }
}

/// The DS as it is: the upper screen over the lower.
pub fn stacked() -> Layout {
    Layout { name: "stacked", size: (256.0, 384.0), screens: [Some(r(0.0, 0.0)), Some(r(0.0, 192.0))], patches: Vec::new(), fit: Fit::default() }
}

/// The screens side by side.
pub fn side_by_side() -> Layout {
    Layout { name: "side by side", size: (512.0, 192.0), screens: [Some(r(0.0, 0.0)), Some(r(256.0, 0.0))], patches: Vec::new(), fit: Fit::default() }
}

/// Only the upper screen.
pub fn top_only() -> Layout {
    Layout { name: "upper only", size: (256.0, 192.0), screens: [Some(r(0.0, 0.0)), None], patches: Vec::new(), fit: Fit::default() }
}

/// Only the lower screen.
pub fn bottom_only() -> Layout {
    Layout { name: "lower only", size: (256.0, 192.0), screens: [None, Some(r(0.0, 0.0))], patches: Vec::new(), fit: Fit::default() }
}

/// One screen, the one the player picks (`lower`); the hotkey, the button and the menu swap it.
pub fn single(lower: bool) -> Layout {
    Layout { name: "single", size: (256.0, 192.0), screens: if lower { [None, Some(r(0.0, 0.0))] } else { [Some(r(0.0, 0.0)), None] }, patches: Vec::new(), fit: Fit::default() }
}

/// The upper screen, and the lower one as a panel in the corner over it while `panel` is open (as
/// the PS2 version shows its device: docs/RECON-SCREENS.md). The lower screen is drawn after the
/// upper, so it is on top; touches land on the panel.
pub fn overlay(panel: bool) -> Layout {
    panel_at(panel, PANEL)
}

/// The dial puzzle's panel: smaller, so that it stays under the row of digits (to y 97 of the upper screen, with
/// four of them) and the answer's text above it.
pub fn puzzle_panel() -> Layout {
    panel_at(true, PUZZLE_PANEL)
}

/// Whether the upper screen has the puzzle's LED red or yellow (its digits and their frames, the question) where
/// the puzzle's panel would be: the panel would cover them.
pub fn led_under_panel(scale: usize, rgba: &[u8]) -> bool {
    let Some(p) = puzzle_panel().screens[1] else { return false };
    let w = 256 * scale;
    let mut n = 0;
    for y in (p.y as usize * scale..(p.y + p.h) as usize * scale).step_by(scale.max(1)) {
        for x in (p.x as usize * scale..(p.x + p.w) as usize * scale).step_by(scale.max(1)) {
            let Some(c) = rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3) else { return false };
            let (r, g, b) = (c[0] as u32, c[1] as u32, c[2] as u32);
            n += (r > 170 && g < 70 && b < 90 || r > 200 && g > 180 && b < 90) as usize;
        }
    }
    n >= 6
}

fn panel_at(panel: bool, size: f32) -> Layout {
    let (w, h) = (256.0 * size, 192.0 * size);
    let lower = Rect { x: 256.0 - w - PANEL_GAP, y: 192.0 - h - PANEL_GAP, w, h };
    Layout { name: "overlay", size: (256.0, 192.0), screens: [Some(r(0.0, 0.0)), panel.then_some(lower)], patches: Vec::new(), fit: Fit::default() }
}

/// The panel's size as a part of a screen, and its gap to the corner, in screen pixels.
const PANEL: f32 = 0.6;
const PUZZLE_PANEL: f32 = 0.45;
const PANEL_GAP: f32 = 6.0;

pub fn all() -> Vec<Layout> {
    vec![stacked(), side_by_side(), single(false), overlay(false), top_only(), bottom_only()]
}

/// The layout a setting names; `single` shows the lower screen if `lower`, `overlay` the panel if
/// `panel`.
/// The colour all the edges of the picture have, when the layout shows one screen and its edge is
/// the same colour all round (a studio logo on white): the bars of a window of another shape are
/// painted so, and the picture seems to run on. `None` for a picture whose edges differ (a scene:
/// the glow behind it is used) or a layout with two screens.
pub fn edge_color(layout: &Layout, scale: usize, rgba: &[u8]) -> Option<[f32; 3]> {
    let s = match layout.screens {
        [Some(_), None] => 0,
        [None, Some(_)] => 1,
        _ => return None,
    };
    let (w, h) = (256 * scale, 192 * scale);
    let base = s * w * h;
    let at = |x: usize, y: usize| rgba.get((base + y * w + x) * 4..(base + y * w + x) * 4 + 3).map(|p| [p[0], p[1], p[2]]);
    let mut points = Vec::new();
    for i in (0..w).step_by(scale.max(1) * 4) {
        points.extend([at(i, 0)?, at(i, h - 1)?]);
    }
    for j in (0..h).step_by(scale.max(1) * 4) {
        points.extend([at(0, j)?, at(w - 1, j)?]);
    }
    let mean: [f32; 3] = std::array::from_fn(|c| points.iter().map(|p| p[c] as f32).sum::<f32>() / points.len() as f32);
    let flat = points.iter().all(|p| (0..3).all(|c| (p[c] as f32 - mean[c]).abs() <= 10.0));
    flat.then(|| mean.map(|m| m / 255.0))
}

/// The hint text of Edo's screens (what the chosen key does), cut from the upper screen and put
/// into the free corner of the lower one, whose keys are the screen's use. The cut starts at 83: the
/// Russian text of this screen takes four lines and ends at 143 (the cut was 80..140 and lost the last).
pub fn edo_patch() -> Patch {
    Patch { screen: 0, src: Rect { x: 106.0, y: 83.0, w: 148.0, h: 60.0 }, dst: Rect { x: 106.0, y: 130.0, w: 148.0, h: 60.0 }, pinned: false, round: false }
}

/// Whether the pink of the title ("Hello!! SAVE DATA") reaches into the hint's place on the upper screen (the
/// title moves in the confirmation screens): a cut of that place would show the lower edge of its letters.
pub fn edo_title_in_hint(scale: usize, rgba: &[u8]) -> bool {
    let (w, s) = (256 * scale, edo_patch().src);
    let (mut n, mut pink) = (0usize, 0usize);
    for y in (s.y as usize * scale..(s.y + s.h) as usize * scale).step_by(scale.max(1)) {
        for x in (s.x as usize * scale..(s.x + s.w) as usize * scale).step_by(scale.max(1)) {
            let Some(p) = rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3) else { return false };
            n += 1;
            pink += (p[0] > 200 && p[1] < 90 && p[2] > 70 && p[2] < 200) as usize;
        }
    }
    n > 0 && pink * 1000 >= n * 3
}

/// Whether the upper screen is one of Edo's (the pink title "Hello!! SAVE DATA" in its top band).
pub fn edo_in(scale: usize, rgba: &[u8]) -> bool {
    let w = 256 * scale;
    let (mut n, mut pink) = (0usize, 0usize);
    for y in (10 * scale..80 * scale).step_by(scale.max(1)) {
        for x in (0..w).step_by(scale.max(1)) {
            let Some(p) = rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3) else { return false };
            n += 1;
            pink += (p[0] > 200 && p[1] < 90 && p[2] > 70 && p[2] < 200) as usize;
        }
    }
    n > 0 && pink * 100 >= n * 5
}

/// How many DS pixels the upper screen's 3D view must be wider by on each side for a window of
/// `win` pixels with pixels `par` wide: the window's shape over the DS's 4:3, in the DS's pixels,
/// at most `max`; 0 for a window as tall as it is wide or narrower.
pub fn wide_each_side(win: (f32, f32), par: f32, max: usize) -> usize {
    let aspect = win.0 / win.1.max(1.0) / par.clamp(0.25, 4.0);
    let e = ((aspect * 192.0 - 256.0) / 2.0).ceil();
    if e.is_finite() && e >= 2.0 { (e as usize).min(max) } else { 0 }
}

/// The title's "Press START" box on the lower screen (a blue box with a white rim on black, the
/// words in the language of the game): where it is, in the lower screen's pixels, with its rim. The
/// upper screen has the film at that time; the box is put over it where it is.
pub fn prompt_box(scale: usize, rgba: &[u8]) -> Option<Rect> {
    let (w, h) = (256 * scale, 192 * scale);
    let top = h;
    if rgba.len() < w * h * 2 * 4 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1, mut blue, mut lit) = (usize::MAX, usize::MAX, 0usize, 0usize, 0usize, 0usize);
    for y in (0..h).step_by(scale.max(1)) {
        for x in (0..w).step_by(scale.max(1)) {
            let o = ((top + y) * w + x) * 4;
            let (r, g, b) = (rgba[o] as i32, rgba[o + 1] as i32, rgba[o + 2] as i32);
            if r + g + b > 60 {
                lit += 1;
            }
            if b >= 100 && r < 60 && b >= g + 40 {
                blue += 1;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    // a box: enough blue, a compact area, and nothing else lit on the screen but its rim
    if blue < 300 || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let area = ((x1 - x0) / scale.max(1) + 1) * ((y1 - y0) / scale.max(1) + 1);
    if blue * 100 < area * 50 || lit > blue * 2 + 400 {
        return None;
    }
    let k = scale.max(1) as f32;
    let rim = 4.0;
    let (x0, y0, x1, y1) = (x0 as f32 / k - rim, y0 as f32 / k - rim, x1 as f32 / k + 1.0 + rim, y1 as f32 / k + 1.0 + rim);
    let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(256.0), y1.min(192.0));
    Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// What the picture says now, for the layouts that depend on it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Live {
    /// The DS frame.
    pub vblank: u64,
    /// The lower screen has the map's counter (`counter_in`).
    pub counter_seen: bool,
    /// The lower screen shows the action icon (`hint_in`).
    pub hint_seen: bool,
    /// A puzzle that needs both screens is on (the dial: the number it shows and the answer's animation are above).
    pub both: bool,
    /// ... and its LED digits are where the panel would be (`led_under_panel`), so both go side by side.
    pub both_side: bool,
    /// The collection's list is on: `Some(true)` with its card (the upper screen) opened, `Some(false)` on the list.
    pub collection: Option<bool>,
    /// The lower screen is the one to show (`lower_wanted`, settled).
    pub lower: bool,
    /// The upper screen is Edo's (`edo_in`).
    pub edo: bool,
    /// The title's "Press START" box on the lower screen (`prompt_box`).
    pub prompt: Option<Rect>,
    /// The upper screen is this many DS pixels wider on each side (a widened 3D view).
    pub wide: f32,
    /// A page of paper spread over both screens (`paper_in`), scrolled this far down (DS pixels, 0 to 192).
    pub scroll: Option<f32>,
}

/// Whether the lower screen is a sheet of paper (the lost-and-found report, a card of the list: a tan
/// page, its text in dark), whose first lines are on the upper screen. With one screen shown, the page is
/// scrolled over the two.
pub fn paper_in(scale: usize, rgba: &[u8]) -> bool {
    let (w, top) = (256 * scale, 192 * scale);
    if rgba.len() < w * top * 2 * 4 {
        return false;
    }
    let (mut n, mut paper) = (0usize, 0usize);
    for y in (0..192).step_by(4) {
        for x in (0..256).step_by(4) {
            let o = (((top + y * scale) * w) + x * scale) * 4;
            let p = &rgba[o..o + 3];
            n += 1;
            paper += ((p[0] as i32 - 226).abs() <= 12 && (p[1] as i32 - 193).abs() <= 12 && (p[2] as i32 - 135).abs() <= 14) as usize;
        }
    }
    paper * 100 >= n * 35
}

/// The two screens as one page 384 high, `scroll` pixels down, seen through a 256x192 window: the upper
/// screen's part, then the lower's. Touches fall on the lower screen's part (`Layout::touch`).
pub fn scrolled(scroll: f32) -> Layout {
    let s = scroll.clamp(0.0, 192.0);
    let mut patches = Vec::new();
    if s < 192.0 {
        patches.push(Patch { screen: 0, src: Rect { x: 0.0, y: s, w: 256.0, h: 192.0 - s }, dst: Rect { x: 0.0, y: 0.0, w: 256.0, h: 192.0 - s }, pinned: false, round: false });
    }
    if s > 0.0 {
        patches.push(Patch { screen: 1, src: Rect { x: 0.0, y: 0.0, w: 256.0, h: s }, dst: Rect { x: 0.0, y: 192.0 - s, w: 256.0, h: s }, pinned: false, round: false });
    }
    Layout { name: "single", size: (256.0, 192.0), screens: [None, None], patches, fit: Fit::default() }
}

/// Which screen the game wants the player at, from the lower screen's picture: the lower, when it
/// has something on it that is not the map (a menu, a list, the case, a puzzle: the lower screen
/// is the touch screen); the upper, when it shows the map with the counter (a scene, a walk, a
/// drive) or is black (the title, a film). `None` for a picture too short to tell.
pub fn lower_wanted(scale: usize, rgba: &[u8]) -> Option<bool> {
    let (w, top) = (256 * scale, 192 * scale);
    if rgba.len() < w * top * 2 * 4 {
        return None;
    }
    let (mut n, mut lit) = (0usize, 0usize);
    for y in (0..192).step_by(4) {
        for x in (0..256).step_by(4) {
            let o = (((top + y * scale) * w) + x * scale) * 4;
            n += 1;
            lit += (rgba[o] as u32 + rgba[o + 1] as u32 + rgba[o + 2] as u32 > 60) as usize;
        }
    }
    let content = lit * 100 >= n * 10;
    // the title's prompt is lit but is no menu: the upper screen is what is looked at
    Some(content && !counter_in(scale, rgba) && prompt_box(scale, rgba).is_none())
}

pub fn find(layouts: &[Layout], v: &crate::config::Video, live: &Live) -> Layout {
    let mut l = pick(layouts, v, live);
    if live.wide > 0.0 {
        l.widen(live.wide);
    }
    l.fit = Fit { mode: Mode::from_name(&v.fit), par: v.pixel_aspect };
    l
}

fn pick(layouts: &[Layout], v: &crate::config::Video, live: &Live) -> Layout {
    let (vblank, counter_seen) = (live.vblank, live.counter_seen);
    // one screen shown: the logos come in turn
    if v.logos_in_turn && matches!(v.layout.as_str(), "single" | "overlay") {
        if let Some(lower) = logo_screen(vblank) {
            return single(lower);
        }
    }
    if let (Some(sc), "single", true) = (live.scroll, v.layout.as_str(), v.auto) {
        return scrolled(sc);
    }
    // the collection on one screen: the list, then the card of the item taken (A), back to the list (B)
    if let (Some(card), "single", true) = (live.collection, v.layout.as_str(), v.auto) {
        return single(!card);
    }
    // a puzzle read on the upper screen and worked on the lower one: both (the PS2's way, the lower as a panel, or side by side)
    if live.both && v.auto && matches!(v.layout.as_str(), "single" | "overlay") {
        return if v.puzzle == "side" || live.both_side { side_by_side() } else { puzzle_panel() };
    }
    let mut l = match v.layout.as_str() {
        "single" => single(if v.auto { live.lower } else { v.lower }),
        "overlay" => overlay(v.panel),
        name => layouts.iter().find(|l| l.name == name).unwrap_or(&layouts[0]).clone(),
    };
    // the title: the upper screen's film with the lower screen's "Press START" over it
    if let (Some(b), [Some(_), None]) = (live.prompt, l.screens) {
        l.patches.push(Patch { screen: 1, src: b, dst: b, pinned: false, round: false });
    }
    // Edo's screen on one: the keys of the lower screen, with the upper's hint text beside them
    if live.edo && l.screens[0].is_none() && l.screens[1].is_some() && l.patches.is_empty() {
        l.patches.push(edo_patch());
    }
    // the number of the lower screen goes on the upper one when that is all there is
    if v.counter && counter_seen && l.screens[0].is_some() && l.screens[1].is_none() {
        l.patches.push(counter_patch());
        // and the icon of what can be acted on, beside it
        if live.hint_seen {
            l.patches.push(hint_patch());
        }
    }
    l
}

impl Layout {
    /// The layout fitted into a window of `win` pixels, centred: each screen's rectangle in
    /// pixels.
    pub fn fit(&self, win: (f32, f32)) -> [Option<Rect>; 2] {
        self.screens.map(|s| s.map(|s| self.fit_rect(win, s)))
    }

    /// The horizontal and vertical scale of the layout's space in the window, and where its corner
    /// is (the layout is centred; cut off where it is larger than the window).
    fn placing(&self, win: (f32, f32)) -> (f32, f32, f32, f32) {
        let par = self.fit.par.clamp(0.25, 4.0);
        let (ew, eh) = (self.size.0 * par, self.size.1);
        let (kx, ky) = match self.fit.mode {
            Mode::Stretch => (win.0 / self.size.0, win.1 / self.size.1),
            mode => {
                let k = match mode {
                    Mode::Fill => (win.0 / ew).max(win.1 / eh),
                    _ => (win.0 / ew).min(win.1 / eh),
                };
                let k = if mode == Mode::Integer && k >= 1.0 { k.floor() } else { k };
                (k * par, k)
            }
        };
        (kx, ky, (win.0 - self.size.0 * kx) / 2.0, (win.1 - self.size.1 * ky) / 2.0)
    }

    /// The upper screen `e` pixels wider on each side: the layout's space grows with it, what is
    /// beside the upper screen moves out, what is below it stays in the middle, what lies over it
    /// keeps its corner, and the patches that lie on it keep their place on the 256 in the middle.
    pub fn widen(&mut self, e: f32) {
        let Some(up) = self.screens[0] else { return };
        let new_up = Rect { w: up.w + 2.0 * e, ..up };
        if let Some(lo) = self.screens[1] {
            let right_of = lo.x >= up.x + up.w;
            let below = lo.x == up.x && lo.w == up.w;
            self.screens[1] = Some(Rect { x: lo.x + if right_of { 2.0 * e } else if below { e } else { 2.0 * e }, ..lo });
        }
        self.screens[0] = Some(new_up);
        self.size.0 += 2.0 * e;
        // a pinned patch in the right half keeps the right corner
        for p in self.patches.iter_mut().filter(|p| p.pinned) {
            if p.dst.x + p.dst.w / 2.0 > up.x + up.w / 2.0 && p.dst.y < up.y + up.h {
                p.dst.x += 2.0 * e;
            }
        }
        // a patch on the upper screen lies on the middle 256
        for p in self.patches.iter_mut().filter(|p| !p.pinned) {
            if p.dst.x >= up.x && p.dst.x < up.x + up.w && p.dst.y >= up.y && p.dst.y < up.y + up.h && self.screens[1].map_or(true, |lo| p.dst.y < lo.y || p.dst.x < lo.x) {
                p.dst.x += e;
            }
        }
    }

    /// A rectangle of the layout's space fitted into the window like the screens.
    pub fn fit_rect(&self, win: (f32, f32), r: Rect) -> Rect {
        let (kx, ky, ox, oy) = self.placing(win);
        Rect { x: ox + r.x * kx, y: oy + r.y * ky, w: r.w * kx, h: r.h * ky }
    }

    /// A window position as a point of the lower screen (x 0-255, y 0-191), if it is on it.
    pub fn touch(&self, win: (f32, f32), p: (f32, f32), clamp: bool) -> Option<(i32, i32)> {
        // a scrolled page: the lower screen is a patch
        let page = self.screens[1].is_none().then(|| self.patches.iter().find(|q| q.screen == 1)).flatten();
        let b = match page {
            Some(q) => self.fit_rect(win, q.dst),
            None => self.fit(win)[1]?,
        };
        let (mut u, mut v) = ((p.0 - b.x) / b.w, (p.1 - b.y) / b.h);
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            if !clamp {
                return None;
            }
            u = u.clamp(0.0, 0.999);
            v = v.clamp(0.0, 0.999);
        }
        match page {
            Some(q) => Some(((q.src.x + u * q.src.w) as i32, (q.src.y + v * q.src.h) as i32)),
            None => Some(((u * 256.0) as i32, (v * 192.0) as i32)),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_wipe_is_a_red_band_not_the_panels_red_lines() {
        let paint = |from: usize, to: usize| {
            let mut px = vec![150u8; 256 * 192 * 4];
            for y in 0..192 {
                for x in from..to {
                    px[(y * 256 + x) * 4..][..3].copy_from_slice(&[255, 0, 0]);
                }
            }
            px
        };
        assert!(!super::wipe_over_row(1, &paint(0, 0), 168.0), "no red");
        assert!(!super::wipe_over_row(1, &paint(48, 50), 168.0), "the portrait's frame line is two columns");
        assert!(super::wipe_over_row(1, &paint(0, 120), 168.0), "half the row red");
        assert!(super::wipe_over_row(1, &paint(0, 256), 168.0));
    }
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
        crate::config::Video { layout: layout.into(), lower, panel, counter, auto: false, ..Default::default() }
    }

    fn live(vblank: u64, counter_seen: bool) -> Live {
        Live { vblank, counter_seen, hint_seen: false, both: false, both_side: false, collection: None, lower: false, edo: false, prompt: None, wide: 0.0, scroll: None }
    }

    #[test]
    fn the_single_layout_shows_the_screen_picked() {
        let all = all();
        assert_eq!(find(&all, &video("single", false, false, false), &live(0, true)).screens.map(|s| s.is_some()), [true, false]);
        assert_eq!(find(&all, &video("single", true, false, false), &live(0, true)).screens.map(|s| s.is_some()), [false, true]);
        assert_eq!(find(&all, &video("nonsense", true, true, false), &live(0, true)).name, "stacked");
        // the picked lower screen takes touch
        assert!(single(true).touch((256.0, 192.0), (10.0, 10.0), false).is_some());
        assert!(single(false).touch((256.0, 192.0), (10.0, 10.0), false).is_none());
    }

    #[test]
    fn the_logos_come_in_turn_on_one_screen_and_only_there() {
        let all = all();
        assert_eq!(logo_screen(100), None);
        assert_eq!(logo_screen(250), Some(false));
        assert_eq!(logo_screen(400), Some(true));
        assert_eq!(logo_screen(750), Some(true));
        assert_eq!(logo_screen(2000), None);
        let one = video("single", false, false, true);
        assert_eq!(find(&all, &one, &live(400, true)).screens.map(|s| s.is_some()), [false, true]);
        // no counter patch on a logo, the chosen screen after them, both screens untouched
        assert!(find(&all, &one, &live(250, true)).patches.is_empty());
        assert_eq!(find(&all, &one, &live(2000, true)).screens.map(|s| s.is_some()), [true, false]);
        assert_eq!(find(&all, &video("stacked", false, false, false), &live(400, true)).screens.map(|s| s.is_some()), [true, true]);
        let mut off = video("single", false, false, false);
        off.logos_in_turn = false;
        assert_eq!(find(&all, &off, &live(400, true)).screens.map(|s| s.is_some()), [true, false]);
    }

    /// Both screens, `scale` times 256x192, all black; the lower one's corner drawn by `paint`.
    fn frame(scale: usize, paint: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let (w, h) = (256 * scale, 384 * scale);
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                let p = if y >= 192 * scale { paint(x / scale, y / scale - 192) } else { [0, 0, 0] };
                v[o..o + 3].copy_from_slice(&p);
            }
        }
        v
    }

    #[test]
    fn the_counter_is_found_by_its_dark_box_and_bright_digits() {
        for scale in [1, 2] {
            // a dark box with a lit rim above and below and digits (every third column bright), as the map's
            let counter = frame(scale, |x, y| if (4..64).contains(&x) && (7..20).contains(&y) && (x % 3 == 0 || y == 7 || y == 19) { [255, 255, 255] } else { [10, 20, 30] });
            assert!(counter_in(scale, &counter));
            // a list's header band: dark, light letters (as many as 8% of the box), no rim: not the counter
            let header = frame(scale, |x, y| if (4..64).contains(&x) && (9..16).contains(&y) && x % 3 == 0 { [255, 255, 255] } else { [10, 20, 30] });
            assert!(!counter_in(scale, &header));
            // black (the title), light (a menu), a dark box with no digits
            assert!(!counter_in(scale, &frame(scale, |_, _| [0, 0, 0])));
            assert!(!counter_in(scale, &frame(scale, |_, _| [200, 120, 40])));
            assert!(!counter_in(scale, &frame(scale, |_, _| [10, 20, 30])));
        }
        assert!(!counter_in(1, &[]));
    }

    #[test]
    fn the_lower_screen_is_wanted_for_a_menu_and_not_for_the_map_or_black() {
        for scale in [1, 2] {
            let counter = frame(scale, |x, y| match ((4..64).contains(&x) && (7..20).contains(&y), x % 3 == 0 || y == 7 || y == 19) {
                (true, true) => [255, 255, 255],
                (true, false) => [10, 20, 30],
                _ => [10, 90, 120],
            });
            // the map with its counter: the upper screen
            assert_eq!(lower_wanted(scale, &counter), Some(false));
            // black (the title, a film): the upper
            assert_eq!(lower_wanted(scale, &frame(scale, |_, _| [0, 0, 0])), Some(false));
            // a menu: a lit picture, no counter: the lower
            assert_eq!(lower_wanted(scale, &frame(scale, |_, _| [200, 120, 40])), Some(true));
        }
        assert_eq!(lower_wanted(1, &[]), None);
    }

    #[test]
    fn auto_follows_the_picture_and_the_setting_off_follows_the_choice() {
        let all = all();
        let mut v = video("single", false, false, false);
        v.auto = true;
        let want_lower = Live { lower: true, ..Default::default() };
        assert_eq!(find(&all, &v, &want_lower).screens.map(|s| s.is_some()), [false, true]);
        v.auto = false;
        assert_eq!(find(&all, &v, &want_lower).screens.map(|s| s.is_some()), [true, false]);
    }

    #[test]
    fn edos_screen_is_known_by_its_pink_title() {
        for scale in [1, 2] {
            let title = |p: [u8; 3]| {
                let (w, h) = (256 * scale, 384 * scale);
                let mut v = vec![0u8; w * h * 4];
                for y in 0..192 * scale {
                    for x in 0..w {
                        let on = (x / scale) % 3 != 0 && (20..70).contains(&(y / scale));
                        let o = (y * w + x) * 4;
                        v[o..o + 3].copy_from_slice(&if on { p } else { [0, 0, 0] });
                    }
                }
                v
            };
            assert!(edo_in(scale, &title([240, 20, 130])));
            assert!(!edo_in(scale, &title([20, 90, 240])));
            assert!(!edo_in(scale, &vec![0u8; 256 * 384 * 4 * scale * scale]));
        }
    }

    #[test]
    fn the_start_prompt_is_a_blue_box_alone_on_the_lower_screen() {
        for scale in [1, 2] {
            let (w, h) = (256 * scale, 384 * scale);
            let mut v = vec![0u8; w * h * 4];
            // blue 46..210 x 78..114 on the lower screen, a white rim of 2 pixels
            for y in 192 * scale..h {
                for x in 0..w {
                    let (px, py) = (x / scale, y / scale - 192);
                    let inside = (46..210).contains(&px) && (78..114).contains(&py);
                    let rim = (44..212).contains(&px) && (76..116).contains(&py);
                    let o = (y * w + x) * 4;
                    if inside && px % 4 == 0 && (86..106).contains(&py) {
                        // the words, white on the blue
                        v[o..o + 3].copy_from_slice(&[255, 255, 255]);
                    } else if inside {
                        v[o..o + 3].copy_from_slice(&[0, 150, 255]);
                    } else if rim {
                        v[o..o + 3].copy_from_slice(&[255, 255, 255]);
                    }
                }
            }
            let b = prompt_box(scale, &v).unwrap();
            assert!((b.x - 42.0).abs() <= 2.0 && (b.y - 74.0).abs() <= 2.0 && (b.w - 172.0).abs() <= 4.0 && (b.h - 44.0).abs() <= 4.0, "{:?}", b);
            // wanted: not the lower screen (the film above is looked at)
            assert_eq!(lower_wanted(scale, &v), Some(false));
            // black, a menu (lit all over): no prompt
            assert_eq!(prompt_box(scale, &vec![0u8; w * h * 4]), None);
            let mut menu = v.clone();
            for y in 192 * scale..h {
                for x in 0..w {
                    let o = (y * w + x) * 4;
                    menu[o..o + 3].copy_from_slice(&[200, 120, 40]);
                }
            }
            assert_eq!(prompt_box(scale, &menu), None);
        }
    }

    #[test]
    fn the_prompt_goes_over_the_upper_screen_where_it_is() {
        let all = all();
        let b = Rect { x: 42.0, y: 74.0, w: 172.0, h: 44.0 };
        let live = Live { prompt: Some(b), ..Default::default() };
        let l = find(&all, &video("single", false, false, false), &live);
        assert_eq!(l.patches, vec![Patch { screen: 1, src: b, dst: b, pinned: false, round: false }]);
        // not when the lower screen is the one shown, or both
        assert!(find(&all, &video("single", true, false, false), &live).patches.is_empty());
        assert!(find(&all, &video("stacked", false, false, false), &live).patches.is_empty());
    }

    #[test]
    fn widening_the_upper_screen_moves_the_rest_out_of_its_way() {
        // stacked: the lower screen stays in the middle under the wider upper one
        let mut l = stacked();
        l.widen(64.0);
        assert_eq!(l.size, (384.0, 384.0));
        assert_eq!(l.screens[0].unwrap(), Rect { x: 0.0, y: 0.0, w: 384.0, h: 192.0 });
        assert_eq!(l.screens[1].unwrap(), Rect { x: 64.0, y: 192.0, w: 256.0, h: 192.0 });
        // side by side: the lower screen goes to the right of the wider one
        let mut l = side_by_side();
        l.widen(64.0);
        assert_eq!((l.size.0, l.screens[1].unwrap().x), (640.0, 384.0));
        // the upper alone: just wider; the lower alone: untouched
        let mut l = top_only();
        l.widen(64.0);
        assert_eq!(l.size, (384.0, 192.0));
        let mut l = bottom_only();
        l.widen(64.0);
        assert_eq!(l.size, (256.0, 192.0));
        // the panel keeps the corner, and a patch on the upper screen the 256 in the middle
        let mut l = overlay(true);
        l.patches.push(counter_patch());
        l.widen(64.0);
        let (up, panel) = (l.screens[0].unwrap(), l.screens[1].unwrap());
        assert!((up.x + up.w) - (panel.x + panel.w) < 7.0);
        // the counter stays in the corner of the whole window
        assert_eq!(l.patches[0].dst.x, counter_patch().dst.x);
    }

    #[test]
    fn the_window_shape_asks_for_the_width() {
        // 4:3, taller, a hair wider: none
        assert_eq!(wide_each_side((1024.0, 768.0), 1.0, 170), 0);
        assert_eq!(wide_each_side((500.0, 700.0), 1.0, 170), 0);
        assert_eq!(wide_each_side((1030.0, 768.0), 1.0, 170), 0);
        // 16:9: 192 * 16 / 9 = 341.3 pixels, 43 more each side
        assert_eq!(wide_each_side((1280.0, 720.0), 1.0, 170), 43);
        // a wider pixel needs less
        assert!(wide_each_side((1280.0, 720.0), 1.33, 170) < 5);
        // very wide: held at the most
        assert_eq!(wide_each_side((4000.0, 500.0), 1.0, 170), 170);
    }

    #[test]
    fn edos_hint_comes_to_the_lower_screen_when_that_alone_is_shown() {
        let all = all();
        let mut v = video("single", true, false, false);
        let edo = Live { edo: true, ..Default::default() };
        assert_eq!(find(&all, &v, &edo).patches, vec![edo_patch()]);
        assert!(find(&all, &v, &Live::default()).patches.is_empty());
        // with the upper screen shown, the hint is where it is
        v.lower = false;
        assert!(find(&all, &v, &edo).patches.is_empty());
        assert_eq!(find(&all, &video("stacked", false, false, false), &edo).patches.len(), 0);
    }

    #[test]
    fn a_pinned_patch_on_the_right_goes_to_the_right_corner_when_widened() {
        let mut l = single(false);
        l.patches = vec![counter_patch(), hint_patch()];
        l.widen(40.0);
        assert_eq!(l.patches[0].dst.x, counter_patch().dst.x);
        assert_eq!(l.patches[1].dst.x, hint_patch().dst.x + 80.0);
    }

    #[test]
    fn a_puzzle_with_both_screens_shows_them_side_by_side() {
        let all = all();
        let mut v = video("single", false, false, true);
        v.auto = true;
        let mut lv = live(0, false);
        assert_eq!(find(&all, &v, &lv).name, "single");
        lv.both = true;
        assert_eq!(find(&all, &v, &lv).name, "overlay");
        assert_eq!(find(&all, &v, &lv).screens[1], puzzle_panel().screens[1]);
        lv.both_side = true;
        assert_eq!(find(&all, &v, &lv).name, "side by side");
        lv.both_side = false;
        v.puzzle = "side".into();
        assert_eq!(find(&all, &v, &lv).name, "side by side");
        v.auto = false;
        assert_eq!(find(&all, &v, &lv).name, "single");
    }

    #[test]
    fn led_digits_under_the_puzzle_panel_are_found() {
        let p = puzzle_panel().screens[1].unwrap();
        for scale in [1usize, 2] {
            let w = 256 * scale;
            let mut f = vec![0u8; w * 192 * scale * 4 * 2];
            assert!(!led_under_panel(scale, &f));
            for y in 60..90 {
                for x in 40..90 {
                    let o = (y * scale * w + x * scale) * 4;
                    f[o..o + 3].copy_from_slice(&[230, 20, 40]);
                }
            }
            assert!(!led_under_panel(scale, &f)); // digits above the panel
            let (x, y) = (p.x as usize + 5, p.y as usize + 5);
            let o = (y * scale * w + x * scale) * 4;
            for k in 0..16 {
                f[o + k * 4..o + k * 4 + 3].copy_from_slice(&[230, 20, 40]);
            }
            assert!(led_under_panel(scale, &f));
        }
    }

    #[test]
    fn the_collection_is_one_screen_at_a_time_list_then_card() {
        let all = all();
        let v = crate::config::Video { layout: "single".into(), auto: true, ..Default::default() };
        let mut lv = live(0, false);
        lv.collection = Some(false);
        let l = find(&all, &v, &lv);
        assert!(l.screens[0].is_none() && l.screens[1].is_some(), "the list: the lower screen");
        lv.collection = Some(true);
        let l = find(&all, &v, &lv);
        assert!(l.screens[0].is_some() && l.screens[1].is_none(), "the card: the upper screen");
        // not with the screen chosen by hand
        let manual = crate::config::Video { auto: false, ..v.clone() };
        assert!(find(&all, &manual, &lv).screens[0].is_some());
    }

    #[test]
    fn the_collection_is_found_by_its_yellow_squares_and_black_band() {
        for scale in [1usize, 2] {
            let w = 256 * scale;
            let mut f = vec![0u8; w * 192 * scale * 4 * 2];
            assert!(!collection_in(scale, &f));
            for y in (80..180).step_by(12) {
                for dy in 0..8 {
                    for x in 9..17 {
                        let o = ((192 * scale + (y + dy) * scale) * w + x * scale) * 4;
                        f[o..o + 3].copy_from_slice(&[255, 220, 60]);
                    }
                }
            }
            assert!(collection_in(scale, &f));
            f[..w * 10 * scale * 4].fill(200); // a lit header band: not the collection
            assert!(!collection_in(scale, &f));
        }
    }

    #[test]
    fn the_action_icon_is_found_by_its_red_ring() {
        for scale in [1usize, 2] {
            let icon = frame(scale, |x, y| if (163..202).contains(&x) && (137..176).contains(&y) && (x + y) % 4 == 0 { [220, 20, 60] } else { [60, 90, 150] });
            assert!(hint_in(scale, &icon));
            assert!(!hint_in(scale, &frame(scale, |_, _| [60, 90, 150])));
        }
        assert!(!hint_in(1, &[]));
    }

    #[test]
    fn no_counter_on_the_screen_no_patch() {
        let all = all();
        assert!(find(&all, &video("single", false, false, true), &live(0, false)).patches.is_empty());
    }

    #[test]
    fn the_counter_goes_on_the_upper_screen_only_when_it_is_alone() {
        let all = all();
        assert_eq!(find(&all, &video("single", false, false, true), &live(0, true)).patches, vec![counter_patch()]);
        assert_eq!(find(&all, &video("overlay", false, false, true), &live(0, true)).patches.len(), 1);
        // not with the panel open, the lower screen showing, both screens, or the setting off
        assert!(find(&all, &video("overlay", false, true, true), &live(0, true)).patches.is_empty());
        assert!(find(&all, &video("single", true, false, true), &live(0, true)).patches.is_empty());
        assert!(find(&all, &video("stacked", false, false, true), &live(0, true)).patches.is_empty());
        assert!(find(&all, &video("single", false, false, false), &live(0, true)).patches.is_empty());
        // fitted like the screens: the corner of a window twice the size is at twice the offset
        let l = find(&all, &video("single", false, false, true), &live(0, true));
        let d = l.fit_rect((512.0, 384.0), l.patches[0].dst);
        assert_eq!((d.x, d.y, d.w), (8.0, 8.0, 118.0));
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

    fn with_fit(mut l: Layout, mode: Mode, par: f32) -> Layout {
        l.fit = Fit { mode, par };
        l
    }

    #[test]
    fn the_modes_put_the_picture_in_a_wide_window_each_its_way() {
        let win = (1000.0, 500.0);
        let one = |mode, par| with_fit(top_only(), mode, par).fit(win)[0].unwrap();
        // fit: the height rules (500 / 192), centred
        let r = one(Mode::Fit, 1.0);
        assert!((r.h - 500.0).abs() < 0.01 && (r.w - 500.0 * 256.0 / 192.0).abs() < 0.01 && (r.x - (1000.0 - r.w) / 2.0).abs() < 0.01);
        // integer: 2 times (500 / 192 = 2.6), 512 x 384, centred
        let r = one(Mode::Integer, 1.0);
        assert_eq!((r.w, r.h, r.x, r.y), (512.0, 384.0, 244.0, 58.0));
        // integer in a window smaller than the DS: no whole times, it fits
        let small = with_fit(top_only(), Mode::Integer, 1.0).fit((128.0, 96.0))[0].unwrap();
        assert_eq!((small.w, small.h), (128.0, 96.0));
        // fill: the width rules (1000 / 256), the height runs past the window
        let r = one(Mode::Fill, 1.0);
        assert!((r.w - 1000.0).abs() < 0.01 && r.h > 500.0 && r.y < 0.0);
        // stretch: the window, whatever its shape
        let r = one(Mode::Stretch, 1.0);
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, 1000.0, 500.0));
        // a wider pixel: 4:3 becomes 16:9-ish (1.5 times wider), still in proportion to the window
        let r = one(Mode::Fit, 1.5);
        assert!((r.w / r.h - 256.0 * 1.5 / 192.0).abs() < 0.001);
    }

    #[test]
    fn touch_follows_the_mode() {
        // stretched out of proportion: the middle of the window is the middle of the screen
        let l = with_fit(bottom_only(), Mode::Stretch, 1.0);
        assert_eq!(l.touch((1000.0, 500.0), (500.0, 250.0), false), Some((128, 96)));
        assert_eq!(l.touch((1000.0, 500.0), (999.0, 499.0), false), Some((255, 191)));
        // a layout cut off by fill: the window's corner is not the screen's corner
        let f = with_fit(bottom_only(), Mode::Fill, 1.0);
        let (x, y) = f.touch((1000.0, 500.0), (0.0, 0.0), false).unwrap();
        assert_eq!(x, 0);
        assert!(y > 0, "{}", y);
    }

    #[test]
    fn a_picture_edged_all_round_in_one_colour_gives_its_colour_to_the_bars() {
        for scale in [1, 2] {
            let (w, h) = (256 * scale, 192 * scale);
            let mut v = vec![0u8; w * h * 2 * 4];
            // the upper screen: white, with a dark mark in the middle
            for y in 0..h {
                for x in 0..w {
                    let o = (y * w + x) * 4;
                    let mark = (w / 3..w / 2).contains(&x) && (h / 3..h / 2).contains(&y);
                    v[o..o + 3].copy_from_slice(&if mark { [10, 10, 10] } else { [250, 250, 252] });
                }
            }
            let c = edge_color(&top_only(), scale, &v).unwrap();
            assert!((c[0] - 250.0 / 255.0).abs() < 0.01 && (c[2] - 252.0 / 255.0).abs() < 0.01);
            // the lower screen is black here: its layout gives black
            assert_eq!(edge_color(&bottom_only(), scale, &v), Some([0.0, 0.0, 0.0]));
            // two screens: none; an edge that is not one colour: none
            assert_eq!(edge_color(&stacked(), scale, &v), None);
            v[(h / 2 * w) * 4] = 0;
            v[(h / 2 * w) * 4 + 1] = 0;
            v[(h / 2 * w) * 4 + 2] = 0;
            assert_eq!(edge_color(&top_only(), scale, &v), None);
        }
    }

    #[test]
    fn the_setting_reaches_the_layout() {
        let all = all();
        let mut v = video("single", false, false, false);
        v.fit = "integer".into();
        v.pixel_aspect = 1.25;
        let l = find(&all, &v, &Live::default());
        assert_eq!(l.fit, Fit { mode: Mode::Integer, par: 1.25 });
        v.fit = "nonsense".into();
        assert_eq!(find(&all, &v, &Live::default()).fit.mode, Mode::Fit);
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

    #[test]
    fn a_page_of_paper_is_scrolled_over_both_screens() {
        let mut rgba = vec![0u8; 256 * 192 * 2 * 4];
        assert!(!paper_in(1, &rgba));
        for px in rgba[256 * 192 * 4..].chunks_exact_mut(4) {
            px.copy_from_slice(&[226, 193, 135, 255]);
        }
        assert!(paper_in(1, &rgba));
        // 48 down: 144 rows of the upper screen, then 48 of the lower; a touch at the window's bottom is the lower's row 47
        let l = scrolled(48.0);
        assert_eq!(l.patches.len(), 2);
        let win = (256.0, 192.0);
        assert_eq!(l.touch(win, (10.0, 191.0), false), Some((10, 47)));
        assert_eq!(l.touch(win, (10.0, 100.0), false).map(|p| p.1).filter(|y| *y < 0), None);
        assert_eq!(scrolled(0.0).patches.len(), 1);
        assert_eq!(scrolled(192.0).patches.len(), 1);
    }
}
