//! The native command-table loader (K-198, step E2c, `docs/NATIVE-ENGINE-PLAN.md`): the game reads the scene's text once
//! and builds a record for each tag (`0x02074BEC`, an array of pointers to records). The pilot is `SETFACE2`
//! (`0x0204245C`): it reads up to three numbers into the record's `+8`, `+0xC`, `+0x10` and ends at the closing tag.
//! Behind `SUNPETAL_NATIVE_LOAD=1`; the lexer (`0x0203F9D8`, `0x0203F804`) stays the game's. The safe points are the
//! game's: one before each call and one for each jump back.
use crate::abi::Cpu;
use crate::nativechar::spend;
use crate::services::{mem_read32, mem_write32};

const STATE: u32 = 0x0207_4084; // +0x1C the index of the record being built, +0x50 the number just read
const TABLE: u32 = 0x0207_4BEC;
const TEXT: u32 = 0x0206_53E4; // +0x44 the token's text, UTF-16
const CLOSE: u32 = 0x0207_40EC; // the closing tag's text to match, 128 units
const NAMES: u32 = 0x0206_5568;
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
    mem_read32(c, TABLE + 4 * mem_read32(c, STATE + 0x1C))
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

unsafe extern "C" fn set_face2(c: *mut Cpu) {
    if std::env::var_os("SUNPETAL_NATIVE_LOAD_LOG").is_some() {
        eprintln!("native SETFACE2");
    }
    let (lr, saved) = ((*c).r[14], [(*c).r[4], (*c).r[5], (*c).r[6]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(16) & !3;
    for (i, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * i as u32, *v);
    }
    mem_write32(c, a + 12, lr);
    (*c).r[13] = sp.wrapping_sub(16);

    let (mut got, mut n) = (false, 0u8);
    for off in [8, 0xC, 0x10] {
        mem_write32(c, record(c) + off, 0);
    }
    loop {
        call(c, 0x0203_F9D9, 0x0204_2483, 0, 0);
        call(c, 0x0203_F805, 0x0204_2489, 1, 0);
        let (close, text) = (CLOSE, mem_read32(c, TEXT + 0x44));
        let units = |base: u32| (0..128).map(|i| rd16(c, base + 2 * i)).collect::<Vec<u16>>();
        let (res, polls) = compare(&units(close), &units(text));
        spend(c, polls);
        if res == Some(true) {
            if !got {
                let cmd = record(c);
                let name = mem_read32(c, NAMES + 4 * mem_read32(c, cmd));
                call(c, 0x0202_20F5, 0x0204_24B9, name, MSG);
            }
            mem_write32(c, STATE + 0x1C, mem_read32(c, STATE + 0x1C).wrapping_add(1));
            break;
        }
        let v = mem_read32(c, STATE + 0x50);
        match n {
            0 => mem_write32(c, record(c) + 8, v),
            1 | 2 => {
                got = true;
                mem_write32(c, record(c) + if n == 1 { 0xC } else { 0x10 }, v);
            }
            _ => {}
        }
        if n < 3 {
            n += 1;
        }
        spend(c, 1); // the jump back
    }
    (*c).r[13] = sp;
    [(*c).r[4], (*c).r[5], (*c).r[6]] = saved;
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
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
    clears: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    ret_lex: u32,
    ret_val: u32,
    ret_warn: u32,
}

fn parse_args(csv: &str) -> Result<Vec<Args>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.split(',').collect();
        if f.len() != 10 || f[1] != "args" {
            return Err(format!("bad row: {l}"));
        }
        let list = |s: &str| s.split_whitespace().map(num).collect::<Result<Vec<_>, _>>();
        let pairs = f[6].split_whitespace().map(|p| p.split_once(':').ok_or(format!("bad pair {p}")).and_then(|(a, b)| Ok((num(a)?, num(b)?)))).collect::<Result<Vec<_>, _>>()?;
        out.push(Args {
            key: num(f[0])?,
            close: num(f[2])?,
            base: num(f[3])?,
            off: num(f[4])?,
            clears: list(f[5])?,
            pairs,
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
    let (lr, saved) = ((*c).r[14], [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]]);
    let sp = (*c).r[13];
    let a = sp.wrapping_sub(24) & !3;
    for (k, v) in saved.iter().enumerate() {
        mem_write32(c, a + 4 * k as u32, *v);
    }
    mem_write32(c, a + 20, lr);
    (*c).r[13] = sp.wrapping_sub(24);

    let (mut got, mut first) = (false, true);
    for off in &r.clears {
        mem_write32(c, record(c) + off, 0);
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
            mem_write32(c, STATE + 0x1C, mem_read32(c, STATE + 0x1C).wrapping_add(1));
            break;
        }
        if first {
            for (from, to) in &r.pairs {
                let v = mem_read32(c, STATE + from);
                mem_write32(c, record(c) + to, v);
            }
            got = true;
            first = false;
        }
        spend(c, 1); // the jump back to the lexer
    }
    (*c).r[13] = sp;
    [(*c).r[3], (*c).r[4], (*c).r[5], (*c).r[6], (*c).r[7]] = saved;
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
    a31 = 31
);

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_LOAD").is_ok_and(|v| v == "1")
}

pub fn install() {
    crate::machine(|m| {
        m.override_fn(9, 0x0204_245D, set_face2);
        for (i, r) in rows().iter().enumerate() {
            m.override_fn(9, r.key, SLOTS[i]);
        }
        for (i, r) in arows().iter().enumerate() {
            m.override_fn(9, r.key, ASLOTS[i]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skip_table_reads() {
        let r = parse(include_str!("../../../docs/native-load-hooks.csv")).unwrap();
        assert!(r.len() == 23 && r.len() <= SLOTS.len() && r.iter().all(|r| r.key & 1 == 1 && r.close == 0x0207_40EC));
    }

    #[test]
    fn the_args_table_reads() {
        let r = parse_args(include_str!("../../../docs/native-load-args.csv")).unwrap();
        assert!(r.len() == 27 && r.len() <= ASLOTS.len() && r.iter().all(|r| !r.pairs.is_empty() && r.key & 1 == 1));
    }

    #[test]
    fn the_compare_stops_at_the_first_difference_or_the_end() {
        assert_eq!(compare(&[1, 2, 0], &[1, 3, 0]), (Some(false), 1));
        assert_eq!(compare(&[1, 2, 0, 9], &[1, 2, 0, 7]), (Some(true), 2));
        assert_eq!(compare(&[1; 128], &[1; 128]), (None, 127));
    }
}
