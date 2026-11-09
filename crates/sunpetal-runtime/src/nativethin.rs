//! The thin script handlers as data (K-193, step E3, `docs/NATIVE-ENGINE-PLAN.md`): `docs/native-thin-hooks.csv` is
//! made by `tools/script/thin_hooks.py` from the recompiled handlers (straight-line code only: loads, stores, add, sub,
//! mul, shift); here is the small interpreter of its `st8/st16/st32(ADDR, VALUE)` rows. A value is a number,
//! `ld8/ld16/ld8s/ld32(E)`, `add/sub/mul/shl(E,E)` or `not(E)`. Behind `SUNPETAL_NATIVE_THIN=1`. These handlers have no
//! calls and no steps back, so the game's safe points are not spent by them and nothing is left to count.
use crate::abi::Cpu;
use crate::services::{mem_read16, mem_read32, mem_read8, mem_write16, mem_write32, mem_write8};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Num(u32),
    Ld(u32, Box<Expr>), // size in bytes; 0 = signed byte
    Bin(char, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// The answer (r0) of the last call.
    Ret,
    /// Register `n` (0..=3) as the handler was entered: what the caller left there.
    In(usize),
    /// Register `n` as it is now: what the function called just before left in it.
    Keep(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Store {
        size: u32,
        addr: Expr,
        val: Expr,
    },
    /// `call(RETURN, TARGET, r0, r1, r2, r3)`: the game's function at `TARGET` (key, bit 0 = Thumb), a safe point spent.
    Call {
        ret: u32,
        target: Expr,
        args: [Expr; 4],
    },
    /// `tail(TARGET, r0..r3)`: the function returns where the handler would have.
    Tail {
        target: Expr,
        args: [Expr; 4],
    },
    /// `frame(N)`: the handler has `N` bytes pushed; the functions it calls see `sp` that much lower.
    Frame(u32),
    If {
        cond: String,
        a: Expr,
        b: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
}

#[derive(Clone, Debug)]
pub struct Hook {
    pub tag: String,
    pub entry: u32,
    pub body: Vec<Stmt>,
}

struct P<'a>(&'a [u8], usize);

impl P<'_> {
    fn name(&mut self) -> String {
        let s = self.1;
        while self.1 < self.0.len() && self.0[self.1].is_ascii_alphanumeric() {
            self.1 += 1;
        }
        String::from_utf8_lossy(&self.0[s..self.1]).into_owned()
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.0.get(self.1) == Some(&c) {
            self.1 += 1;
            Ok(())
        } else {
            Err(format!("`{}` expected at {}", c as char, self.1))
        }
    }
    fn peek(&self) -> Option<u8> {
        self.0.get(self.1).copied()
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let n = self.name();
        if n == "ret" {
            return Ok(Expr::Ret);
        }
        if let Some(k) = n.strip_prefix("keep").and_then(|k| k.parse::<usize>().ok()).filter(|k| *k < 4) {
            return Ok(Expr::Keep(k));
        }
        if let Some(k) = n.strip_prefix("in").and_then(|k| k.parse::<usize>().ok()).filter(|k| *k < 4) {
            return Ok(Expr::In(k));
        }
        if self.peek() != Some(b'(') {
            let v = match n.strip_prefix("0x") {
                Some(h) => u32::from_str_radix(h, 16),
                None => n.parse(),
            };
            return v.map(Expr::Num).map_err(|_| format!("bad number `{}`", n));
        }
        self.eat(b'(')?;
        let a = self.expr()?;
        let e = match n.as_str() {
            "ld8" => Expr::Ld(1, a.into()),
            "ld16" => Expr::Ld(2, a.into()),
            "ld8s" => Expr::Ld(0, a.into()),
            "ld32" => Expr::Ld(4, a.into()),
            "not" => Expr::Not(a.into()),
            "add" | "sub" | "mul" | "shl" | "shr" => {
                self.eat(b',')?;
                let b = self.expr()?;
                let op = match n.as_str() {
                    "add" => '+',
                    "sub" => '-',
                    "mul" => '*',
                    "shl" => '<',
                    _ => '>',
                };
                Expr::Bin(op, a.into(), b.into())
            }
            _ => return Err(format!("unknown `{}`", n)),
        };
        self.eat(b')')?;
        Ok(e)
    }
    fn exprs(&mut self, n: usize) -> Result<Vec<Expr>, String> {
        let mut v = Vec::new();
        for i in 0..n {
            if i > 0 {
                self.eat(b',')?;
            }
            v.push(self.expr()?);
        }
        Ok(v)
    }
    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(b'{')?;
        let v = self.stmts()?;
        self.eat(b'}')?;
        Ok(v)
    }
    fn stmts(&mut self) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'}' {
                break;
            }
            if c == b';' {
                self.1 += 1;
                continue;
            }
            let n = self.name();
            self.eat(b'(')?;
            let st = match n.as_str() {
                "st8" | "st16" | "st32" => {
                    let size = match n.as_str() {
                        "st8" => 1,
                        "st16" => 2,
                        _ => 4,
                    };
                    let mut e = self.exprs(2)?;
                    let val = e.pop().unwrap_or(Expr::Num(0));
                    Stmt::Store { size, addr: e.pop().unwrap_or(Expr::Num(0)), val }
                }
                "call" | "tail" => {
                    let ret = if n == "call" {
                        match self.expr()? {
                            Expr::Num(r) => {
                                self.eat(b',')?;
                                r
                            }
                            _ => return Err("call: the return address is a number".into()),
                        }
                    } else {
                        0
                    };
                    let mut e = self.exprs(5)?;
                    let a3 = e.pop().unwrap_or(Expr::Num(0));
                    let a2 = e.pop().unwrap_or(Expr::Num(0));
                    let a1 = e.pop().unwrap_or(Expr::Num(0));
                    let a0 = e.pop().unwrap_or(Expr::Num(0));
                    let target = e.pop().unwrap_or(Expr::Num(0));
                    let args = [a0, a1, a2, a3];
                    if n == "call" {
                        Stmt::Call { ret, target, args }
                    } else {
                        Stmt::Tail { target, args }
                    }
                }
                "frame" => {
                    let n = match self.expr()? {
                        Expr::Num(n) => n,
                        _ => return Err("frame: a number".into()),
                    };
                    out.push(Stmt::Frame(n));
                    self.eat(b')')?;
                    continue;
                }
                "if" => {
                    let cond = self.name();
                    self.eat(b'(')?;
                    let mut e = self.exprs(2)?;
                    let b = e.pop().unwrap_or(Expr::Num(0));
                    let a = e.pop().unwrap_or(Expr::Num(0));
                    self.eat(b')')?;
                    self.eat(b')')?;
                    let then = self.block()?;
                    let els = if self.name() == "else" { self.block()? } else { Vec::new() };
                    out.push(Stmt::If { cond, a, b, then, els });
                    continue;
                }
                _ => return Err(format!("native-thin-hooks: bad op `{}`", n)),
            };
            self.eat(b')')?;
            out.push(st);
        }
        Ok(out)
    }
}

pub fn parse(csv: &str) -> Result<Vec<Hook>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        // the ops hold commas inside brackets: tag and entry first, the source note last
        let (tag, rest) = l.split_once(',').ok_or("native-thin-hooks: no tag")?;
        let (entry, rest) = rest.split_once(',').ok_or("native-thin-hooks: no ops")?;
        if !entry.starts_with("0x") {
            return Err(format!("native-thin-hooks: bad line `{}`", l));
        }
        let ops = rest.rsplit_once(",handler").map_or(rest, |r| r.0);
        let mut p = P(ops.as_bytes(), 0);
        let body = p.stmts().map_err(|e| format!("{}: {}", tag, e))?;
        if p.1 != ops.len() {
            return Err(format!("native-thin-hooks: `{}` is not all read: bad op near {}", tag, p.1));
        }
        out.push(Hook { tag: tag.into(), entry: u32::from_str_radix(entry.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?, body });
    }
    Ok(out)
}

unsafe fn eval(c: *mut Cpu, e: &Expr) -> u32 {
    match e {
        Expr::Keep(k) => (*c).r[*k],
        Expr::In(k) => ENTRY.with(|r| r.get()[*k]),
        Expr::Num(n) => *n,
        Expr::Ret => (*c).r[0],
        Expr::Ld(size, a) => {
            let a = eval(c, a);
            match size {
                1 => mem_read8(c, a),
                2 => mem_read16(c, a),
                0 => mem_read8(c, a) as u8 as i8 as i32 as u32,
                _ => mem_read32(c, a),
            }
        }
        Expr::Not(a) => !eval(c, a),
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(c, a), eval(c, b));
            match op {
                '+' => x.wrapping_add(y),
                '-' => x.wrapping_sub(y),
                '*' => x.wrapping_mul(y),
                '<' => x.wrapping_shl(y),
                _ => x.wrapping_shr(y),
            }
        }
    }
}

/// Whether the branch condition of a compare of `a` with `b` holds.
pub fn holds(cond: &str, a: u32, b: u32) -> bool {
    let (sa, sb) = (a as i32, b as i32);
    match cond {
        "eq" => a == b,
        "ne" => a != b,
        "lt" => sa < sb,
        "le" => sa <= sb,
        "gt" => sa > sb,
        "ge" => sa >= sb,
        "lo" => a < b,
        "ls" => a <= b,
        "hi" => a > b,
        _ => a >= b, // hs
    }
}

/// Runs the statements; true if a tail call has taken the return.
unsafe fn exec(c: *mut Cpu, body: &[Stmt], lr: u32, depth: &mut u32) -> bool {
    for st in body {
        match st {
            Stmt::Frame(n) => *depth = *n,
            Stmt::Store { size, addr, val } => {
                let (a, v) = (eval(c, addr), eval(c, val));
                match size {
                    1 => mem_write8(c, a, v),
                    2 => mem_write16(c, a, v),
                    _ => mem_write32(c, a, v),
                }
            }
            Stmt::Call { ret, target, args } => {
                let (t, a) = (eval(c, target), args.each_ref().map(|e| eval(c, e)));
                (*c).r[0] = a[0];
                (*c).r[1] = a[1];
                (*c).r[2] = a[2];
                (*c).r[3] = a[3];
                (*c).r[14] = *ret;
                crate::nativechar::spend(c, 1);
                let sp = (*c).r[13];
                (*c).r[13] = sp.wrapping_sub(*depth);
                crate::call(&mut *c, t);
                (*c).r[13] = sp;
            }
            Stmt::Tail { target, args } => {
                let (t, a) = (eval(c, target), args.each_ref().map(|e| eval(c, e)));
                (*c).r[0] = a[0];
                (*c).r[1] = a[1];
                (*c).r[2] = a[2];
                (*c).r[3] = a[3];
                (*c).r[14] = lr;
                let sp = (*c).r[13];
                (*c).r[13] = sp.wrapping_sub(*depth);
                crate::call(&mut *c, t);
                (*c).r[13] = sp;
                return true;
            }
            Stmt::If { cond, a, b, then, els } => {
                let take = holds(cond, eval(c, a), eval(c, b));
                if exec(c, if take { then } else { els }, lr, depth) {
                    return true;
                }
            }
        }
    }
    false
}

static HOOKS: OnceLock<Vec<Hook>> = OnceLock::new();

thread_local! {
    /// r0..r3 at the entry of the running handler.
    static ENTRY: std::cell::Cell<[u32; 4]> = const { std::cell::Cell::new([0; 4]) };
}

/// `SUNPETAL_NATIVE_THIN_LOG=1`: the tag of each handler on its first call, to see what a run has covered.
fn log_first(i: usize, tag: &str) {
    static SEEN: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    if std::env::var_os("SUNPETAL_NATIVE_THIN_LOG").is_some() {
        let mut m = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        if *m >> i & 1 == 0 {
            *m |= 1 << i;
            eprintln!("thin: {}", tag);
        }
    }
}

unsafe fn run(c: *mut Cpu, i: usize) {
    let lr = (*c).r[14];
    let hook = &HOOKS.get().expect("hooks")[i];
    log_first(i, &hook.tag);
    let outer = ENTRY.with(|r| r.replace([(*c).r[0], (*c).r[1], (*c).r[2], (*c).r[3]]));
    let tail = exec(c, &hook.body, lr, &mut 0);
    ENTRY.with(|r| r.set(outer));
    (*c).r[14] = lr;
    if !tail {
        (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
    }
}

macro_rules! slots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run(c, $i) })*
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
    s23 = 23,
    s24 = 24,
    s25 = 25,
    s26 = 26,
    s27 = 27,
    s28 = 28,
    s29 = 29,
    s30 = 30,
    s31 = 31,
    s32 = 32,
    s33 = 33,
    s34 = 34,
    s35 = 35,
    s36 = 36,
    s37 = 37,
    s38 = 38,
    s39 = 39,
    s40 = 40,
    s41 = 41,
    s42 = 42,
    s43 = 43,
    s44 = 44,
    s45 = 45,
    s46 = 46,
    s47 = 47,
    s48 = 48,
    s49 = 49,
    s50 = 50,
    s51 = 51,
    s52 = 52,
    s53 = 53,
    s54 = 54,
    s55 = 55,
    s56 = 56,
    s57 = 57,
    s58 = 58,
    s59 = 59,
    s60 = 60,
    s61 = 61,
    s62 = 62,
    s63 = 63
);

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_THIN").is_ok_and(|v| v == "1")
}

/// The table read from `docs/native-thin-hooks.csv`, once.
fn table() -> Result<&'static [Hook], String> {
    if HOOKS.get().is_none() {
        let hooks = parse(include_str!("../../../docs/native-thin-hooks.csv"))?;
        if hooks.len() > SLOTS.len() {
            return Err("native-thin-hooks: more rows than slots".into());
        }
        let _ = HOOKS.set(hooks);
    }
    Ok(HOOKS.get().map_or(&[], |h| h.as_slice()))
}

/// `(tag, entry key)` of each row, for the checks against the game's own code (`Session::thin_diff`).
pub fn rows() -> Result<Vec<(String, u32)>, String> {
    Ok(table()?.iter().map(|h| (h.tag.clone(), h.entry)).collect())
}

/// Runs row `i` as the installed hook would, without installing it.
///
/// # Safety
/// `c` is the guest's CPU at a point where it may run its functions.
pub unsafe fn run_row(c: *mut Cpu, i: usize) -> Result<(), String> {
    table()?;
    run(c, i);
    Ok(())
}

pub fn install() -> Result<usize, String> {
    let hooks = table()?;
    for (i, h) in hooks.iter().enumerate() {
        crate::machine(|m| m.override_fn(9, h.entry, SLOTS[i]));
    }
    Ok(hooks.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_every_row_does_something() {
        let h = parse(include_str!("../../../docs/native-thin-hooks.csv")).unwrap();
        assert!(h.len() > 9 && h.iter().all(|h| !h.body.is_empty()), "{} rows", h.len());
        let stop = h.iter().find(|h| h.tag == "<STOP_SHAKE>").unwrap();
        assert_eq!(stop.entry, 0x0204_9631);
        assert_eq!(stop.body.len(), 1);
        assert!(matches!(&stop.body[0], Stmt::Store { size: 4, val: Expr::Num(0), .. }));
    }

    #[test]
    fn calls_branches_and_tails_read() {
        let h = parse("t,e,x\nA,0x1,call(0x5,0x20245B1,7,0,0,0);if(eq(ret,0)){}else{st32(add(ret,24),1)};tail(0x9,0,0,0,0),handler\n").unwrap();
        let b = &h[0].body;
        assert_eq!(b.len(), 3);
        assert!(matches!(&b[0], Stmt::Call { ret: 5, .. }));
        assert!(matches!(&b[1], Stmt::If { cond, then, els, .. } if cond == "eq" && then.is_empty() && els.len() == 1));
        assert!(matches!(&b[2], Stmt::Tail { .. }));
    }

    #[test]
    fn frames_and_kept_registers_read() {
        let h = parse("t,e,x\nA,0x1,frame(8);call(0x5,0x20511A9,0x207596C,1,in2,keep3);frame(0),handler\n").unwrap();
        let b = &h[0].body;
        assert!(matches!(&b[0], Stmt::Frame(8)) && matches!(&b[2], Stmt::Frame(0)));
        match &b[1] {
            Stmt::Call { args, .. } => assert!(args[2] == Expr::In(2) && args[3] == Expr::Keep(3)),
            s => panic!("{:?}", s),
        }
    }

    #[test]
    fn a_condition_is_signed_or_unsigned_as_named() {
        assert!(holds("lt", u32::MAX, 0) && !holds("lo", u32::MAX, 0));
        assert!(holds("ge", 3, 3) && holds("hs", 3, 3) && holds("le", 3, 3) && holds("ls", 3, 3));
        assert!(holds("gt", 4, 3) && holds("hi", 4, 3) && holds("ne", 4, 3) && !holds("eq", 4, 3));
    }

    #[test]
    fn a_bad_op_is_named() {
        let e = parse("tag,entry,ops,source\nx,0x1,zap(1,2),\n").unwrap_err();
        assert!(e.contains("zap"), "{}", e);
    }
}
