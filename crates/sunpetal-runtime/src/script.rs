//! The two text formats of a scene (feature `debug-tools`), read from their decoded text: the script
//! (`Cnn_yy.txt`, tags with a parameter on each line) and the lines of speech (`Cnn_yy.s`, `[[ID` then the
//! text, `<R>` where a line breaks). Used to check what the game shows against the files, and to replace
//! the text while the game runs.

/// One tag of a script and the parameters between its opening and closing: `<SETFACE2>`, `1`, `0`, `</SETFACE2>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub tag: String,
    pub params: Vec<String>,
}

/// A line of speech: its id (five digits as written) and its text, `\n` between the lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub text: String,
}

/// A parameter's own text: what is left of the line once the `//` comment is off.
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or("").trim()
}

/// ends the script's last tag at its end.
pub fn commands(src: &str) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    let mut open: Option<Command> = None;
    for line in src.lines().map(code).filter(|l| !l.is_empty()) {
        let name = line.strip_prefix('<').and_then(|l| l.strip_suffix('>'));
        match (name, &mut open) {
            (Some(n), Some(c)) if n.strip_prefix('/') == Some(c.tag.as_str()) => out.extend(open.take()),
            // a tag whose closing never came ends where the next one opens
            (Some(n), _) if !n.starts_with('/') => {
                out.extend(open.take());
                open = Some(Command { tag: n.to_string(), params: Vec::new() });
            }
            (_, Some(c)) => c.params.push(line.to_string()),
            _ => {}
        }
    }
    out.extend(open);
    out
}

/// The lines of speech of a `.s` file, in order. A header is `[[` and the id's digits; the text ends at the
/// next header; `<R>` and the line end after it make one `\n`; blank lines at its end are not text.
pub fn lines(src: &str) -> Vec<Line> {
    let src = src.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out = Vec::new();
    for chunk in src.split("[[").skip(1) {
        let (head, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        let id: String = head.trim().to_string();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // the file puts a line end each side of `<R>`: together they are one break
        let text = body.trim_end_matches('\n').replace("\n<R>\n", "\n").replace("<R>\n", "\n").replace("\n<R>", "\n").replace("<R>", "\n");
        out.push(Line { id, text });
    }
    out
}

/// A file of the game's `.z` kind: LZ10 (a header word, `0x10` and the size, then groups of eight, a flag
/// byte first, most significant bit first: 0 a byte of its own, 1 two bytes of length 3-18 and distance 1-4096
/// back).
pub fn lz10(d: &[u8]) -> Option<Vec<u8>> {
    if *d.first()? != 0x10 {
        return None;
    }
    let size = d[1] as usize | (d[2] as usize) << 8 | (d[3] as usize) << 16;
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut at = 4;
    while out.len() < size {
        let flags = *d.get(at)?;
        at += 1;
        for bit in (0..8).rev() {
            if out.len() >= size {
                break;
            }
            if flags >> bit & 1 == 0 {
                out.push(*d.get(at)?);
                at += 1;
            } else {
                let (a, b) = (*d.get(at)? as usize, *d.get(at + 1)? as usize);
                at += 2;
                let (len, back) = ((a >> 4) + 3, ((a & 15) << 8 | b) + 1);
                if back > out.len() {
                    return None;
                }
                for _ in 0..len {
                    out.push(out[out.len() - back]);
                }
            }
        }
    }
    out.truncate(size);
    Some(out)
}

/// The text of a UTF-16LE file (with or without the byte order mark).
pub fn utf16le(d: &[u8]) -> String {
    let units: Vec<u16> = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string()
}

/// The map a script leads to: the first parameter of its first `CHANGEMAP` or `CHANGEMAP2`.
pub fn map_target(src: &str) -> Option<i32> {
    commands(src).iter().find(|c| c.tag == "CHANGEMAP" || c.tag == "CHANGEMAP2").and_then(|c| c.params.first()).and_then(|p| p.split_whitespace().next()).and_then(|p| p.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_with_their_parameters_and_comments_off() {
        let c = commands("// a note\n<SETFACE2>\n1\n0 // the face\n</SETFACE2>\n<TEXT_EX>\n16001\n</TEXT_EX>\n// <WAIT>\n<WAIT>\n</WAIT>\n");
        assert_eq!(
            c,
            vec![
                Command { tag: "SETFACE2".into(), params: vec!["1".into(), "0".into()] },
                Command { tag: "TEXT_EX".into(), params: vec!["16001".into()] },
                Command { tag: "WAIT".into(), params: vec![] },
            ]
        );
    }

    #[test]
    fn a_tag_left_open_ends_with_the_script() {
        let c = commands("<TEXT_EX>\n7\n");
        assert_eq!(c, vec![Command { tag: "TEXT_EX".into(), params: vec!["7".into()] }]);
    }

    #[test]
    fn lines_of_speech_break_at_r_and_end_at_the_next_header() {
        let l = lines("\u{feff}[[16001\r\nfirst\r\n<R>\r\nsecond\r\n\r\n[[16002\r\nonly\r\n\r\n");
        assert_eq!(
            l,
            vec![Line { id: "16001".into(), text: "first\nsecond".into() }, Line { id: "16002".into(), text: "only".into() }]
        );
    }

    #[test]
    fn lz10_gives_back_literals_and_copies() {
        // "abc", then three bytes copied from three back
        let z = [0x10, 6, 0, 0, 0x10, b'a', b'b', b'c', 0x00, 0x02];
        assert_eq!(lz10(&z), Some(b"abcabc".to_vec()));
        assert_eq!(lz10(&[0x11, 0, 0, 0]), None);
        // a copy from before the start is not a file
        assert_eq!(lz10(&[0x10, 3, 0, 0, 0x80, 0x00, 0x05]), None);
    }

    #[test]
    fn a_door_script_names_its_map() {
        let t = "<CHANGEMAP>\t\t// a note\n45\t\t// map\n3.485\t0.0\t1.9\n180\n</CHANGEMAP>\n";
        assert_eq!(map_target(t), Some(45));
        assert_eq!(map_target("<WAIT>\n</WAIT>\n"), None);
        assert_eq!(utf16le(&[0xFF, 0xFE, b'a', 0, b'b', 0]), "ab");
    }

    #[test]
    fn a_header_without_digits_is_not_a_line() {
        assert_eq!(lines("[[x1\ntext\n[[00001\nyes\n").len(), 1);
    }

    #[test]
    fn the_last_letter_of_a_line_counts_by_its_glyph() {
        use crate::dialogue::measure;
        // every letter: glyph 5, advance 6, so (n - 1) * 7 + 5
        let font = |_: u32| Some((5, 6));
        assert_eq!(measure(&[1], font), Some(5));
        assert_eq!(measure(&[1, 2, 3, 4], font), Some(26));
        assert_eq!(measure(&[], font), Some(0));
        assert_eq!(measure(&[1], |_| None), None);
    }
}
