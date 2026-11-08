//! The native replacement of the dialogue's thin script handlers (K-178, `docs/NATIVE-PILOT.md`, variant A): behind
//! `SUNPETAL_NATIVE_DIALOG=1` the host runs what `TEXT_EX`, `SHOWWINDOW`, `HIDEWINDOW`, `NEXTPAGE` do to the guest's
//! words, in place of the recompiled handlers. What to write where is data (`docs/native-dialog-hooks.csv`, numbers in
//! `docs/native-dialog-params.csv`); this file is the mechanism. The script step, the drawing and the wait for A stay
//! on the ARM.
use crate::abi::Cpu;
use crate::services::{mem_read32, mem_write32, mem_write8};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    W32(u32, u32),
    W8(u32, u32),
    /// `[dst] <- [[ptr] + off]`
    Cp32(u32, u32, u32),
}

#[derive(Clone, Debug)]
pub struct Hook {
    pub tag: String,
    /// Entry key of the handler (bit 0: Thumb).
    pub entry: u32,
    ops: Vec<Op>,
}

fn num(s: &str, mark_start: i32) -> Result<u32, String> {
    let s = s.trim();
    if s == "@mark_start" {
        return Ok(mark_start as u32);
    }
    let (a, b) = s.split_once('+').unwrap_or((s, "0"));
    let one = |t: &str| -> Result<u32, String> {
        let t = t.trim();
        match t.strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16),
            None => t.parse(),
        }
        .map_err(|_| format!("native-dialog-hooks: `{}` is not a number", t))
    };
    Ok(one(a)?.wrapping_add(one(b)?))
}

/// Reads the table (`tag,entry,ops,source`, the head first).
pub fn parse(csv: &str, p: &crate::dialogmodel::Params) -> Result<Vec<Hook>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.splitn(4, ',').collect();
        if f.len() < 3 {
            return Err(format!("native-dialog-hooks: bad line `{}`", l));
        }
        let mut ops = Vec::new();
        for o in f[2].split(';').filter(|o| !o.trim().is_empty()) {
            let a: Vec<&str> = o.split(':').collect();
            let n = |i: usize| num(a.get(i).copied().unwrap_or(""), p.mark_start);
            ops.push(match a[0] {
                "w32" if a.len() == 3 => Op::W32(n(1)?, n(2)?),
                "w8" if a.len() == 3 => Op::W8(n(1)?, n(2)?),
                "cp32" if a.len() == 4 => Op::Cp32(n(1)?, n(2)?, n(3)?),
                _ => return Err(format!("native-dialog-hooks: bad op `{}`", o)),
            });
        }
        out.push(Hook { tag: f[0].into(), entry: num(f[1], 0)?, ops });
    }
    Ok(out)
}

static HOOKS: OnceLock<Vec<Hook>> = OnceLock::new();

unsafe fn run(c: *mut Cpu, i: usize) {
    for op in &HOOKS.get().expect("hooks")[i].ops {
        match *op {
            Op::W32(a, v) => mem_write32(c, a, v),
            Op::W8(a, v) => mem_write8(c, a, v),
            Op::Cp32(dst, ptr, off) => {
                let p = mem_read32(c, ptr);
                let v = mem_read32(c, p.wrapping_add(off));
                mem_write32(c, dst, v);
            }
        }
    }
    (*c).pc = (*c).r[14];
}

macro_rules! slots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run(c, $i) })*
        const SLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
slots!(s0 = 0, s1 = 1, s2 = 2, s3 = 3, s4 = 4, s5 = 5, s6 = 6, s7 = 7);

/// Whether the environment asks for the native handlers.
pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_DIALOG").is_ok_and(|v| v == "1")
}

/// Puts the handlers of the repository's table in place of the guest's (ARM9).
pub fn install() -> Result<usize, String> {
    let hooks = parse(include_str!("../../../docs/native-dialog-hooks.csv"), &crate::dialogmodel::Params::builtin())?;
    if hooks.len() > SLOTS.len() {
        return Err("native-dialog-hooks: more rows than slots".into());
    }
    let n = hooks.len();
    for (i, h) in hooks.iter().enumerate() {
        crate::machine(|m| m.override_fn(9, h.entry, SLOTS[i]));
    }
    let _ = HOOKS.set(hooks);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads() {
        let h = parse(include_str!("../../../docs/native-dialog-hooks.csv"), &crate::dialogmodel::Params::builtin()).unwrap();
        assert_eq!(h.len(), 4);
        assert_eq!(h[0].entry, 0x0204_9235);
        assert!(h[0].ops.contains(&Op::W32(0x0206_577C, 20)));
        assert!(h[0].ops.contains(&Op::Cp32(0x0207_59EC + 0x3C, 0x0207_596C + 0x1C, 8)));
        assert_eq!(h[1].ops, [Op::W32(0x0207_8598, 1)]);
        assert!(h[3].ops.is_empty());
    }

    #[test]
    fn a_bad_op_is_named() {
        let e = parse("tag,entry,ops,source\nx,0x1,zap:1:2,\n", &crate::dialogmodel::Params::builtin()).unwrap_err();
        assert!(e.contains("zap"), "{}", e);
    }
}
