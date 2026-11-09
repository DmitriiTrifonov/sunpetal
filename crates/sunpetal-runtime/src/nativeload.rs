//! The native command-table loader (K-198, step E2c, `docs/NATIVE-ENGINE-PLAN.md`): the game reads the scene's text once
//! and builds a record for each tag (`0x02074BEC`, an array of pointers to records).
//! Behind `SUNPETAL_NATIVE_LOAD=1`; the lexer (`0x0203F9D8`, `0x0203F804`) stays the game's. The safe points are the
//! game's: one before each call and one for each jump back.
use crate::abi::Cpu;
use crate::nativechar::spend;
use crate::services::{mem_read32, mem_write32};

/// The loader's state. The words below (`STATE + ...`) are the ones the loader functions share.
const STATE: u32 = 0x0207_4084;
const ST_NAME_COUNT: u32 = 8; // how many names the names section has read into `NAME_RECS`
const ST_RECORD: u32 = 0x1C; // the index of the record being built (`TABLE[...]`)
const ST_MARK: u32 = 0x24; // zeroed where the names section starts; one less goes into a new record's `REC_MARK`
const ST_NUM0: u32 = 0x50; // the numbers the number reader (`0x0203F804`) has just read, in order
const ST_NUM1: u32 = 0x54;
const ST_LEN_A: u32 = 0x38; // the sizes of the two texts `0x0203FB68` has just made
const ST_LEN_B: u32 = 0x3C;
const ST_ENABLED: u32 = 0x10; // `message`: nonzero when there is something to look for
const ST_RESET_A: u32 = 4; // `message`: cleared at the start of the search
const ST_RESET_B: u32 = 0x18;
/// The table of records (pointers) the loader builds: one per tag.
const TABLE: u32 = 0x0207_4BEC;
/// A record: the tag's name index, then the fields that its function fills.
const REC_MARK: u32 = 4;
const REC_F0: u32 = 8;
const REC_F1: u32 = 0xC;
const REC_F2: u32 = 0x10;
const REC_BLOCKS: u32 = 8; // `fields`: blocks at `+ 8 + 4 * k`, the indexes of the matching names at `+ 0x18 + 4 * k`
const REC_BLOCK_NAMES: u32 = 0x18;
const REC_LIST_NUMS: u32 = 0x10; // `list`: numbers at `+ 0x10 + 4 * k`, the indexes of the matching names at `+ 0x20 + 4 * k`
const REC_LIST_NAMES: u32 = 0x20;
/// The tag names: `NAMES` is the table of pointers to their texts (the first is the line break's).
const NAMES: u32 = 0x0206_5568;
/// The names the names section reads: 8 bytes each, a word and the pointer to the text.
const NAME_RECS: u32 = 0x0207_47EC;
/// The token's text: a pointer for each kind of token, in three banks.
const TOK_A: u32 = 0x0206_5364;
const TOK_B: u32 = 0x0206_53E4;
const TOK_C: u32 = 0x0206_5464;
/// The closing tag's text (128 UTF-16 units), the next tag's (`TAG`), the two texts the names are compared with.
const CLOSE: u32 = 0x0207_40EC;
const TAG: u32 = 0x0207_41EC;
const NAME_TEXT_A: u32 = 0x0207_44EC;
const NAME_TEXT_B: u32 = 0x0207_456C;
const MSG: u32 = 0x0207_57EC;

unsafe fn call(c: *mut Cpu, key: u32, ret: u32, r0: u32, r1: u32) {
    (*c).r[0] = r0;
    (*c).r[1] = r1;
    (*c).r[14] = ret;
    spend(c, 1);
    crate::call(&mut *c, key);
}

unsafe fn rd16(c: *mut Cpu, a: u32) -> u16 {
    (mem_read32(c, a & !3) >> ((a & 2) * 8)) as u16
}

unsafe fn record(c: *mut Cpu) -> u32 {
    mem_read32(c, TABLE + 4 * mem_read32(c, STATE + ST_RECORD))
}

/// Does the token's text equal the closing tag's? `Some(true)` equal to the end, `Some(false)` differs; the loop
/// of 128 units that run equal and not ended falls out as the game's does (`None`), with the safe points it passed.
pub fn compare(close: &[u16], text: &[u16]) -> (Option<bool>, u32) {
    for i in 0..close.len().min(text.len()) {
        if close[i] != text[i] {
            return (Some(false), i as u32);
        }
        if close[i] == 0 {
            return (Some(true), i as u32);
        }
    }
    (None, 127)
}

/// One row of `docs/native-load-hooks.csv`: a function of the `skip` shape (read tokens until the token's text is the
/// closing tag's, then the record index goes up by one).
struct Skip {
    key: u32,
    close: u32,
    base: u32,
    off: u32,
    state: u32,
    idx: u32,
    ret: u32,
}

fn num(s: &str) -> Result<u32, String> {
    let s = s.trim();
    match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse(),
    }
    .map_err(|e| format!("{s}: {e}"))
}

fn parse(csv: &str) -> Result<Vec<Skip>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.split(',').collect();
        if f.len() != 9 || f[1] != "skip" || f[7] != "0" {
            return Err(format!("bad row: {l}"));
        }
        out.push(Skip { key: num(f[0])?, close: num(f[2])?, base: num(f[3])?, off: num(f[4])?, state: num(f[5])?, idx: num(f[6])?, ret: num(f[8])? });
    }
    Ok(out)
}

fn rows() -> &'static Vec<Skip> {
    static ROWS: std::sync::OnceLock<Vec<Skip>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| parse(include_str!("../../../docs/native-load-hooks.csv")).expect("native-load-hooks.csv"))
}

unsafe fn units(c: *mut Cpu, base: u32) -> Vec<u16> {
    (0..128).map(|i| rd16(c, base + 2 * i)).collect()
}

unsafe fn run_skip(c: *mut Cpu, i: usize) {
    let r = &rows()[i];
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load {:#x}", r.key & !1);
    }
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);
    loop {
        call(c, 0x0203_F9D9, r.ret, 0, 0);
        let text = mem_read32(c, r.base + r.off);
        let (res, polls) = compare(&units(c, r.close), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            let a = r.state + r.idx;
            mem_write32(c, a, mem_read32(c, a).wrapping_add(1));
            break;
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

macro_rules! slots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run_skip(c, $i) })*
        const SLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
slots!(
    s0 = 0,
    s1 = 1,
    s2 = 2,
    s3 = 3,
    s4 = 4,
    s5 = 5,
    s6 = 6,
    s7 = 7,
    s8 = 8,
    s9 = 9,
    s10 = 10,
    s11 = 11,
    s12 = 12,
    s13 = 13,
    s14 = 14,
    s15 = 15,
    s16 = 16,
    s17 = 17,
    s18 = 18,
    s19 = 19,
    s20 = 20,
    s21 = 21,
    s22 = 22,
    s23 = 23
);

/// One row of `docs/native-load-args.csv`: reads numbers until the closing tag; on the first turn it copies the words
/// `+0x50…` of the loader state into the record (`pairs`); the record's words in `clears` are zeroed first. Nothing
/// copied by the end: the game's warning.
struct Args {
    key: u32,
    close: u32,
    base: u32,
    off: u32,
    clears: Vec<(u32, u32)>,
    pairs: Vec<(u32, u32, bool, bool)>, // (from, to, got, continues in the next entry: the same case)
    seq: bool,
    push: Vec<usize>,
    ret_lex: u32,
    ret_val: u32,
    ret_warn: u32,
}

fn parse_args(csv: &str) -> Result<Vec<Args>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.split(',').collect();
        if f.len() != 12 || f[1] != "args" || !matches!(f[10], "once" | "seq") {
            return Err(format!("bad row: {l}"));
        }
        let clears =
            f[5].split_whitespace().map(|p| p.split_once('=').ok_or(format!("bad clear {p}")).and_then(|(a, b)| Ok((num(a)?, num(b)?)))).collect::<Result<Vec<_>, String>>()?;
        let pairs = f[6]
            .split_whitespace()
            .map(|p| {
                let v: Vec<&str> = p.split(':').collect();
                if v.len() != 3 {
                    return Err(format!("bad pair {p}"));
                }
                Ok((num(v[0])?, num(v[1])?, v[2] == "1", v[2] == "c"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        out.push(Args {
            key: num(f[0])?,
            close: num(f[2])?,
            base: num(f[3])?,
            off: num(f[4])?,
            clears,
            pairs,
            seq: f[10] == "seq",
            push: f[11].split_whitespace().map(|x| x.parse::<usize>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?,
            ret_lex: num(f[7])?,
            ret_val: num(f[8])?,
            ret_warn: num(f[9])?,
        });
    }
    Ok(out)
}

fn arows() -> &'static Vec<Args> {
    static ROWS: std::sync::OnceLock<Vec<Args>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| parse_args(include_str!("../../../docs/native-load-args.csv")).expect("native-load-args.csv"))
}

unsafe fn run_args(c: *mut Cpu, i: usize) {
    let r = &arows()[i];
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load {:#x}", r.key & !1);
    }
    // the push {…, lr} of the game: the callees see the stack as its code leaves it
    let lr = (*c).r[14];
    let saved: Vec<u32> = r.push.iter().map(|&k| (*c).r[k]).collect();
    let sp = (*c).r[13];
    let frame = 4 * (saved.len() as u32 + 1);
    let a = sp.wrapping_sub(frame) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 4 * saved.len() as u32, lr);
    (*c).r[13] = sp.wrapping_sub(frame);

    let (mut got, mut n) = (false, 0usize);
    for &(off, v) in &r.clears {
        mem_write32(c, record(c) + off, v);
    }
    loop {
        call(c, 0x0203_F9D9, r.ret_lex, 0, 0);
        call(c, 0x0203_F805, r.ret_val, 1, 0);
        let text = mem_read32(c, r.base + r.off);
        let (res, polls) = compare(&units(c, r.close), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if !got {
                let cmd = record(c);
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, cmd));
                call(c, 0x0202_20F5, r.ret_warn, name, MSG);
            }
            mem_write32(c, STATE + ST_RECORD, mem_read32(c, STATE + ST_RECORD).wrapping_add(1));
            break;
        }
        if r.seq {
            // the n-th number goes to the n-th word; after the last the numbers are read and dropped
            // a case may copy several words; the entries marked `continues` belong to the same case
            let mut at = n;
            while let Some(&(from, to, g, more)) = r.pairs.get(at) {
                let v = mem_read32(c, STATE + from);
                mem_write32(c, record(c) + to, v);
                got |= g;
                at += 1;
                if !more {
                    break;
                }
            }
            n = at;
        } else if n == 0 {
            for &(from, to, _, _) in &r.pairs {
                let v = mem_read32(c, STATE + from);
                mem_write32(c, record(c) + to, v);
            }
            got = true;
            n = 1;
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = sp;
    for (&k, v) in r.push.iter().zip(saved) {
        (*c).r[k] = v;
    }
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

macro_rules! aslots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run_args(c, $i) })*
        const ASLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
aslots!(
    a0 = 0,
    a1 = 1,
    a2 = 2,
    a3 = 3,
    a4 = 4,
    a5 = 5,
    a6 = 6,
    a7 = 7,
    a8 = 8,
    a9 = 9,
    a10 = 10,
    a11 = 11,
    a12 = 12,
    a13 = 13,
    a14 = 14,
    a15 = 15,
    a16 = 16,
    a17 = 17,
    a18 = 18,
    a19 = 19,
    a20 = 20,
    a21 = 21,
    a22 = 22,
    a23 = 23,
    a24 = 24,
    a25 = 25,
    a26 = 26,
    a27 = 27,
    a28 = 28,
    a29 = 29,
    a30 = 30,
    a31 = 31,
    a32 = 32,
    a33 = 33,
    a34 = 34,
    a35 = 35,
    a36 = 36,
    a37 = 37,
    a38 = 38,
    a39 = 39,
    a40 = 40,
    a41 = 41,
    a42 = 42,
    a43 = 43,
    a44 = 44,
    a45 = 45,
    a46 = 46,
    a47 = 47,
    a48 = 48,
    a49 = 49,
    a50 = 50,
    a51 = 51,
    a52 = 52,
    a53 = 53,
    a54 = 54,
    a55 = 55,
    a56 = 56,
    a57 = 57,
    a58 = 58,
    a59 = 59,
    a60 = 60,
    a61 = 61,
    a62 = 62,
    a63 = 63,
    a64 = 64,
    a65 = 65,
    a66 = 66,
    a67 = 67,
    a68 = 68,
    a69 = 69,
    a70 = 70,
    a71 = 71,
    a72 = 72,
    a73 = 73,
    a74 = 74,
    a75 = 75,
    a76 = 76,
    a77 = 77,
    a78 = 78,
    a79 = 79
);

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_LOAD").is_ok_and(|v| v == "1")
}

/// `0x0203FC54`: read the names section. For each token that follows a `[[` tag the game allocates the token's
/// length in bytes, copies the tag's text there and appends the record (`0x020747EC`, 8 bytes each).
unsafe extern "C" fn names(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x0203fc54");
    }
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);
    for o in [0x24, 0x28, 0] {
        mem_write32(c, STATE + o, 0);
    }
    loop {
        call(c, 0x0203_F9D9, 0x0203_FC67, 0, 0);
        let t = (*c).r[0];
        if t == u32::MAX {
            break;
        }
        if rd16(c, CLOSE) == 0x5B && rd16(c, CLOSE + 2) == 0x5B {
            let n = t.wrapping_sub(1).wrapping_mul(2);
            let rec = NAME_RECS + (mem_read32(c, STATE + ST_NAME_COUNT) << 3);
            call(c, 0x0202_BFAD, 0x0203_FC8B, n, 0);
            let buf = (*c).r[0];
            mem_write32(c, rec + 4, buf);
            (*c).r[2] = n & 0xFFFF;
            call(c, 0x0200_9624, 0x0203_FC99, 0x0207_40F0, buf);
            let i = mem_read32(c, STATE + ST_NAME_COUNT);
            mem_write32(c, NAME_RECS + (i << 3), mem_read32(c, STATE + ST_MARK).wrapping_sub(1));
            mem_write32(c, STATE + ST_NAME_COUNT, i.wrapping_add(1));
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// `0x0203F774`: read a token and look its text up among the 0x85 tag names (`NAMES`). Returns the name's index after
/// it built the record (0x30 bytes, `TABLE[index of record]`), or -1 at the end of the text or when the table is full.
unsafe extern "C" fn lookup(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x0203f774");
    }
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);
    let out = 'outer: loop {
        call(c, 0x0203_F9D9, 0x0203_F77D, 0, 0);
        if (*c).r[0] == u32::MAX {
            break u32::MAX;
        }
        let close = units(c, CLOSE);
        for n in 0..0x85u32 {
            let text = mem_read32(c, NAMES + 4 * n);
            let (res, polls) = compare(&close, &units(c, text));
            spend(c, polls);
            if res == Some(true) {
                let idx = mem_read32(c, STATE + ST_RECORD);
                if idx >= 0x300 {
                    break 'outer u32::MAX;
                }
                call(c, 0x0202_BFA5, 0x0203_F7B7, 0x30, 0);
                let rec = (*c).r[0];
                if rec != 0 {
                    call(c, 0x0203_EF65, 0x0203_F7BF, rec, 0);
                }
                mem_write32(c, TABLE + 4 * idx, rec);
                mem_write32(c, rec, n);
                mem_write32(c, rec + REC_MARK, mem_read32(c, STATE + ST_MARK).wrapping_sub(1));
                break 'outer n;
            }
            spend(c, 1); // the next name, or the jump back to the lexer
        }
    };
    (*c).r[0] = out;
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// `0x02040D00`: read one token, then the record index goes up by one.
unsafe extern "C" fn bump(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x02040d00");
    }
    let (lr, r3) = ((*c).r[14], (*c).r[3]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(8) & !3;
    mem_write32(c, a, r3);
    mem_write32(c, a + 4, lr);
    (*c).r[13] = sp.wrapping_sub(8);
    call(c, 0x0203_F9D9, 0x0204_0D07, 0, 0);
    let i = mem_read32(c, STATE + ST_RECORD);
    mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
    (*c).r[13] = sp;
    (*c).r[3] = r3;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// push {r3-r7, lr}: the registers go to the stack as the game's, so a callee sees the same sp.
unsafe fn enter(c: *mut Cpu) -> (u32, [u32; 5], u32) {
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);
    (lr, saved, sp)
}

/// pop {r3-r7, pc}; `r3` is what the function left in its first stack slot (the game's own `str r?, [sp]`).
unsafe fn leave(c: *mut Cpu, (lr, saved, sp): (u32, [u32; 5], u32), r3: Option<u32>) {
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    if let Some(v) = r3 {
        (*c).r[3] = v;
    }
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// A function of the text-record shape: the first token that is not the closing tag gets its text copied into a new
/// block (`record + 8`); the closing tag ends it and, if no text came, warns.
struct Text {
    key: u32,
    off: u32,     // the token's text pointer: `0x02065364 + off`
    len_add: u32, // the block's size: the lexer's result plus this
    num: u32,     // return address of the number read after the lexer (0: none)
    ret_lex: u32,
    ret_end: u32,   // the warning at the closing tag
    ret_first: u32, // the warning at the first token
    ret_alloc: u32,
    ret_copy: u32,
}

const TEXTS: &[Text] = &[
    Text { key: 0x0204_02F5, off: 0x20, len_add: 0, num: 0, ret_lex: 0x0204_030D, ret_end: 0x0204_0341, ret_first: 0x0204_0361, ret_alloc: 0x0204_0367, ret_copy: 0x0204_0383 },
    Text {
        key: 0x0204_102D,
        off: 0x68,
        len_add: 1,
        num: 0x0204_104D,
        ret_lex: 0x0204_1045,
        ret_end: 0x0204_107F,
        ret_first: 0x0204_109F,
        ret_alloc: 0x0204_10A5,
        ret_copy: 0x0204_10C1,
    },
];

unsafe fn run_text(c: *mut Cpu, t: &Text) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load {:#x}", t.key & !1);
    }
    let fr = enter(c);
    mem_write32(c, record(c) + REC_F0, 0);
    let mut got = 0u32;
    loop {
        call(c, 0x0203_F9D9, t.ret_lex, 0, 0);
        let len = (*c).r[0].wrapping_add(t.len_add);
        if t.num != 0 {
            call(c, 0x0203_F805, t.num, 1, 0);
        }
        let text = mem_read32(c, TOK_A + t.off);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if got == 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, t.ret_end, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        if got == 0 {
            call(c, 0x0202_20F5, t.ret_first, CLOSE, MSG);
            call(c, 0x0202_BFAD, t.ret_alloc, len, 0);
            let buf = (*c).r[0];
            mem_write32(c, record(c) + REC_F0, buf);
            (*c).r[2] = len & 0xFFFF;
            call(c, 0x0200_9624, t.ret_copy, MSG, mem_read32(c, record(c) + REC_F0));
            got = 1;
        }
        spend(c, 1); // the jump back to the lexer
    }
    leave(c, fr, Some(got));
}

macro_rules! tslots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run_text(c, &TEXTS[$i]) })*
        const TSLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
tslots!(t0 = 0, t1 = 1);

/// A function of the multi-line text shape: the tokens' texts are gathered in a buffer on the stack (a token whose text
/// is the first name's becomes a line break), the closing tag ends it: the buffer is copied into a new block
/// (`record + 8`). `wide`: 16-bit units, the count in units; otherwise bytes (CR LF).
struct Lines {
    key: u32,
    size: u32,
    wide: bool,
    ptr: u32, // where the token's text pointer is
    ret_set: u32,
    ret_lex: u32,
    ret_alloc: u32,
    ret_copy: u32,
    ret_add: u32,
}

const LINES: &[Lines] = &[
    Lines {
        key: 0x0203_FF29,
        size: 0x400,
        wide: false,
        ptr: 0x0206_5368,
        ret_set: 0x0203_FF3F,
        ret_lex: 0x0203_FF43,
        ret_alloc: 0x0203_FF6B,
        ret_copy: 0x0203_FF89,
        ret_add: 0x0203_FFE5,
    },
    Lines {
        key: 0x0203_FCBD,
        size: 0x800,
        wide: true,
        ptr: 0x0206_5358,
        ret_set: 0x0203_FCD7,
        ret_lex: 0x0203_FCDB,
        ret_alloc: 0x0203_FD05,
        ret_copy: 0x0203_FD23,
        ret_add: 0x0203_FD81,
    },
];

unsafe fn run_lines(c: *mut Cpu, l: &Lines) {
    use crate::services::{mem_write16, mem_write8};
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load {:#x}", l.key & !1);
    }
    let fr = enter(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(l.size);
    let buf = (*c).r[13];
    let unit = if l.wide { 2 } else { 1 };
    (*c).r[2] = l.size;
    call(c, 0x0200_9568, l.ret_set, buf, 0);
    let mut n = 0u32; // the game's own count
    loop {
        call(c, 0x0203_F9D9, l.ret_lex, 0, 0);
        let len = (*c).r[0];
        let close = units(c, CLOSE);
        let (res, polls) = compare(&close, &units(c, mem_read32(c, l.ptr)));
        spend(c, polls);
        if res == Some(true) {
            if l.wide {
                mem_write16(c, buf.wrapping_add(2 * n), 0);
            } else {
                mem_write8(c, buf.wrapping_add(n), 0);
            }
            n = n.wrapping_add(1).wrapping_mul(2);
            call(c, 0x0202_BFAD, l.ret_alloc, n, 0);
            let block = (*c).r[0];
            mem_write32(c, record(c) + REC_F0, block);
            (*c).r[2] = n & 0xFFFF;
            call(c, 0x0200_9624, l.ret_copy, buf, mem_read32(c, record(c) + REC_F0));
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        let (res, polls) = compare(&close, &units(c, mem_read32(c, NAMES)));
        spend(c, polls);
        if res == Some(true) {
            if l.wide {
                mem_write16(c, buf.wrapping_add(2 * n), 0x0A);
                n = n.wrapping_add(1);
            } else {
                mem_write8(c, buf.wrapping_add(n), 0x0D);
                mem_write8(c, buf.wrapping_add(n).wrapping_add(1), 0x0A);
                n = n.wrapping_add(2);
            }
        } else {
            (*c).r[2] = len.wrapping_mul(2) & 0xFFFF;
            call(c, 0x0200_9624, l.ret_add, CLOSE, buf.wrapping_add(unit * n));
            n = n.wrapping_add(len);
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = (*c).r[13].wrapping_add(l.size);
    leave(c, fr, None);
}

macro_rules! lslots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run_lines(c, &LINES[$i]) })*
        const LSLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
lslots!(l0 = 0, l1 = 1);

/// `0x02040210`: a pick. The first token that is not the closing tag is looked up in the names read by `0x0203FC54`
/// (`0x020747EC`, 8 bytes each, count at `STATE + ST_NAME_COUNT`); its index goes to `record + 8`.
unsafe extern "C" fn pick(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x02040210");
    }
    let fr = enter(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(8); // the two locals
    mem_write32(c, record(c) + REC_F0, 0);
    let (mut count, mut found) = (0u32, 0u32);
    loop {
        call(c, 0x0203_F9D9, 0x0204_022D, 0, 0);
        let text = mem_read32(c, TOK_A + 0x1C);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if found == 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, 0x0204_025F, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        if count == 0 {
            let n = mem_read32(c, STATE + ST_NAME_COUNT) as i32;
            let mut hit = false;
            for i in 0..n.max(0) as u32 {
                let s = mem_read32(c, NAME_RECS + 8 * i + 4);
                for j in 0..0x20u32 {
                    let (a, b) = (rd16(c, CLOSE + 2 * j), rd16(c, s + 2 * j));
                    if a != b {
                        spend(c, j);
                        break;
                    }
                    if a == 0 {
                        mem_write32(c, record(c) + REC_F0, i);
                        hit = true;
                    }
                    if j == 0x1F {
                        spend(c, j);
                    }
                }
                if i + 1 < n as u32 {
                    spend(c, 1); // the next name
                }
            }
            if hit {
                count = count.wrapping_add(1) & 0xFF;
                found = 1;
            }
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = (*c).r[13].wrapping_add(8);
    leave(c, fr, None);
}

/// push {r4, r5, r6, lr}.
unsafe fn enter3(c: *mut Cpu) -> (u32, [u32; 3], u32) {
    let (lr, saved) = ((*c).r[14], [(*c).r[4], (*c).r[5], (*c).r[6]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(16) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 12, lr);
    (*c).r[13] = sp.wrapping_sub(16);
    (lr, saved, sp)
}

/// One step of a block record's build: a warning, a block of memory, a word.
enum Op {
    Warn(u32, u32),      // r0, return address (r1 is the message)
    Alloc(u32, u32),     // size at `STATE + off`, return address: the block is kept
    Put(u32),            // `record + off` = the kept block
    Copy(u32, u32, u32), // the message copied into `[record + off]`, size at `STATE + off`, return address
    Word(u32, u32),      // `record + off` = `STATE + off`
}

/// A function that reads a token, the tail of the record (`0x0203FB68`) and fills a record's blocks.
struct Blocks {
    key: u32,
    clears: &'static [u32],
    num: u32, // return address of the number read after the lexer (0: none)
    ret_lex: u32,
    ret_fb: u32,
    ptr: u32, // where the token's text pointer is
    ret_end: u32,
    end_flag: u32,
    ops: &'static [Op],
}

use Op::*;
const BLOCKS: &[Blocks] = &[
    Blocks {
        key: 0x0204_03A9,
        clears: &[REC_F0, REC_F1],
        num: 0,
        ret_lex: 0x0204_03C7,
        ret_fb: 0x0204_03CB,
        ptr: TOK_A + 0x24,
        ret_end: 0x0204_03FB,
        end_flag: 1,
        ops: &[
            Warn(NAME_TEXT_A, 0x0204_041B),
            Alloc(ST_LEN_A, 0x0204_0423),
            Put(REC_F0),
            Copy(REC_F0, ST_LEN_A, 0x0204_0443),
            Warn(NAME_TEXT_B, 0x0204_044B),
            Alloc(ST_LEN_B, 0x0204_0453),
            Put(REC_F1),
            Copy(REC_F1, ST_LEN_B, 0x0204_0473),
        ],
    },
    Blocks {
        key: 0x0204_04A1,
        clears: &[REC_F0, REC_F1, REC_F2],
        num: 0x0204_04CD,
        ret_lex: 0x0204_04C7,
        ret_fb: 0x0204_04D1,
        ptr: TOK_A + 0x28,
        ret_end: 0x0204_0501,
        end_flag: 1,
        ops: &[Word(REC_F0, ST_NUM0), Warn(NAME_TEXT_B, 0x0204_052F), Alloc(ST_LEN_B, 0x0204_0537), Put(REC_F1), Copy(REC_F1, ST_LEN_B, 0x0204_0557), Word(REC_F2, ST_NUM1)],
    },
    Blocks {
        key: 0x0204_07B1,
        clears: &[REC_F0, REC_F1],
        num: 0x0204_07D5,
        ret_lex: 0x0204_07CF,
        ret_fb: 0x0204_07D9,
        ptr: TOK_A + 0x3C,
        ret_end: 0x0204_0809,
        end_flag: 0,
        ops: &[Warn(NAME_TEXT_A, 0x0204_0829), Alloc(ST_LEN_A, 0x0204_0831), Put(REC_F0), Copy(REC_F0, ST_LEN_A, 0x0204_0851), Word(REC_F1, ST_NUM0)],
    },
];

unsafe fn run_blocks(c: *mut Cpu, b: &Blocks) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load {:#x}", b.key & !1);
    }
    let fr = enter3(c);
    for o in b.clears {
        mem_write32(c, record(c) + o, 0);
    }
    let (mut flag, mut count) = (0u32, 0u32);
    loop {
        call(c, 0x0203_F9D9, b.ret_lex, 0, 0);
        if b.num != 0 {
            call(c, 0x0203_F805, b.num, 1, 0);
        }
        call(c, 0x0203_FB69, b.ret_fb, 0, 0);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, mem_read32(c, b.ptr)));
        spend(c, polls);
        if res == Some(true) {
            if flag == 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, b.ret_end, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        if count == 0 {
            let mut buf = 0;
            for op in b.ops {
                match *op {
                    Warn(r0, ret) => call(c, 0x0202_20F5, ret, r0, MSG),
                    Alloc(len, ret) => {
                        call(c, 0x0202_BFAD, ret, mem_read32(c, STATE + len), 0);
                        buf = (*c).r[0];
                    }
                    Put(off) => mem_write32(c, record(c) + off, buf),
                    Copy(off, len, ret) => {
                        (*c).r[2] = mem_read32(c, STATE + len) & 0xFFFF;
                        call(c, 0x0200_9624, ret, MSG, mem_read32(c, record(c) + off));
                    }
                    Word(off, from) => mem_write32(c, record(c) + off, mem_read32(c, STATE + from)),
                }
            }
            count = count.wrapping_add(1) & 0xFF;
            flag = b.end_flag;
        }
        spend(c, 1); // the jump back to the lexer
    }
    let (lr, saved, sp) = fr;
    (*c).r[13] = sp;
    [(*c).r[4], (*c).r[5], (*c).r[6]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

macro_rules! bslots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run_blocks(c, &BLOCKS[$i]) })*
        const BSLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
bslots!(b0 = 0, b1 = 1, b2 = 2);

/// `0x02040BF4`: an array of blocks. Each token that is not the closing tag makes a block (`record + 8 + 4 * k`) from the
/// text `0x020744EC`; the names (`0x020747EC`) that equal the text `0x0207456C` get `k` at `record + 4 * k + 0x18`.
unsafe extern "C" fn fields(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x02040bf4");
    }
    let fr = enter(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(8); // the two locals
    let (mut off, mut k, mut flag) = (0u32, 0u32, 0u32);
    loop {
        call(c, 0x0203_F9D9, 0x0204_0C03, 0, 0);
        let text = mem_read32(c, TOK_A + 0x54);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if flag == 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, 0x0204_0C33, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        call(c, 0x0203_FB69, 0x0204_0C4D, 0, 0);
        let len = mem_read32(c, STATE + ST_LEN_A);
        call(c, 0x0202_BFAD, 0x0204_0C57, len.wrapping_mul(2), 0);
        let buf = (*c).r[0];
        mem_write32(c, record(c) + off + REC_BLOCKS, buf);
        (*c).r[2] = len & 0xFFFF;
        call(c, 0x0200_9624, 0x0204_0C7F, NAME_TEXT_A, mem_read32(c, record(c) + off + REC_BLOCKS));
        let n = mem_read32(c, STATE + ST_NAME_COUNT) as i32;
        for i in 0..n.max(0) as u32 {
            let s = mem_read32(c, NAME_RECS + 8 * i + 4);
            for j in 0..0x20u32 {
                let (a, b) = (rd16(c, NAME_TEXT_B + 2 * j), rd16(c, s + 2 * j));
                if a != b {
                    spend(c, j);
                    break;
                }
                if a == 0 {
                    mem_write32(c, record(c) + 4 * k + REC_BLOCK_NAMES, i);
                }
                if j == 0x1F {
                    spend(c, j);
                }
            }
            if i + 1 < n as u32 {
                spend(c, 1); // the next name
            }
        }
        off = off.wrapping_add(4);
        k = k.wrapping_add(1);
        flag = 1;
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = (*c).r[13].wrapping_add(8);
    leave(c, fr, None);
}

/// push {r4, r5, r6, r7, lr}.
unsafe fn enter4(c: *mut Cpu) -> (u32, [u32; 4], u32) {
    let (lr, saved) = ((*c).r[14], [(*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(20) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 16, lr);
    (*c).r[13] = sp.wrapping_sub(20);
    (lr, saved, sp)
}

/// `0x0204253C`: a numbered message. Looks for the tag whose number is the wanted one (`STATE + ST_NUM0`), gathers the
/// text that follows in a 0x800-byte buffer (a token that is the first name's is a line break) up to the next tag
/// and copies it into a new block (`record + 8`). States: 0 looking, 1 gathering, 2 done, 3 not found.
unsafe extern "C" fn message(c: *mut Cpu) {
    use crate::services::mem_write16;
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x0204253c");
    }
    let fr = enter4(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(0x80C);
    let buf = (*c).r[13].wrapping_add(0xC);
    (*c).r[2] = 0x800;
    call(c, 0x0200_9568, 0x0204_2559, buf, 0);
    #[allow(unused_assignments)] // `wanted` is read in state 0 only, after it was set
    let (mut n, mut state, mut wanted) = (0u32, 0u32, 0u32);
    'lex: loop {
        call(c, 0x0203_F9D9, 0x0204_255D, 0, 0);
        call(c, 0x0203_F805, 0x0204_2563, 1, 0);
        let close = units(c, CLOSE);
        let (res, polls) = compare(&close, &units(c, mem_read32(c, TOK_B + 0x48)));
        spend(c, polls);
        if res == Some(true) {
            if state == 2 {
                mem_write16(c, buf.wrapping_add(2 * n), 0);
                n = n.wrapping_add(1).wrapping_mul(2);
                call(c, 0x0202_BFAD, 0x0204_258F, n, 0);
                let block = (*c).r[0];
                mem_write32(c, record(c) + REC_F0, block);
                (*c).r[2] = n & 0xFFFF;
                call(c, 0x0200_9624, 0x0204_25AD, buf, mem_read32(c, record(c) + REC_F0));
            } else {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, 0x0204_25C7, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        if state == 0 {
            wanted = mem_read32(c, STATE + ST_NUM0);
            mem_write32(c, STATE + ST_RESET_B, 0);
            mem_write32(c, STATE + ST_RESET_A, 0);
            let w = wanted as i32;
            if mem_read32(c, STATE + ST_ENABLED) == 0 || !(0..100_000).contains(&w) {
                spend(c, 1);
                continue 'lex;
            }
        } else {
            spend(c, 1);
            continue 'lex;
        }
        // the inner loop: the next token (`0x0203FAE4`)
        loop {
            call(c, 0x0203_FAE5, 0x0204_260F, 0, 0);
            let last = (*c).r[0];
            if state == 0 {
                if rd16(c, TAG) != 0x5B || rd16(c, TAG + 2) != 0x5B {
                    spend(c, 1);
                    continue;
                }
                let mut v = 0u32;
                for i in 0..5u32 {
                    let d = (rd16(c, TAG + 4 + 2 * i) as u32).wrapping_sub(0x30) as i32;
                    if (0..=9).contains(&d) {
                        v = v.wrapping_mul(10).wrapping_add(d as u32);
                    }
                }
                spend(c, 4); // the digit loop
                if v == wanted {
                    state = 1;
                }
                if (v as i32) <= (wanted as i32) {
                    spend(c, 1);
                    continue;
                }
                state = 3;
                spend(c, 1);
                continue 'lex;
            }
            if state != 1 {
                spend(c, 1);
                continue;
            }
            let (res, polls) = compare(&units(c, TAG), &units(c, mem_read32(c, NAMES)));
            spend(c, polls);
            let mut brk = false;
            if res == Some(true) {
                mem_write16(c, buf.wrapping_add(2 * n), 0x0A);
                n = n.wrapping_add(1);
                brk = true;
            }
            if rd16(c, TAG) == 0x5B && rd16(c, TAG + 2) == 0x5B {
                state = 2;
                spend(c, 1);
                continue 'lex;
            }
            if !brk {
                (*c).r[2] = last.wrapping_mul(2) & 0xFFFF;
                call(c, 0x0200_9624, 0x0204_26B9, TAG, buf.wrapping_add(2 * n));
                n = n.wrapping_add(last);
            }
            spend(c, 1);
        }
    }
    (*c).r[13] = (*c).r[13].wrapping_add(0x80C);
    let (lr, saved, sp) = fr;
    (*c).r[13] = sp;
    [(*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// `0x02040E6C`: two numbers, then a list. The first two tokens' numbers go to `record + 8` and `+ 0xC`; each of the
/// next ones (as many as `record + 0xC` says) adds a number at `record + 0x10 + 4 * k` and, looked up among the names
/// (`0x020747EC`) by the text `0x0207456C`, the index at `record + 0x20 + 4 * k` (0 if none; then a warning at the end).
unsafe extern "C" fn list(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x02040e6c");
    }
    let fr = enter4(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(0x14); // the five locals
    for o in [REC_F0, REC_F1, REC_LIST_NUMS, REC_LIST_NAMES] {
        mem_write32(c, record(c) + o, 0);
    }
    spend(c, 3); // the clearing loop of four rounds
    let (mut phase, mut j, mut off, mut done, mut miss) = (0u32, 0u32, 0u32, 0u32, 0u32);
    loop {
        call(c, 0x0203_F9D9, 0x0204_0EB5, 0, 0);
        call(c, 0x0203_F805, 0x0204_0EBB, 1, 0);
        call(c, 0x0203_FB69, 0x0204_0EBF, 0, 0);
        let text = mem_read32(c, TOK_A + 0x64);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if done == 0 || miss != 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, 0x0204_0EF7, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        match phase {
            0 => {
                mem_write32(c, record(c) + REC_F0, mem_read32(c, STATE + ST_NUM0));
                phase = (phase + 1) & 0xFF;
            }
            1 => {
                mem_write32(c, record(c) + REC_F1, mem_read32(c, STATE + ST_NUM0));
                phase = (phase + 1) & 0xFF;
            }
            2 => {
                let rec = record(c);
                if (j as i32) < (mem_read32(c, rec + REC_F1) as i32) {
                    mem_write32(c, rec + off + REC_LIST_NUMS, mem_read32(c, STATE + ST_NUM0));
                    let n = mem_read32(c, STATE + ST_NAME_COUNT) as i32;
                    let mut hit = false;
                    for i in 0..n.max(0) as u32 {
                        let s = mem_read32(c, NAME_RECS + 8 * i + 4);
                        for k in 0..0x20u32 {
                            let (a, b) = (rd16(c, NAME_TEXT_B + 2 * k), rd16(c, s + 2 * k));
                            if a != b {
                                spend(c, k);
                                break;
                            }
                            if a == 0 {
                                mem_write32(c, record(c) + (j << 2) + 0x20, i);
                                hit = true;
                            }
                            if k == 0x1F {
                                spend(c, k);
                            }
                        }
                        if i + 1 < n as u32 {
                            spend(c, 1); // the next name
                        }
                    }
                    if !hit {
                        mem_write32(c, record(c) + off + REC_LIST_NAMES, 0);
                        miss = 1;
                    }
                    off = off.wrapping_add(4);
                    j = j.wrapping_add(1);
                    if j == mem_read32(c, record(c) + REC_F1) {
                        phase = (phase + 1) & 0xFF;
                        done = 1;
                    }
                }
            }
            _ => {}
        }
        spend(c, 1); // the jump back to the lexer
    }
    let (lr, saved, sp) = fr;
    (*c).r[13] = sp;
    [(*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

/// `0x02043844`: four names. Each of the first four tokens that is not the closing tag is looked up among the names
/// (`0x020747EC`) by the closing tag's text; the index goes to `record + 8 + 4 * k` (the one that is found; the word
/// stays 0 if none). A closing tag before the fourth warns.
unsafe extern "C" fn names4(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native load 0x02043844");
    }
    let fr = enter(c);
    (*c).r[13] = (*c).r[13].wrapping_sub(8); // the two locals
    for o in [REC_F0, REC_F1, REC_F2, 0x14] {
        mem_write32(c, record(c) + o, 0);
    }
    let (mut k, mut flag) = (0u32, 0u32);
    loop {
        call(c, 0x0203_F9D9, 0x0204_387F, 0, 0);
        let text = mem_read32(c, TOK_C + 0x38);
        let (res, polls) = compare(&units(c, CLOSE), &units(c, text));
        spend(c, polls);
        if res == Some(true) {
            if flag == 0 {
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, record(c)));
                call(c, 0x0202_20F5, 0x0204_38B1, name, MSG);
            }
            let i = mem_read32(c, STATE + ST_RECORD);
            mem_write32(c, STATE + ST_RECORD, i.wrapping_add(1));
            break;
        }
        if k <= 3 {
            let n = mem_read32(c, STATE + ST_NAME_COUNT) as i32;
            for i in 0..n.max(0) as u32 {
                let s = mem_read32(c, NAME_RECS + 8 * i + 4);
                for j in 0..0x20u32 {
                    let (a, b) = (rd16(c, CLOSE + 2 * j), rd16(c, s + 2 * j));
                    if a != b {
                        spend(c, j);
                        break;
                    }
                    if a == 0 {
                        mem_write32(c, record(c) + (k << 2) + REC_F0, i);
                    }
                    if j == 0x1F {
                        spend(c, j);
                    }
                }
                if i + 1 < n as u32 {
                    spend(c, 1); // the next name
                }
            }
            if k == 3 {
                flag = 1;
            }
            k = (k + 1) & 0xFF;
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = (*c).r[13].wrapping_add(8);
    leave(c, fr, None);
}

pub fn install() {
    type F = unsafe extern "C" fn(*mut Cpu);
    let mut all: Vec<(u32, F)> = Vec::new();
    all.extend(rows().iter().enumerate().map(|(i, r)| (r.key, SLOTS[i])));
    all.extend(arows().iter().enumerate().map(|(i, r)| (r.key, ASLOTS[i])));
    all.extend([
        (0x0203_FC55, names as F),
        (0x0203_F775, lookup as F),
        (0x0204_0D01, bump as F),
        (0x0204_0211, pick as F),
        (0x0204_3845, names4 as F),
        (0x0204_0E6D, list as F),
        (0x0204_253D, message as F),
        (0x0204_0BF5, fields as F),
    ]);
    all.extend(BLOCKS.iter().enumerate().map(|(i, b)| (b.key, BSLOTS[i])));
    all.extend(LINES.iter().enumerate().map(|(i, l)| (l.key, LSLOTS[i])));
    all.extend(TEXTS.iter().enumerate().map(|(i, t)| (t.key, TSLOTS[i])));
    // for hunting a difference: SUNPETAL_NATIVE_LOAD_SKIP=0x0204253C,… leaves these functions to the game
    let skip: Vec<u32> = std::env::var("SUNPETAL_NATIVE_LOAD_SKIP").map(|v| v.split(',').filter_map(|k| num(k).ok()).map(|k| k | 1).collect()).unwrap_or_default();
    crate::machine(|m| {
        for (key, f) in all {
            if !skip.contains(&key) && !skip.contains(&1) {
                m.override_fn(9, key, f);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skip_table_reads() {
        let r = parse(include_str!("../../../docs/native-load-hooks.csv")).unwrap();
        assert!(r.len() == 24 && r.len() <= SLOTS.len() && r.iter().all(|r| r.key & 1 == 1 && r.close == 0x0207_40EC));
    }

    #[test]
    fn the_args_table_reads() {
        let r = parse_args(include_str!("../../../docs/native-load-args.csv")).unwrap();
        assert!(r.len() == 68 && r.len() <= ASLOTS.len() && r.iter().all(|r| !r.pairs.is_empty() && r.key & 1 == 1));
    }

    #[test]
    fn no_two_overrides_share_a_key() {
        let mut keys: Vec<u32> = parse(include_str!("../../../docs/native-load-hooks.csv")).unwrap().iter().map(|r| r.key).collect();
        keys.extend(parse_args(include_str!("../../../docs/native-load-args.csv")).unwrap().iter().map(|r| r.key));
        keys.extend(TEXTS.iter().map(|t| t.key));
        keys.extend(LINES.iter().map(|l| l.key));
        keys.extend(BLOCKS.iter().map(|b| b.key));
        keys.extend([0x0203_FC55, 0x0203_F775, 0x0204_0D01, 0x0204_0211, 0x0204_0BF5, 0x0204_253D, 0x0204_0E6D, 0x0204_3845]);
        assert!(keys.iter().all(|k| k & 1 == 1));
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n);
        assert!(TEXTS.len() <= TSLOTS.len() && LINES.len() <= LSLOTS.len() && BLOCKS.len() <= BSLOTS.len());
    }

    #[test]
    fn the_compare_stops_at_the_first_difference_or_the_end() {
        assert_eq!(compare(&[1, 2, 0], &[1, 3, 0]), (Some(false), 1));
        assert_eq!(compare(&[1, 2, 0, 9], &[1, 2, 0, 7]), (Some(true), 2));
        assert_eq!(compare(&[1; 128], &[1; 128]), (None, 127));
    }
}
