//! The interpreter: ARM and Thumb code run from memory as it is when called, for code no
//! module holds (decrypted, or copied at run time) and functions the emitter could not take.
//!
//! Semantics are `tools/ndskit/armemu.py`'s, instruction for instruction, as the recompiled
//! code's are; decoding is `tools/ndskit/arm.py`'s. The self-test replays every vector through
//! it too (`selftest::run_vectors` with `interp`).
//!
//! `interpret(c, key)` runs from `key` until control comes back to lr as it was at the call.
//! On the way:
//!  * a call (bl, blx, or a transfer that set lr) to an entry some module vouches for runs the
//!    recompiled function; its return goes on where it went, interpreted;
//!  * a jump to an entry is a tail call: the function runs, then its return goes on as above
//!    (to lr: done);
//!  * a jump through a register into a program's own code that is no entry, from code outside
//!    it, is a return elsewhere: `c.pc` says where, and the caller's check (ARM_RET) decides;
//!  * safe points at calls and backward jumps, as in recompiled code.
use crate::abi::{modules, Cpu, SEG_PROGRAM};
use crate::machine::machine;
use crate::services::*;

fn sext(v: u32, bits: u32) -> u32 {
    (((v << (32 - bits)) as i32) >> (32 - bits)) as u32
}

fn ror(v: u32, n: u32) -> u32 {
    v.rotate_right(n & 31)
}

fn iw(x: u32) -> u32 {
    if x & 1 != 0 { x } else { x & !3 }
}

/// The barrel shifter: the result and the carry out into `cy`. `n` as encoded (lsr/asr #32 as
/// 32, rrx as kind 4), or Rs's bottom byte when `by_reg`. Kind -1: no shift.
fn shift_c(kind: i32, v: u32, n: u32, cy: &mut u32, by_reg: bool) -> u32 {
    if kind < 0 {
        return v;
    }
    if kind == 4 {
        let o = v & 1;
        let r = *cy << 31 | v >> 1;
        *cy = o;
        return r;
    }
    if by_reg && n == 0 {
        return v;
    }
    match kind {
        0 => {
            if n < 32 {
                *cy = v.wrapping_shr(32 - n) & 1;
                v << n
            } else {
                *cy = if n == 32 { v & 1 } else { 0 };
                0
            }
        }
        1 => {
            if n < 32 {
                *cy = v >> (n - 1) & 1;
                v >> n
            } else {
                *cy = if n == 32 { v >> 31 } else { 0 };
                0
            }
        }
        2 => {
            if n < 32 {
                *cy = v >> (n - 1) & 1;
                ((v as i32) >> n) as u32
            } else {
                *cy = v >> 31;
                ((v as i32) >> 31) as u32
            }
        }
        _ => {
            if n & 31 == 0 {
                *cy = v >> 31;
                return v;
            }
            let r = ror(v, n);
            *cy = r >> 31;
            r
        }
    }
}

/// An immediate shift's (kind, amount) from bits 11-5: kind -1 none, 4 rrx.
fn imm_shift(w: u32) -> (i32, u32) {
    let mut n = w >> 7 & 31;
    let t = (w >> 5 & 3) as i32;
    if t == 0 {
        return (if n != 0 { 0 } else { -1 }, n);
    }
    if t == 3 && n == 0 {
        return (4, n);
    }
    if n == 0 {
        n = 32;
    }
    (t, n)
}

fn cond_ok(c: &Cpu, cc: u32) -> bool {
    match cc {
        0 => c.z != 0,
        1 => c.z == 0,
        2 => c.c != 0,
        3 => c.c == 0,
        4 => c.n != 0,
        5 => c.n == 0,
        6 => c.v != 0,
        7 => c.v == 0,
        8 => c.c != 0 && c.z == 0,
        9 => c.c == 0 || c.z != 0,
        10 => c.n == c.v,
        11 => c.n != c.v,
        12 => c.z == 0 && c.n == c.v,
        13 => c.z != 0 || c.n != c.v,
        _ => true,
    }
}

fn half(v: u32, top: bool) -> i32 {
    (if top { v >> 16 } else { v }) as u16 as i16 as i32
}

#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Next,
    Jump,
    Call,
    Eret,
}

struct Interp<'a> {
    c: &'a mut Cpu,
    cpu: i32,
    arch: u32,
    /// lr at the call: back there, it is done.
    exit: u32,
    /// The code began inside a program's own segment.
    from_program: bool,
    interp_only: bool,
    /// The instruction being executed.
    pc: u32,
    /// Thumb.
    t: bool,
    flow: Flow,
    /// Jump/Call: the address, bit 0 the state.
    to: u32,
    /// Jump: through a register (not an immediate branch).
    reg: bool,
}

impl<'a> Interp<'a> {
    fn cp(&mut self) -> *mut Cpu {
        self.c as *mut Cpu
    }

    fn ld8(&mut self, a: u32) -> u32 {
        unsafe { mem_read8(self.cp(), a) }
    }
    fn ld16(&mut self, a: u32) -> u32 {
        unsafe { mem_read16(self.cp(), a) }
    }
    fn ld32(&mut self, a: u32) -> u32 {
        unsafe { mem_read32(self.cp(), a) }
    }
    fn st8(&mut self, a: u32, v: u32) {
        unsafe { mem_write8(self.cp(), a, v) }
    }
    fn st16(&mut self, a: u32, v: u32) {
        unsafe { mem_write16(self.cp(), a, v) }
    }
    fn st32(&mut self, a: u32, v: u32) {
        unsafe { mem_write32(self.cp(), a, v) }
    }
    fn ldr32(&mut self, a: u32) -> u32 {
        ror(self.ld32(a & !3), (a & 3) * 8)
    }
    fn ldrh(&mut self, a: u32) -> u32 {
        if self.cpu == 9 || a & 1 == 0 {
            self.ld16(a & !1)
        } else {
            ror(self.ld16(a & !1), 8)
        }
    }
    fn ldrsh(&mut self, a: u32) -> u32 {
        if self.cpu == 9 || a & 1 == 0 {
            self.ld16(a & !1) as u16 as i16 as u32
        } else {
            self.ld8(a) as u8 as i8 as u32
        }
    }

    fn poll(&mut self) {
        self.c.budget -= 1;
        if self.c.budget < 0 {
            unsafe { arm_poll(self.cp()) }
        }
    }

    fn key(&self) -> u32 {
        if self.t { self.pc | 1 } else { self.pc }
    }

    /// Register n as an operand: pc reads +8 (ARM; +12 for a register-specified shift and a
    /// stored pc) or +4 (Thumb).
    fn rv(&self, n: usize, extra: u32) -> u32 {
        if n == 15 {
            self.pc.wrapping_add(if self.t { 4 } else { 8 + extra })
        } else {
            self.c.r[n]
        }
    }

    /// pc = target; with interwork bit 0 picks the state (armemu's branch).
    fn branch(&mut self, target: u32, interwork: bool) {
        let nt = if interwork { target & 1 != 0 } else { self.t };
        self.to = if nt { (target & !1) | 1 } else { target & !3 };
        self.flow = Flow::Jump;
        self.reg = true;
    }
    fn write_pc(&mut self, v: u32) {
        self.branch(v, self.arch >= 5);
    }
    fn link_to(&mut self, key: u32) {
        self.to = key;
        self.flow = Flow::Call;
    }
    fn undefined(&mut self) {
        let k = self.key();
        unsafe { arm_undefined(self.cp(), k) }
    }
    fn eret(&mut self, target: u32) {
        unsafe { arm_exception_return(self.cp(), target) }
        self.flow = Flow::Eret;
    }

    // ---- data processing ----------------------------------------------------------
    /// op: and eor sub rsb add adc sbc rsc tst teq cmp cmn orr mov bic mvn (0-15)
    fn dp(&mut self, op: u32, s: bool, rd: usize, a: u32, b: u32, carry: u32) {
        let (mut write, mut full, mut x, mut y) = (true, 0u64, 0u32, 0u32);
        let (r, logic);
        match op {
            0 => { r = a & b; logic = true; }
            1 => { r = a ^ b; logic = true; }
            8 => { r = a & b; logic = true; write = false; }
            9 => { r = a ^ b; logic = true; write = false; }
            12 => { r = a | b; logic = true; }
            13 => { r = b; logic = true; }
            14 => { r = a & !b; logic = true; }
            15 => { r = !b; logic = true; }
            _ => {
                logic = false;
                let ci;
                match op {
                    2 | 10 => { x = a; y = !b; ci = 1; }
                    3 => { x = b; y = !a; ci = 1; }
                    4 | 11 => { x = a; y = b; ci = 0; }
                    5 => { x = a; y = b; ci = self.c.c; }
                    6 => { x = a; y = !b; ci = self.c.c; }
                    _ => { x = b; y = !a; ci = self.c.c; }
                }
                full = x as u64 + y as u64 + ci as u64;
                r = full as u32;
                write = op != 10 && op != 11;
            }
        }
        if logic {
            if !write || (s && rd != 15) {
                self.c.n = r >> 31;
                self.c.z = (r == 0) as u32;
                self.c.c = carry;
            }
        } else if !write || (s && rd != 15) {
            self.c.n = r >> 31;
            self.c.z = (r == 0) as u32;
            self.c.c = (full >> 32) as u32;
            self.c.v = ((x ^ r) & (y ^ r)) >> 31;
        }
        if !write {
            return;
        }
        if rd == 15 {
            if s {
                self.eret(r);
                return;
            }
            self.branch(r, false);
            return;
        }
        self.c.r[rd] = r;
    }

    // ---- loads and stores -----------------------------------------------------------
    /// op: 0 ldr, 1 ldrb, 2 ldrh, 3 ldrsh, 4 ldrsb, 5 ldrd; 8 str, 9 strb, 10 strh, 11 strd
    #[allow(clippy::too_many_arguments)]
    fn ldst(&mut self, op: u32, rd: usize, rn: usize, base: u32, off: u32, pre: bool, add: bool, wb: bool) {
        let moved = if add { base.wrapping_add(off) } else { base.wrapping_sub(off) };
        let a = if pre { moved } else { base };
        let w = wb && rn != 15;
        if op < 8 {
            let v = match op {
                0 => self.ldr32(a),
                1 => self.ld8(a),
                2 => self.ldrh(a),
                3 => self.ldrsh(a),
                4 => self.ld8(a) as u8 as i8 as u32,
                _ => {
                    let lo = self.ld32(a & !3);
                    let hi = self.ld32((a & !3) + 4);
                    if w {
                        self.c.r[rn] = moved;
                    }
                    self.c.r[rd] = lo;
                    self.c.r[rd + 1] = hi;
                    return;
                }
            };
            if w {
                self.c.r[rn] = moved; // the loaded value wins over the base
            }
            if rd == 15 {
                self.write_pc(v);
            } else {
                self.c.r[rd] = v;
            }
            return;
        }
        let v = self.rv(rd, 4); // str pc: + 12
        match op {
            8 => self.st32(a & !3, v),
            9 => self.st8(a, v),
            10 => self.st16(a & !1, v),
            _ => {
                let v0 = self.c.r[rd];
                let v1 = self.rv(rd + 1, 4);
                self.st32(a & !3, v0);
                self.st32((a & !3) + 4, v1);
            }
        }
        if w {
            self.c.r[rn] = moved;
        }
    }

    /// mode: 0 da, 1 ia, 2 db, 3 ib
    fn block(&mut self, load: bool, rn: usize, rl: u32, mode: u32, wb: bool, user: bool, thumb: bool) {
        let n = rl.count_ones();
        let base = self.c.r[rn];
        let (a, nb) = match mode {
            1 => (base, base.wrapping_add(4 * n)),
            3 => (base.wrapping_add(4), base.wrapping_add(4 * n)),
            0 => (base.wrapping_sub(4 * n).wrapping_add(4), base.wrapping_sub(4 * n)),
            _ => (base.wrapping_sub(4 * n), base.wrapping_sub(4 * n)),
        };
        let a = a & !3;
        if load {
            let mut vals = [0u32; 16];
            let mut k = 0;
            for i in 0..16 {
                if rl >> i & 1 != 0 {
                    vals[i] = self.ld32(a.wrapping_add(4 * k));
                    k += 1;
                }
            }
            let u = user && rl & 0x8000 == 0;
            for i in 0..15 {
                if rl >> i & 1 != 0 {
                    if u {
                        unsafe { arm_set_user_reg(self.cp(), i as i32, vals[i]) }
                    } else {
                        self.c.r[i] = vals[i];
                    }
                }
            }
            if wb {
                if rl >> rn & 1 == 0 {
                    self.c.r[rn] = nb;
                } else if self.arch >= 5 && !thumb && (rl == 1 << rn || (rl >> (rn + 1)) != 0) {
                    self.c.r[rn] = nb;
                }
            }
            if rl & 0x8000 != 0 {
                if user {
                    self.eret(vals[15]);
                } else {
                    self.write_pc(vals[15]);
                }
            }
            return;
        }
        let lowest = rl.trailing_zeros() as usize;
        let mut k = 0;
        for i in 0..16usize {
            if rl >> i & 1 == 0 {
                continue;
            }
            let v = if user {
                unsafe { arm_user_reg(self.cp(), i as i32) }
            } else if i == 15 {
                self.rv(15, 4)
            } else if i == rn && wb && self.arch < 5 && i != lowest {
                nb
            } else {
                self.c.r[i]
            };
            self.st32(a.wrapping_add(4 * k), v);
            k += 1;
        }
        if wb {
            self.c.r[rn] = nb;
        }
    }

    // ---- ARM ---------------------------------------------------------------------------
    fn arm(&mut self, w: u32) {
        let cond = w >> 28;
        if cond == 15 {
            self.arm_nv(w);
            return;
        }
        if cond != 14 && !cond_ok(self.c, cond) {
            return;
        }
        let k = w >> 25 & 7;
        let rn = (w >> 16 & 15) as usize;
        let rd = (w >> 12 & 15) as usize;
        let rs = (w >> 8 & 15) as usize;
        let rm = (w & 15) as usize;
        let mut k = k;
        if k == 0 {
            if w & 0x90 == 0x90 {
                self.mul_extra(w);
                return;
            }
            if w & 0x0190_0000 == 0x0100_0000 {
                self.misc(w);
                return;
            }
            k = 0; // falls through to data processing
        }
        match k {
            0 | 1 => {
                if k == 1 && w & 0x0190_0000 == 0x0100_0000 {
                    if w & 0x0020_0000 == 0 || w & 0xF000 != 0xF000 {
                        self.undefined();
                        return;
                    }
                    let mask = w >> 16 & 15;
                    if mask == 0 && w & 0x0040_0000 == 0 {
                        return; // nop
                    }
                    let v = ror(w & 0xFF, (w >> 8 & 15) * 2);
                    self.msr(w & 0x0040_0000 != 0, mask, v);
                    return;
                }
                let op = w >> 21 & 15;
                let s = w & 0x0010_0000 != 0;
                let mut carry = self.c.c;
                let (a, b);
                if k == 1 {
                    let rot = (w >> 8 & 15) * 2;
                    b = ror(w & 0xFF, rot);
                    if rot != 0 {
                        carry = b >> 31;
                    }
                    a = self.rv(rn, 0);
                } else if w & 0x10 != 0 {
                    // shift by a register
                    let amount = self.c.r[rs] & 0xFF;
                    let v = self.rv(rm, 4);
                    b = shift_c((w >> 5 & 3) as i32, v, amount, &mut carry, true);
                    a = self.rv(rn, 4);
                } else {
                    let (kind, n) = imm_shift(w);
                    let v = self.rv(rm, 0);
                    b = shift_c(kind, v, n, &mut carry, false);
                    a = self.rv(rn, 0);
                }
                self.dp(op, s || (8..=11).contains(&op), rd, a, b, carry);
            }
            2 | 3 => {
                if k == 3 && w & 0x10 != 0 {
                    self.undefined();
                    return;
                }
                let p = w >> 24 & 1 != 0;
                let u = w >> 23 & 1 != 0;
                let b = w >> 22 & 1 != 0;
                let wb = w >> 21 & 1 != 0;
                let ld = w >> 20 & 1 != 0;
                let off = if k == 3 {
                    let (kind, n) = imm_shift(w);
                    let mut cy = self.c.c;
                    let v = self.rv(rm, 0);
                    shift_c(kind, v, n, &mut cy, false)
                } else {
                    w & 0xFFF
                };
                let op = if ld { if b { 1 } else { 0 } } else if b { 9 } else { 8 };
                let base = self.rv(rn, 0);
                self.ldst(op, rd, rn, base, off, p, u, wb || !p);
            }
            4 => {
                let rl = w & 0xFFFF;
                if rl == 0 {
                    self.undefined();
                    return;
                }
                let p = w >> 24 & 1;
                let u = w >> 23 & 1;
                self.block(w >> 20 & 1 != 0, rn, rl, p * 2 + u, w >> 21 & 1 != 0, w >> 22 & 1 != 0, false);
            }
            5 => {
                let target = self.pc.wrapping_add(8).wrapping_add(sext(w & 0xFF_FFFF, 24) << 2);
                if w & 0x0100_0000 != 0 {
                    self.c.r[14] = self.pc + 4;
                    self.link_to(target & !3);
                } else {
                    self.to = target & !3;
                    self.flow = Flow::Jump;
                    self.reg = false;
                }
            }
            6 => self.undefined(), // ldc/stc/mcrr
            _ => {
                if w & 0x0100_0000 != 0 {
                    let ret = self.pc + 4;
                    unsafe { arm_swi(self.cp(), (w >> 16) & 0xFF, ret) }
                    return;
                }
                if w & 0x10 == 0 {
                    self.undefined(); // cdp
                    return;
                }
                self.cop(w);
            }
        }
    }

    fn arm_nv(&mut self, w: u32) {
        if self.arch < 5 {
            self.undefined();
            return;
        }
        if w & 0x0E00_0000 == 0x0A00_0000 {
            // blx imm
            let off = (sext(w & 0xFF_FFFF, 24) << 2) | (w >> 23 & 2);
            self.c.r[14] = self.pc + 4;
            self.link_to(self.pc.wrapping_add(8).wrapping_add(off) | 1);
            return;
        }
        if w & 0x0D70_F000 == 0x0550_F000 {
            // pld
            if w & 0x0200_0000 != 0 && w & 0x10 != 0 {
                self.undefined();
            }
            return;
        }
        self.undefined(); // mcr2/mrc2, cdp2, ldc2...
    }

    fn cop(&mut self, w: u32) {
        if w >> 8 & 15 != 15 {
            self.undefined();
            return;
        }
        let op1 = (w >> 21 & 7) as i32;
        let rd = (w >> 12 & 15) as usize;
        let crn = (w >> 16 & 15) as i32;
        let crm = (w & 15) as i32;
        let op2 = (w >> 5 & 7) as i32;
        if w & 0x0010_0000 == 0 {
            let v = self.rv(rd, 4);
            unsafe { arm_cp15_write(self.cp(), op1, crn, crm, op2, v) }
            return;
        }
        let v = unsafe { arm_cp15_read(self.cp(), op1, crn, crm, op2) };
        if rd == 15 {
            self.c.n = v >> 31;
            self.c.z = v >> 30 & 1;
            self.c.c = v >> 29 & 1;
            self.c.v = v >> 28 & 1;
        } else {
            self.c.r[rd] = v;
        }
    }

    fn msr(&mut self, spsr: bool, mask: u32, v: u32) {
        if spsr {
            unsafe { arm_set_spsr(self.cp(), v, mask) }
            return;
        }
        set_cpsr(self.c, v, mask);
        if mask & 1 != 0 {
            self.poll();
        }
    }

    fn mul_extra(&mut self, w: u32) {
        let sh = w >> 5 & 3;
        let r19 = (w >> 16 & 15) as usize;
        let r15 = (w >> 12 & 15) as usize;
        let rs = (w >> 8 & 15) as usize;
        let rm = (w & 15) as usize;
        if sh == 0 {
            if w & 0x0F00_0000 == 0 {
                let op = w >> 21 & 7;
                let s = w & 0x0010_0000 != 0;
                if op == 0 || op == 1 {
                    let mut r = self.c.r[rm].wrapping_mul(self.c.r[rs]);
                    if op == 1 {
                        r = r.wrapping_add(self.c.r[r15]);
                    }
                    self.c.r[r19] = r;
                    if s {
                        self.c.n = r >> 31;
                        self.c.z = (r == 0) as u32;
                    }
                    return;
                }
                if op < 4 {
                    self.undefined();
                    return;
                }
                let mut p = if op & 2 != 0 {
                    ((self.c.r[rm] as i32 as i64).wrapping_mul(self.c.r[rs] as i32 as i64)) as u64
                } else {
                    (self.c.r[rm] as u64).wrapping_mul(self.c.r[rs] as u64)
                };
                if op & 1 != 0 {
                    p = p.wrapping_add((self.c.r[r19] as u64) << 32 | self.c.r[r15] as u64);
                }
                self.c.r[r15] = p as u32;
                self.c.r[r19] = (p >> 32) as u32;
                if s {
                    self.c.n = (p >> 63) as u32;
                    self.c.z = (p == 0) as u32;
                }
                return;
            }
            if w & 0x0FB0_0000 == 0x0100_0000 {
                // swp, swpb
                let a = self.c.r[r19];
                let m = self.c.r[rm];
                let v;
                if w & 0x0040_0000 != 0 {
                    v = self.ld8(a);
                    self.st8(a, m);
                } else {
                    v = self.ldr32(a);
                    self.st32(a & !3, m);
                }
                self.c.r[r15] = v;
                return;
            }
            self.undefined();
            return;
        }
        let p = w >> 24 & 1 != 0;
        let u = w >> 23 & 1 != 0;
        let i = w >> 22 & 1 != 0;
        let wb = w >> 21 & 1 != 0;
        let ld = w >> 20 & 1 != 0;
        if !p && wb {
            self.undefined();
            return;
        }
        let op;
        if ld {
            op = if sh == 1 { 2 } else if sh == 2 { 4 } else { 3 };
        } else {
            if sh >= 2 && (self.arch < 5 || r15 == 15) {
                self.undefined();
                return;
            }
            op = if sh == 1 { 10 } else if sh == 2 { 5 } else { 11 };
        }
        let mut off = if i { ((w >> 4) & 0xF0) | (w & 15) } else { self.c.r[rm] };
        if !i && rm == 15 {
            off = self.rv(15, 0);
        }
        let base = self.rv(r19, 0);
        self.ldst(op, r15, r19, base, off, p, u, wb || !p);
    }

    fn misc(&mut self, w: u32) {
        let op = w >> 21 & 3;
        let b74 = w >> 4 & 15;
        let rd = (w >> 12 & 15) as usize;
        let rn = (w >> 16 & 15) as usize;
        let rs = (w >> 8 & 15) as usize;
        let rm = (w & 15) as usize;
        if b74 == 0 {
            if op & 1 == 0 {
                if w & 0x000F_0FFF != 0x000F_0000 {
                    self.undefined();
                    return;
                }
                self.c.r[rd] = if op & 2 != 0 { unsafe { arm_get_spsr(self.c) } } else { self.c.cpsr() };
                return;
            }
            if w & 0xFFF0 != 0xF000 {
                self.undefined();
                return;
            }
            let v = self.c.r[rm];
            self.msr(op & 2 != 0, rn as u32, v);
            return;
        }
        let sbo = w & 0x000F_FF00 == 0x000F_FF00;
        if b74 == 1 && op == 1 && sbo {
            let v = self.rv(rm, 0);
            self.branch(v, true); // bx
            return;
        }
        if self.arch < 5 {
            self.undefined();
            return;
        }
        if b74 == 1 && op == 3 && w & 0x000F_0F00 == 0x000F_0F00 {
            self.c.r[rd] = if self.c.r[rm] != 0 { self.c.r[rm].leading_zeros() } else { 32 };
            return;
        }
        if b74 == 3 && op == 1 && sbo {
            // blx rm
            let x = self.rv(rm, 0);
            self.c.r[14] = self.pc + 4;
            self.link_to(iw(x));
            return;
        }
        if b74 == 5 {
            let (a, b) = (self.c.r[rm] as i32 as i64, self.c.r[rn] as i32 as i64);
            let c = &mut *self.c;
            let sat = |c: &mut Cpu, x: i64| -> u32 {
                if x > i32::MAX as i64 {
                    c.q = 1;
                    i32::MAX as u32
                } else if x < i32::MIN as i64 {
                    c.q = 1;
                    i32::MIN as u32
                } else {
                    x as u32
                }
            };
            let r = match op {
                0 => sat(c, a + b),
                1 => sat(c, a - b),
                2 => {
                    let d = sat(c, 2 * b) as i32 as i64;
                    sat(c, a + d)
                }
                _ => {
                    let d = sat(c, 2 * b) as i32 as i64;
                    sat(c, a - d)
                }
            };
            c.r[rd] = r;
            return;
        }
        if b74 & 9 == 8 {
            // the DSP multiplies
            let x = w >> 5 & 1 != 0;
            let y = w >> 6 & 1 != 0;
            let (rmv, rsv, rdv) = (self.c.r[rm], self.c.r[rs], self.c.r[rd]);
            match op {
                0 => {
                    // smla<x><y>
                    let s = (half(rmv, x) * half(rsv, y)) as i64 + rdv as i32 as i64;
                    if s != s as i32 as i64 {
                        self.c.q = 1;
                    }
                    self.c.r[rn] = s as u32;
                }
                1 => {
                    let p = (((rmv as i32 as i64) * half(rsv, y) as i64) >> 16) as i32;
                    if x {
                        self.c.r[rn] = p as u32; // smulw<y>
                        return;
                    }
                    let s = p as i64 + rdv as i32 as i64; // smlaw<y>
                    if s != s as i32 as i64 {
                        self.c.q = 1;
                    }
                    self.c.r[rn] = s as u32;
                }
                2 => {
                    // smlal<x><y>
                    let p = half(rmv, x) as i64 * half(rsv, y) as i64;
                    let s = ((self.c.r[rn] as u64) << 32 | rdv as u64).wrapping_add(p as u64);
                    self.c.r[rd] = s as u32;
                    self.c.r[rn] = (s >> 32) as u32;
                }
                _ => {
                    // smul<x><y>
                    self.c.r[rn] = half(rmv, x).wrapping_mul(half(rsv, y)) as u32;
                }
            }
            return;
        }
        self.undefined(); // bkpt and the rest
    }

    // ---- Thumb -------------------------------------------------------------------------
    /// Returns the instruction's size.
    fn thumb(&mut self, h: u32) -> u32 {
        let top = h >> 11;
        let lo3 = (h & 7) as usize;
        let mid3 = (h >> 3 & 7) as usize;
        if top < 3 {
            // lsl/lsr/asr rd, rm, #imm5
            let n = h >> 6 & 31;
            let mut cy = self.c.c;
            let v = self.c.r[mid3];
            let r = if top == 0 {
                if n != 0 { shift_c(0, v, n, &mut cy, false) } else { v }
            } else {
                shift_c(top as i32, v, if n != 0 { n } else { 32 }, &mut cy, false)
            };
            self.c.r[lo3] = r;
            self.c.n = r >> 31;
            self.c.z = (r == 0) as u32;
            self.c.c = cy;
            return 2;
        }
        if top == 3 {
            // add/sub rd, rn, rm|#imm3
            let b = if h & 0x400 != 0 { h >> 6 & 7 } else { self.c.r[(h >> 6 & 7) as usize] };
            let (a, cc) = (self.c.r[mid3], self.c.c);
            self.dp(if h & 0x200 != 0 { 2 } else { 4 }, true, lo3, a, b, cc);
            return 2;
        }
        if top < 8 {
            // mov/cmp/add/sub rd, #imm8
            const OPS: [u32; 4] = [13, 10, 4, 2];
            let rd = (h >> 8 & 7) as usize;
            let (a, cc) = (self.c.r[rd], self.c.c);
            self.dp(OPS[(top - 4) as usize], true, rd, a, h & 0xFF, cc);
            return 2;
        }
        if top == 8 {
            if h & 0x400 == 0 {
                // ALU rd, rs
                let alu = h >> 6 & 15;
                let mut cy = self.c.c;
                let (d, s) = (self.c.r[lo3], self.c.r[mid3]);
                match alu {
                    0 => self.dp(0, true, lo3, d, s, cy),
                    1 => self.dp(1, true, lo3, d, s, cy),
                    2 | 3 | 4 | 7 => {
                        let kind = match alu { 2 => 0, 3 => 1, 4 => 2, _ => 3 };
                        let r = shift_c(kind, d, s & 0xFF, &mut cy, true);
                        self.dp(13, true, lo3, 0, r, cy);
                    }
                    5 => self.dp(5, true, lo3, d, s, cy),
                    6 => self.dp(6, true, lo3, d, s, cy),
                    8 => self.dp(8, true, 0, d, s, cy),
                    9 => self.dp(3, true, lo3, s, 0, cy), // neg: rsb rd, rn, #0
                    10 => self.dp(10, true, 0, d, s, cy),
                    11 => self.dp(11, true, 0, d, s, cy),
                    12 => self.dp(12, true, lo3, d, s, cy),
                    13 => {
                        let r = s.wrapping_mul(d);
                        self.c.r[lo3] = r;
                        self.c.n = r >> 31;
                        self.c.z = (r == 0) as u32;
                    }
                    14 => self.dp(14, true, lo3, d, s, cy),
                    _ => self.dp(15, true, lo3, 0, s, cy),
                }
                return 2;
            }
            let op = h >> 8 & 3;
            let rd = lo3 | (h >> 4 & 8) as usize;
            let rm = (h >> 3 & 15) as usize;
            let cc = self.c.c;
            match op {
                0 => {
                    let (a, b) = (self.rv(rd, 0), self.rv(rm, 0));
                    self.dp(4, false, rd, a, b, cc);
                }
                1 => {
                    let (a, b) = (self.rv(rd, 0), self.rv(rm, 0));
                    self.dp(10, true, 0, a, b, cc);
                }
                2 => {
                    let b = self.rv(rm, 0);
                    self.dp(13, false, rd, 0, b, cc);
                }
                _ => {
                    if h & 0x80 != 0 {
                        if self.arch < 5 {
                            self.undefined();
                        } else {
                            let x = self.rv(rm, 0);
                            self.c.r[14] = (self.pc + 2) | 1;
                            self.link_to(iw(x));
                        }
                    } else {
                        let v = self.rv(rm, 0);
                        self.branch(v, true);
                    }
                }
            }
            return 2;
        }
        if top == 9 {
            // ldr rd, [pc, #imm8*4]
            let base = (self.pc + 4) & !3;
            self.ldst(0, (h >> 8 & 7) as usize, 15, base, (h & 0xFF) * 4, true, true, false);
            return 2;
        }
        if top < 12 {
            // register offset
            const OPS: [u32; 8] = [8, 10, 9, 4, 0, 2, 1, 3];
            let (base, off) = (self.c.r[mid3], self.c.r[(h >> 6 & 7) as usize]);
            self.ldst(OPS[(h >> 9 & 7) as usize], lo3, mid3, base, off, true, true, false);
            return 2;
        }
        if top < 18 {
            // immediate offset
            const OPS: [u32; 6] = [8, 0, 9, 1, 10, 2];
            const SCALE: [u32; 6] = [4, 4, 1, 1, 2, 2];
            let i = (top - 12) as usize;
            let base = self.c.r[mid3];
            self.ldst(OPS[i], lo3, mid3, base, (h >> 6 & 31) * SCALE[i], true, true, false);
            return 2;
        }
        if top < 20 {
            // str/ldr rd, [sp, #imm8*4]
            let base = self.c.r[13];
            self.ldst(if top & 1 != 0 { 0 } else { 8 }, (h >> 8 & 7) as usize, 13, base, (h & 0xFF) * 4, true, true, false);
            return 2;
        }
        if top < 22 {
            // add rd, pc|sp, #imm8*4
            let imm = (h & 0xFF) * 4;
            let base = if top & 1 != 0 { self.c.r[13] } else { (self.pc + 4) & !3 };
            self.c.r[(h >> 8 & 7) as usize] = base.wrapping_add(imm);
            return 2;
        }
        if top < 24 {
            if h & 0xFF00 == 0xB000 {
                // add/sub sp, #imm7*4
                let imm = (h & 0x7F) * 4;
                self.c.r[13] = if h & 0x80 != 0 { self.c.r[13].wrapping_sub(imm) } else { self.c.r[13].wrapping_add(imm) };
                return 2;
            }
            if h & 0xF600 == 0xB400 {
                // push / pop
                let pop = h & 0x800 != 0;
                let rl = (h & 0xFF) | ((h & 0x100) << if pop { 7 } else { 6 });
                if rl == 0 {
                    self.undefined();
                    return 2;
                }
                self.block(pop, 13, rl, if pop { 1 } else { 2 }, true, false, true);
                return 2;
            }
            self.undefined();
            return 2;
        }
        if top < 26 {
            // stmia/ldmia rn!, {list}
            let rn = (h >> 8 & 7) as usize;
            let rl = h & 0xFF;
            if rl == 0 {
                self.undefined();
                return 2;
            }
            let ld = top & 1 != 0;
            self.block(ld, rn, rl, 1, !(ld && (rl >> rn & 1) != 0), false, true);
            return 2;
        }
        if top < 28 {
            let cond = h >> 8 & 15;
            if cond == 15 {
                let ret = (self.pc + 2) | 1;
                unsafe { arm_swi(self.cp(), h & 0xFF, ret) }
                return 2;
            }
            if cond == 14 {
                self.undefined();
                return 2;
            }
            if cond_ok(self.c, cond) {
                self.to = self.pc.wrapping_add(4).wrapping_add(sext(h & 0xFF, 8) << 1) | 1;
                self.flow = Flow::Jump;
                self.reg = false;
            }
            return 2;
        }
        if top == 28 {
            self.to = self.pc.wrapping_add(4).wrapping_add(sext(h & 0x7FF, 11) << 1) | 1;
            self.flow = Flow::Jump;
            self.reg = false;
            return 2;
        }
        if top == 30 {
            // BL/BLX prefix
            let hi = sext(h & 0x7FF, 11) << 12;
            let n = self.ld16(self.pc + 2);
            if n & 0xF800 == 0xF800 || (self.arch >= 5 && n & 0xF801 == 0xE800) {
                let target = self.pc.wrapping_add(4).wrapping_add(hi).wrapping_add((n & 0x7FF) << 1);
                self.c.r[14] = (self.pc + 4) | 1;
                self.link_to(if n & 0x1000 != 0 { target | 1 } else { target & !3 });
                return 4;
            }
            self.c.r[14] = self.pc.wrapping_add(4).wrapping_add(hi); // bl_hi
            return 2;
        }
        if top == 31 || (self.arch >= 5 && h & 1 == 0) {
            // a lone suffix: pc = lr + imm
            let target = self.c.r[14].wrapping_add((h & 0x7FF) << 1);
            self.c.r[14] = (self.pc + 2) | 1;
            self.link_to(if top == 31 { (target & !1) | 1 } else { target & !3 });
            return 2;
        }
        self.undefined();
        2
    }

    // ---- the run -----------------------------------------------------------------------
    fn in_program(&self, k: u32) -> bool {
        modules().into_iter().any(|m| {
            m.cpu == self.cpu
                && m.overlay < 0
                && m.segs().iter().any(|s| s.kind == SEG_PROGRAM && (k & !1).wrapping_sub(s.base) < s.size)
        })
    }

    /// Control goes to `k` (bit 0 the state); `link`: lr was set for a return here. False when
    /// the interpreted call is over (`c.pc` says where it went).
    fn transfer(&mut self, mut k: u32, mut link: bool, mut by_reg: bool) -> bool {
        loop {
            if !link && k == self.exit {
                self.c.pc = k;
                return false;
            }
            if !self.interp_only {
                let cpu = self.cpu;
                if let Some(f) = machine(|m| m.lookup(cpu, k)) {
                    let ret = self.c.r[14];
                    self.poll();
                    unsafe { f(self.cp()) };
                    if link && self.c.pc == ret {
                        k = self.c.pc; // back from the call: go on here
                        break;
                    }
                    k = self.c.pc;
                    link = false;
                    by_reg = true;
                    continue;
                }
                if !link && by_reg && !self.from_program && self.in_program(k) {
                    self.c.pc = k;
                    return false;
                }
            }
            break;
        }
        if link || (k & !1) <= self.pc {
            self.poll();
        }
        self.pc = k & !1;
        self.t = k & 1 != 0;
        true
    }

    fn run(&mut self, start: u32) {
        self.from_program = self.in_program(start);
        self.pc = start & !1;
        self.t = start & 1 != 0;
        if !self.t {
            self.pc &= !3;
        }
        loop {
            crate::services::INTERPRETED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.flow = Flow::Next;
            let size;
            if self.t {
                let h = self.ld16(self.pc);
                size = self.thumb(h);
            } else {
                let w = self.ld32(self.pc);
                self.arm(w);
                size = 4;
            }
            match self.flow {
                Flow::Next => self.pc = self.pc.wrapping_add(size),
                Flow::Jump => {
                    if !self.transfer(self.to, false, self.reg) {
                        return;
                    }
                }
                Flow::Call => {
                    if !self.transfer(self.to, true, true) {
                        return;
                    }
                }
                Flow::Eret => {
                    if !self.transfer(self.c.pc, false, true) {
                        return;
                    }
                }
            }
        }
    }
}

/// Runs the guest code at `key` (bit 0: Thumb) from memory as it is now, until it returns to
/// lr as it was at the call.
pub fn interpret(c: &mut Cpu, key: u32) {
    let (cpu, exit) = (c.cpu, c.r[14]);
    let interp_only = machine(|m| m.interp_only);
    let mut it = Interp {
        c,
        cpu,
        arch: if cpu == 9 { 5 } else { 4 },
        exit,
        from_program: false,
        interp_only,
        pc: 0,
        t: false,
        flow: Flow::Next,
        to: 0,
        reg: false,
    };
    it.run(key);
}
