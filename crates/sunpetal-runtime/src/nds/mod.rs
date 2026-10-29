//! The DS around the CPUs: what recompiled code reaches through the slow path of memory (the
//! I/O registers, VRAM, the cartridge, the serial devices), the events of time, interrupts, and
//! the boot. `NdsHw` plugs it into the machine as its `Hardware`.
//!
//! Time is counted in ticks of the ARM9's clock (67.03 MHz); the system clock that the timers,
//! the display and the ARM7 run on (33.51 MHz) is 2 ticks. A line is 355 dots of 6 system
//! cycles, a frame 263 lines.
//!
//! The state lives in one `Nds` per thread (`nds()`). Its methods may call `machine()` for short
//! borrows; nothing may call `nds()` from inside them (the DMA, the timers and the rest use
//! their own `&mut self`).
mod cart;
#[cfg(feature = "debug-tools")]
pub mod debug;
mod gpu2d;
mod gx;
mod input;
mod jobdump;
mod mmio;
mod render3d;
#[cfg(feature = "gpu")]
mod render3d_gpu;
mod sound;
mod texpack;
mod spi;
mod system;
mod video;

use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::abi::{modules, Cpu, SEG_LOADED, SEG_PROGRAM};
use crate::machine::{fatal_msg, machine, Hardware};
use crate::sched;

pub const LINE_TICKS: u64 = 355 * 6 * 2;
pub const FRAME_TICKS: u64 = 263 * LINE_TICKS;
/// Into the line, where HBlank starts.
pub const HBLANK_TICKS: u64 = 256 * 6 * 2;
const QUANTUM: u64 = LINE_TICKS / 4;

/// What a run is told.
#[derive(Clone, Debug)]
pub struct Config {
    /// The cartridge.
    pub rom: PathBuf,
    /// Where the run writes (the save, logs).
    pub out: PathBuf,
    /// The backup memory's file (default: out/<rom>.sav).
    pub save: Option<PathBuf>,
    /// Its size when the file does not exist yet (the chip's type): 64 KiB, which serves the
    /// EEPROMs with a 2-byte address (8 KiB and 64 KiB) alike. An existing file keeps its own size.
    pub save_size: usize,
    /// Stop after this many VBlanks (0: never).
    pub stop_vblanks: u64,
    /// Log the machine's events.
    pub trace: bool,
    /// Write each texture the game draws as a PNG into this folder (`texpack`).
    pub dump_textures: Option<PathBuf>,
    /// A folder of PNG replacements for textures, named by their keys (`texpack`).
    pub texture_pack: Option<PathBuf>,
    /// A folder of files that replace the ROM's by their NitroFS paths (the translation).
    pub overlay: Option<PathBuf>,
    /// The data file (`sunpetal.data`) and the language of its translation to put in (`ru`).
    pub data: Option<PathBuf>,
    pub language: Option<String>,
    /// Ticks a safe point stands for on the ARM9 and on the ARM7.
    pub ticks9: u64,
    pub ticks7: u64,
    /// Buttons and taps by VBlank (`input.rs`).
    pub input: String,
}

impl Config {
    pub fn new(rom: impl Into<PathBuf>) -> Config {
        Config {
            rom: rom.into(),
            out: PathBuf::from("build/run"),
            save: None,
            save_size: 65536,
            stop_vblanks: 0,
            trace: false,
            dump_textures: None,
            texture_pack: None,
            overlay: None,
            data: None,
            language: None,
            ticks9: 16,
            ticks7: 24,
            input: String::new(),
        }
    }
}

/// The files that replace the ROM's: the translation of `cfg.language` from the data file, then the
/// files of the overlay folder (they win). `rom` is the ROM as it is on disk.
pub fn overlay_files(cfg: &Config, rom: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    if let (None, Some(lang)) = (&cfg.data, &cfg.language) {
        return Err(format!("the language `{}` is asked for, but there is no data file with translations (--data FILE)", lang));
    }
    if let (Some(data), Some(lang)) = (&cfg.data, &cfg.language) {
        let c = sunpetal_data::container::Container::read_only(data, |t| t == b"TRAN" || t == b"META")?;
        if let Some(meta) = c.meta()? {
            meta.check_rom(sunpetal_data::delta::crc32(rom)).map_err(|e| format!("{}: {}", data.display(), e))?;
        }
        let all: Vec<_> = c.all(b"TRAN").filter_map(|s| sunpetal_data::translate::Translation::from_bytes(&s.data).ok()).collect();
        let t = all.iter().find(|t| &t.lang == lang).ok_or_else(|| {
            let have: Vec<&str> = all.iter().map(|t| t.lang.as_str()).collect();
            if have.is_empty() {
                format!("{} has no translations (it holds only textures); play without a language", data.display())
            } else {
                format!("{} has no translation `{}`; it has: {}", data.display(), lang, have.join(", "))
            }
        })?;
        files.extend(t.overlay(rom)?);
    }
    if let Some(dir) = &cfg.overlay {
        files.extend(crate::nitrofs::read_overlay_dir(dir)?);
    }
    Ok(files)
}

fn ci(cpu: i32) -> usize {
    if cpu == 9 { 0 } else { 1 }
}

// ---- events ---------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Ev {
    Hblank,
    Line,
    Timer(i32, usize),
}

#[derive(Clone, Copy)]
struct Event {
    kind: Ev,
    when: u64,
    on: bool,
}

const EV_COUNT: usize = 10;

fn ev_index(e: Ev) -> usize {
    match e {
        Ev::Hblank => 0,
        Ev::Line => 1,
        Ev::Timer(cpu, n) => 2 + if cpu == 9 { 0 } else { 4 } + n,
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Irq {
    pub ime: u32,
    pub ie: u32,
    pub iflags: u32,
}

pub struct Nds {
    pub(crate) cfg: Config,
    io: [Vec<u8>; 2],
    wifi: Vec<u8>,
    palette: Vec<u8>,
    oam: Vec<u8>,
    keys: u16,
    extkeys: u16,
    pub(crate) irq: [Irq; 2],
    irq_count: [[u64; 32]; 2],
    events: [Event; EV_COUNT],
    sys: system::System,
    video: video::Video,
    gpu: gpu2d::Gpu2d,
    gx: gx::Gx,
    snd: sound::Sound,
    texpack: texpack::TexPack,
    /// The 3D picture the rendering engine drew last, 256x192 (`render3d`).
    frame3d: Vec<u32>,
    /// Pixels the 3D picture has on each side beyond the 256 (a widened view, `set_wide`).
    frame3d_wide: usize,
    /// The widening asked for, in DS pixels each side.
    wide: usize,
    /// The scale it was drawn at.
    frame3d_scale: usize,
    /// The picture's scale: the 3D is drawn, and the screens put together, this many times as large
    /// each way (1-8).
    scale: usize,
    extras: Extras,
    script: Vec<input::Press>,
    cart: cart::Cart,
    spi: spi::Spi,
    told: HashSet<(&'static str, u32)>,
    pub(crate) vblanks: u64,
    input_at: u64,
    stop: Option<&'static str>,
}

thread_local! {
    static NDS: UnsafeCell<Nds> = UnsafeCell::new(Nds::new(Config::new("")));
}

/// This thread's DS. Do not nest.
pub fn nds<R>(f: impl FnOnce(&mut Nds) -> R) -> R {
    NDS.with(|n| f(unsafe { &mut *n.get() }))
}

/// The running CPU's time, or the machine's between quanta.
pub fn now() -> u64 {
    machine(|m| if m.sched.running != 0 { m.sched.time[ci(m.sched.running)] } else { m.sched.now })
}

fn tag() -> &'static str {
    match machine(|m| m.sched.running) {
        9 => "9",
        7 => "7",
        _ => "-",
    }
}

fn say(msg: &str) {
    let t = now();
    eprintln!("[{:5}:{:03} {}] {}", t / FRAME_TICKS, t % FRAME_TICKS / LINE_TICKS, tag(), msg);
}

/// A line of the run's log.
pub fn note(msg: &str) {
    say(msg)
}

impl Nds {
    fn new(cfg: Config) -> Nds {
        let ev = |kind| Event { kind, when: 0, on: false };
        Nds {
            cfg,
            io: [vec![0; 0x2000], vec![0; 0x2000]],
            wifi: vec![0; 0x10000],
            palette: vec![0; 0x800],
            oam: vec![0; 0x800],
            keys: 0x03FF,
            extkeys: 0x007F,
            irq: [Irq::default(); 2],
            irq_count: [[0; 32]; 2],
            events: [
                ev(Ev::Hblank),
                ev(Ev::Line),
                ev(Ev::Timer(9, 0)),
                ev(Ev::Timer(9, 1)),
                ev(Ev::Timer(9, 2)),
                ev(Ev::Timer(9, 3)),
                ev(Ev::Timer(7, 0)),
                ev(Ev::Timer(7, 1)),
                ev(Ev::Timer(7, 2)),
                ev(Ev::Timer(7, 3)),
            ],
            sys: system::System::new(),
            video: video::Video::new(),
            gpu: gpu2d::Gpu2d::new(),
            gx: gx::Gx::new(),
            snd: sound::Sound::new(),
            texpack: texpack::TexPack::new(),
            frame3d: vec![0; 256 * 192],
            frame3d_wide: 0,
            wide: 0,
            frame3d_scale: 1,
            scale: 1,
            extras: Extras::default(),
            script: Vec::new(),
            cart: cart::Cart::new(),
            spi: spi::Spi::new(),
            told: HashSet::new(),
            vblanks: 0,
            input_at: 0,
            stop: None,
        }
    }

    // ---- logging ------------------------------------------------------------------------
    pub(crate) fn trace(&self, msg: impl FnOnce() -> String) {
        if self.cfg.trace {
            say(&msg());
        }
    }

    /// The first time only for this (`what`, `key`).
    pub(crate) fn once(&mut self, what: &'static str, key: u32, msg: impl FnOnce() -> String) {
        if self.told.insert((what, key)) {
            say(&msg());
        }
    }

    // ---- events -------------------------------------------------------------------------
    pub(super) fn schedule(&mut self, e: Ev, when: u64) {
        let ev = &mut self.events[ev_index(e)];
        ev.when = when;
        ev.on = true;
    }

    pub(super) fn cancel(&mut self, e: Ev) {
        self.events[ev_index(e)].on = false;
    }

    fn next_event(&self) -> u64 {
        self.events.iter().filter(|e| e.on).map(|e| e.when).min().unwrap_or(u64::MAX)
    }

    fn fire_events(&mut self, now: u64) {
        loop {
            let best = self
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.on && e.when <= now)
                .min_by_key(|(_, e)| e.when)
                .map(|(i, _)| i);
            let Some(i) = best else { return };
            self.events[i].on = false;
            let (kind, when) = (self.events[i].kind, self.events[i].when);
            match kind {
                Ev::Hblank => self.hblank(when),
                Ev::Line => self.line(when),
                Ev::Timer(cpu, n) => self.timer_event(cpu, n, when),
            }
        }
    }

    // ---- interrupts ---------------------------------------------------------------------
    /// IF bits by number (0 VBlank, 1 HBlank, 2 VCount, 3-6 timers, 7 RTC, 8-11 DMA, 12 keys,
    /// 16 IPC sync, 17 IPC send empty, 18 IPC receive not empty, 19 card done, 21 geometry FIFO,
    /// 22 lid, 23 SPI).
    pub(crate) fn irq_raise(&mut self, cpu: i32, bit: u32) {
        self.irq[ci(cpu)].iflags |= 1 << bit;
        self.irq_count[ci(cpu)][bit as usize] += 1;
        self.irq_update(cpu);
    }

    /// After IME, IE or IF changed: the CPU's interrupt line, and a poll at once when it is open.
    pub(crate) fn irq_update(&mut self, cpu: i32) {
        let p = self.irq[ci(cpu)];
        let pending = p.ime & 1 != 0 && p.ie & p.iflags != 0;
        machine(|m| {
            let open = match m.sched.cpu_mut(cpu) {
                Some(c) => {
                    c.irq = pending as u32;
                    pending && c.i == 0
                }
                None => false,
            };
            if open {
                m.sched.force_poll(cpu);
            }
        });
    }

    /// IME (a SWI sets it), then the interrupt line.
    pub(crate) fn set_ime(&mut self, cpu: i32, v: u32) {
        self.irq[ci(cpu)].ime = v;
        self.irq_update(cpu);
    }

    pub(crate) fn stop(&mut self, why: &'static str) {
        if self.stop.is_none() {
            self.stop = Some(why);
        }
    }

    // ---- guest memory, as DMA and the devices reach it ---------------------------------------
    pub(crate) fn rd(&mut self, cpu: i32, a: u32, size: usize) -> u32 {
        match machine(|m| m.read_mem(cpu, a, size)) {
            Some(v) => v,
            None => self.mmio_read(cpu, a, size),
        }
    }

    pub(crate) fn wr(&mut self, cpu: i32, a: u32, v: u32, size: usize) {
        if !machine(|m| m.write_mem(cpu, a, v, size)) {
            self.mmio_write(cpu, a, v, size);
        }
    }
}

/// The machine's `Hardware`: all of it is the thread's `Nds`.
pub struct NdsHw;

impl Hardware for NdsHw {
    fn read(&mut self, cpu: i32, addr: u32, size: u32) -> u32 {
        nds(|n| n.mmio_read(cpu, addr, size as usize))
    }
    fn write(&mut self, cpu: i32, addr: u32, val: u32, size: u32) {
        nds(|n| n.mmio_write(cpu, addr, val, size as usize))
    }
    fn pending_irq(&mut self, cpu: i32) -> bool {
        nds(|n| {
            let p = n.irq[ci(cpu)];
            p.ie & p.iflags != 0
        })
    }
    fn irq(&mut self, cpu: &mut Cpu) {
        crate::bios::irq(cpu)
    }
    fn swi(&mut self, cpu: &mut Cpu, num: u32) {
        crate::bios::swi(cpu, num)
    }
}

// ---- the boot -----------------------------------------------------------------------------------
fn find_module(cpu: i32) -> Option<&'static crate::abi::ArmModule> {
    modules().into_iter().find(|m| {
        if m.cpu != cpu || m.nsegs == 0 || m.overlay >= 0 {
            return false;
        }
        // the block at its load address (the autoloads are not in place yet), or the whole program
        // as loaded when it is stored compressed
        m.segs().iter().enumerate().any(|(k, s)| {
            if k > 0 && s.kind != SEG_LOADED {
                return false;
            }
            let mut buf = vec![0u8; s.size as usize];
            machine(|mm| mm.mem.read_bytes(cpu, s.base, &mut buf)) && crate::machine::crc32(&buf, 0) == s.crc
        })
    })
}

/// The machine as the BIOS leaves it: the ROM read, the programs in memory, the CPUs started at
/// their entries. Fails if the ROM cannot be read or no module of this build matches a program.
pub fn boot(cfg: Config) -> Result<(), String> {
    let (t9, t7) = (cfg.ticks9, cfg.ticks7);
    nds(|n| *n = Nds::new(cfg));
    machine(|m| {
        m.sched.ticks = [t9, t7];
        m.hw = Box::new(NdsHw);
    });
    nds(|n| -> Result<(), String> {
        n.script = input::parse(&n.cfg.input)?;
        n.texpack.dump = n.cfg.dump_textures.clone();
        if let Some(dir) = n.cfg.texture_pack.clone() {
            n.load_pack(&dir)?;
        }
        n.cart_open()?;
        n.mmio_reset();
        n.spi_reset();
        n.video_reset();
        Ok(())
    })?;
    crate::services::cp15_boot();
    let entries = nds(|n| crate::bios::direct_boot_memory(n))?;
    for cpu in [9, 7] {
        let m = find_module(cpu).ok_or_else(|| format!("no module of this build matches the ARM{} program", cpu))?;
        machine(|mm| mm.activate(m));
        note(&format!("ARM{}: module {}, entry {:08X}", cpu, m.name(), entries[ci(cpu)]));
        sched::start(cpu, entries[ci(cpu)]);
        crate::bios::direct_boot_cpu(cpu, entries[ci(cpu)]);
    }
    Ok(())
}

/// The 2D engines draw their lines (a headless run draws only for a picture's frame).
pub fn set_draw(on: bool) {
    nds(|n| n.gpu.on = on);
}

pub use input::KEY_NAMES;

/// Live input for the next frame: the buttons held (bit i of `KEY_NAMES`) and the stylus on the
/// lower screen (x 0-255, y 0-191), if down. A script (`Config::input`) is replaced from here on.
pub fn set_input(keys: u32, pen: Option<(i32, i32)>) {
    nds(|n| {
        n.script.clear();
        n.set_input(keys, pen)
    });
}

/// The picture's scale: the 3D is drawn at 256 x `n` by 192 x `n`, and the screens come out that
/// large (1-8). Takes effect with the next frame drawn.
/// Widens the 3D view by this many DS pixels on each side (0: as the DS draws it): the frustum is
/// opened sideways, so what lies beyond the DS's own picture shows, and `screens_wide` gives the
/// upper screen that much wider while the 3D is on it. The game decides what it draws: what it cuts
/// away before the 3D engine does not come back.
pub fn set_wide(each_side: usize) {
    nds(|d| {
        d.wide = each_side.min(WIDE_MAX);
        d.gx.set_wide(d.wide);
    });
}

/// The most the view is widened by, each side (a 2.4:1 window from 4:3).
pub const WIDE_MAX: usize = 170;

pub fn set_scale(n: usize) {
    nds(|d| d.scale = n.clamp(1, 8));
}

/// Our own additions to the 3D picture (`render3d`), each off at 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    /// 0-1: an outline round the objects.
    pub outline: f32,
    /// The outline's width in DS pixels (0.25-2).
    pub outline_width: f32,
    /// 0-1: soft shading where the scene is closed in (ambient occlusion from the depth).
    pub shadow: f32,
    /// How far it reaches, in DS pixels (0.5-6).
    pub shadow_radius: f32,
    /// How textures are read: the nearest texel (as the DS does) or four blended.
    pub filter: TexFilter,
    /// Anti-aliasing of polygon edges: 0 off, 2 (2x2 samples per pixel), 4 (4x4); supersampling.
    pub msaa: u8,
}

/// Texture filtering of the 3D.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TexFilter {
    #[default]
    Nearest,
    Bilinear,
}

/// The comparison of a renderer's picture of a dump with the one stored in it (этап 0 of
/// `docs/GPU-BACKEND.md`).
#[derive(Clone, Debug)]
pub struct ParityReport {
    /// The dump's frame and file.
    pub vblank: u64,
    pub path: String,
    /// Words of the 3D picture.
    pub pixels: usize,
    /// Bit-identical words.
    pub exact: usize,
    /// Mean |difference| per colour channel, in 1/63 steps.
    pub mean_abs: [f32; 3],
    /// Pixels with any channel off by more than 2 steps.
    pub over2: usize,
    /// Pixels with the outline or the shade bits different.
    pub fx_off: usize,
    /// The renderer's notes word matches the stored one.
    pub notes_equal: bool,
    /// The renderer produced as many words as stored.
    pub same_size: bool,
}

/// Renders a dump's job and compares against the picture the dump stores. The software renderer
/// re-renders its own dumps byte-exact; the GPU backend is scored by the same function
/// (`docs/GPU-BACKEND.md`, этап 0).
pub fn parity_check(path: &std::path::Path) -> Result<ParityReport, String> {
    let d = jobdump::read(path)?;
    let want = d.job.width * d.scale * 192 * d.scale;
    if d.px.len() != want {
        return Err(format!("{}: picture {} words, expected {}", path.display(), d.px.len(), want));
    }
    let (px, notes) = render3d::render(&d.job, d.scale);
    let mut rep = ParityReport {
        vblank: d.vblank,
        path: path.display().to_string(),
        pixels: d.px.len(),
        exact: 0,
        mean_abs: [0.0; 3],
        over2: 0,
        fx_off: 0,
        notes_equal: notes == d.notes,
        same_size: px.len() == d.px.len(),
    };
    if !rep.same_size {
        return Ok(rep);
    }
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for (a, b) in px.iter().zip(d.px.iter()) {
        if a == b {
            rep.exact += 1;
        }
        let dr = (a & 63).abs_diff(b & 63);
        let dg = (a >> 6 & 63).abs_diff(b >> 6 & 63);
        let db = (a >> 12 & 63).abs_diff(b >> 12 & 63);
        let da = (a >> 18 & 31).abs_diff(b >> 18 & 31);
        sum[0] += dr as u64;
        sum[1] += dg as u64;
        sum[2] += db as u64;
        n += 1;
        if dr.max(dg).max(db).max(da) > 2 {
            rep.over2 += 1;
        }
        if a >> 24 & 15 != b >> 24 & 15 || a >> 28 & 15 != b >> 28 & 15 {
            rep.fx_off += 1;
        }
    }
    if n > 0 {
        rep.mean_abs = sum.map(|s| s as f32 / n as f32);
    }
    Ok(rep)
}

/// The GPU backend's parity with the software renderer on a dump: the job is filtered to what the
/// GPU draws so far (`render3d_gpu::stage3_job`, этап 3 of `docs/GPU-BACKEND.md`), rendered by both
/// engines, compared. `diff` is the difference map (red where the words differ, black where they
/// match), `edge_diffs` the differing pixels next to a colour change (a polygon's edge or its
/// silhouette) — the acceptance wants that empty.
#[cfg(feature = "gpu")]
#[derive(Clone, Debug)]
pub struct GpuParity {
    /// GPU words against the software engine's on the same filtered job.
    pub report: ParityReport,
    /// The difference map, RGBA: red where the words differ, black where they match.
    pub diff: Vec<u8>,
    /// The picture's width and height (the map's).
    pub w: usize,
    pub h: usize,
    /// Differing pixels beside a colour change in the reference.
    pub edge_diffs: usize,
}

#[cfg(feature = "gpu")]
pub fn gpu_parity_check(path: &std::path::Path) -> Result<GpuParity, String> {
    let d = jobdump::read(path)?;
    let job = render3d_gpu::stage3_job(&d.job);
    let (w, h) = (job.width * d.scale, 192 * d.scale);
    let (ref_px, ref_notes) = render3d::render(&job, d.scale);
    let (px, notes) = render3d_gpu::render(&job, d.scale)?;
    let mut rep = ParityReport {
        vblank: d.vblank,
        path: path.display().to_string(),
        pixels: ref_px.len(),
        exact: 0,
        mean_abs: [0.0; 3],
        over2: 0,
        fx_off: 0,
        notes_equal: notes == ref_notes,
        same_size: px.len() == ref_px.len(),
    };
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    let mut diff = vec![0u8; ref_px.len() * 4];
    let mut edge_diffs = 0;
    let mut shown = 0;
    for (i, (a, b)) in px.iter().zip(ref_px.iter()).enumerate() {
        if a == b {
            rep.exact += 1;
            continue;
        }
        if shown < 8 {
            eprintln!("  {}: ({},{}) gpu {:08x} soft {:08x}", rep.path, i % w, i / w, a, b);
            shown += 1;
        }
        diff[i * 4] = 255;
        diff[i * 4 + 3] = 255;
        let dr = (a & 63).abs_diff(b & 63);
        let dg = (a >> 6 & 63).abs_diff(b >> 6 & 63);
        let db = (a >> 12 & 63).abs_diff(b >> 12 & 63);
        let da = (a >> 18 & 31).abs_diff(b >> 18 & 31);
        sum[0] += dr as u64;
        sum[1] += dg as u64;
        sum[2] += db as u64;
        n += 1;
        if dr.max(dg).max(db).max(da) > 2 {
            rep.over2 += 1;
        }
        if a >> 24 & 15 != b >> 24 & 15 || a >> 28 & 15 != b >> 28 & 15 {
            rep.fx_off += 1;
        }
        // a colour change across a 4-neighbour: a polygon edge or a silhouette against the 2D
        let (x, y) = (i % w, i / w);
        if (x + 1 < w && ref_px[i + 1] != *b) || (x > 0 && ref_px[i - 1] != *b) || (y + 1 < h && ref_px[i + w] != *b) || (y > 0 && ref_px[i - w] != *b) {
            edge_diffs += 1;
        }
    }
    if n > 0 {
        rep.mean_abs = sum.map(|s| s as f32 / n as f32);
    }
    Ok(GpuParity { report: rep, diff, w, h, edge_diffs })
}

impl TexFilter {
    pub fn parse(s: &str) -> TexFilter {
        match s {
            "bilinear" | "linear" => TexFilter::Bilinear,
            _ => TexFilter::Nearest,
        }
    }
}

impl Default for Extras {
    fn default() -> Extras {
        Extras { outline: 0.0, outline_width: 0.5, shadow: 0.0, shadow_radius: 2.0, filter: TexFilter::Nearest, msaa: 0 }
    }
}

/// Swaps the texture pack (a data file or a folder; `None`: the game's own pictures) between two frames. The
/// pictures of the new pack are read as the game draws them; an error keeps the pack that was.
pub fn set_texture_pack(path: Option<&std::path::Path>) -> Result<(), String> {
    nds(|d| match path {
        Some(p) => d.load_pack(p),
        None => {
            d.texpack.pack = std::sync::Arc::new(texpack::Pack::empty());
            note("texture pack: none");
            Ok(())
        }
    })
}

/// Sets the 3D's extras (out-of-range and non-numbers are brought in). Takes effect with the next
/// frame drawn.
pub fn set_extras(e: Extras) {
    let c = |v: f32, lo: f32, hi: f32, or: f32| if v.is_finite() { v.clamp(lo, hi) } else { or };
    nds(|d| {
        d.extras = Extras {
            outline: c(e.outline, 0.0, 1.0, 0.0),
            outline_width: c(e.outline_width, 0.25, 2.0, 0.5),
            shadow: c(e.shadow, 0.0, 1.0, 0.0),
            shadow_radius: c(e.shadow_radius, 0.5, 6.0, 2.0),
            filter: e.filter,
            msaa: match e.msaa { 0..=1 => 0, 2..=3 => 2, _ => 4 },
        }
    });
}

/// Both screens as RGBA at the current scale, the upper first: 2 x (256 x n) x (192 x n) x 4 bytes.
/// Returns the scale too.
pub fn screens_rgba() -> (usize, Vec<u8>) {
    let (scale, rgb) = nds(|n| (n.scale, n.screens_rgb(n.scale)));
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for p in rgb.chunks_exact(3) {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    (scale, out)
}

/// The upper screen widened (`set_wide`) as RGBA: `(scale, each side, picture)`, (256 + 2 * each
/// side) x scale wide, 192 x scale high; `None` unless the 3D is on the upper screen.
pub fn upper_wide_rgba() -> Option<(usize, usize, Vec<u8>)> {
    let (scale, e, rgb) = nds(|n| n.upper_wide_rgb(n.scale).map(|(e, rgb)| (n.scale, e, rgb)))?;
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for p in rgb.chunks_exact(3) {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    Some((scale, e, out))
}

/// Keep the sound the unit mixes (someone listens): `take_samples` gives it.
pub fn set_sound_keep(on: bool) {
    nds(|n| n.snd.keep = on);
}

/// The stereo samples (16-bit, interleaved, 32 728.5 Hz) mixed since the last call.
pub fn take_samples() -> Vec<i16> {
    nds(|n| std::mem::take(&mut n.snd.out))
}

/// Both screens as a PNG, 256x384 times the scale, the upper first.
pub fn screenshot(path: &std::path::Path) -> Result<(), String> {
    let (scale, rgb) = nds(|n| (n.scale, n.screens_rgb(n.scale)));
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), 256 * scale as u32, 384 * scale as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgb).map_err(|e| e.to_string())
}

/// The widened upper screen as PNGs of its parts: `<stem>_wide.png` (all), `<stem>_2d.png` and `<stem>_3d.png`;
/// false if there is no widened picture.
pub fn screenshot_wide_layers(stem: &std::path::Path) -> Result<bool, String> {
    for (suffix, layers) in [("wide", 3u8), ("2d", 1), ("3d", 2)] {
        let Some((scale, e, rgb)) = nds(|n| n.upper_wide_rgb_layers(n.scale, layers).map(|(e, rgb)| (n.scale, e, rgb))) else { return Ok(false) };
        let path = std::path::PathBuf::from(format!("{}_{}.png", stem.display(), suffix));
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let f = std::fs::File::create(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), (256 + 2 * e as u32) * scale as u32, 192 * scale as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&rgb).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// The widened upper screen (`upper_wide_rgba`) as a PNG; false if there is none to write.
pub fn screenshot_wide(path: &std::path::Path) -> Result<bool, String> {
    let Some((scale, e, rgb)) = nds(|n| n.upper_wide_rgb(n.scale).map(|(e, rgb)| (n.scale, e, rgb))) else { return Ok(false) };
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), (256 + 2 * e as u32) * scale as u32, 192 * scale as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgb).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Writes both engines' background layers as PNGs into `dir` (see `gpu2d::dump_backgrounds`);
/// returns how many.
pub fn dump_backgrounds(dir: &std::path::Path) -> usize {
    nds(|n| n.dump_backgrounds(dir))
}

/// Runs to the next VBlank; false once stopped.
/// How a frame's time goes (a diagnostic for the app's status line): scheduler quanta run, then
/// nanoseconds in the guest CPUs, in the hardware (events: lines, timers, DMA), in the sound.
pub static QUANTA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static NS_GUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static NS_HW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static NS_SOUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn timing() -> (u64, u64, u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (QUANTA.load(Relaxed), NS_GUEST.load(Relaxed), NS_HW.load(Relaxed), NS_SOUND.load(Relaxed))
}

pub fn run_frame() -> bool {
    let vbl = nds(|n| n.vblanks);
    while nds(|n| n.stop.is_none() && n.vblanks == vbl) {
        let now = machine(|m| m.sched.now);
        let next = nds(|n| n.next_event());
        let ticks = next.saturating_sub(now).min(QUANTUM);
        use std::sync::atomic::Ordering::Relaxed;
        let t0 = std::time::Instant::now();
        sched::run_quantum(ticks);
        let t1 = std::time::Instant::now();
        QUANTA.fetch_add(1, Relaxed);
        NS_GUEST.fetch_add((t1 - t0).as_nanos() as u64, Relaxed);
        let now = machine(|m| m.sched.now);
        nds(|n| {
            n.fire_events(now);
            let t2 = std::time::Instant::now();
            NS_HW.fetch_add((t2 - t1).as_nanos() as u64, Relaxed);
            n.sound_sync(now); // the sound in step with the memory the CPUs write
            NS_SOUND.fetch_add(t2.elapsed().as_nanos() as u64, Relaxed);
            let vbl = n.vblanks;
            if vbl != n.input_at {
                n.input_at = vbl;
                n.input_vblank(vbl);
            }
            if n.cfg.stop_vblanks != 0 && n.vblanks >= n.cfg.stop_vblanks {
                n.stop("VBlank limit");
            }
        });
    }
    nds(|n| n.vblanks != vbl)
}

/// Why the run stopped, if it has.
pub fn stopped() -> Option<&'static str> {
    nds(|n| n.stop)
}

pub fn vblanks() -> u64 {
    nds(|n| n.vblanks)
}

/// Whether the game's script runner is idle (the scene's script has ended and the player has the control),
/// by the mark the game keeps for it (`0206577C` is -2 then). Only the European dump (`YHNP`) is known; `None`
/// for any other game, or before the game has set the mark up.
/// Whether a scene's script is loaded and the runner is at one of its commands: the command the runner holds
/// (`02075988`) is one of the scene's table of commands (`02074BEC`, `020740A0` of them). The title, the logos and
/// the menus have none, whatever their memory holds from an earlier scene.
pub fn script_loaded() -> Option<bool> {
    nds(|n| n.cart.rom.get(0xC..0x10) == Some(b"YHNP".as_slice())).then(|| {
        crate::machine::machine(|m| {
            let word = |m: &mut crate::machine::Machine, a: u32| {
                let mut w = [0u8; 4];
                m.mem.read_bytes(9, a, &mut w).then(|| u32::from_le_bytes(w))
            };
            let (cmd, count) = (word(m, 0x0207_5988)?, word(m, 0x0207_40A0)?);
            Some(count > 0 && count < 4096 && (0..count).any(|i| word(m, 0x0207_4BEC + 4 * i) == Some(cmd)))
        })
    })?
}

/// The game's step of its script (ARM9 `02045BC5`), called once a frame while a script runs.
const SCRIPT_STEP: u32 = 0x0204_5BC5;

thread_local! {
    static LAST_STEP: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Starts noting the script's steps (`script_stepping`); costs a call per function entry of the guest.
pub fn track_script() {
    unsafe { crate::services::g_arm_trace = 1 };
    #[cfg(feature = "debug-tools")]
    debug::log_card_reads();
}

/// `ARM_ENTER`'s service: notes the vblank of the script's step.
pub(crate) fn note_enter(c: &crate::abi::Cpu, key: u32) {
    if key == SCRIPT_STEP && c.cpu == 9 {
        let v = vblanks();
        LAST_STEP.with(|l| l.set(Some(v)));
    }
}

/// Whether the game's script took a step in the last frames (needs `track_script`): outside a scene (the
/// title, the logos, the menus) nothing steps, whatever the memory holds from an earlier scene.
pub fn script_stepping() -> bool {
    LAST_STEP.with(|l| l.get()).is_some_and(|v| vblanks().saturating_sub(v) <= 2)
}

#[cfg(feature = "debug-tools")]
thread_local! {
    static SCENE: std::cell::RefCell<(Option<crate::nitrofs::NitroFs>, Option<String>)> = const { std::cell::RefCell::new((None, None)) };
}

/// The scene whose script (`Chapter/Pack/<scene>.txt.z`) the game read last, by the cartridge's reads (needs
/// `debug::log_card_reads`); call it every frame or so. `C19_01` is the title's demo behind "Press START Button".
#[cfg(feature = "debug-tools")]
pub fn script_scene() -> Option<String> {
    let reads = debug::take_card_reads();
    SCENE.with(|s| {
        let mut s = s.borrow_mut();
        if s.0.is_none() {
            s.0 = nds(|n| crate::nitrofs::NitroFs::parse(&n.cart.rom).ok());
        }
        let found = s.0.as_ref().and_then(|fs| {
            reads.iter().rev().find_map(|(a, _)| {
                let p = fs.at(*a)?;
                p.strip_prefix("Chapter/Pack/")?.rsplit('/').next()?.strip_suffix(".txt.z").map(str::to_string)
            })
        });
        if found.is_some() {
            s.1 = found;
        }
        s.1.clone()
    })
}

/// Without `debug-tools` the cartridge's reads are not logged: the scene is not known.
#[cfg(not(feature = "debug-tools"))]
pub fn script_scene() -> Option<String> {
    None
}

pub fn script_idle() -> Option<bool> {
    // a running script holds -1 or the count of a wait (a few hundred at most); -2 is idle; anything else is
    // what an old scene left behind (the title after a reset shows `000047C4`): no script runs
    let loaded = script_loaded()?;
    script_mark().map(|w| !loaded || !script_stepping() || script_scene().as_deref() == Some("C19_01") || !(w == u32::MAX || w <= 0x1000))
}

/// The game's mark of a running script itself (`0xFFFFFFFE` when idle), for the European dump.
pub fn script_mark() -> Option<u32> {
    nds(|n| n.cart.rom.get(0xC..0x10) == Some(b"YHNP".as_slice())).then(|| {
        let mut w = [0u8; 4];
        crate::machine::machine(|m| m.mem.read_bytes(9, 0x0206_577C, &mut w)).then(|| u32::from_le_bytes(w))
    })?
}

/// Whether the game's script is at a `WAIT`, a command that waits for the player's A (a line of speech is
/// followed by one), by the command the runner holds (`0207596C + 0x1C`); `None` as `script_idle`.
pub fn script_waits() -> Option<bool> {
    nds(|n| n.cart.rom.get(0xC..0x10) == Some(b"YHNP".as_slice())).then(|| {
        crate::machine::machine(|m| {
            let mut w = [0u8; 4];
            m.mem.read_bytes(9, 0x0207_5988, &mut w).then(|| u32::from_le_bytes(w)).and_then(|cmd| {
                m.mem.read_bytes(9, cmd, &mut w).then(|| u32::from_le_bytes(w) == OP_WAIT)
            })
        })
    })?
}

/// Where the game's script is: the command the runner holds and the mark (`script_mark`: the count of a
/// `WAITFRAME` running down). Changes while the script goes on; `None` as `script_idle`.
pub fn script_step() -> Option<(u32, u32)> {
    let cmd = nds(|n| n.cart.rom.get(0xC..0x10) == Some(b"YHNP".as_slice())).then(|| {
        let mut w = [0u8; 4];
        crate::machine::machine(|m| m.mem.read_bytes(9, 0x0207_5988, &mut w)).then(|| u32::from_le_bytes(w))
    })??;
    Some((cmd, script_mark()?))
}

/// The command `<WAIT>` compiles to.
const OP_WAIT: u32 = 9;

/// The report of a run's end: the time, each CPU's state, the interrupts taken.
pub fn report() {
    let (vbl, t) = (vblanks(), now());
    eprintln!("  {} VBlanks, {:.3} s of DS time", vbl, t as f64 / 67_027_964.0);
    for cpu in [9, 7] {
        let (ime, ie, iflags, counts) = nds(|n| {
            let p = n.irq[ci(cpu)];
            (p.ime, p.ie, p.iflags, n.irq_count[ci(cpu)])
        });
        let (mode, sp, lr, regs, halted, thread, threads) = machine(|m| {
            let (mode, sp, lr, regs) = match m.sched.cpu_mut(cpu) {
                Some(c) => (c.mode, c.r[13], c.r[14], c.r),
                None => (0, 0, 0, [0; 16]),
            };
            (mode, sp, lr, regs, m.sched.halted[ci(cpu)], m.sched.current(cpu), m.sched.thread_count(cpu))
        });
        eprintln!(
            "  ARM{} mode {:02X}{}, thread {} of {}, sp {:08X} lr {:08X}, IME {} IE {:08X} IF {:08X}",
            cpu,
            mode,
            if halted { " halted" } else { "" },
            thread,
            threads,
            sp,
            lr,
            ime,
            ie,
            iflags
        );
        eprintln!(
            "    r0-r12: {}",
            regs[..13].iter().map(|r| format!("{:08X}", r)).collect::<Vec<_>>().join(" ")
        );
        let irqs: Vec<String> =
            counts.iter().enumerate().filter(|(_, c)| **c != 0).map(|(b, c)| format!("{}:{}", b, c)).collect();
        eprintln!("    interrupts: {}", irqs.join(" "));
    }
}

/// Writes the backup memory if it changed (the file is written whole, then moved into place).
pub fn finish() {
    nds(|n| n.cart_save());
}

/// The same, for now and then while the game runs: a game that is closed or killed keeps what it
/// had saved.
pub fn flush_save() {
    nds(|n| n.cart_save_quiet());
}

pub(crate) fn fatal(msg: String) -> ! {
    fatal_msg(&msg)
}

#[allow(dead_code)]
const _: u32 = SEG_PROGRAM;
