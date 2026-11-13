//! Looking into the guest from the host (feature `debug-tools`): memory by address and by
//! contents, and observers of function entries, which the recompiled code reports through
//! `ARM_ENTER` once `g_arm_trace` is on (nothing is on without a call to `watch` or `log_calls`).
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::abi::Cpu;
use crate::machine::machine;
use crate::services::g_arm_trace;

/// The main RAM of the DS: where the games' data and scripts live.
const MAIN_RAM: std::ops::Range<u32> = 0x0200_0000..0x0240_0000;

/// An entry into a guest function: the key is the address, bit 0 set for Thumb.
#[derive(Clone, Copy, Debug)]
pub struct Call {
    pub vblank: u64,
    pub cpu: i32,
    pub key: u32,
    pub lr: u32,
    pub args: [u32; 4],
}

type Observer = Rc<RefCell<dyn FnMut(&mut Cpu)>>;

#[derive(Default)]
struct Observers {
    watched: HashMap<(i32, u32), Vec<Observer>>,
    log: Option<Vec<Call>>,
    log_cap: usize,
    /// The cartridge's reads (address, length) since the last `take_card_reads`, when asked for.
    card: Option<Vec<(u32, u32)>>,
}

thread_local! {
    /// The ARM9 function entered last (while `trace_everything` is on), and how often each read the clip matrix.
    static LAST_ENTER: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
    static CLIP_CALLERS: RefCell<HashMap<u32, u64>> = RefCell::new(HashMap::new());
    static CLIP_READS: RefCell<HashMap<(u32, u32), u64>> = RefCell::new(HashMap::new());
    static OBSERVERS: RefCell<Observers> = RefCell::new(Observers::default());
}

fn trace_on(on: bool) {
    unsafe { g_arm_trace = on as _ };
}

/// Runs `f` at each entry into the function at `key` of `cpu` (9 or 7), with the registers as
/// the call has them.
pub fn watch(cpu: i32, key: u32, mut f: impl FnMut(&Cpu) + 'static) {
    add(cpu, key, Rc::new(RefCell::new(move |c: &mut Cpu| f(c))));
}

fn add(cpu: i32, key: u32, w: Observer) {
    OBSERVERS.with(|o| o.borrow_mut().watched.entry((cpu, key)).or_default().push(w));
    trace_on(true);
}

/// Runs `f` once at the next entry into the ARM9 function at `key`, with its registers (which come back as
/// they were): the guest is at a point of its own making, so `f` may call its functions (`crate::call`).
pub fn inject(key: u32, f: impl FnOnce(&mut Cpu) + 'static) {
    let slot = Rc::new(RefCell::new(Some(f)));
    let me: Rc<RefCell<Option<Observer>>> = Rc::new(RefCell::new(None));
    let (slot2, me2) = (slot.clone(), me.clone());
    let w: Observer = Rc::new(RefCell::new(move |c: &mut Cpu| {
        let Some(f) = slot2.borrow_mut().take() else { return };
        if let Some(me) = me2.borrow_mut().take() {
            OBSERVERS.with(|o| {
                if let Some(v) = o.borrow_mut().watched.get_mut(&(9, key)) {
                    v.retain(|x| !Rc::ptr_eq(x, &me));
                }
            });
        }
        let saved = *c;
        f(c);
        *c = saved;
    }));
    *me.borrow_mut() = Some(w.clone());
    add(9, key, w);
}

/// Starts (or, with `cap` 0, stops) recording every entry of ARM9 and ARM7 code, up to `cap`.
pub fn log_calls(cap: usize) {
    OBSERVERS.with(|o| {
        let mut o = o.borrow_mut();
        o.log = (cap != 0).then(Vec::new);
        o.log_cap = cap;
        trace_on(cap != 0 || !o.watched.is_empty());
    });
}

/// The entries recorded since the last call.
pub fn take_calls() -> Vec<Call> {
    OBSERVERS.with(|o| o.borrow_mut().log.as_mut().map(std::mem::take).unwrap_or_default())
}

/// Starts recording the cartridge's reads (the game's files are read from it).
pub fn log_card_reads() {
    OBSERVERS.with(|o| o.borrow_mut().card.get_or_insert_with(Vec::new).clear());
}

/// The cartridge's reads since the last call.
pub fn take_card_reads() -> Vec<(u32, u32)> {
    OBSERVERS.with(|o| o.borrow_mut().card.as_mut().map(std::mem::take).unwrap_or_default())
}

/// The cartridge read a block (`nds::cart`).
pub(crate) fn on_card_read(addr: u32, len: u32) {
    OBSERVERS.with(|o| {
        if let Some(c) = o.borrow_mut().card.as_mut() {
            c.push((addr, len));
        }
    });
}

/// Every function entry is reported from now on (slow), for `clip_report`.
pub fn trace_everything(on: bool) {
    trace_on(on);
}

/// A read of the GX clip matrix: counted against the function entered last.
pub(crate) fn note_clip_read() {
    let f = LAST_ENTER.with(|l| l.get());
    CLIP_READS.with(|h| *h.borrow_mut().entry(f).or_default() += 1);
}

/// The functions entered last at the clip matrix's reads, most reads first.
pub fn clip_report() -> Vec<((u32, u32), u64)> {
    let mut v: Vec<((u32, u32), u64)> = CLIP_READS.with(|h| h.borrow().iter().map(|(k, n)| (*k, *n)).collect());
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v
}

/// The return addresses of the calls of the SDK's clip matrix getter (`02004EAC`), most calls first.
pub fn clip_callers() -> Vec<(u32, u64)> {
    let mut v: Vec<(u32, u64)> = CLIP_CALLERS.with(|h| h.borrow().iter().map(|(k, n)| (*k, *n)).collect());
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v
}

/// One store of the ARM9 into the watched range (`watch_writes`).
#[derive(Clone, Debug)]
pub struct WriteEvent {
    pub vblank: u64,
    pub addr: u32,
    pub size: u8,
    pub value: u32,
    /// A load (`watch_writes` with `reads`), not a store.
    pub read: bool,
    /// The ARM9 function entered last and the return address it was entered with (the store is in it or in what it called).
    pub func: u32,
    pub func_lr: u32,
    /// The link register at the store (a leaf function's caller).
    pub lr: u32,
}

static WRITES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static READS_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

thread_local! {
    static WRITES: RefCell<(u32, u32, usize, Vec<WriteEvent>)> = const { RefCell::new((0, 0, 0, Vec::new())) };
}

/// Records the ARM9's stores into `[addr, addr + len)` (at most `cap` of them, then the oldest are kept) until
/// `stop_writes`. Turns on the entry trace (slow) so the writer's function is known.
pub fn watch_writes(addr: u32, len: u32, cap: usize, reads: bool) {
    READS_ON.store(reads, std::sync::atomic::Ordering::Relaxed);
    WRITES.with(|w| *w.borrow_mut() = (addr, len, cap, Vec::new()));
    machine(|m| m.set_write_range(addr, len));
    WRITES_ON.store(true, std::sync::atomic::Ordering::Relaxed);
    trace_on(true);
}

/// Stops the write watch; the stores seen so far stay for `take_writes`.
pub fn stop_writes() {
    READS_ON.store(false, std::sync::atomic::Ordering::Relaxed);
    WRITES_ON.store(false, std::sync::atomic::Ordering::Relaxed);
    machine(|m| m.set_write_range(0, 0));
}

/// The stores seen since the last call.
pub fn take_writes() -> Vec<WriteEvent> {
    WRITES.with(|w| std::mem::take(&mut w.borrow_mut().3))
}

/// `services::slow_write`'s call: a store of the ARM9 (the watch is checked there, cheaply).
pub(crate) fn on_write(c: &Cpu, a: u32, v: u32, size: usize) {
    if !WRITES_ON.load(std::sync::atomic::Ordering::Relaxed) || c.cpu != 9 {
        return;
    }
    let (func, func_lr) = LAST_ENTER.with(|l| l.get());
    WRITES.with(|w| {
        let mut w = w.borrow_mut();
        let (lo, len, cap) = (w.0, w.1, w.2);
        if a.wrapping_sub(lo) < len && w.3.len() < cap {
            w.3.push(WriteEvent { vblank: super::vblanks(), addr: a, size: size as u8, value: v, read: false, func, func_lr, lr: c.r[14] });
        }
    });
}

/// `services::slow_read`'s call: a load of the ARM9 from the watched range (only with `reads`).
pub(crate) fn on_read(c: &Cpu, a: u32, v: u32, size: usize) {
    if !READS_ON.load(std::sync::atomic::Ordering::Relaxed) || c.cpu != 9 {
        return;
    }
    let (func, func_lr) = LAST_ENTER.with(|l| l.get());
    WRITES.with(|w| {
        let mut w = w.borrow_mut();
        let (lo, len, cap) = (w.0, w.1, w.2);
        if a.wrapping_sub(lo) < len && w.3.len() < cap {
            w.3.push(WriteEvent { vblank: super::vblanks(), addr: a, size: size as u8, value: v, read: true, func, func_lr, lr: c.r[14] });
        }
    });
}

/// `ARM_ENTER`'s service (`services::arm_trace_enter`).
pub(crate) fn on_enter(c: &mut Cpu, key: u32) {
    if c.cpu == 9 {
        if key == 0x0200_4EAC {
            CLIP_CALLERS.with(|h| *h.borrow_mut().entry(c.r[14]).or_default() += 1);
        }
        LAST_ENTER.with(|l| l.set((key, c.r[14])));
    }
    let watchers = OBSERVERS.with(|o| {
        let mut o = o.borrow_mut();
        if o.log.as_ref().is_some_and(|l| l.len() < o.log_cap) {
            let call = Call { vblank: super::vblanks(), cpu: c.cpu, key, lr: c.r[14], args: [c.r[0], c.r[1], c.r[2], c.r[3]] };
            o.log.as_mut().unwrap().push(call);
        }
        o.watched.get(&(c.cpu, key)).cloned().unwrap_or_default()
    });
    for w in watchers {
        (w.borrow_mut())(c);
    }
}

/// The frame being drawn, by model and mesh: the polygons of each (`None`: batches not out of a model file) and
/// the matrix-stack slot of the last MTX_RESTORE before it (255: none) and the position matrix (fx 20.12) at its first batch.
pub fn frame_models() -> Vec<(Option<super::modelsrc::ModelAt>, u32, u8, [i32; 16])> {
    super::nds(|n| n.frame_models())
}

/// `len` bytes of ARM9 memory at `addr`, if all of them are memory.
pub fn peek(addr: u32, len: usize) -> Option<Vec<u8>> {
    if (0x0400_0000..0x0800_0000).contains(&addr) {
        // registers, palettes, video memory, OAM: as the ARM9 sees them (0 where nothing is mapped)
        return Some(super::nds(|n| (0..len as u32).map(|i| n.mmio_read(9, addr + i, 1) as u8).collect()));
    }
    let mut out = vec![0; len];
    machine(|m| m.mem.read_bytes(9, addr, &mut out)).then_some(out)
}

/// Writes `data` at `addr` of the ARM9's memory; false if it is not memory.
pub fn poke(addr: u32, data: &[u8]) -> bool {
    machine(|m| m.mem.write_bytes(9, addr, data))
}

/// An NNS animation object found in RAM (docs/ULTIMATE-IMPORT.md, У3.6).
#[derive(Debug, PartialEq, Eq)]
pub struct AnimObject {
    pub object: u32,
    /// Address and size of the raw `.nsbca` file the object plays.
    pub file: u32,
    pub size: u32,
    /// Frame, fx32 (1.0 = 0x1000).
    pub frame: u32,
}

/// Animation objects in a main-RAM image `ram` (based at `base`): the game loads each `.nsbca` raw (`BCA0`),
/// its record (`J\0AC`) is at `JNT0 + 0x30`; an object is a block whose +8 points to that record (+0 frame).
pub fn anim_objects_in(ram: &[u8], base: u32) -> Vec<AnimObject> {
    let u32_at = |o: usize| u32::from_le_bytes([ram[o], ram[o + 1], ram[o + 2], ram[o + 3]]);
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(p) = ram[at..].windows(6).position(|w| w == b"BCA0\xff\xfe") {
        let f = at + p;
        at = f + 1;
        if f + 0x18 > ram.len() {
            break;
        }
        let size = u32_at(f + 8) as usize;
        if size == 0 || size > 0x80000 || f + size > ram.len() || &ram[f + 0x14..f + 0x18] != b"JNT0" {
            continue;
        }
        let count = u16::from_le_bytes([ram[f + 14], ram[f + 15]]) as usize;
        for k in 0..count {
            if 16 + 4 * k + 4 > size {
                break;
            }
            let rec = base + (f as u32) + u32_at(f + 16 + 4 * k) + 0x30;
            let needle = rec.to_le_bytes();
            for (i, w) in ram.windows(4).enumerate() {
                if i % 4 == 0 && i >= 8 && w == needle && i + 4 <= ram.len() - 4 {
                    let next = u32_at(i + 4);
                    if (base..base + ram.len() as u32).contains(&next) {
                        out.push(AnimObject { object: base + i as u32 - 8, file: base + f as u32, size: size as u32, frame: u32_at(i - 8) });
                    }
                }
            }
        }
    }
    out
}

/// A motion table (consecutive pointers to animation objects) with the record that points at it.
#[derive(Debug, PartialEq, Eq)]
pub struct MotionSet {
    /// Address of the record's field that holds the table pointer (+8 of it is the current object).
    pub record_field: u32,
    pub table: u32,
    /// Objects of the table, in table order.
    pub objects: Vec<u32>,
    /// Index in `objects` of the object the record plays now (`None` if the +0 slot of the field is not one of them).
    pub current: Option<usize>,
}

/// Motion sets in `ram`: a table is >= 3 consecutive words that are animation objects (`known`); the record
/// that plays it keeps the table pointer at +8 after the current object pointer (the hero at C02_01: 0228FE10
/// current, 0228FE18 table 0229C990; stand = M0101, walk = M0102).
pub fn motion_sets_in(ram: &[u8], base: u32, known: &[u32]) -> Vec<MotionSet> {
    let u32_at = |o: usize| u32::from_le_bytes([ram[o], ram[o + 1], ram[o + 2], ram[o + 3]]);
    let n = ram.len() / 4;
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !known.contains(&u32_at(i * 4)) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && known.contains(&u32_at(i * 4)) {
            i += 1;
        }
        if i - start < 3 {
            continue;
        }
        let table = base + 4 * start as u32;
        let objects: Vec<u32> = (start..i).map(|k| u32_at(k * 4)).collect();
        for k in 2..n {
            if u32_at(k * 4) == table {
                let cur = u32_at((k - 2) * 4);
                out.push(MotionSet { record_field: base + 4 * k as u32, table, current: objects.iter().position(|&o| o == cur), objects: objects.clone() });
            }
        }
    }
    out
}

/// The motion sets the game holds now.
pub fn motion_sets() -> Vec<MotionSet> {
    let Some(ram) = peek(MAIN_RAM.start, (MAIN_RAM.end - MAIN_RAM.start) as usize) else {
        return Vec::new();
    };
    let known: Vec<u32> = anim_objects_in(&ram, MAIN_RAM.start).iter().map(|a| a.object).collect();
    motion_sets_in(&ram, MAIN_RAM.start, &known)
}

/// The animation objects the game holds now.
pub fn anim_objects() -> Vec<AnimObject> {
    peek(MAIN_RAM.start, (MAIN_RAM.end - MAIN_RAM.start) as usize).map(|ram| anim_objects_in(&ram, MAIN_RAM.start)).unwrap_or_default()
}

/// The addresses of main RAM where `needle` is.
pub fn find(needle: &[u8]) -> Vec<u32> {
    let Some(ram) = peek(MAIN_RAM.start, (MAIN_RAM.end - MAIN_RAM.start) as usize) else {
        return Vec::new();
    };
    let mut at = Vec::new();
    let mut from = 0;
    while let Some(i) = ram[from..].windows(needle.len()).position(|w| w == needle) {
        at.push(MAIN_RAM.start + (from + i) as u32);
        from += i + 1;
    }
    at
}

/// `s` as UTF-16LE bytes, the script text's encoding.
pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[cfg(test)]
mod anim_tests {
    use super::*;

    #[test]
    fn finds_object_by_record_pointer() {
        let base = 0x0200_0000u32;
        let mut ram = vec![0u8; 0x400];
        let f = 0x40;
        ram[f..f + 6].copy_from_slice(b"BCA0\xff\xfe");
        ram[f + 8..f + 12].copy_from_slice(&0x100u32.to_le_bytes());
        ram[f + 14..f + 16].copy_from_slice(&1u16.to_le_bytes());
        ram[f + 16..f + 20].copy_from_slice(&0x4u32.to_le_bytes()); // JNT0-relative record offset
        ram[f + 0x14..f + 0x18].copy_from_slice(b"JNT0");
        let rec = base + f as u32 + 4 + 0x30;
        let o = 0x200;
        ram[o..o + 4].copy_from_slice(&0x3000u32.to_le_bytes());
        ram[o + 8..o + 12].copy_from_slice(&rec.to_le_bytes());
        ram[o + 12..o + 16].copy_from_slice(&(base + 0x10).to_le_bytes());
        // the pointer is at +8 and the field after it (+0xC) must point into RAM (as in anim_probe.py: word at +4 of the hit)
        let got = anim_objects_in(&ram, base);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].object, base + o as u32);
        assert_eq!(got[0].frame, 0x3000);
    }

    #[test]
    fn finds_motion_set_and_current() {
        let base = 0x0200_0000u32;
        let mut ram = vec![0u8; 0x200];
        let objs = [0x0200_0100u32, 0x0200_0110, 0x0200_0120];
        for (k, o) in objs.iter().enumerate() {
            ram[0x40 + 4 * k..0x44 + 4 * k].copy_from_slice(&o.to_le_bytes());
        }
        // record: current object at 0x80, table pointer at 0x88
        ram[0x80..0x84].copy_from_slice(&objs[1].to_le_bytes());
        ram[0x88..0x8C].copy_from_slice(&(base + 0x40).to_le_bytes());
        let sets = motion_sets_in(&ram, base, &objs);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].table, base + 0x40);
        assert_eq!(sets[0].record_field, base + 0x88);
        assert_eq!(sets[0].current, Some(1));
    }
}
