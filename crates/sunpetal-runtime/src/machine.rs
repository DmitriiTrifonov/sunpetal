//! The state recompiled code reaches through the runtime's services: the memories the
//! CPUs address directly, the active module of each CPU, CP15, the overlay lookup.
//!
//! One `Machine` per thread (`machine()`); the C callbacks (`mem_read*`, `arm_*`) reach
//! it there. A `Hardware` implementation is called from inside those callbacks and must
//! not call `machine()` itself.
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::sched::Sched;
use crate::abi::{modules, ArmFunc, ArmModule, Cpu, MemMap, SEG_PROGRAM};

pub const MAIN_SIZE: usize = 0x40_0000;
pub const ITCM_SIZE: usize = 0x8000;
pub const DTCM_SIZE: usize = 0x4000;
pub const WRAM7_SIZE: usize = 0x1_0000;
pub const SWRAM_SIZE: usize = 0x8000;
/// A word read at the last byte of a memory stays inside it.
const PAD: usize = 8;

/// What is not memory the CPUs hold: the I/O registers, VRAM, the cartridge, and the events
/// of time. The default (`NoHardware`) ends the run on any of them: it serves the self-test
/// and anything that only runs the game's own code.
///
/// The methods are called with no borrow of the machine held (`Machine::hw_ptr`): they may call
/// `machine()` for short borrows, and `swi` and `irq` may run guest code and yield.
pub trait Hardware {
    fn read(&mut self, cpu: i32, addr: u32, size: u32) -> u32;
    fn write(&mut self, cpu: i32, addr: u32, val: u32, size: u32);
    /// The budget of safe points is spent: time, interrupts.
    fn poll(&mut self, cpu: &mut Cpu) {
        fatal(cpu, "safe-point budget spent; lr", cpu.r[14]);
    }
    /// Whether an interrupt wakes a halted CPU (IE & IF).
    fn pending_irq(&mut self, _cpu: i32) -> bool {
        false
    }
    /// A pending interrupt is to be taken at a safe point (`cpu.irq` set, interrupts unmasked):
    /// the BIOS's entry.
    fn irq(&mut self, _cpu: &mut Cpu) {}
    /// `swi num`.
    fn swi(&mut self, cpu: &mut Cpu, num: u32) {
        fatal(cpu, "swi (no BIOS)", num);
    }
}

pub struct NoHardware;

impl Hardware for NoHardware {
    fn read(&mut self, cpu: i32, addr: u32, size: u32) -> u32 {
        fatal_msg(&format!("ARM{} {}-byte read of {:08X} (no hardware)", cpu, size, addr))
    }
    fn write(&mut self, cpu: i32, addr: u32, val: u32, size: u32) {
        fatal_msg(&format!("ARM{} {}-byte write of {:08X} to {:08X} (no hardware)", cpu, size, val, addr))
    }
}

pub fn fatal_msg(msg: &str) -> ! {
    eprintln!("sunpetal: {}", msg);
    std::process::abort()
}

pub fn fatal(cpu: &Cpu, what: &str, a: u32) -> ! {
    fatal_msg(&format!("{} {:08X} (ARM{}, lr {:08X}, sp {:08X})", what, a, cpu.cpu, cpu.r[14], cpu.r[13]))
}

pub struct Memory {
    pub main: Vec<u8>,
    pub itcm: Vec<u8>,
    pub dtcm: Vec<u8>,
    pub wram7: Vec<u8>,
    pub swram: Vec<u8>,
    /// What the C fast path reads: pointers into the vectors above (never reallocated), where
    /// the DTCM is (the SDK's crt0 puts it at 0x027E0000, CP15 c9,c1,0; size 0x4000 while CP15
    /// enables it, else 0) and the overlays' areas.
    map: Box<MemMap>,
}

impl Memory {
    fn new() -> Memory {
        let mut m = Memory {
            main: vec![0; MAIN_SIZE + PAD],
            itcm: vec![0; ITCM_SIZE + PAD],
            dtcm: vec![0; DTCM_SIZE + PAD],
            wram7: vec![0; WRAM7_SIZE + PAD],
            swram: vec![0; SWRAM_SIZE + PAD],
            map: Box::new(MemMap {
                main: std::ptr::null_mut(),
                itcm: std::ptr::null_mut(),
                dtcm: std::ptr::null_mut(),
                wram7: std::ptr::null_mut(),
                swram: std::ptr::null_mut(),
                dtcm_base: 0x027E_0000,
                dtcm_size: DTCM_SIZE as u32,
                ovl_lo: [0; 2],
                ovl_len: [0; 2],
            }),
        };
        m.map.main = m.main.as_mut_ptr();
        m.map.itcm = m.itcm.as_mut_ptr();
        m.map.dtcm = m.dtcm.as_mut_ptr();
        m.map.wram7 = m.wram7.as_mut_ptr();
        m.map.swram = m.swram.as_mut_ptr();
        m
    }

    pub fn dtcm_base(&self) -> u32 {
        self.map.dtcm_base
    }

    /// The table the contexts of this machine point at.
    pub fn map(&self) -> *const MemMap {
        &*self.map
    }

    pub fn set_dtcm(&mut self, base: u32, size: u32) {
        self.map.dtcm_base = base;
        self.map.dtcm_size = size;
    }

    pub fn clear(&mut self) {
        for m in [&mut self.main, &mut self.itcm, &mut self.dtcm, &mut self.wram7, &mut self.swram] {
            m.fill(0);
        }
    }

    /// The memory an address of `cpu` lands in, and the offset in it; `None` for the rest
    /// of the address map (the hardware). Main RAM is 4 MiB repeated to 0x02FFFFFF, the
    /// ITCM 32 KiB repeated from 0, the DTCM 16 KiB over everything else, the ARM7's WRAM
    /// 64 KiB at 0x03800000 repeated, the shared WRAM (all of it the ARM7's, as the SDK's
    /// boot leaves WRAMCNT) over 0x03000000-0x037FFFFF.
    #[inline]
    fn region(&mut self, cpu: i32, a: u32) -> Option<(&mut [u8], usize)> {
        if cpu == 9 {
            if a.wrapping_sub(self.map.dtcm_base) < self.map.dtcm_size {
                return Some((&mut self.dtcm, (a - self.map.dtcm_base) as usize));
            }
            if a < 0x0200_0000 {
                return Some((&mut self.itcm, (a & 0x7FFF) as usize));
            }
        }
        if a >> 24 == 0x02 {
            return Some((&mut self.main, (a & 0x3F_FFFF) as usize));
        }
        if cpu == 7 {
            if a >> 23 == 7 {
                return Some((&mut self.wram7, (a & 0xFFFF) as usize));
            }
            if a >> 23 == 6 {
                return Some((&mut self.swram, (a & 0x7FFF) as usize));
            }
        }
        None
    }

    /// Guest memory as bytes, for loaders and tests. False if some byte is not memory held here.
    pub fn write_bytes(&mut self, cpu: i32, addr: u32, data: &[u8]) -> bool {
        for (i, b) in data.iter().enumerate() {
            match self.region(cpu, addr.wrapping_add(i as u32)) {
                Some((m, o)) => m[o] = *b,
                None => return false,
            }
        }
        true
    }

    pub fn read_bytes(&mut self, cpu: i32, addr: u32, out: &mut [u8]) -> bool {
        for (i, b) in out.iter_mut().enumerate() {
            match self.region(cpu, addr.wrapping_add(i as u32)) {
                Some((m, o)) => *b = m[o],
                None => return false,
            }
        }
        true
    }
}

/// An overlay function of the build, by key.
struct OvFn {
    key: u32,
    module: &'static ArmModule,
    index: usize,
}

/// One CPU's overlays: a call into their load area is looked up among the overlay modules'
/// functions, the one whose bytes are in memory as built (`ArmModule::extents` and `crcs`)
/// runs. A store into the area forgets every answer.
#[derive(Default)]
struct Overlays {
    lo: u32,
    len: u32,
    fns: Vec<OvFn>,
    cache: HashMap<u32, Option<ArmFunc>>,
}

pub struct Machine {
    pub mem: Memory,
    pub hw: Box<dyn Hardware>,
    pub cp15: Vec<u32>,
    /// CP15 as the machine has it (the TCMs' places, the wait for an interrupt) instead of an array.
    pub cp15_real: bool,
    pub cp15_control: u32,
    /// The CPUs in time and the guest's threads (`sched`).
    pub sched: Sched,
    /// Guest functions run by host code instead (HLE of SDK routines, tests): looked up first.
    overrides: HashMap<(i32, u32), ArmFunc>,
    /// The self-test's interpreter mode: no recompiled code, callees interpreted too.
    pub interp_only: bool,
    active: [Option<&'static ArmModule>; 2],
    ovl: [Overlays; 2],
    ovl_made: bool,
}

thread_local! {
    static MACHINE: UnsafeCell<Machine> = UnsafeCell::new(Machine::new());
}

/// This thread's machine. Do not nest.
pub fn machine<R>(f: impl FnOnce(&mut Machine) -> R) -> R {
    MACHINE.with(|m| f(unsafe { &mut *m.get() }))
}

fn ci(cpu: i32) -> usize {
    if cpu == 9 { 0 } else { 1 }
}

fn crc_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut k = i as u32;
            for _ in 0..8 {
                k = if k & 1 != 0 { 0xEDB8_8320 ^ (k >> 1) } else { k >> 1 };
            }
            *e = k;
        }
        t
    })
}

/// crc32 (zlib's), continuing from `crc` (0 to start).
pub fn crc32(data: &[u8], crc: u32) -> u32 {
    let t = crc_table();
    let mut c = !crc;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

impl Machine {
    fn new() -> Machine {
        Machine {
            mem: Memory::new(),
            hw: Box::new(NoHardware),
            cp15: vec![0; 8 * 16 * 16 * 8],
            cp15_real: false,
            cp15_control: 0x0005_2078,
            interp_only: false,
            sched: Sched::new(),
            overrides: HashMap::new(),
            active: [None, None],
            ovl: [Overlays::default(), Overlays::default()],
            ovl_made: false,
        }
    }

    // ---- memory ---------------------------------------------------------------
    /// Reads the memory the CPUs hold; `None` for the rest of the address map.
    #[inline]
    pub fn read_mem(&mut self, cpu: i32, a: u32, size: usize) -> Option<u32> {
        let (m, o) = self.mem.region(cpu, a)?;
        Some(match size {
            1 => m[o] as u32,
            2 => u16::from_le_bytes([m[o], m[o + 1]]) as u32,
            _ => u32::from_le_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]]),
        })
    }

    /// Writes the memory the CPUs hold; false for the rest of the address map. A store into the
    /// overlays' area forgets which overlay functions are in memory.
    #[inline]
    pub fn write_mem(&mut self, cpu: i32, a: u32, v: u32, size: usize) -> bool {
        let i = ci(cpu);
        if a.wrapping_sub(self.ovl[i].lo) < self.ovl[i].len {
            self.ovl[i].cache.clear();
        }
        match self.mem.region(cpu, a) {
            Some((m, o)) => {
                match size {
                    1 => m[o] = v as u8,
                    2 => m[o..o + 2].copy_from_slice(&(v as u16).to_le_bytes()),
                    _ => m[o..o + 4].copy_from_slice(&v.to_le_bytes()),
                }
                true
            }
            None => false,
        }
    }

    /// The hardware, for calls made with no borrow of the machine held.
    pub fn hw_ptr(&mut self) -> *mut dyn Hardware {
        &mut *self.hw as *mut dyn Hardware
    }

    // ---- modules ----------------------------------------------------------------
    /// Whether the memory holds the module's program segments as built.
    pub fn identify(&mut self, m: &ArmModule) -> bool {
        for s in m.segs() {
            if s.kind != SEG_PROGRAM {
                continue; // not there until the program copies or loads it
            }
            let mut buf = vec![0u8; s.size as usize];
            if self.mem.region(m.cpu, s.base).is_none() {
                continue; // not memory held here (VRAM): not checked
            }
            if !self.mem.read_bytes(m.cpu, s.base, &mut buf) || crc32(&buf, 0) != s.crc {
                return false;
            }
        }
        true
    }

    /// Makes `m` the module of its CPU.
    pub fn activate(&mut self, m: &'static ArmModule) {
        if !self.ovl_made {
            self.make_overlays(); // before the first store the fast path must send to the slow one
        }
        self.active[ci(m.cpu)] = Some(m);
    }

    fn make_overlays(&mut self) {
        self.ovl_made = true;
        for cpu in [9, 7] {
            let o = &mut self.ovl[ci(cpu)];
            let (mut lo, mut hi, mut any) = (0u32, 0u32, false);
            for m in modules().into_iter().filter(|m| m.cpu == cpu && m.overlay >= 0 && m.nsegs > 0) {
                let s = m.segs()[0];
                if !any || s.base < lo {
                    lo = s.base;
                }
                if !any || s.base + s.size > hi {
                    hi = s.base + s.size;
                }
                any = true;
                for (index, e) in m.funcs().iter().enumerate() {
                    o.fns.push(OvFn { key: e.key, module: m, index });
                }
            }
            if any {
                o.lo = lo & !0xFF;
                o.len = hi - o.lo;
                o.fns.sort_by_key(|f| f.key);
                self.mem.map.ovl_lo[ci(cpu)] = o.lo;
                self.mem.map.ovl_len[ci(cpu)] = o.len;
            }
        }
    }

    fn bytes_there(&mut self, cpu: i32, m: &ArmModule, i: usize) -> bool {
        let e = m.extents()[i];
        let mut buf = vec![0u8; e.size as usize];
        self.mem.read_bytes(cpu, e.base, &mut buf) && crc32(&buf, 0) == m.crcs()[i]
    }

    fn overlay_fn(&mut self, cpu: i32, key: u32) -> Option<ArmFunc> {
        if !self.ovl_made {
            self.make_overlays();
        }
        let i = ci(cpu);
        if (key & !1).wrapping_sub(self.ovl[i].lo) >= self.ovl[i].len {
            return None;
        }
        if let Some(r) = self.ovl[i].cache.get(&key) {
            return *r;
        }
        let first = self.ovl[i].fns.partition_point(|f| f.key < key);
        let mut found = None;
        let mut j = first;
        while j < self.ovl[i].fns.len() && self.ovl[i].fns[j].key == key {
            let (m, idx) = (self.ovl[i].fns[j].module, self.ovl[i].fns[j].index);
            if self.bytes_there(cpu, m, idx) {
                found = Some(m.funcs()[idx].func);
                break;
            }
            j += 1;
        }
        self.ovl[i].cache.insert(key, found);
        found
    }

    /// The recompiled function at `key` (address, bit 0 Thumb) of the active module or of
    /// an overlay found in memory.
    pub fn lookup(&mut self, cpu: i32, key: u32) -> Option<ArmFunc> {
        if let Some(f) = self.overrides.get(&(cpu, key)) {
            return Some(*f);
        }
        let m = self.active[ci(cpu)]?;
        let f = m.funcs();
        let i = f.partition_point(|e| e.key < key);
        if i < f.len() && f[i].key == key {
            return Some(f[i].func);
        }
        self.overlay_fn(cpu, key)
    }

    /// Runs `f` in place of the guest function at `key` (bit 0: Thumb) of `cpu`: it is an entry
    /// for calls, jumps and thread starts, and must return to `lr` as recompiled code does.
    pub fn override_fn(&mut self, cpu: i32, key: u32, f: ArmFunc) {
        self.overrides.insert((cpu, key), f);
    }

    /// A return that went to a landing: a saved context was loaded.
    pub fn is_landing(&mut self, cpu: i32, key: u32) -> bool {
        let has = |m: &ArmModule| m.landings().binary_search(&key).is_ok();
        if self.active[ci(cpu)].map_or(false, has) {
            return true;
        }
        if !self.ovl_made {
            self.make_overlays();
        }
        let a = key & !1;
        modules().into_iter().any(|m| {
            m.cpu == cpu
                && m.overlay >= 0
                && m.nsegs > 0
                && a.wrapping_sub(m.segs()[0].base) < m.segs()[0].size
                && has(m)
        })
    }

    /// Memory changed behind the stores' back: every overlay is checked again.
    pub fn overlays_unknown(&mut self) {
        for o in &mut self.ovl {
            o.cache.clear();
        }
    }
}
