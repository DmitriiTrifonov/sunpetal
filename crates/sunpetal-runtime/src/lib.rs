//! Runtime of statically recompiled Nintendo DS programs: the Rust side of the ABI
//! that generated C code calls (docs/ABI.md, include/sunpetal_abi.h).
pub mod abi;
pub mod bios;
pub mod interp;
pub mod machine;
pub mod nds;
pub mod sched;
pub mod selftest;
pub mod services;
#[cfg(feature = "debug-tools")]
pub mod dialogue;
#[cfg(feature = "debug-tools")]
pub mod script;
#[cfg(feature = "debug-tools")]
pub mod evn;
pub use sunpetal_data::nitrofs;
#[cfg(feature = "debug-tools")]
pub mod session;

pub use abi::{modules, ArmModule, Cpu};
pub use machine::{machine, Hardware, Machine, NoHardware};

use std::ffi::CStr;

/// The generated module called `name` (`ARM9`, `ARM9_OV1`, `ARM7`, ...).
pub fn module(name: &str) -> Option<&'static ArmModule> {
    modules().into_iter().find(|m| unsafe { CStr::from_ptr(m.name) }.to_bytes() == name.as_bytes())
}

/// Calls the recompiled function at `key` (address, bit 0 for Thumb) of the active module of
/// `cpu.cpu` with the registers as they are; the function returns to `cpu.r[14]`.
pub fn call(cpu: &mut Cpu, key: u32) {
    unsafe { services::arm_call(cpu, key) }
}
