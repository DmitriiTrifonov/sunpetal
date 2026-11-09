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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    pub size: u32,
    pub addr: Expr,
    pub val: Expr,
}

#[derive(Clone, Debug)]
pub struct Hook {
    pub tag: String,
    pub entry: u32,
    pub stores: Vec<Store>,
}

struct P<'a>(&'a [u8], usize);

impl P<'_> {
    fn name(&mut self) -> String {
        let s = self.1;
        while self.1 < self.0.len() && (self.0[self.1].is_ascii_alphanumeric()) {
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
    fn expr(&mut self) -> Result<Expr, String> {
        let n = self.name();
        if self.0.get(self.1) != Some(&b'(') {
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
            "add" | "sub" | "mul" | "shl" => {
                self.eat(b',')?;
                let b = self.expr()?;
                Expr::Bin(n.chars().next().map(|c| if n == "shl" { '<' } else { c }).unwrap_or('+'), a.into(), b.into())
            }
            _ => return Err(format!("unknown `{}`", n)),
        };
        self.eat(b')')?;
        Ok(match e {
            Expr::Bin('a', a, b) => Expr::Bin('+', a, b),
            Expr::Bin('s', a, b) => Expr::Bin('-', a, b),
            Expr::Bin('m', a, b) => Expr::Bin('*', a, b),
            e => e,
        })
    }
}

/// Splits `st32(A,B);st8(C,D)` at the `;` outside brackets.
fn split_ops(s: &str) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0, 0, Vec::new());
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter().filter(|o| !o.trim().is_empty()).collect()
}

pub fn parse(csv: &str) -> Result<Vec<Hook>, String> {
    let mut out = Vec::new();
    for l in csv.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = l.splitn(4, ',').collect();
        if f.len() < 3 || !f[1].starts_with("0x") {
            return Err(format!("native-thin-hooks: bad line `{}`", l));
        }
        // the ops hold commas inside brackets: re-join the first fields by the entry's end
        let (tag, rest) = l.split_once(',').ok_or("no tag")?;
        let (entry, rest) = rest.split_once(',').ok_or("no ops")?;
        let ops = rest.rsplit_once(",handler").map_or(rest, |r| r.0);
        let mut stores = Vec::new();
        for o in split_ops(ops) {
            let (kind, args) = o.split_once('(').ok_or_else(|| format!("bad op `{}`", o))?;
            let size = match kind {
                "st8" => 1,
                "st16" => 2,
                "st32" => 4,
                _ => return Err(format!("native-thin-hooks: bad op `{}`", o)),
            };
            let mut p = P(args.as_bytes(), 0);
            let addr = p.expr()?;
            p.eat(b',')?;
            let val = p.expr()?;
            stores.push(Store { size, addr, val });
        }
        out.push(Hook { tag: tag.into(), entry: u32::from_str_radix(entry.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?, stores });
    }
    Ok(out)
}

unsafe fn eval(c: *mut Cpu, e: &Expr) -> u32 {
    match e {
        Expr::Num(n) => *n,
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
                _ => x.wrapping_shl(y),
            }
        }
    }
}

static HOOKS: OnceLock<Vec<Hook>> = OnceLock::new();

unsafe fn run(c: *mut Cpu, i: usize) {
    for s in &HOOKS.get().expect("hooks")[i].stores {
        let (a, v) = (eval(c, &s.addr), eval(c, &s.val));
        match s.size {
            1 => mem_write8(c, a, v),
            2 => mem_write16(c, a, v),
            _ => mem_write32(c, a, v),
        }
    }
    let lr = (*c).r[14];
    (*c).pc = if lr & 1 != 0 { lr } else { lr & !3 };
}

macro_rules! slots {
    ($($n:ident = $i:expr),*) => {
        $(unsafe extern "C" fn $n(c: *mut Cpu) { run(c, $i) })*
        const SLOTS: &[unsafe extern "C" fn(*mut Cpu)] = &[$($n),*];
    };
}
slots!(s0 = 0, s1 = 1, s2 = 2, s3 = 3, s4 = 4, s5 = 5, s6 = 6, s7 = 7, s8 = 8, s9 = 9, s10 = 10, s11 = 11, s12 = 12, s13 = 13, s14 = 14, s15 = 15);

pub fn wanted() -> bool {
    std::env::var("SUNPETAL_NATIVE_THIN").is_ok_and(|v| v == "1")
}

pub fn install() -> Result<usize, String> {
    let hooks = parse(include_str!("../../../docs/native-thin-hooks.csv"))?;
    if hooks.len() > SLOTS.len() {
        return Err("native-thin-hooks: more rows than slots".into());
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
    fn the_table_reads_and_a_row_has_its_stores() {
        let h = parse(include_str!("../../../docs/native-thin-hooks.csv")).unwrap();
        assert!(h.iter().all(|h| !h.stores.is_empty()));
        let stop = h.iter().find(|h| h.tag == "<STOP_SHAKE>").unwrap();
        assert_eq!(stop.entry, 0x0204_9631);
        assert_eq!(stop.stores.len(), 1);
        assert_eq!(stop.stores[0].size, 4);
        assert_eq!(stop.stores[0].val, Expr::Num(0));
    }

    #[test]
    fn an_expression_with_nesting_parses() {
        let mut p = P(b"sub(ld32(add(ld32(0x10),32)),not(1))", 0);
        let e = p.expr().unwrap();
        assert!(matches!(e, Expr::Bin('-', _, _)));
    }

    #[test]
    fn a_bad_op_is_named() {
        let e = parse("tag,entry,ops,source\nx,0x1,zap(1,2),\n").unwrap_err();
        assert!(e.contains("zap"), "{}", e);
    }
}
