//! Layouts shared with the generated C (`include/sunpetal_abi.h`). Change both together.
use std::os::raw::c_char;

pub const BANK_USR: usize = 0;
pub const BANK_FIQ: usize = 1;
pub const BANK_IRQ: usize = 2;
pub const BANK_SVC: usize = 3;
pub const BANK_ABT: usize = 4;
pub const BANK_UND: usize = 5;
pub const NBANKS: usize = 6;

/// The memories the CPUs address directly, as the C fast path of `ld*`/`st*` sees them.
#[repr(C)]
pub struct MemMap {
    pub main: *mut u8,
    pub itcm: *mut u8,
    pub dtcm: *mut u8,
    pub wram7: *mut u8,
    pub swram: *mut u8,
    pub dtcm_base: u32,
    pub dtcm_size: u32,
    pub ovl_lo: [u32; 2],
    pub ovl_len: [u32; 2],
}

/// Registers and flags of one CPU as recompiled code sees them.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Cpu {
    /// r15 is unused: recompiled code knows its own pc.
    pub r: [u32; 16],
    pub n: u32,
    pub z: u32,
    pub c: u32,
    pub v: u32,
    pub q: u32,
    pub i: u32,
    pub f: u32,
    pub mode: u32,
    /// Where the last return or jump went (bit 0: Thumb).
    pub pc: u32,
    /// Safe points left before `arm_poll`.
    pub budget: i32,
    /// 9 or 7.
    pub cpu: i32,
    pub bank: [[u32; 2]; NBANKS],
    pub fiq_other: [u32; 5],
    pub spsr: [u32; NBANKS],
    /// An interrupt is pending (IME, IE & IF): the runtime keeps it.
    pub irq: u32,
    /// The machine's memories (this thread's), for the fast path.
    pub mm: *const MemMap,
}

impl Cpu {
    /// A zeroed CPU in system mode, on this thread's machine.
    pub fn new(cpu: i32) -> Cpu {
        Cpu::with_map(cpu, crate::machine::machine(|m| m.mem.map()))
    }

    /// Like `new`, for use where the machine is already borrowed.
    pub fn with_map(cpu: i32, mm: *const MemMap) -> Cpu {
        Cpu {
            r: [0; 16],
            n: 0,
            z: 0,
            c: 0,
            v: 0,
            q: 0,
            i: 0,
            f: 0,
            mode: 0x1F,
            pc: 0,
            budget: 0,
            cpu,
            bank: [[0; 2]; NBANKS],
            fiq_other: [0; 5],
            spsr: [0; NBANKS],
            irq: 0,
            mm,
        }
    }

    pub fn cpsr(&self) -> u32 {
        self.n << 31 | self.z << 30 | self.c << 29 | self.v << 28 | self.q << 27 | self.i << 7 | self.f << 6 | self.mode
    }
}

pub type ArmFunc = unsafe extern "C" fn(*mut Cpu);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArmFuncEntry {
    pub key: u32,
    pub func: ArmFunc,
}

pub const SEG_PROGRAM: u32 = 0;
pub const SEG_COPIED: u32 = 1;
pub const SEG_OVERLAY: u32 = 2;
pub const SEG_LOADED: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArmSegment {
    pub base: u32,
    pub size: u32,
    pub crc: u32,
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArmRange {
    pub base: u32,
    pub size: u32,
}

/// One recompiled program (or one overlay of it): the generated `p_<name>_module`.
#[repr(C)]
pub struct ArmModule {
    pub name: *const c_char,
    pub cpu: i32,
    pub segs: *const ArmSegment,
    pub nsegs: u32,
    pub funcs: *const ArmFuncEntry,
    pub nfuncs: u32,
    pub landings: *const u32,
    pub nlandings: u32,
    /// Overlay id, or -1 for a program's own module.
    pub overlay: i32,
    pub extents: *const ArmRange,
    pub crcs: *const u32,
}

// The tables are immutable statics in the generated C.
unsafe impl Sync for ArmModule {}

impl ArmModule {
    pub fn name(&self) -> &str {
        unsafe { std::ffi::CStr::from_ptr(self.name) }.to_str().unwrap_or("?")
    }
    pub fn segs(&self) -> &[ArmSegment] {
        if self.nsegs == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.segs, self.nsegs as usize) } }
    }
    pub fn funcs(&self) -> &[ArmFuncEntry] {
        if self.nfuncs == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.funcs, self.nfuncs as usize) } }
    }
    pub fn landings(&self) -> &[u32] {
        if self.nlandings == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.landings, self.nlandings as usize) } }
    }
    pub fn extents(&self) -> &[ArmRange] {
        if self.overlay < 0 || self.nfuncs == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.extents, self.nfuncs as usize) } }
    }
    pub fn crcs(&self) -> &[u32] {
        if self.overlay < 0 || self.nfuncs == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.crcs, self.nfuncs as usize) } }
    }
}

#[cfg(has_generated)]
extern "C" {
    static g_arm_modules: [*const ArmModule; 0];
    static g_arm_nmodules: i32;
}

/// Every module of the generated build.
#[cfg(has_generated)]
pub fn modules() -> Vec<&'static ArmModule> {
    unsafe {
        let p = std::ptr::addr_of!(g_arm_modules) as *const *const ArmModule;
        (0..g_arm_nmodules as usize).map(|i| &**p.add(i)).collect()
    }
}

#[cfg(not(has_generated))]
pub fn modules() -> Vec<&'static ArmModule> {
    Vec::new()
}
