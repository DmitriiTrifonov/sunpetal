//! The menu's texts from files: a language is a TOML table `"English text" = "translation"` in
//! `ui/<code>.toml` beside the settings (`ru.toml`, `pl.toml`...). The built-in English and Russian
//! stay as the fallback; a table overrides them text by text, and a text it leaves out (or leaves
//! empty) stays as it was. `sunpetal-app --dump-ui-strings` writes the table to start from.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The menu's own source: the texts are found in it, so a new text in the menu is in the dump
/// with no list to keep.
const SOURCE: &str = include_str!("ui.rs");

/// A Rust string literal at the start of `s` (after blanks): its text and what follows.
fn literal(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '\'' => out.push('\''),
                _ => return None,
            },
            c => out.push(c),
        }
    }
    None
}

/// The English texts of the menu in the order they come, each once, with the built-in Russian
/// where the source gives it next to it.
pub fn keys() -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut add = |en: String, ru: Option<String>| {
        if !en.is_empty() && !out.iter().any(|(k, _)| *k == en) {
            out.push((en, ru));
        }
    };
    for line in SOURCE.lines() {
        // tx("English", "Русский")
        let mut rest = line;
        while let Some(at) = rest.find("tx(\"") {
            let after = &rest[at + 3..];
            let before = &rest[..at];
            rest = &after[1..];
            if before.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                continue;
            }
            if let Some((en, r)) = literal(after) {
                if let Some((ru, _)) = r.trim_start().strip_prefix(',').and_then(literal) {
                    add(en, Some(ru));
                }
            }
        }
        // the label tables: ("id", Lang::En) => "English", and ("id", false) => "English"
        if line.contains("Lang::En) =>") || line.contains(", false) =>") {
            if let Some((en, _)) = line.split_once("=>").and_then(|(_, r)| literal(r)) {
                add(en, None);
            }
        }
    }
    out
}

/// A table to start a language from: every English text, the translation empty.
pub fn dump() -> String {
    let mut s = String::from("# Sunpetal menu texts. Put the translation after the `=`; an empty one stays as it is.\n# Save as ui/<code>.toml beside config.toml (ru.toml, pl.toml...), then choose the language in the menu.\n\n");
    for (en, _) in keys() {
        s += &format!("{:?} = \"\"\n", en);
    }
    s
}

fn folder() -> PathBuf {
    crate::config::dir().join("ui")
}

/// The codes of the tables in the folder (`pl` for `pl.toml`), sorted.
pub fn codes() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.path().file_stem().filter(|_| e.path().extension().is_some_and(|x| x == "toml")).map(|s| s.to_string_lossy().into_owned()))
        .filter(|c| !c.is_empty())
        .collect();
    v.sort();
    v
}

/// What a table file says: the texts it translates.
pub fn parse(text: &str) -> Result<HashMap<String, String>, String> {
    let all: HashMap<String, String> = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(all.into_iter().filter(|(_, v)| !v.is_empty()).collect())
}

type Cached = Option<(String, Arc<HashMap<String, String>>)>;
static CACHE: Mutex<Cached> = Mutex::new(None);

/// The table of a language code, read once and again when the code changes; none if there is
/// no file or it cannot be read (said once on the terminal).
pub fn table(code: &str) -> Option<Arc<HashMap<String, String>>> {
    let mut c = CACHE.lock().ok()?;
    if let Some((k, t)) = c.as_ref() {
        if k == code {
            return Some(t.clone()).filter(|t| !t.is_empty());
        }
    }
    let path = folder().join(format!("{}.toml", code));
    let t = match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).unwrap_or_else(|e| {
            eprintln!("{}: {}", path.display(), e);
            HashMap::new()
        }),
        Err(_) => HashMap::new(),
    };
    let t = Arc::new(t);
    *c = Some((code.to_string(), t.clone()));
    Some(t).filter(|t| !t.is_empty())
}

/// The code a language setting names: `auto` is the system's language (`ru` for `ru_RU.UTF-8`).
pub fn code_of(setting: &str) -> String {
    if setting != "auto" {
        return setting.to_string();
    }
    sys_locale::get_locale().and_then(|l| l.split(['-', '_', '.']).next().map(|s| s.to_ascii_lowercase())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_a_literal_with_escapes() {
        assert_eq!(literal(r#" "a \"b\" \\ c\nd" rest"#), Some(("a \"b\" \\ c\nd".to_string(), " rest")));
        assert_eq!(literal("no quote"), None);
        assert_eq!(literal("\"open"), None);
    }

    #[test]
    fn the_dump_has_the_menus_texts_each_once_and_reads_back_empty() {
        let k = keys();
        assert!(k.len() > 100, "{} texts", k.len());
        assert!(k.iter().any(|(en, ru)| en == "Video" && ru.as_deref() == Some("Графика")));
        assert!(k.iter().any(|(en, _)| en == "stretch"), "the label tables are in it");
        let mut names: Vec<&String> = k.iter().map(|(en, _)| en).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), k.len());
        // the dump is a table nothing is translated in
        let d = dump();
        let t: HashMap<String, String> = toml::from_str(&d).unwrap();
        assert_eq!(t.len(), k.len());
        assert!(parse(&d).unwrap().is_empty());
    }

    #[test]
    fn a_table_translates_what_it_has_and_skips_the_empty() {
        let t = parse("\"Video\" = \"Wideo\"\n\"Effects\" = \"\"\n").unwrap();
        assert_eq!(t.get("Video").map(String::as_str), Some("Wideo"));
        assert!(!t.contains_key("Effects"));
        assert!(parse("not toml at all =").is_err());
    }
}
