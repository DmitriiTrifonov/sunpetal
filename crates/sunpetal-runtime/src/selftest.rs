//! The recompiler's self-test: recompiled functions run on the vectors that
//! `python -m ndskit.recomp.selftest` recorded with the interpreter (armemu), and every
//! result is compared.
//!
//! A vectors file is lines of:
//! ```text
//! cpu 9|7                 the CPU of what follows (its memory map; memory cleared)
//! image BASE PATH         load a file at BASE (hex) into that CPU's memory
//! module NAME             activate that module (its segments must be in memory)
//! func KEY LABEL          the function the next vectors call (KEY: address, bit 0 Thumb)
//! v IN... OUT...          16 hex words each: r0-r14, cpsr (lr in: the return sentinel)
//! mem CRC...              crc32 of main RAM, then of the ITCM and DTCM (ARM9) or of
//!                         the ARM7's WRAM and the shared WRAM, after the function's vectors
//! ```
//! Memory is shared by all vectors in order, as in the interpreter's run.
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::abi::Cpu;
use crate::machine::{crc32, machine, MAIN_SIZE};
use crate::services::set_cpsr;
use crate::{call, module};

/// armemu.RETURN_SENTINEL
const SENTINEL: u32 = 0xFFFF_FFF0;
const NAMES: [&str; 16] = [
    "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "sp", "lr", "cpsr",
];

#[derive(Debug, Default)]
pub struct Report {
    pub functions: u32,
    pub vectors: u32,
    pub failures: u32,
    /// What went wrong, for the first few failures of each function.
    pub messages: Vec<String>,
}

fn hex(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s, 16).map_err(|e| format!("bad hex {:?}: {}", s, e))
}

fn crcs(cpu: i32) -> Vec<u32> {
    machine(|m| {
        let mem = &m.mem;
        let mut out = vec![crc32(&mem.main[..MAIN_SIZE], 0)];
        if cpu == 9 {
            out.push(crc32(&mem.itcm[..0x8000], 0));
            out.push(crc32(&mem.dtcm[..0x4000], 0));
        } else {
            out.push(crc32(&mem.wram7[..0x1_0000], 0));
            out.push(crc32(&mem.swram[..0x8000], 0));
        }
        out
    })
}

fn clear() {
    machine(|m| {
        m.mem.clear();
        m.overlays_unknown(); // memory changed behind the stores' back
    })
}

/// Replays a vectors file on the recompiled code, or with `interp` on the interpreter alone
/// (no recompiled code, callees interpreted too). `Err` is a problem with the file or the
/// build (a missing module, an image that cannot be loaded), not a failed vector.
pub fn run_vectors(path: &Path, interp: bool) -> Result<Report, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let mut rep = Report::default();
    let (mut cpu, mut key, mut label) = (9i32, 0u32, String::new());
    let (mut fvec, mut fbad) = (0u32, 0u32);
    machine(|m| m.interp_only = interp);
    clear();
    for line in text.lines() {
        let (kw, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kw {
            "" => {}
            k if k.starts_with('#') => {}
            "cpu" => {
                cpu = rest.trim().parse().map_err(|e| format!("bad cpu: {}", e))?;
                clear();
            }
            "image" => {
                let (base, img) = rest.split_once(' ').ok_or("image: no path")?;
                let data = fs::read(img.trim()).map_err(|e| format!("cannot load {}: {}", img.trim(), e))?;
                if data.is_empty() || !machine(|m| m.mem.write_bytes(cpu, hex(base).unwrap_or(0), &data)) {
                    return Err(format!("cannot load {} at {}", img.trim(), base));
                }
                machine(|m| m.overlays_unknown());
            }
            "module" => {
                let name = rest.trim();
                let md = module(name).ok_or_else(|| format!("no module {} in this build", name))?;
                if !machine(|m| m.identify(md)) {
                    return Err(format!("the memory does not hold module {} as it was built", name));
                }
                machine(|m| m.activate(md));
            }
            "func" => {
                let (e, l) = rest.split_once(' ').unwrap_or((rest, ""));
                key = hex(e)?;
                label = l.trim().to_string();
                rep.functions += 1;
                fbad = 0;
                fvec = 0;
            }
            "v" => {
                let w: Vec<u32> = rest.split_whitespace().map(hex).collect::<Result<_, _>>()?;
                if w.len() != 32 {
                    return Err(format!("vector with {} words", w.len()));
                }
                let mut c = Cpu::new(cpu);
                set_cpsr(&mut c, w[15], 0xF);
                c.r[..15].copy_from_slice(&w[..15]);
                c.r[14] = SENTINEL;
                c.pc = 0;
                c.budget = 1 << 20;
                if interp {
                    unsafe { crate::services::arm_interpret(&mut c, key) }
                } else {
                    call(&mut c, key);
                }
                let mut got = [0u32; 16];
                got[..15].copy_from_slice(&c.r[..15]);
                got[15] = c.cpsr();
                rep.vectors += 1;
                fvec += 1;
                if c.pc != SENTINEL || got[..] != w[16..] {
                    rep.failures += 1;
                    if fbad < 3 {
                        let mut s = format!("FAIL {:08X} {}, vector {}:", key, label, fvec);
                        if c.pc != SENTINEL {
                            let _ = write!(s, " returned to {:08X}", c.pc);
                        }
                        for i in 0..16 {
                            if got[i] != w[16 + i] {
                                let _ = write!(s, " {}={:08X} (want {:08X})", NAMES[i], got[i], w[16 + i]);
                            }
                        }
                        let _ = write!(s, "\n      in: {}", w[..16].iter().map(|x| format!("{:08X}", x)).collect::<Vec<_>>().join(" "));
                        rep.messages.push(s);
                    }
                    fbad += 1;
                }
            }
            "mem" => {
                let want: Vec<u32> = rest.split_whitespace().map(hex).collect::<Result<_, _>>()?;
                let have = crcs(cpu);
                if want != have {
                    rep.failures += 1;
                    let which: Vec<&str> =
                        have.iter().zip(&want).map(|(h, w)| if h == w { "same" } else { "differs" }).collect();
                    rep.messages.push(format!(
                        "FAIL {:08X} {}: memory differs after its vectors ({})",
                        key,
                        label,
                        which.join(", ")
                    ));
                    break; // going on from the interpreter's state is not possible
                }
            }
            other => return Err(format!("unknown line {:?}", other)),
        }
    }
    Ok(rep)
}
