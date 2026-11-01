//! Layouts as files: a TOML text for a size, the rectangles of the screens and the patches. The
//! built-in plain layouts are files of the same format (`layouts/*.toml`); a file of the player's
//! (`<config dir>/layouts/*.toml`) is read the same way, and a broken one is reported and left out
//! (the built-in ones stay).
use serde::Deserialize;

use crate::layout::{Fit, Layout, Patch, Rect};

#[derive(Deserialize)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl From<R> for Rect {
    fn from(r: R) -> Rect {
        Rect { x: r.x, y: r.y, w: r.w, h: r.h }
    }
}

#[derive(Deserialize)]
struct P {
    /// 0: the upper screen, 1: the lower.
    screen: usize,
    src: R,
    dst: R,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    round: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: String,
    size: [f32; 2],
    upper: Option<R>,
    lower: Option<R>,
    #[serde(default, rename = "patch")]
    patches: Vec<P>,
}

/// The layout a file's text describes; the error says what is wrong in the player's terms.
pub fn parse(text: &str) -> Result<Layout, String> {
    let f: File = toml::from_str(text).map_err(|e| e.message().to_string())?;
    if f.name.trim().is_empty() {
        return Err("the layout has no name".into());
    }
    if !(f.size[0] > 0.0 && f.size[1] > 0.0) {
        return Err("the size must be positive".into());
    }
    let screens: [Option<Rect>; 2] = [f.upper.map(Rect::from), f.lower.map(Rect::from)];
    if screens.iter().all(Option::is_none) {
        return Err("the layout shows no screen (give `upper` or `lower`)".into());
    }
    for (name, s) in ["upper", "lower"].iter().zip(&screens) {
        if let Some(r) = s {
            if !(r.w > 0.0 && r.h > 0.0) {
                return Err(format!("the {} screen has no area", name));
            }
        }
    }
    let mut patches = Vec::new();
    for p in f.patches {
        if p.screen > 1 {
            return Err("a patch's `screen` is 0 (upper) or 1 (lower)".into());
        }
        patches.push(Patch { screen: p.screen, src: p.src.into(), dst: p.dst.into(), pinned: p.pinned, round: p.round });
    }
    // the name lives as long as the program: a handful of layouts, read once
    let name: &'static str = Box::leak(f.name.into_boxed_str());
    Ok(Layout { name, size: (f.size[0], f.size[1]), screens, patches, fit: Fit::default() })
}

/// The built-in layouts that are files.
pub fn builtin() -> Vec<Layout> {
    [
        include_str!("../layouts/stacked.toml"),
        include_str!("../layouts/side-by-side.toml"),
        include_str!("../layouts/upper-only.toml"),
        include_str!("../layouts/lower-only.toml"),
    ]
    .iter()
    .map(|t| parse(t).expect("a built-in layout file reads"))
    .collect()
}

/// The player's layouts in `dir` (`*.toml`, by name), and the errors of those that do not read.
pub fn load_dir(dir: &std::path::Path) -> (Vec<Layout>, Vec<String>) {
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    let Ok(rd) = std::fs::read_dir(dir) else { return (ok, bad) };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    files.sort();
    for p in files {
        let shown = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| parse(&t)) {
            Ok(l) => ok.push(l),
            Err(e) => bad.push(format!("{}: {}", shown, e)),
        }
    }
    (ok, bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;

    #[test]
    fn a_built_in_layout_is_the_file_it_comes_from() {
        let files = builtin();
        for (file, code) in files.iter().zip([layout::stacked(), layout::side_by_side(), layout::top_only(), layout::bottom_only()]) {
            assert_eq!(file, &code, "{}", code.name);
        }
    }

    /// The panels the game state brings up are geometry too: their files read to the code's
    /// rectangles (the state decides which one shows, in `layout::pick`).
    #[test]
    fn the_panel_files_are_the_codes_geometry() {
        for (text, code) in [
            (include_str!("../layouts/overlay-open.toml"), layout::overlay(true)),
            (include_str!("../layouts/puzzle-panel.toml"), layout::puzzle_panel()),
        ] {
            let file = parse(text).unwrap();
            assert_eq!(file.name, code.name);
            assert_eq!(file.screens[0], code.screens[0]);
            let (a, b) = (file.screens[1].unwrap(), code.screens[1].unwrap());
            for (x, y) in [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)] {
                assert!((x - y).abs() < 0.01, "{:?} {:?}", a, b);
            }
        }
    }

    #[test]
    fn a_patch_and_a_pinned_flag_read() {
        let l = parse(
            "name = \"mine\"\nsize = [256.0, 192.0]\nupper = { x = 0.0, y = 0.0, w = 256.0, h = 192.0 }\n[[patch]]\nscreen = 1\nsrc = { x = 0.0, y = 0.0, w = 40.0, h = 20.0 }\ndst = { x = 10.0, y = 10.0, w = 40.0, h = 20.0 }\npinned = true\n",
        )
        .unwrap();
        assert_eq!(l.patches.len(), 1);
        assert!(l.patches[0].pinned && !l.patches[0].round);
        assert!(l.screens[0].is_some() && l.screens[1].is_none());
    }

    #[test]
    fn a_broken_file_gives_a_plain_error() {
        for (text, word) in [
            ("name = \"x\"\nsize = [256.0, 192.0]\n", "no screen"),
            ("name = \"\"\nsize = [1.0, 1.0]\nupper = { x = 0.0, y = 0.0, w = 1.0, h = 1.0 }\n", "no name"),
            ("name = \"x\"\nsize = [0.0, 1.0]\nupper = { x = 0.0, y = 0.0, w = 1.0, h = 1.0 }\n", "positive"),
            ("name = \"x\"\nsize = [1.0, 1.0]\nupper = { x = 0.0, y = 0.0, w = 0.0, h = 1.0 }\n", "no area"),
            ("name = \"x\"\nsize = [1.0, 1.0]\nupper = { x = 0.0, y = 0.0, w = 1.0, h = 1.0 }\nbogus = 1\n", "bogus"),
            ("not toml at all [", ""),
        ] {
            let e = parse(text).err().unwrap_or_else(|| panic!("should not read: {}", text));
            assert!(e.contains(word), "{:?} lacks {:?}", e, word);
        }
    }

    #[test]
    fn a_folder_keeps_the_good_files_and_names_the_bad_one() {
        let dir = std::env::temp_dir().join(format!("sunpetal-layouts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.toml"), include_str!("../layouts/stacked.toml").replace("stacked", "mine")).unwrap();
        std::fs::write(dir.join("b.toml"), "name = ").unwrap();
        let (ok, bad) = load_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].name, "mine");
        assert_eq!(bad.len(), 1);
        assert!(bad[0].starts_with("b.toml: "), "{:?}", bad);
        assert!(load_dir(std::path::Path::new("/no/such/dir")).0.is_empty());
    }
}
