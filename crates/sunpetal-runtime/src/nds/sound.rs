//! The ARM7's sound unit: 16 channels, 2 capture units, the mixer.
//!
//! The unit makes a stereo sample every 1 024 system cycles (2 048 ticks, 32 728.5 Hz). It runs in
//! virtual time (`sound_sync`): up to the machine's time after each quantum (a quarter of a line,
//! half a sample), and up to the ARM7's at each access to its registers. So a run is the same run
//! every time, with or without anyone listening, a channel's busy bit ends when its samples do, and
//! a channel reads memory as it stands at its own time.
//!
//! Each output sample, every channel's timer (the system clock / 2, counting up from the channel's
//! reload) advances 512, and each overflow steps the channel: PCM8 and PCM16 read the next sample,
//! IMA-ADPCM the next nibble (the first word is its header: the value and the step index; the state
//! after the loop's first nibble is kept for a loop to go back to), PSG (8-13) the next eighth of
//! a square wave of the duty given, noise (14-15) the next bit of a 15-bit LFSR. A sampled channel
//! starts 3 steps late (ADPCM spends the first 8 nibbles' time on its header); with less than 16
//! bytes in all it plays nothing. A one-shot channel ends after its last sample: silent, the busy
//! bit cleared. The samples come through a FIFO as the hardware's do: 32 bytes fetched as the
//! channel starts, 16 more each time it is down to 16. A stream refilled behind the play position
//! counts on that read-ahead.
//!
//! The mix, in integers at the hardware's widths (GBATEK; melonDS's arithmetic): a channel's sample
//! times 16 >> the divider's shift (0, 1, 2, 4) times its volume (0-127, 127 as 128); panned by
//! (128 - pan) and pan (127 as 128), >> 10; channels 1 and 3 left out of the mixer by SOUNDCNT bits
//! 12-13; capture 0 takes the left mixer (or channel 0), capture 1 the right (or channel 2), at the
//! timer of channel 1 or 3, as PCM8 or PCM16 into the ARM7's memory; each output is the mixer,
//! channel 1, 3 or both (SOUNDCNT bits 8-11); times the master volume (127 as 128) >> 7, >> 8, plus
//! the bias << 6, centred on 0x200's (the bias every SDK sets), clipped to 16 bits. Silent unless
//! POWCNT2 bit 0 (the speakers) and SOUNDCNT bit 15. The DS truncates the output to 10 bits: kept
//! at 16 here.
use super::Nds;
use crate::machine::machine;

/// An output sample: 1 024 system cycles (32 728.5 Hz).
pub(super) const SAMPLE_TICKS: u64 = 2048;

#[derive(Clone, Copy, Default)]
struct Channel {
    cnt: u32,
    src: u32,
    /// Loop and length in bytes.
    loop_: u32,
    len: u32,
    reload: u16,
    /// Started: takes effect at the next step.
    key_on: bool,
    timer: u32,
    /// Sample (ADPCM: nibble) index; < 0 before the first.
    pos: i32,
    /// The sample being output.
    cur: i32,
    lfsr: u16,
    adpcm: i32,
    index: i32,
    adpcm_loop: i32,
    index_loop: i32,
    /// ADPCM: the byte whose nibbles play.
    byte: u8,
    /// The samples fetched ahead.
    fifo: [u8; 32],
    fifo_rd: u32,
    fifo_wr: u32,
    fifo_level: u32,
    /// The next fetch's offset from the source.
    fetch: u32,
}

#[derive(Clone, Copy, Default)]
struct Capture {
    cnt: u8,
    dst: u32,
    len: u32,
    reload: u16,
    timer: u32,
    pos: u32,
}

pub(super) struct Sound {
    regs: Vec<u8>,
    ch: [Channel; 16],
    cap: [Capture; 2],
    /// The time of the next output sample.
    next: u64,
    pub(super) out: Vec<i16>,
    /// Keep the output (someone listens): stereo, interleaved.
    pub(super) keep: bool,
}

impl Sound {
    pub(super) fn new() -> Sound {
        let mut regs = vec![0u8; 0x120];
        regs[0x105] = 0x02; // SOUNDBIAS 0x200, as the boot leaves it
        Sound { regs, ch: [Channel::default(); 16], cap: [Capture::default(); 2], next: 0, out: Vec::new(), keep: false }
    }

    fn reg32(&self, o: usize) -> u32 {
        u32::from_le_bytes([self.regs[o], self.regs[o + 1], self.regs[o + 2], self.regs[o + 3]])
    }
    fn reg16(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.regs[o], self.regs[o + 1]])
    }
}

const ADPCM_STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107,
    118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
const ADPCM_INDEX: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

fn fmt(c: &Channel) -> u32 {
    c.cnt >> 29 & 3
}

/// The FIFO: a burst of 16 bytes from where the fetches stand (back to the loop start past the
/// end; nothing past a one-shot's end).
fn fetch(c: &mut Channel) {
    let end = c.loop_ + c.len;
    if c.fetch >= end {
        if c.cnt & 1 << 27 != 0 {
            c.fetch = c.loop_;
        } else {
            return;
        }
    }
    let n = 16.min(end - c.fetch);
    let mut i = 0;
    while i < n {
        // memory only (VRAM: an inference)
        let w = machine(|m| m.read_mem(7, c.src.wrapping_add(c.fetch), 4)).unwrap_or(0);
        c.fifo[c.fifo_wr as usize..c.fifo_wr as usize + 4].copy_from_slice(&w.to_le_bytes());
        c.fifo_wr = (c.fifo_wr + 4) & 31;
        c.fetch += 4;
        i += 4;
    }
    c.fifo_level += n;
}

/// The next sample's bytes from the FIFO, which fetches again once half empty.
fn take(c: &mut Channel, size: usize) -> u32 {
    let mut v = 0u32;
    for i in 0..size {
        v |= (c.fifo[((c.fifo_rd as usize) + i) & 31] as u32) << (8 * i);
    }
    c.fifo_rd = (c.fifo_rd + size as u32) & 31;
    c.fifo_level = c.fifo_level.wrapping_sub(size as u32);
    if c.fifo_level <= 16 {
        fetch(c);
    }
    v
}

fn start(c: &mut Channel) {
    c.timer = c.reload as u32;
    c.pos = if fmt(c) == 3 { -1 } else { -3 };
    c.cur = 0;
    c.lfsr = 0x7FFF;
    c.fifo_rd = 0;
    c.fifo_wr = 0;
    c.fifo_level = 0;
    c.fetch = 0;
    if fmt(c) != 3 {
        fetch(c); // 32 bytes as the channel starts
        fetch(c);
    }
}

/// One step of a channel: its next sample. True if a one-shot ended.
fn step(c: &mut Channel, n: usize) -> bool {
    let end = c.loop_ + c.len;
    match fmt(c) {
        0 => {
            c.pos += 1;
            if c.pos < 0 {
                return false;
            }
            if c.pos as u32 >= end {
                if c.cnt & 1 << 27 != 0 {
                    c.pos = c.loop_ as i32;
                } else {
                    return true;
                }
            }
            c.cur = take(c, 1) as u8 as i8 as i32 * 256;
            false
        }
        1 => {
            c.pos += 1;
            if c.pos < 0 {
                return false;
            }
            if c.pos as u32 * 2 >= end {
                if c.cnt & 1 << 27 != 0 {
                    c.pos = (c.loop_ / 2) as i32;
                } else {
                    return true;
                }
            }
            c.cur = take(c, 2) as u16 as i16 as i32;
            false
        }
        2 => {
            c.pos += 1;
            if c.pos < 8 {
                if c.pos == 0 {
                    let h = take(c, 4);
                    c.adpcm = h as u16 as i16 as i32;
                    c.adpcm_loop = c.adpcm;
                    c.index = ((h >> 16 & 0x7F) as i32).min(88);
                    c.index_loop = c.index;
                }
                return false;
            }
            if c.pos as u32 / 2 >= end {
                // back to the state kept after the loop's first nibble
                if c.cnt & 1 << 27 == 0 {
                    return true;
                }
                c.pos = (c.loop_ * 2) as i32;
                c.adpcm = c.adpcm_loop;
                c.cur = c.adpcm;
                c.index = c.index_loop;
                c.byte = take(c, 1) as u8;
                return false;
            }
            if c.pos & 1 != 0 {
                c.byte >>= 4;
            } else {
                c.byte = take(c, 1) as u8;
            }
            let nib = (c.byte & 15) as usize;
            let s = ADPCM_STEP[c.index as usize];
            let mut d = s >> 3;
            if nib & 1 != 0 {
                d += s >> 2;
            }
            if nib & 2 != 0 {
                d += s >> 1;
            }
            if nib & 4 != 0 {
                d += s;
            }
            c.adpcm = if nib & 8 != 0 { (c.adpcm - d).max(-0x7FFF) } else { (c.adpcm + d).min(0x7FFF) };
            c.index = (c.index + ADPCM_INDEX[nib & 7]).clamp(0, 88);
            if c.pos as u32 == c.loop_ * 2 {
                c.adpcm_loop = c.adpcm;
                c.index_loop = c.index;
            }
            c.cur = c.adpcm;
            false
        }
        _ => {
            if n >= 14 {
                // noise
                if c.lfsr & 1 != 0 {
                    c.lfsr = (c.lfsr >> 1) ^ 0x6000;
                    c.cur = -0x7FFF;
                } else {
                    c.lfsr >>= 1;
                    c.cur = 0x7FFF;
                }
            } else if n >= 8 {
                // PSG: duty (d + 1) / 8 high, 7 is always low
                c.pos += 1;
                let duty = (c.cnt >> 24 & 7) as i32;
                c.cur = if duty < 7 && (c.pos & 7) >= 7 - duty { 0x7FFF } else { -0x7FFF };
            }
            false
        }
    }
}

/// A channel through one output sample: its value after volume, before panning, and whether a
/// one-shot ended.
fn run(c: &mut Channel, n: usize) -> (i32, bool) {
    if c.cnt & 0x8000_0000 == 0 {
        return (0, false);
    }
    if fmt(c) != 3 && c.loop_ + c.len < 16 {
        return (0, false);
    }
    if fmt(c) == 3 && n < 8 {
        return (0, false);
    }
    if c.key_on {
        start(c);
        c.key_on = false;
    }
    c.timer += 512;
    let mut ended = false;
    while c.timer >> 16 != 0 {
        c.timer = c.reload as u32 + (c.timer - 0x10000);
        if step(c, n) {
            c.cur = 0;
            c.cnt &= 0x7FFF_FFFF;
            ended = true;
            break;
        }
        if c.cnt & 0x8000_0000 == 0 {
            break;
        }
    }
    const SHIFT: [u32; 4] = [4, 3, 2, 0];
    let vol = (c.cnt & 0x7F) as i32;
    ((c.cur << SHIFT[(c.cnt >> 8 & 3) as usize]) * if vol == 127 { 128 } else { vol }, ended)
}

fn pan(c: &Channel) -> i64 {
    let p = (c.cnt >> 16 & 0x7F) as i64;
    if p == 127 { 128 } else { p }
}

impl Nds {
    fn sound_stop(&mut self, n: usize) {
        let s = &mut self.snd;
        s.ch[n].cur = 0;
        s.ch[n].cnt &= 0x7FFF_FFFF;
        s.regs[0x10 * n + 3] &= 0x7F;
        let v = s.reg16(0x10 * n + 2);
        self.io_set16(7, 0x0400_0402 + 0x10 * n as u32, v);
    }

    /// A capture unit through one output sample.
    fn sound_capture(&mut self, k: usize, v: i32) {
        self.snd.cap[k].timer += 512;
        while self.snd.cap[k].timer >> 16 != 0 {
            let c = &mut self.snd.cap[k];
            c.timer = c.reload as u32 + (c.timer - 0x10000);
            let a = c.dst.wrapping_add(c.pos);
            if c.cnt & 8 != 0 {
                c.pos += 1;
                self.wr(7, a, (v >> 8) as u32 & 0xFF, 1);
            } else {
                c.pos += 2;
                self.wr(7, a & !1, v as u32 & 0xFFFF, 2);
            }
            let c = &mut self.snd.cap[k];
            if c.pos >= c.len {
                if c.cnt & 4 != 0 {
                    c.cnt &= 0x7F;
                    let cnt = c.cnt;
                    self.snd.regs[0x108 + k] = cnt;
                    let v = self.snd.reg16(0x108);
                    self.io_set16(7, 0x0400_0508, v);
                    return;
                }
                c.pos = 0;
            }
        }
    }

    fn sound_mix_one(&mut self) {
        let sc = self.snd.reg16(0x100) as u32;
        let (mut left, mut right, mut out_l, mut out_r) = (0i32, 0i32, 0i32, 0i32);
        if sc & 0x8000 != 0 {
            let mut v = [0i32; 16];
            for n in 0..16 {
                let mut c = self.snd.ch[n];
                let (x, ended) = run(&mut c, n);
                self.snd.ch[n] = c;
                if ended {
                    self.sound_stop(n);
                }
                v[n] = x;
                if x == 0 {
                    continue;
                }
                if (n == 1 && sc & 0x1000 != 0) || (n == 3 && sc & 0x2000 != 0) {
                    continue;
                }
                let p = pan(&self.snd.ch[n]);
                left += ((x as i64 * (128 - p)) >> 10) as i32;
                right += ((x as i64 * p) >> 10) as i32;
            }
            for k in 0..2 {
                if self.snd.cap[k].cnt & 0x80 == 0 {
                    continue;
                }
                let s = if self.snd.cap[k].cnt & 2 != 0 { v[2 * k] } else if k == 1 { right } else { left };
                self.sound_capture(k, (s >> 8).clamp(-0x8000, 0x7FFF));
            }
            let one = |snd: &Sound, n: usize, r: bool| -> i32 {
                let p = pan(&snd.ch[n]);
                ((v[n] as i64 * if r { p } else { 128 - p }) >> 10) as i32
            };
            out_l = match sc >> 8 & 3 {
                0 => left,
                1 => one(&self.snd, 1, false),
                2 => one(&self.snd, 3, false),
                _ => one(&self.snd, 1, false) + one(&self.snd, 3, false),
            };
            out_r = match sc >> 10 & 3 {
                0 => right,
                1 => one(&self.snd, 1, true),
                2 => one(&self.snd, 3, true),
                _ => one(&self.snd, 1, true) + one(&self.snd, 3, true),
            };
        }
        if !self.snd.keep {
            return;
        }
        let mut mv = (sc & 0x7F) as i64;
        if mv == 127 {
            mv = 128;
        }
        let bias = (((self.snd.reg16(0x104) & 0x3FF) as i32) - 0x200) << 6;
        let on = sc & 0x8000 != 0 && self.io_reg16(7, 0x0400_0304) & 1 != 0;
        for o in [out_l, out_r] {
            let s = ((o as i64 * mv) >> 7 >> 8) as i32 + bias;
            self.snd.out.push(if on { s.clamp(-0x8000, 0x7FFF) as i16 } else { 0 });
        }
    }

    /// Mixes up to time `t`.
    pub(super) fn sound_sync(&mut self, t: u64) {
        while self.snd.next <= t {
            self.sound_mix_one();
            self.snd.next += SAMPLE_TICKS;
        }
    }

    fn sound_parse(&mut self, n: usize) {
        let b = 0x10 * n;
        let s = &mut self.snd;
        let cnt = s.reg32(b);
        let started = cnt & 0x8000_0000 != 0 && s.ch[n].cnt & 0x8000_0000 == 0;
        let (src, reload, loop_, len) = (s.reg32(b + 4) & 0x07FF_FFFC, s.reg16(b + 8), 4 * s.reg16(b + 0xA) as u32, 4 * (s.reg32(b + 0xC) & 0x3F_FFFF));
        let c = &mut s.ch[n];
        if started {
            c.key_on = true;
        }
        c.cnt = cnt;
        c.src = src;
        c.reload = reload;
        c.loop_ = loop_;
        c.len = len;
        if started {
            const FMT: [&str; 4] = ["PCM8", "PCM16", "IMA-ADPCM", "PSG/noise"];
            const REP: [&str; 4] = ["manual", "looped", "one-shot", "repeat mode 3"];
            self.once("sound-fmt", (cnt >> 29 & 3) * 4 + (cnt >> 27 & 3), || {
                format!("sound: channel {} starts: {}, {}", n, FMT[(cnt >> 29 & 3) as usize], REP[(cnt >> 27 & 3) as usize])
            });
        }
    }

    fn sound_parse_capture(&mut self, k: usize) {
        let cnt = self.snd.regs[0x108 + k];
        let began = cnt & 0x80 != 0 && self.snd.cap[k].cnt & 0x80 == 0;
        let s = &mut self.snd;
        let reload = s.ch[1 + 2 * k].reload;
        let (dst, len) = (s.reg32(0x110 + 8 * k) & 0x07FF_FFFC, (4 * s.reg16(0x114 + 8 * k) as u32).max(4));
        let p = &mut s.cap[k];
        if began {
            p.timer = p.reload as u32;
            p.pos = 0;
        }
        p.cnt = cnt & 0x8F;
        if p.cnt & 0x80 == 0 {
            p.cnt &= !1;
        }
        let pcnt = p.cnt;
        p.dst = dst;
        p.len = len;
        p.reload = reload;
        s.regs[0x108 + k] = pcnt;
        if began {
            self.once("sound-cap", cnt as u32, || format!("sound: capture {} starts, control {:02X}", k, cnt));
            if cnt & 1 != 0 {
                self.once("sound-cap-add", k as u32, || format!("sound: capture {}'s channel added to the next (not done)", k));
            }
        }
    }

    pub(crate) fn sound_write(&mut self, addr: u32, v: u32, size: usize) {
        let off = (addr - 0x0400_0400) as usize;
        if off + size > self.snd.regs.len() {
            return;
        }
        self.sound_sync(super::now());
        for i in 0..size {
            self.snd.regs[off + i] = (v >> (8 * i)) as u8;
        }
        if off < 0x100 {
            let n = off >> 4;
            self.sound_parse(n);
            if n == 1 || n == 3 {
                self.snd.cap[n / 2].reload = self.snd.ch[n].reload; // the capture units run on these timers
            }
        } else if off < 0x104 {
            self.snd.regs[0x100] &= 0x7F; // SOUNDCNT: bits 7 and 14 are not kept
            self.snd.regs[0x101] &= 0xBF;
        } else if off >= 0x108 {
            for k in 0..2 {
                self.sound_parse_capture(k);
            }
        }
        let o = off & !1;
        let h = self.snd.reg16(o);
        self.io_set16(7, addr & !1, h);
    }

    pub(super) fn sound_read(&mut self, addr: u32, size: usize) -> u32 {
        let off = (addr - 0x0400_0400) as usize;
        if off + size > self.snd.regs.len() {
            return 0;
        }
        self.sound_sync(super::now());
        let mut v = 0u32;
        for i in 0..size {
            v |= (self.snd.regs[off + i] as u32) << (8 * i);
        }
        v
    }
}
