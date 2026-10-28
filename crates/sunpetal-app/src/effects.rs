//! The picture effects: a few parameters the screens' shader applies in one pass (sharp pixels,
//! scanlines, an LCD grid, a vignette and colour correction), and the presets that set them.
//! Those cost next to nothing; bloom and FXAA are passes of their own (`post.rs`) and run only when on.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Effects {
    /// The preset these values started from ("custom" once they are edited).
    pub preset: String,
    /// Crisp pixel edges when the picture is stretched (a sharp-bilinear filter); overrides the
    /// plain linear/nearest choice.
    pub sharp: bool,
    /// 0-1: the dark gap between the DS's lines.
    pub scanlines: f32,
    /// 0-1: the dark gap between the DS's pixels (an LCD's grid).
    pub grid: f32,
    /// 0-1: the corners darkened.
    pub vignette: f32,
    /// 0-2, 1 is as drawn.
    pub saturation: f32,
    /// 0.5-1.5, 1 is as drawn.
    pub contrast: f32,
    /// -0.3-0.3, 0 is as drawn.
    pub brightness: f32,
    /// 0.5-2, 1 is as drawn.
    pub gamma: f32,
    /// 0-1: light spilling from the bright parts (a half-size blur added back).
    pub bloom: f32,
    /// Smooths jagged edges (FXAA) over the whole picture.
    pub fxaa: bool,
    /// 0-1: an outline round the 3D objects, drawn by the 3D renderer from its depth (`render3d`).
    pub outline: f32,
    /// 0.25-2: the outline's width, in DS pixels.
    pub outline_width: f32,
    /// 0-1: soft shading where the scene is closed in, from the depth (`render3d`).
    pub shadow: f32,
    /// 0.5-6: how far the shading reaches, in DS pixels.
    pub shadow_radius: f32,
    /// "nearest" (as the DS) or "bilinear": how the 3D reads its textures (`render3d`).
    pub texture_filter: String,
    /// "off", "smooth" or "soft": text and interface in the 2D layers rebuilt as smooth shapes (see `gfx.rs`).
    pub smooth_2d: String,
    /// 0-1: how much of the rebuilt picture is mixed in.
    pub smooth_strength: f32,
    /// Polygon-edge anti-aliasing in the 3D renderer: 0 off, 2 or 4 (samples per side).
    pub msaa: u8,
}

impl Default for Effects {
    fn default() -> Effects {
        Effects { preset: "none".into(), sharp: false, scanlines: 0.0, grid: 0.0, vignette: 0.0, saturation: 1.0, contrast: 1.0, brightness: 0.0, gamma: 1.0, bloom: 0.0, fxaa: false, outline: 0.25, outline_width: 0.5, shadow: 0.0, shadow_radius: 2.0, texture_filter: "nearest".into(), smooth_2d: "off".into(), smooth_strength: 1.0, msaa: 0 }
    }
}

/// The presets' ids, in the order the menu and the hotkey go through them.
pub const PRESETS: [&str; 6] = ["none", "sharp", "lcd", "crt", "vivid", "glow"];

/// The most the 3D is drawn at (its scale times the samples per side) in the window, which has to keep up with the game;
/// `sunpetal-headless` has no such limit.
pub const MSAA_BUDGET: u32 = 4;

/// The samples per side the window really uses: the asked ones, brought down (4 to 2 to none) until scale times
/// samples is within `MSAA_BUDGET`.
pub fn effective_msaa(scale: u32, msaa: u8) -> u8 {
    let mut m = msaa;
    while m > 0 && scale * m as u32 > MSAA_BUDGET {
        m = if m >= 4 { 2 } else { 0 };
    }
    m
}

impl Effects {
    /// What the 3D renderer is to add.
    pub fn extras(&self) -> sunpetal_runtime::nds::Extras {
        sunpetal_runtime::nds::Extras { outline: self.outline, outline_width: self.outline_width, shadow: self.shadow, shadow_radius: self.shadow_radius, filter: sunpetal_runtime::nds::TexFilter::parse(&self.texture_filter), msaa: self.msaa }
    }

    /// The shader's mode: 0 off, 1 smooth, 2 soft.
    pub fn smooth_mode(&self) -> f32 {
        match self.smooth_2d.as_str() {
            "smooth" => 1.0,
            "soft" => 2.0,
            _ => 0.0,
        }
    }

    pub fn preset(id: &str) -> Effects {
        let base = Effects { preset: id.into(), ..Effects::default() };
        match id {
            "sharp" => Effects { sharp: true, ..base },
            "lcd" => Effects { grid: 0.4, saturation: 1.1, ..base },
            "crt" => Effects { sharp: true, scanlines: 0.5, vignette: 0.4, saturation: 1.15, contrast: 1.1, bloom: 0.25, ..base },
            "vivid" => Effects { saturation: 1.35, contrast: 1.1, ..base },
            "glow" => Effects { sharp: true, bloom: 0.4, saturation: 1.1, ..base },
            _ => Effects::default(),
        }
    }

    /// The next preset after the current one (a custom one goes back to the first).
    pub fn next_preset(&self) -> Effects {
        let i = PRESETS.iter().position(|p| *p == self.preset).map_or(0, |i| (i + 1) % PRESETS.len());
        Effects::preset(PRESETS[i])
    }

    pub fn sanitize(&mut self) {
        let c = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 1.0f32.clamp(lo, hi) };
        self.scanlines = c(self.scanlines, 0.0, 1.0);
        self.grid = c(self.grid, 0.0, 1.0);
        self.vignette = c(self.vignette, 0.0, 1.0);
        self.saturation = c(self.saturation, 0.0, 2.0);
        self.contrast = c(self.contrast, 0.5, 1.5);
        self.brightness = if self.brightness.is_finite() { self.brightness.clamp(-0.3, 0.3) } else { 0.0 };
        self.gamma = c(self.gamma, 0.5, 2.0);
        self.outline = if self.outline.is_finite() { self.outline.clamp(0.0, 1.0) } else { 0.0 };
        self.outline_width = if self.outline_width.is_finite() { self.outline_width.clamp(0.25, 2.0) } else { 0.5 };
        self.shadow = if self.shadow.is_finite() { self.shadow.clamp(0.0, 1.0) } else { 0.0 };
        self.shadow_radius = if self.shadow_radius.is_finite() { self.shadow_radius.clamp(0.5, 6.0) } else { 2.0 };
        if self.texture_filter != "bilinear" {
            self.texture_filter = "nearest".into();
        }
        if !["smooth", "soft"].contains(&self.smooth_2d.as_str()) {
            self.smooth_2d = "off".into();
        }
        self.smooth_strength = if self.smooth_strength.is_finite() { self.smooth_strength.clamp(0.0, 1.0) } else { 1.0 };
        self.msaa = match self.msaa { 0..=1 => 0, 2..=3 => 2, _ => 4 };
        self.bloom = if self.bloom.is_finite() { self.bloom.clamp(0.0, 1.0) } else { 0.0 };
        if !PRESETS.contains(&self.preset.as_str()) {
            self.preset = "custom".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_itself_after_a_round_trip() {
        for p in PRESETS {
            let mut e = Effects::preset(p);
            let before = e.clone();
            e.sanitize();
            assert_eq!(e, before);
            assert_eq!(e.preset, p);
        }
    }

    #[test]
    fn the_hotkey_goes_through_all_presets_and_back() {
        let mut e = Effects::default();
        for p in PRESETS.iter().skip(1).chain(std::iter::once(&PRESETS[0])) {
            e = e.next_preset();
            assert_eq!(e.preset, *p);
        }
        let custom = Effects { preset: "custom".into(), ..Effects::default() };
        assert_eq!(custom.next_preset().preset, "none");
    }

    #[test]
    fn sanitize_clamps_and_marks_unknown_presets_custom() {
        let mut e = Effects { preset: "weird".into(), scanlines: 5.0, gamma: f32::NAN, brightness: -9.0, ..Effects::default() };
        e.sanitize();
        assert_eq!((e.scanlines, e.gamma, e.brightness), (1.0, 1.0, -0.3));
        assert_eq!(e.preset, "custom");
    }
}

#[cfg(test)]
mod msaa_tests {
    use super::*;

    #[test]
    fn the_window_keeps_the_supersampling_within_its_budget() {
        assert_eq!(effective_msaa(1, 4), 4);
        assert_eq!(effective_msaa(2, 4), 2);
        assert_eq!(effective_msaa(2, 2), 2);
        assert_eq!(effective_msaa(3, 2), 0);
        assert_eq!(effective_msaa(4, 4), 0);
        assert_eq!(effective_msaa(5, 0), 0);
    }
}
