//! The scheduler and the guest threads, on guest functions implemented by the host (no ROM, no
//! generated code): a context switch through an exception return, quanta, and a halted CPU.
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use sunpetal_runtime::sched;
use sunpetal_runtime::services::{arm_context_saved, arm_exception_return, arm_poll};
use sunpetal_runtime::{machine, Cpu, Hardware};

thread_local! {
    static LOG: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

fn log(s: &str) {
    LOG.with(|l| l.borrow_mut().push(s.to_string()));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

const E0: u32 = 0x0200_0000; // thread 0's entry
const E1: u32 = 0x0200_0040; // thread 1's entry
const E2: u32 = 0x0200_0080; // where thread 0 goes on when its entry returns
const BX_LR: u32 = 0x0200_0100; // a `bx lr` in memory: where a saved context returns

unsafe extern "C" fn thread0(c: *mut Cpu) {
    let c = &mut *c;
    log("A1");
    // what OS_SaveContext leaves: the save returned, with this sp, to its caller
    c.r[13] = 0x1000;
    c.r[14] = E2;
    arm_context_saved(c);
    // OS_LoadContext of another context: its sp, then an exception return to its pc
    c.r[13] = 0x2000;
    arm_exception_return(c, E1);
    // switched back in: here, with lr as loaded
    log("A2");
    c.pc = c.r[14];
}

unsafe extern "C" fn thread1(c: *mut Cpu) {
    let c = &mut *c;
    log("B1");
    // load thread 0's saved context: its sp, lr, and the pc of its save's return
    c.r[13] = 0x1000;
    c.r[14] = E2;
    arm_exception_return(c, BX_LR);
    log("B2 (never)");
}

unsafe extern "C" fn exited(c: *mut Cpu) {
    let c = &mut *c;
    log("A3");
    loop {
        c.budget = 0;
        arm_poll(c);
        log("tick");
    }
}

static WAKE: AtomicBool = AtomicBool::new(false);

struct Wake;
impl Hardware for Wake {
    fn read(&mut self, _: i32, _: u32, _: u32) -> u32 {
        0
    }
    fn write(&mut self, _: i32, _: u32, _: u32, _: u32) {}
    fn pending_irq(&mut self, _cpu: i32) -> bool {
        WAKE.load(Ordering::SeqCst)
    }
}

unsafe extern "C" fn sleeper(c: *mut Cpu) {
    let c = &mut *c;
    log("H1");
    sched::halt(c);
    log("H2");
    loop {
        c.budget = 0;
        arm_poll(c);
    }
}

#[test]
fn context_switch_and_quanta() {
    machine(|m| {
        m.override_fn(9, E0, thread0);
        m.override_fn(9, E1, thread1);
        m.override_fn(9, E2, exited);
        m.mem.write_bytes(9, BX_LR, &0xE12F_FF1Eu32.to_le_bytes());
    });
    sched::start(9, E0);
    sched::run_quantum(1000);
    let l = take_log();
    assert_eq!(&l[..4], ["A1", "B1", "A2", "A3"], "{:?}", l);
    let ticks = l.iter().filter(|s| *s == "tick").count();
    assert!(ticks > 5 && ticks < 40, "the quantum ends after about 1000/64 polls: {}", ticks);
    // the next quantum goes on in the same loop, in thread 0
    sched::run_quantum(1000);
    let l = take_log();
    assert!(l.iter().all(|s| s == "tick") && !l.is_empty(), "{:?}", l);
    machine(|m| {
        assert_eq!(m.sched.thread_count(9), 2);
        assert_eq!(m.sched.current(9), 0);
        assert_eq!(m.sched.now, 2000);
    });
}

#[test]
fn halted_cpu_sleeps_until_woken() {
    machine(|m| {
        m.hw = Box::new(Wake);
        m.override_fn(7, E0, sleeper);
    });
    WAKE.store(false, Ordering::SeqCst);
    sched::start(7, E0);
    sched::run_quantum(1000);
    assert_eq!(take_log(), ["H1"]);
    sched::run_quantum(1000);
    assert!(take_log().is_empty(), "halted: nothing runs");
    WAKE.store(true, Ordering::SeqCst);
    sched::run_quantum(1000);
    assert_eq!(take_log(), ["H2"]);
}
