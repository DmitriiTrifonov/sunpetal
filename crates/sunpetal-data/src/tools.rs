//! What a translator needs of the table: a template made from the ROM's own text, and a check of a
//! filled table against the ROM and the dialogue window. The template holds the original text, so
//! it stays on the translator's machine (like the ROM); the table that goes into a language pack
//! holds only the translation.
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::nftr::Font;
use crate::{lz10, nitrofs::NitroFs, script, translate::parse_csv};

/// Folders whose files are one text each (ID `-`).
const RAW_DIRS: [&str; 3] = ["2D", "EdoComment", "Profile"];
/// The font of the dialogue window.
pub const DIALOGUE_FONT: &str = "Chapter/Pack/Common/font.nftr";
/// The dialogue window: lines at most so many pixels wide (the translation project measured it in
/// the game: 188 clears the portrait on the left), and so many lines.
pub const MAX_WIDTH: usize = 188;
pub const MAX_LINES: usize = 3;

/// One text of the game: where it is and what it says in the base language.
pub struct Entry {
    pub chapter: String,
    pub scene: String,
    /// The ID in the scene; a repeated one is `ID#2`; `-` for a whole file.
    pub id: String,
    /// The lines joined by line breaks.
    pub text: String,
}

/// All the texts of the base language `base` (`ue`, the European English) in the ROM.
pub fn entries(rom: &[u8], base: &str) -> Result<Vec<Entry>, String> {
    let fs = NitroFs::parse(rom)?;
    let mut paths: Vec<&str> = fs.paths().collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["Chapter", "Pack", chap, lang, file] if *lang == base && file.ends_with(".s.z") => {
                let z = fs.file(rom, path).ok_or("a file listed is not there")?;
                let (_, items) = script::parse(&script::decode_utf16(&lz10::decompress(z)?)?);
                let mut seen = HashMap::<String, usize>::new();
                for (id, body) in items {
                    let n = seen.entry(id.clone()).or_insert(0);
                    *n += 1;
                    let key = if *n == 1 { id.clone() } else { format!("{}#{}", id, n) };
                    out.push(Entry { chapter: chap.to_string(), scene: file.trim_end_matches(".s.z").to_string(), id: key, text: script::split_body(&body).text.join("\n") });
                }
            }
            [dir, lang, file] if RAW_DIRS.contains(dir) && *lang == base && file.ends_with(".txt") => {
                let text = script::decode_utf16(fs.file(rom, path).ok_or("a file listed is not there")?)?;
                out.push(Entry { chapter: dir.to_string(), scene: file.trim_end_matches(".txt").to_string(), id: "-".into(), text: text.trim_matches(['\r', '\n']).replace("\r\n", "\n") });
            }
            _ => {}
        }
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A CSV for translating: the columns of the translation tables (chapter, scene, ID), the base
/// text, and an empty column for the translation. A table already filled is taken in: its
/// translations stay in the new column where the ID is the same.
pub fn template_csv(entries: &[Entry], lang_column: &str, filled: Option<(&str, &str)>) -> String {
    let mut done: HashMap<(String, String, String), String> = HashMap::new();
    if let Some((text, column)) = filled {
        let rows = parse_csv(text);
        if let Some(col) = rows.first().and_then(|h| h.iter().position(|c| c == column)) {
            for r in &rows[1..] {
                if let (Some(a), Some(b), Some(c), Some(t)) = (r.first(), r.get(1), r.get(2), r.get(col)) {
                    done.insert((a.clone(), b.clone(), c.clone()), t.clone());
                }
            }
        }
    }
    let mut out = format!("\u{FEFF}Chapter,Scene,ID,Original,{}\n", quote(lang_column));
    for e in entries {
        let t = done.get(&(e.chapter.clone(), e.scene.clone(), e.id.clone())).map_or("", |s| s.as_str());
        out.push_str(&format!("{},{},{},{},{}\n", e.chapter, e.scene, quote(&e.id), quote(&e.text), quote(t)));
    }
    out
}

/// The result of a check.
#[derive(Default)]
pub struct Report {
    pub rows: usize,
    pub translated: usize,
    /// How many texts of the ROM have no translation yet.
    pub missing: usize,
    pub problems: Vec<String>,
}

/// Checks the column `column` of a CSV table against the ROM's texts: rows the ROM has no text for
/// (a wrong ID, a scene that is not there), empty or untranslated texts, and for dialogue: more
/// lines than the window holds, lines wider than it, and characters the font lacks.
pub fn check(rom: &[u8], base: &str, csv: &str, column: &str, font: &Font, max_width: usize) -> Result<Report, String> {
    let rows = parse_csv(csv);
    let head = rows.first().ok_or("an empty table")?;
    let col = head.iter().position(|h| h == column).ok_or_else(|| format!("the table has no column {}", column))?;
    let known: HashSet<(String, String, String)> = entries(rom, base)?.into_iter().map(|e| (e.chapter, e.scene, e.id)).collect();
    let mut rep = Report::default();
    let mut given: HashSet<(String, String, String)> = HashSet::new();
    for (n, r) in rows.iter().enumerate().skip(1) {
        let (Some(chap), Some(scene), Some(id), Some(text)) = (r.first(), r.get(1), r.get(2), r.get(col)) else { continue };
        rep.rows += 1;
        let key = (chap.clone(), scene.clone(), id.clone());
        let at = format!("line {} ({}/{} {})", n + 1, chap, scene, id);
        if !known.contains(&key) {
            rep.problems.push(format!("{}: the ROM has no such text", at));
            continue;
        }
        if !given.insert(key) {
            rep.problems.push(format!("{}: the same text twice", at));
        }
        if text.trim().is_empty() {
            continue;
        }
        rep.translated += 1;
        if RAW_DIRS.contains(&chap.as_str()) {
            continue; // other fonts and windows: not measured here
        }
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.len() > MAX_LINES {
            rep.problems.push(format!("{}: {} lines, the window holds {}", at, lines.len(), MAX_LINES));
        }
        for l in &lines {
            let w = font.dialogue_width(l);
            if w > max_width {
                rep.problems.push(format!("{}: {} px, {} too wide: {}", at, w, w - max_width, l));
            }
        }
        let lacking: Vec<char> = text.chars().filter(|c| *c != '\n' && !c.is_control() && !font.has(*c)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        if !lacking.is_empty() {
            rep.problems.push(format!("{}: the font lacks {}", at, lacking.iter().collect::<String>()));
        }
    }
    rep.missing = known.len().saturating_sub(rep.translated);
    Ok(rep)
}

/// How many texts each chapter has and how many are done, for a progress line.
pub fn progress(rom: &[u8], base: &str, csv: &str, column: &str) -> Result<BTreeMap<String, (usize, usize)>, String> {
    let rows = parse_csv(csv);
    let col = rows.first().and_then(|h| h.iter().position(|c| c == column)).ok_or_else(|| format!("the table has no column {}", column))?;
    let mut done: HashSet<(String, String, String)> = HashSet::new();
    for r in &rows[1..] {
        if let (Some(a), Some(b), Some(c), Some(t)) = (r.first(), r.get(1), r.get(2), r.get(col)) {
            if !t.trim().is_empty() {
                done.insert((a.clone(), b.clone(), c.clone()));
            }
        }
    }
    let mut out: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in entries(rom, base)? {
        let v = out.entry(e.chapter.clone()).or_default();
        v.0 += 1;
        if done.contains(&(e.chapter, e.scene, e.id)) {
            v.1 += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_table_goes_into_the_new_template() {
        let e = vec![
            Entry { chapter: "C00".into(), scene: "C00_01".into(), id: "01001".into(), text: "Hello,\nthere".into() },
            Entry { chapter: "C00".into(), scene: "C00_01".into(), id: "01002".into(), text: "Bye".into() },
        ];
        let old = "Глава,Сцена,ID,Русский\nC00,C00_01,01001,\"Привет,\nтам\"\n";
        let csv = template_csv(&e, "Русский", Some((old, "Русский")));
        let rows = parse_csv(&csv);
        assert_eq!(rows[0], ["Chapter", "Scene", "ID", "Original", "Русский"]);
        assert_eq!(rows[1], ["C00", "C00_01", "01001", "Hello,\nthere", "Привет,\nтам"]);
        assert_eq!(rows[2][4], "");
    }
}
