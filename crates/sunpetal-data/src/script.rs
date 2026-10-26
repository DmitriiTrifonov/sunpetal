//! The dialogue scripts (`.s`, UTF-16): a head, then entries, each a header line `[[ID` and a body
//! of the entry's lines; inside a body the text lines are separated by `<R>` lines, and a line that
//! starts with `//` is the translator's note. The same format and the same rules as the
//! translation project's `insert.py`.

/// UTF-16 with a byte-order mark (little-endian without one) to text.
pub fn decode_utf16(bytes: &[u8]) -> Result<String, String> {
    let (le, body) = match bytes {
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        _ => (true, bytes),
    };
    if body.len() % 2 != 0 {
        return Err("UTF-16 text of an odd length".into());
    }
    let units: Vec<u16> = body.chunks(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
    String::from_utf16(&units).map_err(|e| e.to_string())
}

/// Text to UTF-16, little-endian, with the byte-order mark.
pub fn encode_utf16(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

/// The head (what is before the first header) and the entries (ID, body).
pub fn parse(text: &str) -> (String, Vec<(String, String)>) {
    let mut heads = Vec::new(); // (start of the header line, id, start of the body)
    let mut at = 0;
    while at < text.len() {
        let line_end = text[at..].find('\n').map(|i| at + i + 1);
        if let (true, Some(end)) = (text[at..].starts_with("[["), line_end) {
            let rest = &text[at + 2..end - 1];
            let rest = rest.strip_suffix('\r').unwrap_or(rest);
            let id_len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            if rest[id_len..].chars().all(|c| c == ' ' || c == '\t') {
                heads.push((at, rest[..id_len].to_string(), end));
            }
        }
        match line_end {
            Some(e) => at = e,
            None => break,
        }
    }
    let head = text[..heads.first().map_or(text.len(), |h| h.0)].to_string();
    let entries = heads
        .iter()
        .enumerate()
        .map(|(i, (_, id, body))| (id.clone(), text[*body..heads.get(i + 1).map_or(text.len(), |h| h.0)].to_string()))
        .collect();
    (head, entries)
}

pub fn build(head: &str, entries: &[(String, String)]) -> String {
    let mut out = head.to_string();
    for (id, body) in entries {
        out.push_str(&format!("[[{}\r\n{}", id, body));
    }
    out
}

/// A body split into the lines before the text, the text lines (without the `<R>` lines and the
/// notes) and the lines after; and whether notes stand inside the text.
pub struct Body {
    pub head: Vec<String>,
    pub text: Vec<String>,
    pub tail: Vec<String>,
    pub inner_notes: bool,
}

pub fn split_body(body: &str) -> Body {
    let lines: Vec<&str> = body.split("\r\n").collect();
    let used: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].is_empty() && !lines[i].starts_with("//")).collect();
    let (Some(&f), Some(&l)) = (used.first(), used.last()) else {
        return Body { head: lines.iter().map(|s| s.to_string()).collect(), text: Vec::new(), tail: Vec::new(), inner_notes: false };
    };
    Body {
        head: lines[..f].iter().map(|s| s.to_string()).collect(),
        text: lines[f..=l].iter().filter(|x| **x != "<R>" && !x.starts_with("//")).map(|s| s.to_string()).collect(),
        tail: lines[l + 1..].iter().map(|s| s.to_string()).collect(),
        inner_notes: lines[f..=l].iter().any(|x| x.starts_with("//")),
    }
}

pub fn make_body(head: &[String], text: &[String], tail: &[String]) -> String {
    let mut parts: Vec<String> = head.to_vec();
    parts.push(text.join("\r\n<R>\r\n"));
    parts.extend(tail.iter().cloned());
    parts.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "// note\r\n[[01001\r\nHello\r\n<R>\r\nthere\r\n\r\n[[01002  \r\n// who says it\r\nBye\r\n";

    #[test]
    fn it_parses_and_builds_back() {
        let (head, entries) = parse(SAMPLE);
        assert_eq!(head, "// note\r\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "01001");
        assert_eq!(entries[1].0, "01002");
        assert_eq!(build(&head, &entries), SAMPLE.replace("[[01002  ", "[[01002"));
    }

    #[test]
    fn it_splits_the_text_out_of_a_body() {
        let b = split_body("// who\r\nHello\r\n<R>\r\nthere\r\n\r\n");
        assert_eq!(b.text, ["Hello", "there"]);
        assert_eq!(b.head, ["// who"]);
        assert_eq!(b.tail, ["", ""]);
        assert!(!b.inner_notes);
        let b2 = split_body("Hello\r\n// inside\r\nthere\r\n");
        assert!(b2.inner_notes);
        assert_eq!(make_body(&b.head, &["A".into(), "B".into()], &b.tail), "// who\r\nA\r\n<R>\r\nB\r\n\r\n");
    }

    #[test]
    fn utf16_goes_both_ways() {
        let t = "Привет, мир\r\n";
        assert_eq!(decode_utf16(&encode_utf16(t)).unwrap(), t);
        assert_eq!(decode_utf16(&[b'a', 0, b'b', 0]).unwrap(), "ab");
    }
}
