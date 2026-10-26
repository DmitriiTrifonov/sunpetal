//! The functions of `sunpetal_abi.h` that the runtime provides to recompiled C: the bus,
//! processor modes, calls through registers, CP15, and the services that end the run.
//!
//! Each is a thin extern "C" shim over `Machine`; see `docs/ABI.md`.
#![allow(clippy::missing_safety_doc)]
use std::os::raw::c_int;

use crate::abi::*;
use crate::machine::{fatal, fatal_msg, machine};

#[no_mangle]
pub static mut g_arm_trace: c_int = 0;

// ---- memory ---------------------------------------------------------------------------
// The slow path of ld*/st*: what the fast path of the header does not hold.
unsafe fn slow_read(c: *mut Cpu, a: u32, size: usize) -> u32 {
    let cpu = (*c).cpu;
    if let Some(v) = machine(|m| m.read_mem(cpu, a, size)) {
        return v;
    }
    let hw = machine(|m| m.hw_ptr());
    let v = (*hw).read(cpu, a, size as u32);
    match size {
        1 => v & 0xFF,
        2 => v & 0xFFFF,
        _ => v,
    }
}

unsafe fn slow_write(c: *mut Cpu, a: u32, v: u32, size: usize) {
    let cpu = (*c).cpu;
    if machine(|m| m.write_mem(cpu, a, v, size)) {
        return;
    }
    let v = match size {
        1 => v & 0xFF,
        2 => v & 0xFFFF,
        _ => v,
    };
    let hw = machine(|m| m.hw_ptr());
    (*hw).write(cpu, a, v, size as u32);
    // a write to HALTCNT: the wait is the CPU's, not the hardware's (nothing is borrowed now)
    if machine(|m| std::mem::take(&mut m.sched.halt_requested[if cpu == 9 { 0 } else { 1 }])) {
        crate::sched::halt(&mut *c);
    }
}

#[no_mangle]
pub unsafe extern "C" fn mem_read8(c: *mut Cpu, a: u32) -> u32 {
    slow_read(c, a, 1)
}
#[no_mangle]
pub unsafe extern "C" fn mem_read16(c: *mut Cpu, a: u32) -> u32 {
    slow_read(c, a, 2)
}
#[no_mangle]
pub unsafe extern "C" fn mem_read32(c: *mut Cpu, a: u32) -> u32 {
    slow_read(c, a, 4)
}
#[no_mangle]
pub unsafe extern "C" fn mem_write8(c: *mut Cpu, a: u32, v: u32) {
    slow_write(c, a, v, 1)
}
#[no_mangle]
pub unsafe extern "C" fn mem_write16(c: *mut Cpu, a: u32, v: u32) {
    slow_write(c, a, v, 2)
}
#[no_mangle]
pub unsafe extern "C" fn mem_write32(c: *mut Cpu, a: u32, v: u32) {
    slow_write(c, a, v, 4)
}

// ---- processor modes --------------------------------------------------------------------
fn bank_of(mode: u32) -> usize {
    match mode {
        0x11 => BANK_FIQ,
        0x12 => BANK_IRQ,
        0x13 => BANK_SVC,
        0x17 => BANK_ABT,
        0x1B => BANK_UND,
        _ => BANK_USR, // user, system (and the reserved modes)
    }
}

/// Switches the banked registers.
pub fn set_mode(c: &mut Cpu, mode: u32) {
    let (ob, nb) = (bank_of(c.mode), bank_of(mode));
    if ob != nb {
        c.bank[ob] = [c.r[13], c.r[14]];
        if (ob == BANK_FIQ) != (nb == BANK_FIQ) {
            for i in 0..5 {
                std::mem::swap(&mut c.r[8 + i], &mut c.fiq_other[i]);
            }
        }
        c.r[13] = c.bank[nb][0];
        c.r[14] = c.bank[nb][1];
    }
    c.mode = mode;
}

/// `msr cpsr_<fields>`: mask c=1, x=2, s=4, f=8; in user mode only the flags change.
pub fn set_cpsr(c: &mut Cpu, v: u32, mask: u32) {
    if mask & 8 != 0 {
        c.n = v >> 31;
        c.z = v >> 30 & 1;
        c.c = v >> 29 & 1;
        c.v = v >> 28 & 1;
        if c.cpu == 9 {
            c.q = v >> 27 & 1;
        }
    }
    if mask & 1 != 0 && c.mode != 0x10 {
        c.i = v >> 7 & 1;
        c.f = v >> 6 & 1;
        set_mode(c, (v & 0x1F) | 0x10);
        if c.irq != 0 && c.i == 0 {
            c.budget = 0; // unmasked with one pending: taken at the next safe point
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_set_mode(c: *mut Cpu, mode: u32) {
    set_mode(&mut *c, mode)
}

#[no_mangle]
pub unsafe extern "C" fn arm_get_spsr(c: *const Cpu) -> u32 {
    let c = &*c;
    let b = bank_of(c.mode);
    if b == BANK_USR { c.cpsr() } else { c.spsr[b] }
}

#[no_mangle]
pub unsafe extern "C" fn arm_set_spsr(c: *mut Cpu, v: u32, mask: u32) {
    let c = &mut *c;
    let b = bank_of(c.mode);
    if b == BANK_USR {
        return;
    }
    let m = (if mask & 1 != 0 { 0xFF } else { 0 })
        | (if mask & 2 != 0 { 0xFF00 } else { 0 })
        | (if mask & 4 != 0 { 0xFF_0000 } else { 0 })
        | (if mask & 8 != 0 { 0xFF00_0000 } else { 0 });
    c.spsr[b] = (c.spsr[b] & !m) | (v & m);
}

/// CPSR = SPSR, then `pc` = the target in the restored state.
pub fn restore_cpsr(c: &mut Cpu, target: u32) {
    let mut t = 0;
    let b = bank_of(c.mode);
    if b != BANK_USR {
        let s = c.spsr[b];
        set_cpsr(c, s, 0xF);
        t = s >> 5 & 1;
    }
    c.pc = if t != 0 { (target & !1) | 1 } else { target & !3 };
}

#[no_mangle]
pub unsafe extern "C" fn arm_restore_cpsr(c: *mut Cpu, target: u32) {
    restore_cpsr(&mut *c, target)
}

#[no_mangle]
pub unsafe extern "C" fn arm_exception_return(c: *mut Cpu, target: u32) {
    if machine(|m| m.sched.active) {
        crate::sched::exception_return(&mut *c, target)
    } else {
        restore_cpsr(&mut *c, target)
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_user_reg(c: *const Cpu, n: c_int) -> u32 {
    let c = &*c;
    let n = n as usize;
    let b = bank_of(c.mode);
    if n >= 13 && b != BANK_USR {
        c.bank[BANK_USR][n - 13]
    } else if (8..=12).contains(&n) && b == BANK_FIQ {
        c.fiq_other[n - 8]
    } else {
        c.r[n]
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_set_user_reg(c: *mut Cpu, n: c_int, v: u32) {
    let c = &mut *c;
    let n = n as usize;
    let b = bank_of(c.mode);
    if n >= 13 && b != BANK_USR {
        c.bank[BANK_USR][n - 13] = v;
    } else if (8..=12).contains(&n) && b == BANK_FIQ {
        c.fiq_other[n - 8] = v;
    } else {
        c.r[n] = v;
    }
}

/// How often the hot service calls ran (a diagnostic for the app's status line): polls, calls
/// through registers, interpreted instructions.
pub static POLLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static INTERPRETED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn counters() -> (u64, u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (POLLS.load(Relaxed), CALLS.load(Relaxed), INTERPRETED.load(Relaxed))
}

// ---- calls through registers ---------------------------------------------------------------
/// A call to `key` through a register: an entry of a module, else the code there is
/// interpreted (code no module holds: decrypted, copied at run time).
#[no_mangle]
pub unsafe extern "C" fn arm_call(c: *mut Cpu, key: u32) {
    CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match machine(|m| m.lookup((*c).cpu, key)) {
        Some(f) => f(c),
        None => crate::interp::interpret(&mut *c, key),
    }
}

/// A jump through a register whose target analysis did not know: an entry is a tail call;
/// anything else is taken for a return (a copy of lr, a saved return address) and checked
/// by the caller's ARM_RET.
#[no_mangle]
pub unsafe extern "C" fn arm_jump(c: *mut Cpu, key: u32) {
    match machine(|m| m.lookup((*c).cpu, key)) {
        Some(f) => f(c),
        None => (*c).pc = key,
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_poll(c: *mut Cpu) {
    POLLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if machine(|m| m.sched.active) {
        crate::sched::poll(&mut *c)
    } else {
        let hw = machine(|m| m.hw_ptr());
        (*hw).poll(&mut *c)
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_swi(c: *mut Cpu, num: u32, _ret: u32) {
    let hw = machine(|m| m.hw_ptr());
    (*hw).swi(&mut *c, num)
}

#[no_mangle]
pub unsafe extern "C" fn arm_undefined(c: *mut Cpu, pc: u32) {
    fatal(&*c, "undefined instruction at", pc)
}

#[no_mangle]
pub unsafe extern "C" fn arm_bad_return(c: *mut Cpu, expected: u32) {
    let (cpu, pc) = ((*c).cpu, (*c).pc);
    if machine(|m| m.is_landing(cpu, pc)) {
        return; // a saved context was loaded: the callers return up to the landing
    }
    fatal_msg(&format!("ARM{} returned to {:08X}, expected {:08X}", cpu, pc, expected))
}

#[no_mangle]
pub unsafe extern "C" fn arm_context_saved(c: *mut Cpu) {
    if machine(|m| m.sched.active) {
        crate::sched::context_saved(&*c)
    }
}

#[no_mangle]
pub unsafe extern "C" fn arm_interpret(c: *mut Cpu, key: u32) {
    crate::interp::interpret(&mut *c, key)
}

#[no_mangle]
pub unsafe extern "C" fn arm_trace_enter(_c: *mut Cpu, _key: u32) {
    crate::nds::note_enter(&*_c, _key);
    #[cfg(feature = "debug-tools")]
    crate::nds::debug::on_enter(&mut *_c, _key);
}

// ---- CP15 ------------------------------------------------------------------------------------
// Without the machine (`Machine::cp15_real` off): keeps what is written to it and gives it back,
// as armemu does with no handler set. With it (the ARM9): the TCMs' places and the wait for an
// interrupt; the rest kept.
fn cp15_index(op1: c_int, crn: c_int, crm: c_int, op2: c_int) -> usize {
    (((op1 & 7) as usize * 16 + (crn & 15) as usize) * 16 + (crm & 15) as usize) * 8 + (op2 & 7) as usize
}

fn tcm_update(m: &mut crate::machine::Machine) {
    let r = m.cp15[cp15_index(0, 9, 1, 0)];
    let size = if m.cp15_control >> 16 & 1 != 0 { 0x4000 } else { 0 };
    m.mem.set_dtcm(r & 0xFFFF_F000, size);
}

/// CP15 as the DS boots: the DTCM at 0x03000000, 16 KiB; the ITCM 32 MiB at 0.
pub fn cp15_boot() {
    machine(|m| {
        m.cp15_real = true;
        m.cp15[cp15_index(0, 9, 1, 0)] = 0x0300_000A;
        m.cp15[cp15_index(0, 9, 1, 1)] = 0x0000_0020;
        tcm_update(m);
    })
}

#[no_mangle]
pub unsafe extern "C" fn arm_cp15_read(c: *mut Cpu, op1: c_int, crn: c_int, crm: c_int, op2: c_int) -> u32 {
    machine(|m| {
        if m.cp15_real {
            if (*c).cpu != 9 {
                fatal(&*c, "CP15 read on the ARM7 at", 0);
            }
            if op1 == 0 && crn == 0 && crm == 0 {
                return match op2 {
                    1 => 0x0F0D_2112,
                    2 => 0x0014_0180,
                    _ => 0x4105_9461,
                };
            }
            if op1 == 0 && crn == 1 && crm == 0 && op2 == 0 {
                return m.cp15_control;
            }
        }
        m.cp15[cp15_index(op1, crn, crm, op2)]
    })
}

#[no_mangle]
pub unsafe extern "C" fn arm_cp15_write(c: *mut Cpu, op1: c_int, crn: c_int, crm: c_int, op2: c_int, v: u32) {
    let halt = machine(|m| {
        if !m.cp15_real {
            m.cp15[cp15_index(op1, crn, crm, op2)] = v;
            return false;
        }
        if (*c).cpu != 9 {
            fatal(&*c, "CP15 write on the ARM7 at", 0);
        }
        if op1 == 0 && crn == 1 && crm == 0 && op2 == 0 {
            m.cp15_control = (v & 0x000F_F085) | 0x78;
            tcm_update(m);
            return false;
        }
        m.cp15[cp15_index(op1, crn, crm, op2)] = v;
        if crn == 9 && crm == 1 {
            tcm_update(m);
        }
        (crn == 7 && crm == 0 && op2 == 4) || (crn == 7 && crm == 8 && op2 == 2)
    });
    if halt {
        crate::sched::halt(&mut *c);
    }
}
