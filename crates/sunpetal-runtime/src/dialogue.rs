//! What the game shows in its dialogue window, read from the guest's memory (feature
//! `debug-tools`). The addresses are those of the European dump (`YHNP`); they were found by
//! tracing the recompiled game and are listed in docs/DEBUG-TOOLS-PLAN.md.
//!
//! The game loads every line of a scene when the scene starts: the loader at 0204253C reads the
//! `[[ID` line of the id in `LOADING_ID` from the scene's `.s` and fills the next slot of the
//! table at `SLOTS` (its number is counted at `SLOT_COUNT`; the slot's `+8` is the text). The
//! window keeps the text it shows at `SHOWN`. The slots are all of the scene's compiled commands
//! (`[op, place in the script, parameters...]`): a `SETFACE2` before the line gives the speaker. So the id of a shown line is the id loaded into
//! the slot that holds its text.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::nds::debug;

/// The function that reads a line of a script (Thumb), and where the loader of a line by its id
/// calls it from: by then the loader has found the id in the scene.
const READ_LINE: u32 = 0x0203_FAE5;
const READ_LINE_FROM_LOADER: u32 = 0x0204_260F;
/// The id the loader is asked for, and the number of the slot it fills next.
const LOADING_ID: u32 = 0x0207_40D4;
const SLOT_COUNT: u32 = 0x0207_40A0;
/// The table of pointers to the loaded lines' slots; a slot's `+8` points to its UTF-16 text.
const SLOTS: u32 = 0x0207_4BEC;
/// The pointer to the text of the line the window shows (0 when none).
const SHOWN: u32 = 0x0207_5A28;

/// The word the game sets when `SETFACE2` has put a face in the window (`0204917C` handler: `[020759EC + 0x34]`).
const PORTRAIT_SHOWN: u32 = 0x0207_5A20;
/// Where the dialogue window's text is drawn: from `TEXT_X0`, up to (not with) `TEXT_X1`, on the screen's 256
/// columns. The game has one layout, the portrait at the left (its frame's rule at x = 48); nothing the
/// scripts give (`SETFACE2` parameters) moves either. Measured, the text of a line of `W` of known widths
/// (docs/debug-tools.md): ink starts at 58 and the last column that is drawn is 242, whatever the portrait.
pub const TEXT_X0: u32 = 58;
pub const TEXT_X1: u32 = 243;

fn word(addr: u32) -> Option<u32> {
    debug::peek(addr, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The writer the dialogue window draws with: its first word is the font.
const WRITER: u32 = 0x0207_5AFC;
/// The space the writer puts between letters, and the most the window takes.
const LETTER_SPACE: i32 = 1;
pub const MAX_WIDTH: u32 = 194;
pub const MAX_LINES: usize = 3;
/// The command the game compiles `SETFACE2` to, and `TEXT_EX`.
const OP_SETFACE2: u32 = 0x35;
const OP_TEXT_EX: u32 = 0x36;

fn half(addr: u32) -> Option<u32> {
    debug::peek(addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32)
}

/// A line's width in pixels: each letter but the last takes its advance and the space between letters, the
/// last takes the width of its glyph (as the game draws it, measured in-game). `metrics` gives a UTF-16 unit's
/// (glyph width, advance).
pub fn measure(line: &[u16], metrics: impl Fn(u32) -> Option<(i32, i32)>) -> Option<u32> {
    let mut sum = 0;
    for (i, u) in line.iter().enumerate() {
        let (glyph, advance) = metrics(*u as u32)?;
        sum += if i + 1 < line.len() { advance + LETTER_SPACE } else { glyph };
    }
    Some(sum.max(0) as u32)
}

/// The width of `text`'s lines in pixels as the game's font gives it (see `measure`).
pub fn line_widths(text: &str) -> Option<Vec<u32>> {
    let font = word(WRITER)?;
    let (default_glyph, default_width) = (half(font + 2)?, font + 4);
    let (cwdh, cmap) = (word(font + 0xC)?, word(font + 0x10)?);
    let glyph_of = |c: u32| -> Option<u32> {
        let mut node = cmap;
        while node != 0 {
            let (first, last, kind) = (half(node)?, half(node + 2)?, half(node + 4)?);
            if (first..=last).contains(&c) {
                let g = match kind {
                    0 => half(node + 0xC)? + (c - first),
                    1 => half(node + 0xC + 2 * (c - first))?,
                    _ => {
                        let n = half(node + 0xC)?;
                        let mut found = 0xFFFF;
                        for i in 0..n {
                            if half(node + 0xE + 4 * i)? == c {
                                found = half(node + 0x10 + 4 * i)?;
                                break;
                            }
                        }
                        found
                    }
                };
                return Some(g);
            }
            node = word(node + 8)?;
        }
        Some(0xFFFF)
    };
    // the (glyph width, advance) of a glyph from the CWDH chain: three signed bytes, left, glyph, advance
    let widths = |glyph: u32| -> Option<(i32, i32)> {
        let mut node = cwdh;
        while node != 0 {
            let (first, last) = (half(node)?, half(node + 2)?);
            if (first..=last).contains(&glyph) {
                return debug::peek(node + 8 + 3 * (glyph - first), 3).map(|b| (b[1] as i8 as i32, b[2] as i8 as i32));
            }
            node = word(node + 4)?;
        }
        debug::peek(default_width, 3).map(|b| (b[1] as i8 as i32, b[2] as i8 as i32))
    };
    text.split('\n')
        .map(|line| {
            let units: Vec<u16> = line.encode_utf16().collect();
            measure(&units, |u| {
                let mut g = glyph_of(u)?;
                if g == 0xFFFF {
                    g = default_glyph;
                }
                widths(g)
            })
        })
        .collect()
}

/// A line the window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// Where the game keeps its text.
    pub ptr: u32,
    /// The id of the `[[ID` line it was loaded from, if the loader was seen to load it.
    pub id: Option<u32>,
    /// The text as the game has it, line breaks as `\n`.
    pub text: String,
    /// The speaker: the first number of the `SETFACE2` before the line in the scene's commands.
    pub face: Option<u32>,
    /// The window shows a portrait now (`SETFACE2` with a face, not -1).
    pub portrait: bool,
}

/// Watches the loader: the slots and the ids loaded into them.
#[derive(Clone, Default)]
pub struct Tracker {
    ids: Rc<RefCell<HashMap<u32, u32>>>,
}

impl Tracker {
    pub fn install() -> Tracker {
        let t = Tracker::default();
        let ids = t.ids.clone();
        // the loader has the id by the time it first reads a line of the `.s`
        debug::watch(9, READ_LINE, move |c| {
            if c.r[14] == READ_LINE_FROM_LOADER {
                if let (Some(slot), Some(id)) = (word(SLOT_COUNT), word(LOADING_ID)) {
                    ids.borrow_mut().insert(slot, id);
                }
            }
        });
        t
    }

    /// The lines the game has loaded for the scene: (the command's address, the id, the pointer to its text).
    pub fn loaded(&self) -> Vec<(u32, u32, u32)> {
        let count = word(SLOT_COUNT).unwrap_or(0);
        let ids = self.ids.borrow();
        (0..count)
            .filter_map(|i| {
                let cmd = word(SLOTS + 4 * i)?;
                let (id, ptr) = (*ids.get(&i)?, word(cmd + 8)?);
                (ptr != 0 && word(cmd)? == OP_TEXT_EX).then_some((cmd, id, ptr))
            })
            .collect()
    }

    /// Makes the window show the text at `ptr` where it shows the one at `old`.
    pub fn rebind_shown(old: u32, ptr: u32) {
        if word(SHOWN) == Some(old) {
            debug::poke(SHOWN, &ptr.to_le_bytes());
        }
    }

    /// The line in the window now, if there is one.
    pub fn shown(&self) -> Option<Shown> {
        let ptr = word(SHOWN).filter(|p| *p != 0)?;
        let text = utf16_at(ptr)?;
        let count = word(SLOT_COUNT)?;
        let slot = (0..count).rev().find(|i| {
            word(SLOTS + 4 * i).and_then(|s| word(s + 8)) == Some(ptr)
        });
        let id = slot.and_then(|s| self.ids.borrow().get(&s).copied());
        let face = slot.and_then(|s| {
            (0..s).rev().find_map(|j| {
                let cmd = word(SLOTS + 4 * j)?;
                (word(cmd)? == OP_SETFACE2).then(|| word(cmd + 8)).flatten()
            })
        });
        let portrait = word(PORTRAIT_SHOWN).is_some_and(|f| f != 0);
        Some(Shown { ptr, id, text, face, portrait })
    }
}

/// The text of the game's UTF-16 string at `ptr`, up to its end (a bounded read).
pub fn utf16_at(ptr: u32) -> Option<String> {
    let bytes = debug::peek(ptr, 1024)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    Some(String::from_utf16_lossy(&units).replace("\r\n", "\n").replace('\r', "\n"))
}
