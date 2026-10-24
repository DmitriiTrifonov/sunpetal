//! The CPUs in time, and the guest's threads.
//!
//! Time. Recompiled code has no cycle counts; it has safe points (loop back-edges, calls,
//! msr). A safe point stands for a fixed number of ticks (`Sched::ticks`). The machine runs in
//! quanta: the ARM9 runs until its clock passes the quantum's end, then the ARM7. A CPU that
//! halts skips its quanta until the hardware says an interrupt is pending. A run is the same
//! run every time.
//!
//! Threads. Recompiled code nests its calls on the host stack, so each guest thread runs on its
//! own coroutine, and a CPU runs one at a time. A thread is switched out where the guest
//! switches contexts:
//!  * an exception return in recompiled code (the SDK's OS_LoadContext:
//!    `ldm r0, {r0-r14}^; subs pc, lr, #4`), whose target is where the loaded context goes
//!    (`exception_return`);
//!  * the BIOS's return from an interrupt (`irq_return`), when the SDK's handler has put another
//!    thread's context in the IRQ stack's frame.
//!
//! The target names the thread: an interrupt taken at a safe point pushes a marker as the return
//! address (`MARKER_BASE` + 16 x the thread's number), which the SDK saves in the thread's
//! context and gives back when it loads it; a context saved by OS_SaveContext (a function that
//! returns twice) is found by its sp, which the runtime noted when the save returned
//! (`context_saved`); a function's entry is a new thread. A thread switched back in at its save
//! point returns there through `bx lr`, and the recompiled callers of the save return up to the
//! landing (docs/ABI.md).
//!
//! The coroutines are asymmetric (`corosensei`): a thread that switches or ends its quantum
//! suspends to `run_cpu`, which resumes whichever thread is current.
use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};

use crate::abi::Cpu;
use crate::machine::{fatal_msg, machine, Machine};
use crate::services::{arm_call, restore_cpsr};

pub const STACK_SIZE: usize = 16 << 20;
/// Safe points between two calls of the scheduler.
pub const BUDGET: i32 = 32;
pub const MARKER_BASE: u32 = 0x0F00_0000;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Yield {
    /// The CPU's quantum is over.
    Quantum,
    /// Another thread of this CPU is current now.
    Switch,
}

type Co = Coroutine<(), Yield, (), DefaultStack>;

pub struct Thread {
    co: Option<Co>,
    yielder: *const Yielder<(), Yield>,
    pub num: usize,
    /// The return address an interrupt taken in it pushes.
    pub marker: u32,
    /// Where it was last switched in (its entry, first).
    pub key: u32,
    /// A context save returned in it and it has not run on since.
    saved: bool,
    saved_sp: u32,
}

impl Drop for Thread {
    fn drop(&mut self) {
        // A suspended coroutine dropped would unwind its stack, through the C frames of
        // recompiled code (no unwind tables): leak it, the stack goes with the process.
        if let Some(co) = self.co.take() {
            std::mem::forget(co);
        }
    }
}

pub struct Sched {
    /// Threads exist and the CPUs run in time (else `arm_poll` and friends are the stubs').
    pub active: bool,
    cpus: [Option<Box<Cpu>>; 2],
    threads: [Vec<Thread>; 2],
    cur: [usize; 2],
    /// Each CPU's clock, in ticks.
    pub time: [u64; 2],
    pub halted: [bool; 2],
    /// A write to HALTCNT: the CPU waits when the write has returned to it (`arm_*` services).
    pub halt_requested: [bool; 2],
    budget_start: [i32; 2],
    /// Ticks one safe point stands for, per CPU.
    pub ticks: [u64; 2],
    pub now: u64,
    pub qend: u64,
    /// 9, 7 or 0.
    pub running: i32,
}

impl Sched {
    pub(crate) fn new() -> Sched {
        Sched {
            active: false,
            cpus: [None, None],
            threads: [Vec::new(), Vec::new()],
            cur: [0; 2],
            time: [0; 2],
            halted: [false; 2],
            halt_requested: [false; 2],
            budget_start: [BUDGET; 2],
            ticks: [2, 4],
            now: 0,
            qend: 0,
            running: 0,
        }
    }

    pub fn thread_count(&self, cpu: i32) -> usize {
        self.threads[ci(cpu)].len()
    }

    /// Makes the CPU take its pending interrupt at the next safe point.
    pub fn force_poll(&mut self, cpu: i32) {
        let i = ci(cpu);
        if let Some(c) = self.cpus[i].as_mut() {
            if c.budget > 0 {
                self.budget_start[i] -= c.budget;
                c.budget = 0;
            }
        }
    }

    /// The CPU's context, once started.
    pub fn cpu_mut(&mut self, cpu: i32) -> Option<&mut Cpu> {
        self.cpus[ci(cpu)].as_deref_mut()
    }

    /// The thread the CPU is running (or last ran).
    pub fn current(&self, cpu: i32) -> usize {
        self.cur[ci(cpu)]
    }
}

fn ci(cpu: i32) -> usize {
    if cpu == 9 { 0 } else { 1 }
}

fn is_marker(a: u32) -> bool {
    a & 0xFFF0_000F == MARKER_BASE
}

/// This CPU's context: one per CPU, shared by all its threads (the SDK saves and loads it).
pub fn cpu_ptr(cpu: i32) -> *mut Cpu {
    machine(|m| &mut **m.sched.cpus[ci(cpu)].as_mut().expect("scheduler not started for this CPU") as *mut Cpu)
}

fn thread_main(cpu: i32, num: usize, y: &Yielder<(), Yield>) {
    machine(|m| m.sched.threads[ci(cpu)][num].yielder = y as *const _);
    let c = cpu_ptr(cpu);
    let mut key = machine(|m| m.sched.threads[ci(cpu)][num].key);
    loop {
        unsafe { arm_call(c, key) };
        let pc = unsafe { (*c).pc };
        if machine(|m| m.is_landing(cpu, pc)) {
            fatal_msg(&format!("thread {}: an unwind to {:08X} reached its base", num, pc));
        }
        if machine(|m| m.lookup(cpu, pc)).is_none() {
            fatal_msg(&format!("thread {} (from {:08X}) returned to {:08X}", num, key, pc));
        }
        key = pc; // returned to an entry: the SDK's OS_ExitThread
    }
}

fn new_thread(m: &mut Machine, cpu: i32, key: u32) -> usize {
    let i = ci(cpu);
    let num = m.sched.threads[i].len();
    let stack = DefaultStack::new(STACK_SIZE).unwrap_or_else(|e| fatal_msg(&format!("no stack for a thread: {}", e)));
    let co: Co = Coroutine::with_stack(stack, move |y: &Yielder<(), Yield>, ()| thread_main(cpu, num, y));
    m.sched.threads[i].push(Thread {
        co: Some(co),
        yielder: std::ptr::null(),
        num,
        marker: MARKER_BASE + 16 * num as u32,
        key,
        saved: false,
        saved_sp: 0,
    });
    num
}

/// Makes the scheduler run `cpu` (9 or 7): its context, and a first thread at `entry`. The CPU's
/// module must be active.
pub fn start(cpu: i32, entry: u32) {
    machine(|m| {
        let i = ci(cpu);
        let mut c = Box::new(Cpu::with_map(cpu, m.mem.map()));
        c.budget = BUDGET;
        m.sched.cpus[i] = Some(c);
        m.sched.budget_start[i] = BUDGET;
        m.sched.threads[i].clear();
        let t = new_thread(m, cpu, entry);
        m.sched.cur[i] = t;
        m.sched.active = true;
    });
}

fn suspend(yielder: *const Yielder<(), Yield>, y: Yield) {
    assert!(!yielder.is_null(), "a thread suspends before it started");
    unsafe { (*yielder).suspend(y) }
}

fn cur_yielder(cpu: i32) -> *const Yielder<(), Yield> {
    machine(|m| m.sched.threads[ci(cpu)][m.sched.cur[ci(cpu)]].yielder)
}

/// Runs the CPU's current thread (and the threads it switches to) until its quantum is over.
pub fn run_cpu(cpu: i32) {
    let i = ci(cpu);
    let hw = machine(|m| m.hw_ptr());
    let wake = machine(|m| m.sched.halted[i]) && unsafe { (*hw).pending_irq(cpu) };
    let go = machine(|m| {
        if m.sched.threads[i].is_empty() || m.sched.time[i] >= m.sched.qend {
            return false;
        }
        if m.sched.halted[i] {
            if !wake {
                m.sched.time[i] = m.sched.qend;
                return false;
            }
            m.sched.halted[i] = false;
            if m.sched.time[i] < m.sched.now {
                m.sched.time[i] = m.sched.now;
            }
        }
        m.sched.running = cpu;
        true
    });
    if !go {
        return;
    }
    loop {
        let idx = machine(|m| m.sched.cur[i]);
        let mut co = machine(|m| m.sched.threads[i][idx].co.take()).expect("a thread resumed while running");
        let r = co.resume(());
        machine(|m| m.sched.threads[i][idx].co = Some(co));
        match r {
            CoroutineResult::Yield(Yield::Quantum) => break,
            CoroutineResult::Yield(Yield::Switch) => continue,
            CoroutineResult::Return(()) => fatal_msg("a guest thread's coroutine returned"),
        }
    }
    machine(|m| m.sched.running = 0);
}

/// One quantum of `ticks`: the ARM9 runs, then the ARM7.
pub fn run_quantum(ticks: u64) {
    machine(|m| m.sched.qend = m.sched.now + ticks);
    run_cpu(9);
    run_cpu(7);
    machine(|m| m.sched.now = m.sched.qend);
}

fn account(m: &mut Machine, c: &mut Cpu) {
    let i = ci(c.cpu);
    let spent = (m.sched.budget_start[i] - c.budget).max(0) as u64;
    m.sched.time[i] += spent * m.sched.ticks[i];
    c.budget = BUDGET;
    m.sched.budget_start[i] = BUDGET;
}

fn take_irqs(c: &mut Cpu) {
    if c.irq != 0 && c.i == 0 {
        let hw = machine(|m| m.hw_ptr());
        unsafe { (*hw).irq(c) }
    }
}

/// `arm_poll` with the scheduler running: the safe points' time, the quantum, interrupts.
pub fn poll(c: &mut Cpu) {
    let cpu = c.cpu;
    let over = machine(|m| {
        account(m, c);
        m.sched.time[ci(cpu)] >= m.sched.qend
    });
    if over {
        suspend(cur_yielder(cpu), Yield::Quantum);
    }
    take_irqs(c);
}

/// The CPU waits for an interrupt (CP15's wait, HALTCNT, a BIOS wait).
pub fn halt(c: &mut Cpu) {
    let cpu = c.cpu;
    machine(|m| {
        account(m, c);
        m.sched.halted[ci(cpu)] = true;
    });
    while machine(|m| m.sched.halted[ci(cpu)]) {
        suspend(cur_yielder(cpu), Yield::Quantum);
    }
    take_irqs(c);
}

/// Time a BIOS call takes: past the quantum's end, the other CPU runs first (a wait by loop is
/// how one CPU gives the other time to answer).
pub fn spend(c: &mut Cpu, ticks: u64) {
    let cpu = c.cpu;
    let over = machine(|m| {
        m.sched.time[ci(cpu)] += ticks;
        m.sched.time[ci(cpu)] >= m.sched.qend
    });
    if over {
        suspend(cur_yielder(cpu), Yield::Quantum);
    }
}

/// A context save returned (its caller's sp): the thread that called it can be loaded again.
pub fn context_saved(c: &Cpu) {
    let (cpu, sp) = (c.cpu, c.r[13]);
    machine(|m| {
        let (i, t) = (ci(cpu), m.sched.cur[ci(cpu)]);
        let th = &mut m.sched.threads[i][t];
        th.saved = true;
        th.saved_sp = sp;
    });
}

/// The thread a context goes to, from its target and the sp.
fn thread_for(m: &mut Machine, cpu: i32, sp: u32, key: u32) -> usize {
    let i = ci(cpu);
    if is_marker(key) {
        let n = ((key - MARKER_BASE) / 16) as usize;
        if n < m.sched.threads[i].len() {
            return n;
        }
        fatal_msg(&format!("a context goes to the marker {:08X} of no thread", key));
    }
    if let Some(t) = m.sched.threads[i].iter().find(|t| t.saved && t.saved_sp == sp) {
        return t.num;
    }
    if m.lookup(cpu, key).is_some() {
        return new_thread(m, cpu, key);
    }
    fatal_msg(&format!("a context goes to {:08X} (sp {:08X}): no thread saved there, not an entry", key, sp))
}

/// Switches the CPU to thread `t`, which goes to `key`; returns when the calling thread is
/// switched back in (its own key is then the one it was switched in at).
fn switch_to(cpu: i32, t: usize, key: u32) {
    let y = machine(|m| {
        let i = ci(cpu);
        let prev = m.sched.cur[i];
        let y = m.sched.threads[i][prev].yielder;
        let th = &mut m.sched.threads[i][t];
        th.key = key;
        th.saved = false;
        m.sched.cur[i] = t;
        y
    });
    suspend(y, Yield::Switch);
}

fn ld_insn(cpu: i32, k: u32) -> u32 {
    machine(|m| m.read_mem(cpu, k & !1, if k & 1 != 0 { 2 } else { 4 })).unwrap_or(0)
}

/// An exception return in recompiled code: CPSR = SPSR, then to the target, which is another
/// thread's context. Returns (in the calling thread) when it is switched back in, at the return
/// of a context save: `bx lr` there.
pub fn exception_return(c: &mut Cpu, target: u32) {
    restore_cpsr(c, target);
    let (cpu, i) = (c.cpu, ci(c.cpu));
    let (me, t) = machine(|m| (m.sched.cur[i], thread_for(m, cpu, c.r[13], c.pc)));
    if t == me {
        fatal_msg(&format!("an exception return to {:08X} in its own thread", c.pc));
    }
    switch_to(cpu, t, c.pc);
    let k = machine(|m| m.sched.threads[i][me].key);
    let ins = ld_insn(cpu, k);
    let bx_lr = if k & 1 != 0 { ins & 0xFFFF == 0x4770 } else { ins == 0xE12F_FF1E };
    if bx_lr {
        c.pc = if c.r[14] & 1 != 0 { c.r[14] } else { c.r[14] & !3 };
        return;
    }
    fatal_msg(&format!("thread {} switched back in at {:08X}, not a return", me, k));
}

/// The BIOS's return from an interrupt: the frame popped, CPSR = SPSR_irq, `c.pc` the target.
/// Back to the interrupted code, or to another thread.
pub fn irq_return(c: &mut Cpu) {
    let (cpu, i) = (c.cpu, ci(c.cpu));
    let me = machine(|m| m.sched.cur[i]);
    let marker = machine(|m| m.sched.threads[i][me].marker);
    if c.pc == marker {
        return;
    }
    let t = machine(|m| thread_for(m, cpu, c.r[13], c.pc));
    if t == me {
        fatal_msg(&format!("the interrupt returned to {:08X} in its own thread", c.pc));
    }
    switch_to(cpu, t, c.pc);
    let k = machine(|m| m.sched.threads[i][me].key);
    if k != marker {
        fatal_msg(&format!("thread {} (interrupted) switched back in at {:08X}", me, k));
    }
}

/// The return address an interrupt taken in the CPU's current thread pushes.
pub fn irq_marker(cpu: i32) -> u32 {
    machine(|m| {
        let i = ci(cpu);
        let cur = m.sched.cur[i];
        let t = &mut m.sched.threads[i][cur];
        t.saved = false;
        t.marker
    })
}
