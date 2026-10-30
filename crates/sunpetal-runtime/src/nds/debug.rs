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
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// The return addresses of the calls of the SDK's clip matrix getter (`02004EAC`), most calls first.
pub fn clip_callers() -> Vec<(u32, u64)> {
    let mut v: Vec<(u32, u64)> = CLIP_CALLERS.with(|h| h.borrow().iter().map(|(k, n)| (*k, *n)).collect());
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
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
        if o.log.as_ref().map_or(false, |l| l.len() < o.log_cap) {
            let call = Call { vblank: super::vblanks(), cpu: c.cpu, key, lr: c.r[14], args: [c.r[0], c.r[1], c.r[2], c.r[3]] };
            o.log.as_mut().unwrap().push(call);
        }
        o.watched.get(&(c.cpu, key)).cloned().unwrap_or_default()
    });
    for w in watchers {
        (w.borrow_mut())(c);
    }
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
