//! The events of a chapter (`Chapter/Pack/Cnn/Cnn.EVN`, feature `debug-tools`): where in the hotel a scene
//! starts and what the game needs of the story for it. Records of 104 bytes; the name `Cnn_yy.TXT` starts
//! each, then 20 words (the list is not all records: names are looked for). What is known of the words was read from the game's own check of an event
//! (docs/DEBUG-TOOLS-PLAN.md): the rest is kept in `words`.

const RECORD: usize = 104;

/// One event: the scene it starts, where, and the story step it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// `Cnn_yy`, the scene's name without `.TXT`.
    pub scene: String,
    /// The map it is on.
    pub map: i32,
    /// Where it triggers (x, y, z, 1/4096 of a unit).
    pub pos: [i32; 3],
    /// The story value (`SETVALUE 0 n`) it needs at least; it also has a range, in `flags`' bit 5 (`low`..`high`).
    pub value_min: i32,
    pub flags: u32,
    pub low: i32,
    pub high: i32,
}

fn word(d: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// The scene name at `at`, if a record's name is there: `Xnn_` and the rest up to `.TXT`.
fn name(d: &[u8], at: usize) -> Option<&str> {
    let end = d.get(at..at + 24)?.iter().position(|b| *b == 0)?;
    let s = std::str::from_utf8(&d[at..at + end]).ok()?;
    let s = s.strip_suffix(".TXT")?;
    let b = s.as_bytes();
    (b.len() >= 6 && b[0].is_ascii_uppercase() && b[1].is_ascii_digit() && b[2].is_ascii_digit() && b[3] == b'_').then_some(s)
}

/// The events of a `.EVN` file, in file order: every place a record's name is (the records are 104 bytes
/// apart, but not all of the file is records).
pub fn parse(d: &[u8]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + RECORD <= d.len() {
        let Some(scene) = name(d, at) else {
            at += 4;
            continue;
        };
        let w = |i: usize| word(d, at + 4 * i);
        out.push(Event { scene: scene.to_string(), map: w(15), pos: [w(8), w(9), w(10)], value_min: w(13), flags: w(14) as u32, low: w(17), high: w(18) });
        at += RECORD;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, words: &[(usize, i32)]) -> Vec<u8> {
        let mut r = vec![0u8; RECORD];
        r[..name.len()].copy_from_slice(name.as_bytes());
        for (i, v) in words {
            r[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        r
    }

    #[test]
    fn records_follow_the_header_at_their_size() {
        let mut d = vec![0u8; 16];
        d.extend(record("C04_16.TXT", &[(8, -19333), (9, 13926), (10, -76595), (13, 60), (14, 13), (15, 27)]));
        d.extend(record("C04_17.TXT", &[(15, 27), (17, 70), (18, 89)]));
        let e = parse(&d);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], Event { scene: "C04_16".into(), map: 27, pos: [-19333, 13926, -76595], value_min: 60, flags: 13, low: 0, high: 0 });
        assert_eq!((e[1].scene.as_str(), e[1].low, e[1].high), ("C04_17", 70, 89));
    }

    #[test]
    fn a_file_without_names_has_no_events() {
        assert!(parse(&[0u8; 400]).is_empty());
    }
}
