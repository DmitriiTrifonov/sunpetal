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
    const NAMES: [&str; 12] = ["A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L", "X", "Y"];
    NAMES.iter().position(|n| *n == k).map(|i| 1 << i).ok_or_else(|| format!("input: no button {}", k))
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
