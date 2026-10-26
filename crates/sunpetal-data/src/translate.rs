//! A translation as a table of strings, put into the game's files: for the dialogue scripts
//! (`Chapter/Pack/<chapter>/<lang>/<scene>.s.z`, LZ10) the entries by ID; for the plain files
//! (`2D`, `EdoComment`, `Profile`: `<dir>/<lang>/<scene>.txt`, UTF-16) the whole text. The result is
//! an overlay for `nitrofs::apply_overlay`: only the files that changed, built from the ROM's own
//! files, so the table holds nothing of the original.
use std::collections::{BTreeMap, HashMap};

use crate::{lz10, nitrofs::NitroFs, script};

/// Folders whose files are one text each (ID `-` in the table).
const RAW_DIRS: [&str; 3] = ["2D", "EdoComment", "Profile"];

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
