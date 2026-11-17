//! Input from a script, for headless runs: buttons and the stylus at given VBlanks.
//!
//! ```text
//! VBL:BUTTONS[:FRAMES],VBL:tap:X:Y[:FRAMES],...
//! ```
//!
//! BUTTONS are DS names joined by '+' (A B X Y L R START SELECT UP DOWN LEFT RIGHT); a tap holds
//! the stylus at (X, Y) of the lower screen. Each lasts FRAMES VBlanks (9 by default, about the
//! oracle's 0.15 s press).
use super::Nds;

#[derive(Clone, Copy)]
pub(super) struct Press {
    from: u64,
    to: u64,
    keys: u32,
    tap: bool,
    x: i32,
    y: i32,
}

fn key_bit(k: &str) -> Result<u32, String> {
    // bits 10, 11: X and Y (EXTKEYIN 0, 1)
    KEY_NAMES.iter().position(|n| *n == k).map(|i| 1 << i).ok_or_else(|| format!("input: no button {}", k))
}

pub(super) fn parse(s: &str) -> Result<Vec<Press>, String> {
    let mut out = Vec::new();
    for item in s.split(',').filter(|i| !i.is_empty()) {
        let f: Vec<&str> = item.split(':').collect();
        if f.len() < 2 {
            return Err(format!("input: {} is not VBL:WHAT", item));
        }
        let from: u64 = f[0].parse().map_err(|_| format!("input: {}: bad VBlank", item))?;
        let mut p = Press { from, to: 0, keys: 0, tap: false, x: 0, y: 0 };
        let mut dur = 2;
        if f[1] == "tap" {
            if f.len() < 4 {
                return Err(format!("input: {}: tap:X:Y", item));
            }
            p.tap = true;
            p.x = f[2].parse().map_err(|_| format!("input: {}: bad X", item))?;
            p.y = f[3].parse().map_err(|_| format!("input: {}: bad Y", item))?;
            dur = 4;
        } else {
            for k in f[1].split('+') {
                p.keys |= key_bit(k)?;
            }
        }
        let frames: u64 = match f.get(dur) {
            Some(n) => n.parse().map_err(|_| format!("input: {}: bad length", item))?,
            None => 9,
        };
        p.to = from + frames;
        out.push(p);
    }
    Ok(out)
}

impl Nds {
    /// At each VBlank: what is held now.
    pub(super) fn input_vblank(&mut self, vbl: u64) {
        if self.script.is_empty() {
            return;
        }
        let (mut keys, mut pen, mut x, mut y) = (0u32, false, 0, 0);
        for p in &self.script {
            if vbl < p.from || vbl >= p.to {
                continue;
            }
            keys |= p.keys;
            if p.tap {
                pen = true;
                x = p.x;
                y = p.y;
            }
        }
        self.keys = !keys as u16 & 0x3FF;
        self.extkeys = 0x7F & !((keys >> 10) & 3) as u16 & !(if pen { 0x40 } else { 0 });
        self.spi_set_pen(pen, x, y);
    }
}

impl Nds {
    /// Live input (a window, a pad): the buttons held (bit i of `KEY_NAMES`: A B SELECT START RIGHT
    /// LEFT UP DOWN R L X Y) and the stylus on the lower screen, if down. Replaces the script's.
    pub(super) fn set_input(&mut self, keys: u32, pen: Option<(i32, i32)>) {
        self.input_record(keys, pen);
        self.keys = !keys as u16 & 0x3FF;
        let down = pen.is_some();
        self.extkeys = 0x7F & !((keys >> 10) & 3) as u16 & !(if down { 0x40 } else { 0 });
        let (x, y) = pen.unwrap_or((0, 0));
        self.spi_set_pen(down, x, y);
    }
}

/// A recording of live input in the script's own format (`SUNPETAL_INPUT_REC=FILE`): each change
/// of what is held closes the previous press, so that the file replays as `--input "$(cat FILE)"`
/// at the same VBlanks.
pub(super) struct InputRec {
    path: std::path::PathBuf,
    cur: (u32, Option<(i32, i32)>),
    since: u64,
    /// the next VBlank a checkpoint (`FILE.chk`, see `checkpoint_line`) is written at
    next_check: u64,
}

/// A hotkey's line in `FILE.hot` of a recording: `VBL:WHAT`. The hotkeys (the originals' graphics F12, the skip F7) change
/// what is drawn and how fast the machine runs, never the game's state, so a replay of `FILE` needs them only to be seen again.
pub fn hotkey_line(vblank: u64, what: &str) -> String {
    format!("{}:{}", vblank, what)
}

/// Checkpoints go in every this many VBlanks of a recording.
pub const CHECK_EVERY: u64 = 300;

impl InputRec {
    pub(super) fn from_env() -> Option<InputRec> {
        let path = std::path::PathBuf::from(std::env::var_os("SUNPETAL_INPUT_REC")?);
        let _ = std::fs::write(&path, "");
        let _ = std::fs::write(path.with_extension("chk"), "");
        let _ = std::fs::write(path.with_extension("hot"), "");
        Some(InputRec { path, cur: (0, None), since: 0, next_check: 0 })
    }
}

impl Nds {
    /// The file a checkpoint is due into, if a recording runs and `vblanks` has reached the next checkpoint.
    pub(super) fn check_due(&mut self) -> Option<(std::path::PathBuf, u64)> {
        let now = self.vblanks;
        let r = self.inrec.as_mut()?;
        if now < r.next_check {
            return None;
        }
        r.next_check = now / CHECK_EVERY * CHECK_EVERY + CHECK_EVERY;
        Some((r.path.with_extension("chk"), now))
    }

    /// Writes a hotkey's line to the recording's `.hot` file; nothing when no recording runs.
    pub(super) fn note_hotkey(&mut self, what: &str) {
        let now = self.vblanks;
        let Some(r) = &self.inrec else { return };
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(r.path.with_extension("hot")) {
            let _ = writeln!(f, "{}", hotkey_line(now, what));
        }
    }

    fn input_record(&mut self, keys: u32, pen: Option<(i32, i32)>) {
        let now = self.vblanks;
        let Some(r) = &mut self.inrec else { return };
        if r.cur == (keys, pen) {
            return;
        }
        let (k, p) = r.cur;
        let frames = now - r.since;
        let mut item = String::new();
        if k != 0 {
            let names: Vec<&str> = KEY_NAMES.iter().enumerate().filter(|(i, _)| k >> i & 1 != 0).map(|(_, n)| *n).collect();
            item.push_str(&format!("{}:{}:{},", r.since, names.join("+"), frames.max(1)));
        }
        if let Some((x, y)) = p {
            item.push_str(&format!("{}:tap:{}:{}:{},", r.since, x, y, frames.max(1)));
        }
        if !item.is_empty() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&r.path) {
                let _ = f.write_all(item.as_bytes());
            }
        }
        r.cur = (keys, pen);
        r.since = now;
    }
}

/// The names of the buttons, by bit of the mask `set_input` takes.
pub const KEY_NAMES: [&str; 12] = ["A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L", "X", "Y"];

#[cfg(test)]
mod hotkey_tests {
    #[test]
    fn a_hotkey_line_is_the_vblank_and_the_word() {
        assert_eq!(super::hotkey_line(30245, "originals:on"), "30245:originals:on");
    }
}
