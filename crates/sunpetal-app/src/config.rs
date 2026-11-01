//! The settings, kept in a TOML file in the user's configuration folder, with named profiles
//! beside it. Every field has a default, so a file from an older version, or with fields left out,
//! still loads; a file that does not parse is set aside and the defaults used.
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sunpetal_runtime::nds;
use winit::keyboard::KeyCode;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Video {
    /// The 3D's internal resolution, times the DS's (1-8).
    pub res: usize,
    /// Linear filtering of the screens (else the nearest pixel).
    pub linear: bool,
    /// The layout's name (`layout::all`).
    pub layout: String,
    /// In the `single` layout: whether the lower screen is the one shown.
    pub lower: bool,
    /// In the `overlay` layout: whether the panel with the lower screen is open.
    pub panel: bool,
    /// With one screen shown (upper only): the lower screen's counter on it, in the corner.
    pub counter: bool,
    /// In the `single` layout: the screen follows the game (the lower for a menu, the upper for a
    /// scene) instead of the choice in `lower`.
    pub auto: bool,
    /// The 3D view is widened to the window's shape when the upper screen is shown alone (more of the
    /// scene at the sides, not a stretched picture).
    pub wide: bool,
    /// The bars of a window that is not the picture's shape take the colour of the picture's edge
    /// when it is all one (a logo on white), instead of the glow.
    pub bars_match: bool,
    /// How the picture fills the window: `fit`, `integer`, `fill` or `stretch` (`layout::Mode`).
    pub fit: String,
    /// The width of a pixel over its height (1: square), 0.5 to 2.
    pub pixel_aspect: f32,
    /// With one screen shown: the logos at the start, which are on both screens, one after another.
    pub logos_in_turn: bool,
    /// The dial puzzle, which is read above and worked below: `inset` (the upper screen whole, the lower a panel in
    /// its corner, as on the PS2) or `side` (both screens side by side).
    pub puzzle: String,
    /// The dialogue lines: `crisp` (the app draws them in the window's resolution), `pixel` (the game's own raster text)
    /// or `auto` (crisp from 2 window pixels to a DS pixel).
    pub dialogue_text: String,
    /// The font of the crisp dialogue text: `noto`, `rubik` or `ubuntu` (`fonts::DIALOGUE_FONTS`).
    pub dialogue_font: String,
    /// The window takes the shape of the chosen layout's picture when the layout changes (no bars at the sides).
    pub follow_shape: bool,
    pub fullscreen: bool,
    /// Behind the screens: "edge" (a lone screen's edge rows run on), "glow" (their colours, blurred) or "black".
    pub background: String,
    /// Speed figures over the picture (frame, fps, time per frame).
    pub show_stats: bool,
    /// A data file (`.spdata`) or a folder of replacement textures (empty: none); read when the game starts.
    pub texture_pack: String,
    /// The engine that draws the 3D: `software` (the reference) or `gpu` (the device; the software
    /// one takes over, with a note, if it cannot be made).
    pub renderer: String,
    /// The window's size in logical points when it was closed.
    pub window: [u32; 2],
}

/// The engine a fresh setting names: the graphics card everywhere (faster in every measured setting
/// on the phone and the Mac, parity with the software engine on both, `docs/GPU-BACKEND.md` 9.6);
/// the software engine stays the reference and the fallback.
pub fn default_renderer() -> &'static str {
    "gpu"
}

/// The 3D engine a setting or `--renderer` names (`software` or `gpu`, any case).
pub fn parse_renderer(name: &str) -> Option<nds::Renderer> {
    match name.to_ascii_lowercase().as_str() {
        "software" => Some(nds::Renderer::Software),
        "gpu" => Some(nds::Renderer::Gpu),
        _ => None,
    }
}

impl Default for Video {
    fn default() -> Video {
        Video { res: 2, linear: true, layout: "stacked".into(), lower: false, panel: false, counter: true, auto: true, wide: true, bars_match: true, fit: "fit".into(), pixel_aspect: 1.0, logos_in_turn: true, puzzle: "inset".into(), dialogue_text: "auto".into(), dialogue_font: "noto".into(), follow_shape: false, fullscreen: false, background: "edge".into(), show_stats: false, texture_pack: String::new(), renderer: default_renderer().into(), window: [512, 768] }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Audio {
    /// 0.0-1.0.
    pub volume: f32,
    pub mute: bool,
}

impl Default for Audio {
    fn default() -> Audio {
        Audio { volume: 1.0, mute: false }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Input {
    /// The DS's button (`nds::KEY_NAMES`) to the key that presses it (a `winit` key code's name).
    pub keys: BTreeMap<String, String>,
    /// How far a stick must go to count as the D-pad (0.1-0.9).
    pub stick_threshold: f32,
    /// On-screen buttons: "auto" (on a phone), "on" or "off".
    pub touch_controls: String,
    /// A swipe turns the dial puzzle's dial (see `dial.rs`).
    pub dial_swipe: bool,
    /// Turn of the dial, in radians, for a swipe of one dial radius; negative turns the other way.
    pub dial_gain: f32,
}

impl Default for Input {
    fn default() -> Input {
        let k = |a: &str, b: &str| (a.to_string(), b.to_string());
        Input {
            keys: BTreeMap::from([
                k("A", "KeyZ"),
                k("B", "KeyX"),
                k("X", "KeyS"),
                k("Y", "KeyA"),
                k("L", "KeyQ"),
                k("R", "KeyW"),
                k("START", "Enter"),
                k("SELECT", "Backspace"),
                k("UP", "ArrowUp"),
                k("DOWN", "ArrowDown"),
                k("LEFT", "ArrowLeft"),
                k("RIGHT", "ArrowRight"),
            ]),
            stick_threshold: 0.5,
            touch_controls: "auto".into(),
            dial_swipe: false,
            dial_gain: 1.5,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Emulation {
    /// The machine waits while the menu is open.
    pub pause_in_menu: bool,
    /// Tab fast-forwards while held.
    pub fast_key: bool,
    /// A fast-forward button beside the menu button (a phone with the controls off).
    pub fast_button: bool,
    /// A skip button beside them: fast-forward until the picture stops changing.
    pub skip_button: bool,
    /// A click (a tap) while the logos at the start run races through them.
    pub click_skips_logos: bool,
}

impl Default for Emulation {
    fn default() -> Emulation {
        Emulation { pause_in_menu: true, fast_key: true, fast_button: true, skip_button: true, click_skips_logos: true }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Interface {
    /// "auto" (the system's language), "en" or "ru".
    pub language: String,
    /// A font file (.ttf/.otf/.ttc) for the letters egui's own font lacks; empty: the OS's.
    pub font: String,
}

impl Default for Interface {
    fn default() -> Interface {
        Interface { language: "auto".into(), font: String::new() }
    }
}

/// The game itself.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Game {
    /// The ROM to play when none is named on the command line; empty: look in the usual folders.
    pub rom: String,
    /// The language of the translation in the data file to play with (`ru`); empty: the ROM's own
    /// text. Read when the game starts.
    pub language: String,
}

/// The languages the interface has.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// The language a locale name stands for (`ru_RU.UTF-8`, `ru-RU`, `en-US`...): Russian for
    /// Russian, else English.
    pub fn from_locale(locale: &str) -> Lang {
        let l = locale.trim().to_ascii_lowercase();
        if l == "ru" || l.starts_with("ru-") || l.starts_with("ru_") || l.starts_with("ru.") {
            Lang::Ru
        } else {
            Lang::En
        }
    }

    /// The language a setting names; "auto" asks the system.
    pub fn resolve(setting: &str) -> Lang {
        match setting {
            "ru" => Lang::Ru,
            "en" => Lang::En,
            "auto" => sys_locale::get_locale().map_or(Lang::En, |l| Lang::from_locale(&l)),
            // a language that has a table in ui/ (uitext.rs): English underneath
            _ => Lang::En,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default, Debug)]
#[serde(default)]
pub struct Settings {
    pub interface: Interface,
    pub game: Game,
    pub video: Video,
    pub audio: Audio,
    pub input: Input,
    pub emulation: Emulation,
    pub effects: crate::effects::Effects,
}

/// The keys the menu offers for a button, by the name `KeyCode`'s `Debug` gives them.
pub fn key_table() -> Vec<(String, KeyCode)> {
    use KeyCode::*;
    let all = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR,
        KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ, Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6,
        Digit7, Digit8, Digit9, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Enter, Space, Backspace, ShiftLeft,
        ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight, Comma, Period, Slash, Semicolon, Quote,
        BracketLeft, BracketRight, Backslash, Minus, Equal, Backquote, NumpadEnter, Numpad0, Numpad1, Numpad2,
        Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9,
    ];
    all.iter().map(|k| (format!("{:?}", k), *k)).collect()
}

/// The key a name stands for.
pub fn parse_key(name: &str) -> Option<KeyCode> {
    key_table().into_iter().find(|(n, _)| n == name).map(|(_, k)| k)
}

impl Settings {
    /// Whether the on-screen buttons are shown.
    pub fn touch_controls_on(&self) -> bool {
        match self.input.touch_controls.as_str() {
            "on" => true,
            "off" => false,
            _ => cfg!(target_os = "android"),
        }
    }

    /// Fixes what a hand-edited file may have got wrong.
    pub fn sanitize(&mut self) {
        self.video.pixel_aspect = if self.video.pixel_aspect.is_finite() { self.video.pixel_aspect.clamp(0.5, 2.0) } else { 1.0 };
        if !matches!(self.video.fit.as_str(), "fit" | "integer" | "fill" | "stretch") {
            self.video.fit = "fit".into();
        }
        if parse_renderer(&self.video.renderer).is_none() {
            self.video.renderer = default_renderer().into();
        }
        if !matches!(self.interface.language.as_str(), "auto" | "en" | "ru") {
            self.interface.language = "auto".into();
        }
        if !crate::fonts::DIALOGUE_FONTS.contains(&self.video.dialogue_font.as_str()) {
            self.video.dialogue_font = "noto".into();
        }
        if !matches!(self.video.dialogue_text.as_str(), "auto" | "pixel" | "crisp") {
            self.video.dialogue_text = "auto".into();
        }
        self.video.res = self.video.res.clamp(1, 8);
        if !matches!(self.video.background.as_str(), "edge" | "glow" | "black") {
            self.video.background = "edge".into();
        }
        self.video.window = [self.video.window[0].clamp(128, 16384), self.video.window[1].clamp(128, 16384)];
        if !matches!(self.input.touch_controls.as_str(), "auto" | "on" | "off") {
            self.input.touch_controls = "auto".into();
        }
        self.input.dial_gain = if self.input.dial_gain.is_finite() { self.input.dial_gain.clamp(-4.0, 4.0) } else { 1.5 };
        self.effects.sanitize();
        self.audio.volume = self.audio.volume.clamp(0.0, 1.0);
        self.input.stick_threshold = self.input.stick_threshold.clamp(0.1, 0.9);
        let defaults = Input::default().keys;
        for (button, key) in defaults {
            let ok = self.input.keys.get(&button).map_or(false, |k| parse_key(k).is_some());
            if !ok {
                self.input.keys.insert(button, key);
            }
        }
    }
}

/// The configuration folder: `~/Library/Application Support/sunpetal` on macOS, `$XDG_CONFIG_HOME/sunpetal`
/// (or `~/.config/sunpetal`) on Linux, `%APPDATA%\sunpetal` on Windows. `SUNPETAL_CONFIG` overrides it.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SUNPETAL_CONFIG") {
        return PathBuf::from(d);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("sunpetal")
}

fn file() -> PathBuf {
    dir().join("config.toml")
}

fn profile_file(name: &str) -> PathBuf {
    dir().join("profiles").join(format!("{}.toml", name))
}

fn read(path: &PathBuf) -> Result<Settings, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut s: Settings = toml::from_str(&text).map_err(|e| e.to_string())?;
    s.sanitize();
    Ok(s)
}

fn write(path: &PathBuf, s: &Settings) -> Result<(), String> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

/// The saved settings, or the defaults (a file that does not parse is renamed `.bad`).
pub fn load() -> Settings {
    let path = file();
    if !path.exists() {
        return Settings::default();
    }
    match read(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {} (set aside as .bad, the defaults are used)", path.display(), e);
            let _ = fs::rename(&path, path.with_extension("toml.bad"));
            Settings::default()
        }
    }
}

pub fn save(s: &Settings) {
    if let Err(e) = write(&file(), s) {
        eprintln!("cannot save the settings: {}", e);
    }
}

/// The profiles' names, sorted.
pub fn profiles() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir().join("profiles")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(false, |x| x == "toml") {
                if let Some(n) = p.file_stem() {
                    v.push(n.to_string_lossy().into_owned());
                }
            }
        }
    }
    v.sort();
    v
}

/// A profile's name made safe as a file name.
pub fn clean_name(name: &str) -> String {
    name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect()
}

pub fn save_profile(name: &str, s: &Settings) -> Result<(), String> {
    let name = clean_name(name);
    if name.is_empty() {
        return Err("empty name".into());
    }
    write(&profile_file(&name), s)
}

pub fn load_profile(name: &str) -> Result<Settings, String> {
    read(&profile_file(&clean_name(name)))
}

pub fn delete_profile(name: &str) {
    let _ = fs::remove_file(profile_file(&clean_name(name)));
}

/// Ready window shapes (stage 8в): name and width over height. The window is any shape the user
/// drags it to; these set it to a common one in one click.
pub const SHAPES: [(&str, f32); 6] = [("4:3", 4.0 / 3.0), ("16:10", 16.0 / 10.0), ("16:9", 16.0 / 9.0), ("21:9", 21.0 / 9.0), ("3:4", 3.0 / 4.0), ("9:16", 9.0 / 16.0)];

/// The window (logical size) of the shape `ratio` that keeps the area of `cur`, in whole pixels,
/// neither side under 240 nor over 8192.
pub fn window_of_shape(cur: [u32; 2], ratio: f32) -> [u32; 2] {
    let area = (cur[0].max(1) as f32) * (cur[1].max(1) as f32);
    let ratio = ratio.clamp(0.25, 4.0);
    let w = (area * ratio).sqrt();
    let h = w / ratio;
    [(w.round() as u32).clamp(240, 8192), (h.round() as u32).clamp(240, 8192)]
}

/// The shape (name) closest to a window of `size`, when within 2 % of it.
pub fn shape_name(size: [u32; 2]) -> Option<&'static str> {
    let r = size[0] as f32 / size[1].max(1) as f32;
    SHAPES.iter().find(|(_, q)| (r / q - 1.0).abs() < 0.02).map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_keep_the_area_and_are_found_again() {
        for (name, r) in SHAPES {
            let w = window_of_shape([512, 768], r);
            let got = w[0] as f32 / w[1] as f32;
            assert!((got / r - 1.0).abs() < 0.01, "{}: {:?}", name, w);
            let area = (w[0] * w[1]) as f32 / (512.0 * 768.0);
            assert!((area - 1.0).abs() < 0.02, "{}: area {}", name, area);
            assert_eq!(shape_name(w), Some(name));
        }
        assert_eq!(shape_name([1000, 1000]), None);
        assert_eq!(window_of_shape([10, 10], 4.0), [240, 240]);
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let s = Settings::default();
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn the_renderer_setting_defaults_per_platform_and_parses() {
        let s: Settings = toml::from_str("[video]\nres = 4\n").unwrap();
        assert_eq!(s.video.renderer, default_renderer());
        assert_eq!(parse_renderer("gpu"), Some(nds::Renderer::Gpu));
        assert_eq!(parse_renderer("software"), Some(nds::Renderer::Software));
        assert_eq!(parse_renderer("GPU"), Some(nds::Renderer::Gpu));
        assert_eq!(parse_renderer("vulkan"), None);
        let s: Settings = toml::from_str("[video]\nrenderer = \"gpu\"\n").unwrap();
        assert_eq!(s.video.renderer, "gpu");
    }

    #[test]
    fn a_file_with_fields_left_out_gets_the_defaults() {
        let s: Settings = toml::from_str("[video]\nres = 4\n").unwrap();
        assert_eq!(s.video.res, 4);
        assert!(s.video.linear);
        assert_eq!(s.input.keys["A"], "KeyZ");
    }

    #[test]
    fn sanitize_clamps_and_restores_bad_keys() {
        let mut s: Settings = toml::from_str("[video]\nres = 99\n[audio]\nvolume = 7.0\n[input.keys]\nA = \"NoSuchKey\"\n").unwrap();
        s.sanitize();
        assert_eq!(s.video.res, 8);
        assert_eq!(s.video.renderer, default_renderer());
        assert_eq!(s.audio.volume, 1.0);
        assert_eq!(s.input.keys["A"], "KeyZ");
        assert_eq!(parse_key(&s.input.keys["START"]), Some(KeyCode::Enter));
    }

    #[test]
    fn every_default_key_is_in_the_table() {
        for k in Input::default().keys.values() {
            assert!(parse_key(k).is_some(), "{}", k);
        }
    }

    #[test]
    fn the_language_follows_the_locale() {
        assert_eq!(Lang::from_locale("ru_RU.UTF-8"), Lang::Ru);
        assert_eq!(Lang::from_locale("ru-RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("uk-UA"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
        assert_eq!(Lang::resolve("ru"), Lang::Ru);
        assert_eq!(Lang::resolve("en"), Lang::En);
    }

    #[test]
    fn a_bad_language_falls_back_to_auto() {
        let mut s: Settings = toml::from_str("[interface]\nlanguage = \"xx\"\n").unwrap();
        s.sanitize();
        assert_eq!(s.interface.language, "auto");
    }

    #[test]
    fn profile_names_are_made_safe() {
        assert_eq!(clean_name("  ../x/y "), "___x_y");
    }
}
