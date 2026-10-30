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

/// The fonts for egui: the bundled ones, and `bytes` after them as the last resort in both families.
pub fn definitions(bytes: Option<Vec<u8>>) -> egui::FontDefinitions {
    let mut d = egui::FontDefinitions::default();
    if let Some(b) = bytes {
        d.font_data.insert("system".into(), Arc::new(egui::FontData::from_owned(b)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            d.families.entry(fam).or_default().push("system".into());
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
    fn this_machine_has_a_wide_font() {
        if cfg!(target_os = "macos") {
            assert!(pick("").is_some());
        }
    }
}
