//! A system font behind egui's bundled one, for the alphabets the bundled font lacks (Greek,
//! Polish letters, CJK...). The font is not shipped: it is looked for by OS path, or taken from the
//! `interface.font` setting (a .ttf/.otf/.ttc file).
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Wide-coverage fonts that come with the OS, best first.
pub fn candidates() -> Vec<PathBuf> {
    let list: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Supplemental/Arial Unicode.ttf", "/Library/Fonts/Arial Unicode.ttf", "/System/Library/Fonts/Helvetica.ttc"]
    } else if cfg!(target_os = "windows") {
        &["C:\\Windows\\Fonts\\arialuni.ttf", "C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
    } else if cfg!(target_os = "android") {
        &["/system/fonts/NotoSans-Regular.ttf", "/system/fonts/Roboto-Regular.ttf", "/system/fonts/DroidSans.ttf"]
    } else {
        &["/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]
    };
    list.iter().map(PathBuf::from).collect()
}

/// The font file to use: the chosen one if it reads, otherwise the first system candidate.
pub fn pick(chosen: &str) -> Option<(PathBuf, Vec<u8>)> {
    let own = (!chosen.is_empty()).then(|| PathBuf::from(chosen));
    own.into_iter().chain(candidates()).find_map(|p| read(&p).map(|b| (p, b)))
}

fn read(p: &Path) -> Option<Vec<u8>> {
    std::fs::read(p).ok().filter(|b| !b.is_empty())
}

const NOTO_SANS: &[u8] = include_bytes!("../fonts/NotoSans.ttf");
const RUBIK: &[u8] = include_bytes!("../fonts/Rubik.ttf");

/// The fonts of the dialogue text (SIL OFL 1.1, the licences are beside the files in `fonts/`): the name in
/// `video.dialogue_font` and the family that draws it. `ubuntu` is the menu's bundled font.
pub const DIALOGUE_FONTS: [&str; 3] = ["noto", "rubik", "ubuntu"];

/// The family for a `video.dialogue_font` name; anything unknown is the first one.
pub fn dialogue_family(name: &str) -> egui::FontFamily {
    match name {
        "ubuntu" => egui::FontFamily::Proportional,
        "rubik" => egui::FontFamily::Name("dialogue-rubik".into()),
        _ => egui::FontFamily::Name("dialogue-noto".into()),
    }
}

/// The variable fonts are set to the regular weight (their default axis may be thinner).
fn dialogue_font(bytes: &'static [u8]) -> egui::FontData {
    egui::FontData::from_static(bytes).tweak(egui::FontTweak { coords: egui::epaint::text::VariationCoords::new([(b"wght", 400.0)]), ..Default::default() })
}

/// The fonts for egui: the bundled ones, the dialogue fonts, and `bytes` after them as the last resort in both families.
pub fn definitions(bytes: Option<Vec<u8>>) -> egui::FontDefinitions {
    let mut d = egui::FontDefinitions::default();
    for (key, data) in [("dialogue-noto", NOTO_SANS), ("dialogue-rubik", RUBIK)] {
        d.font_data.insert(key.into(), Arc::new(dialogue_font(data)));
        // a character the font lacks falls to the bundled fonts
        let mut list = vec![key.to_string()];
        list.extend(d.families[&egui::FontFamily::Proportional].iter().cloned());
        d.families.insert(egui::FontFamily::Name(key.into()), list);
    }
    if let Some(b) = bytes {
        d.font_data.insert("system".into(), Arc::new(egui::FontData::from_owned(b)));
        for fam in d.families.values_mut() {
            fam.push("system".into());
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_font_comes_first_and_a_bad_one_falls_back() {
        let dir = std::env::temp_dir().join(format!("sunpetal-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.ttf");
        std::fs::write(&f, b"font").unwrap();
        assert_eq!(pick(f.to_str().unwrap()).unwrap().0, f);
        let missing = dir.join("none.ttf");
        let got = pick(missing.to_str().unwrap()).map(|(p, _)| p);
        assert!(got.is_none_or(|p| candidates().contains(&p)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_system_font_goes_behind_the_bundled_ones() {
        let plain = definitions(None);
        let with = definitions(Some(vec![1, 2, 3]));
        let fam = egui::FontFamily::Proportional;
        assert_eq!(with.families[&fam].len(), plain.families[&fam].len() + 1);
        assert_eq!(with.families[&fam].last().unwrap(), "system");
    }

    #[test]
    fn each_dialogue_font_has_a_family_that_reads_cyrillic() {
        let d = definitions(None);
        for name in DIALOGUE_FONTS {
            let f = dialogue_family(name);
            assert!(d.families.contains_key(&f), "{}", name);
        }
        for bytes in [NOTO_SANS, RUBIK] {
            let font = ab_glyph::FontRef::try_from_slice(bytes).unwrap();
            for c in "Ёё…—«»Привет".chars() {
                assert!(ab_glyph::Font::glyph_id(&font, c).0 != 0, "{}", c);
            }
        }
    }

    #[test]
    fn egui_lays_a_russian_line_out_in_each_dialogue_font() {
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions(None));
        let mut widths = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx().clone();
            for name in DIALOGUE_FONTS {
                let g = ctx.fonts_mut(|f| f.layout_no_wrap("Ты опоздал, искатель…".into(), egui::FontId::new(26.0, dialogue_family(name)), egui::Color32::BLACK));
                widths.push(g.size().x);
            }
        });
        out.textures_delta.clear();
        assert_eq!(widths.len(), 3);
        assert!(widths.iter().all(|w| *w > 100.0), "{:?}", widths);
        assert!(widths[0] != widths[1] && widths[1] != widths[2], "{:?}", widths);
    }

    #[test]
    fn this_machine_has_a_wide_font() {
        if cfg!(target_os = "macos") {
            assert!(pick("").is_some());
        }
    }
}
