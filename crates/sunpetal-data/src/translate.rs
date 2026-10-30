//! A translation as a table of strings, put into the game's files: for the dialogue scripts
//! (`Chapter/Pack/<chapter>/<lang>/<scene>.s.z`, LZ10) the entries by ID; for the plain files
//! (`2D`, `EdoComment`, `Profile`: `<dir>/<lang>/<scene>.txt`, UTF-16) the whole text. The result is
//! an overlay for `nitrofs::apply_overlay`: only the files that changed, built from the ROM's own
//! files, so the table holds nothing of the original.
use std::collections::{BTreeMap, HashMap};

use crate::delta::{crc32, diff_file, get_varint, patch_file, put_varint};
use crate::{lz10, nitrofs::NitroFs, script};

/// Folders whose files are one text each (ID `-` in the table).
const RAW_DIRS: [&str; 3] = ["2D", "EdoComment", "Profile"];

/// The font the dialogue window and the game's menus draw with.
pub const DIALOGUE_FONT: &str = "Chapter/Pack/Common/font.nftr";

/// The table: (chapter or folder, scene) → ID → text.
#[derive(Default)]
pub struct Table {
    files: BTreeMap<(String, String), HashMap<String, String>>,
}

/// CSV (RFC 4180: quotes, doubled quotes, line breaks inside quotes) into rows.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cell.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\r', false) if chars.peek() == Some(&'\n') => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

impl Table {
    /// From CSV with the columns of the translation project (chapter, scene, ID, ..., `column`).
    /// Empty cells are not translations.
    pub fn from_csv(text: &str, column: &str) -> Result<Table, String> {
        let rows = parse_csv(text);
        let header = rows.first().ok_or("an empty table")?;
        let col = header.iter().position(|h| h == column).ok_or_else(|| format!("the table has no column {}", column))?;
        let mut t = Table::default();
        for r in &rows[1..] {
            let (Some(chap), Some(scene), Some(id), Some(cell)) = (r.first(), r.get(1), r.get(2), r.get(col)) else { continue };
            if cell.trim().is_empty() {
                continue;
            }
            t.files.entry((chap.clone(), scene.clone())).or_default().insert(id.clone(), cell.replace("\r\n", "\n"));
        }
        Ok(t)
    }

    pub fn files(&self) -> usize {
        self.files.len()
    }

    /// The number of texts.
    pub fn texts(&self) -> usize {
        self.files.values().map(|f| f.len()).sum()
    }

    fn write(&self, out: &mut Vec<u8>) {
        put_varint(out, self.files.len() as u64);
        for ((chap, scene), cells) in &self.files {
            put_str(out, chap);
            put_str(out, scene);
            put_varint(out, cells.len() as u64);
            let mut cells: Vec<_> = cells.iter().collect();
            cells.sort();
            for (id, text) in cells {
                put_str(out, id);
                put_str(out, text);
            }
        }
    }

    fn read(data: &[u8], at: &mut usize) -> Result<Table, String> {
        let mut t = Table::default();
        for _ in 0..get_varint(data, at)? {
            let (chap, scene) = (get_str(data, at)?, get_str(data, at)?);
            let cells = t.files.entry((chap, scene)).or_default();
            for _ in 0..get_varint(data, at)? {
                let (id, text) = (get_str(data, at)?, get_str(data, at)?);
                cells.insert(id, text);
            }
        }
        Ok(t)
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn get_str(data: &[u8], at: &mut usize) -> Result<String, String> {
    let n = get_varint(data, at)? as usize;
    let bytes = data.get(*at..*at + n).ok_or("the translation ends early")?;
    *at += n;
    String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
}

/// A translation as the data file keeps it (section `TRAN`): the table of texts and the deltas of
/// the files that are not text (fonts, pictures), for the files of the ROM it was made against.
/// Nothing of the original is in it: the texts are the translation, a delta only the new bytes.
///
/// ```text
/// "TRN1"  u8 len, language  u8 len, base language (the ROM's folder: `ue`)  u32 CRC-32 of the ROM
/// u32 size of the rest unpacked  LZ10 of: the table, varint count of deltas, each a string (the
/// file's path) and a varint length and the delta (`delta`)
/// ```
#[derive(Default)]
pub struct Translation {
    pub lang: String,
    pub base: String,
    pub rom_crc: u32,
    pub table: Table,
    pub deltas: Vec<(String, Vec<u8>)>,
}

impl Translation {
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut raw = Vec::new();
        self.table.write(&mut raw);
        put_varint(&mut raw, self.deltas.len() as u64);
        for (path, d) in &self.deltas {
            put_str(&mut raw, path);
            put_varint(&mut raw, d.len() as u64);
            raw.extend_from_slice(d);
        }
        if raw.len() >= 1 << 24 {
            return Err("the translation is too big for LZ10 (16 MiB)".into());
        }
        let mut out = b"TRN1".to_vec();
        for s in [&self.lang, &self.base] {
            out.push(u8::try_from(s.len()).map_err(|_| "a language name too long")?);
            out.extend_from_slice(s.as_bytes());
        }
        out.extend_from_slice(&self.rom_crc.to_le_bytes());
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&lz10::compress(&raw));
        Ok(out)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Translation, String> {
        if data.get(..4) != Some(b"TRN1") {
            return Err("not a translation, or a newer version of it".into());
        }
        let mut at = 4;
        let mut name = || -> Result<String, String> {
            let n = *data.get(at).ok_or("the translation ends early")? as usize;
            let s = data.get(at + 1..at + 1 + n).ok_or("the translation ends early")?;
            at += 1 + n;
            String::from_utf8(s.to_vec()).map_err(|e| e.to_string())
        };
        let (lang, base) = (name()?, name()?);
        let rom_crc = u32::from_le_bytes(data.get(at..at + 4).ok_or("the translation ends early")?.try_into().unwrap());
        let raw_len = u32::from_le_bytes(data.get(at + 4..at + 8).ok_or("the translation ends early")?.try_into().unwrap()) as usize;
        let raw = lz10::decompress(&data[at + 8..])?;
        if raw.len() != raw_len {
            return Err("the translation has the wrong size".into());
        }
        let mut at = 0;
        let table = Table::read(&raw, &mut at)?;
        let mut deltas = Vec::new();
        for _ in 0..get_varint(&raw, &mut at)? {
            let path = get_str(&raw, &mut at)?;
            let n = get_varint(&raw, &mut at)? as usize;
            deltas.push((path, raw.get(at..at + n).ok_or("the translation ends early")?.to_vec()));
            at += n;
        }
        Ok(Translation { lang, base, rom_crc, table, deltas })
    }

    /// Makes the translation of `rom` that `done` is: the table's texts where it makes the files,
    /// a delta for every other file that differs. Returns it with what is wrong: files of the
    /// table that do not come out as in `done` (compared unpacked, without a zero tail).
    pub fn make(rom: &[u8], done: &[u8], table: Table, lang: &str, base: &str) -> Result<(Translation, Vec<String>), String> {
        let (fs_rom, fs_done) = (NitroFs::parse(rom)?, NitroFs::parse(done)?);
        let built = build_overlay(rom, &table, base)?;
        let mut problems = Vec::new();
        for (path, bytes) in &built.files {
            let want = fs_done.file(done, path).ok_or_else(|| format!("{} is not in the translated ROM", path))?;
            if !same_file(path, bytes, want)? {
                problems.push(format!("the table makes {} differently", path));
            }
        }
        let mut deltas = Vec::new();
        for path in fs_done.paths() {
            let (a, b) = (fs_rom.file(rom, path).ok_or_else(|| format!("{} is not in the ROM", path))?, fs_done.file(done, path).unwrap());
            if a != b && !built.files.iter().any(|f| f.0 == path) {
                deltas.push((path.to_string(), diff_file(path, a, b)?));
            }
        }
        // files the table made but the translated ROM keeps as in the ROM are not changes
        Ok((Translation { lang: lang.into(), base: base.into(), rom_crc: crc32(rom), table, deltas }, problems))
    }

    /// Puts `font` (an NFTR) in place of the dialogue font of the ROM, as a delta; one the
    /// translated ROM brought is replaced. The ROM must be the one the translation is for.
    pub fn set_font(&mut self, rom: &[u8], font: &[u8]) -> Result<(), String> {
        crate::nftr::Nftr::parse(font).map_err(|e| format!("the font: {}", e))?;
        if crc32(rom) != self.rom_crc {
            return Err(format!("the translation `{}` is for another ROM", self.lang));
        }
        let orig = NitroFs::parse(rom)?.file(rom, DIALOGUE_FONT).ok_or_else(|| format!("{} is not in the ROM", DIALOGUE_FONT))?.to_vec();
        self.deltas.retain(|(p, _)| p != DIALOGUE_FONT);
        self.deltas.push((DIALOGUE_FONT.to_string(), diff_file(DIALOGUE_FONT, &orig, font)?));
        Ok(())
    }

    /// The overlay of this translation for `rom` (`nitrofs::apply_overlay`): the files of the
    /// table, then the files of the deltas. The ROM must be the one it was made against.
    pub fn overlay(&self, rom: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
        if crc32(rom) != self.rom_crc {
            return Err(format!("the translation `{}` is for another ROM", self.lang));
        }
        let fs = NitroFs::parse(rom)?;
        let mut files = build_overlay(rom, &self.table, &self.base)?.files;
        for (path, d) in &self.deltas {
            let orig = fs.file(rom, path).ok_or_else(|| format!("{} is not in the ROM", path))?;
            files.push((path.clone(), patch_file(path, orig, d)?));
        }
        Ok(files)
    }
}

/// Two versions of a file are the same: unpacked if it is `.z`, without a zero tail otherwise (a
/// translation project may pad a file to its old size).
fn same_file(path: &str, a: &[u8], b: &[u8]) -> Result<bool, String> {
    let trim = |x: &[u8]| x[..x.iter().rposition(|v| *v != 0).map_or(0, |i| i + 1)].to_vec();
    Ok(if path.ends_with(".z") { lz10::decompress(a)? == lz10::decompress(b)? } else { trim(a) == trim(b) })
}

/// What came out of a table and a ROM.
#[derive(Default)]
pub struct Built {
    /// The changed files, by NitroFS path.
    pub files: Vec<(String, Vec<u8>)>,
    pub entries_changed: usize,
    pub warnings: Vec<String>,
}

/// The files of the language `lang` (`ue` is the English of the European ROM) with the table's
/// texts in, from the files of `rom`.
pub fn build_overlay(rom: &[u8], table: &Table, lang: &str) -> Result<Built, String> {
    let fs = NitroFs::parse(rom)?;
    let mut built = Built::default();
    for ((chap, scene), cells) in &table.files {
        if RAW_DIRS.contains(&chap.as_str()) {
            let path = format!("{}/{}/{}.txt", chap, lang, scene);
            let (Some(text), Some(bytes)) = (cells.get("-"), fs.file(rom, &path)) else { continue };
            let orig = script::decode_utf16(bytes)?;
            let body = orig.trim_start_matches(['\r', '\n']);
            let lead = &orig[..orig.len() - body.len()];
            let tail = &orig[orig.trim_end_matches(['\r', '\n']).len()..];
            let new = format!("{}{}{}", lead, text.trim_matches('\n').replace('\n', "\r\n"), tail);
            if new != orig {
                built.files.push((path, script::encode_utf16(&new)));
                built.entries_changed += 1;
            }
            continue;
        }
        let path = format!("Chapter/Pack/{}/{}/{}.s.z", chap, lang, scene);
        let Some(z) = fs.file(rom, &path) else { continue };
        let (head, entries) = script::parse(&script::decode_utf16(&lz10::decompress(z)?)?);
        let (mut out, mut touched, mut seen) = (Vec::new(), 0, HashMap::<String, usize>::new());
        for (id, body) in entries {
            let key = if id.is_empty() {
                id.clone()
            } else {
                let n = seen.entry(id.clone()).or_insert(0);
                *n += 1;
                if *n == 1 { id.clone() } else { format!("{}#{}", id, n) }
            };
            let mut body = body;
            if let Some(cell) = cells.get(&key) {
                let b = script::split_body(&body);
                let new: Vec<String> = cell.split('\n').map(str::to_string).collect();
                if new != b.text {
                    if b.inner_notes {
                        built.warnings.push(format!("comment lines inside the text, dropped: {}/{} {}", chap, scene, id));
                    }
                    body = script::make_body(&b.head, &new, &b.tail);
                    touched += 1;
                }
            }
            out.push((id, body));
        }
        if touched > 0 {
            built.files.push((path, lz10::compress(&script::encode_utf16(&script::build(&head, &out)))));
            built.entries_changed += touched;
        }
    }
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_has_quotes_and_line_breaks_in_cells() {
        let rows = parse_csv("\u{FEFF}a,b,c\r\n1,\"x, \"\"y\"\"\nz\",3\r\n4,,\r\n");
        assert_eq!(rows, vec![vec!["a", "b", "c"], vec!["1", "x, \"y\"\nz", "3"], vec!["4", "", ""]]);
    }

    #[test]
    fn a_table_keeps_the_filled_cells() {
        let t = Table::from_csv("Глава,Сцена,ID,Русский\nC00,C00_01,01001,Привет\nC00,C00_01,01002,\n", "Русский").unwrap();
        assert_eq!(t.files(), 1);
        assert!(Table::from_csv("a,b\n", "Русский").is_err());
    }
}
