//! A ring of the window's last frames, for debugging. The main thread only drops raw frames into a
//! bounded queue; a writer thread turns each into a PNG and keeps the newest ones for `save_last`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

/// How the bytes of a frame are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pixels {
    Rgba,
    Bgra,
}

/// One raw frame from the window.
pub struct RawFrame {
    pub vblank: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    /// 4 bytes per pixel, `width * height * 4` bytes.
    pub data: Vec<u8>,
    /// One line of JSON, without a newline.
    pub journal: String,
}

/// The recording's limits.
pub struct RecConfig {
    /// Cap of the ring's PNG bytes (default 512 MiB).
    pub max_bytes: usize,
    /// The queue's capacity (default 4).
    pub queue: usize,
    /// Burn the frame number into the picture (default true).
    pub burn_number: bool,
    /// Integer downscale of the picture before encoding (default 2; 1 = full size).
    pub shrink: u32,
}

impl Default for RecConfig {
    fn default() -> Self {
        RecConfig { max_bytes: 512 * 1024 * 1024, queue: 4, burn_number: true, shrink: 2 }
    }
}

/// A finished frame kept in the ring.
#[derive(Clone)]
struct Entry {
    vblank: u64,
    width: u32,
    height: u32,
    png: Vec<u8>,
    journal: String,
}

#[derive(Default)]
struct Ring {
    entries: VecDeque<Entry>,
    bytes: usize,
}

/// The recorder: a bounded queue into a writer thread, and the ring of the newest frames.
pub struct Recorder {
    sender: Mutex<Option<SyncSender<RawFrame>>>,
    ring: Arc<Mutex<Ring>>,
    writer: Option<std::thread::JoinHandle<()>>,
    last: Mutex<Option<u64>>,
    pending: Arc<AtomicUsize>,
}

/// What `save_last` wrote.
pub struct Saved {
    pub dir: PathBuf,
    pub frames: usize,
    pub first_vblank: u64,
    pub last_vblank: u64,
}

/// The folder for a recording saved now: `artifacts/rec/t<unix seconds>_v<vblank>` (the local time is not
/// available without a dependency; the epoch seconds are enough to tell saves apart).
pub fn save_dir(vblank: u64) -> PathBuf {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    save_dir_at(secs, vblank)
}

/// The folder's name from the epoch seconds and the vblank, split out for the test.
pub fn save_dir_at(secs: u64, vblank: u64) -> PathBuf {
    PathBuf::from(format!("artifacts/rec/t{}_v{}", secs, vblank))
}

impl Recorder {
    /// Starts the writer thread.
    pub fn new(cfg: RecConfig) -> Recorder {
        let (tx, rx) = std::sync::mpsc::sync_channel(cfg.queue);
        let ring = Arc::new(Mutex::new(Ring::default()));
        let ring2 = Arc::clone(&ring);
        let pending = Arc::new(AtomicUsize::new(0));
        let pending2 = Arc::clone(&pending);
        let max_bytes = cfg.max_bytes;
        let burn = cfg.burn_number;
        let shrink = cfg.shrink.max(1);
        let writer = std::thread::spawn(move || {
            while let Ok(f) = rx.recv() {
                let RawFrame { vblank, width, height, pixels, data, journal } = f;
                let mut rgba = data;
                let (mut width, mut height) = (width, height);
                if pixels == Pixels::Bgra {
                    bgra_to_rgba(&mut rgba);
                }
                if shrink > 1 {
                    (rgba, width, height) = shrink_box(&rgba, width, height, shrink);
                }
                if burn {
                    burn_number(&mut rgba, width, height, vblank);
                }
                if let Some(png) = encode_png(&rgba, width, height) {
                    let size = png.len();
                    let mut ring = ring2.lock().unwrap();
                    ring.entries.push_back(Entry { vblank, width, height, png, journal });
                    ring.bytes += size;
                    while ring.bytes > max_bytes && ring.entries.len() > 1 {
                        if let Some(old) = ring.entries.pop_front() {
                            ring.bytes = ring.bytes.saturating_sub(old.png.len());
                        }
                    }
                }
                pending2.fetch_sub(1, Ordering::AcqRel);
            }
        });
        Recorder { sender: Mutex::new(Some(tx)), ring, writer: Some(writer), last: Mutex::new(None), pending }
    }

    /// Puts a raw frame into the queue without blocking; false when it was dropped (full queue, a
    /// vblank no newer than the last accepted, or data not matching `width * height * 4`).
    pub fn push(&self, f: RawFrame) -> bool {
        let expected = (f.width as u64).checked_mul(f.height as u64).and_then(|n| n.checked_mul(4));
        if expected != Some(f.data.len() as u64) {
            return false;
        }
        let vblank = f.vblank;
        let mut last = self.last.lock().unwrap();
        if last.is_some_and(|l| vblank <= l) {
            return false;
        }
        let tx_guard = self.sender.lock().unwrap();
        let Some(tx) = tx_guard.as_ref() else { return false };
        self.pending.fetch_add(1, Ordering::AcqRel);
        match tx.try_send(f) {
            Ok(()) => {
                *last = Some(vblank);
                true
            }
            Err(_) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                false
            }
        }
    }

    /// Frames in the ring.
    pub fn len(&self) -> usize {
        self.ring.lock().unwrap().entries.len()
    }

    /// Sum of the PNG sizes in the ring.
    pub fn bytes(&self) -> usize {
        self.ring.lock().unwrap().bytes
    }

    /// Waits until everything put into the queue has been processed.
    pub fn flush(&self) {
        while self.pending.load(Ordering::Acquire) > 0 {
            std::thread::yield_now();
        }
    }

    /// Saves the frames with `vblank >= last - frames` (the caller turns seconds into frames) into
    /// `dir/frames/*.png` plus `frames.jsonl` and `index.json`. With `mark`, the last frame's journal
    /// line gets `"mark":true` (the key was pressed on it).
    pub fn save_last(&self, frames: u64, dir: &Path, mark: bool) -> std::io::Result<Saved> {
        self.flush();
        let chosen: Vec<Entry> = {
            let ring = self.ring.lock().unwrap();
            match ring.entries.back() {
                None => Vec::new(),
                Some(last) => {
                    let threshold = last.vblank.saturating_sub(frames);
                    ring.entries.iter().filter(|e| e.vblank >= threshold).cloned().collect()
                }
            }
        };
        std::fs::create_dir_all(dir)?;
        let frames_dir = dir.join("frames");
        std::fs::create_dir_all(&frames_dir)?;
        if chosen.is_empty() {
            return Ok(Saved { dir: dir.to_path_buf(), frames: 0, first_vblank: 0, last_vblank: 0 });
        }
        for e in &chosen {
            std::fs::write(frames_dir.join(format!("v{:08}.png", e.vblank)), &e.png)?;
        }
        let mut jsonl = String::new();
        for (i, e) in chosen.iter().enumerate() {
            let mark = mark && i + 1 == chosen.len();
            if e.journal.is_empty() {
                if mark {
                    jsonl.push_str(&format!("{{\"vblank\":{},\"mark\":true}}\n", e.vblank));
                } else {
                    jsonl.push_str(&format!("{{\"vblank\":{}}}\n", e.vblank));
                }
            } else if mark {
                // the journal is a JSON object: the mark goes in before its closing brace
                let mut line = e.journal.clone();
                if line.ends_with('}') {
                    line.pop();
                }
                jsonl.push_str(&format!("{},\"mark\":true}}\n", line));
            } else {
                jsonl.push_str(&e.journal);
                jsonl.push('\n');
            }
        }
        std::fs::write(dir.join("frames.jsonl"), jsonl)?;
        let first = chosen.first().unwrap().vblank;
        let last = chosen.last().unwrap().vblank;
        let (width, height) = (chosen.last().unwrap().width, chosen.last().unwrap().height);
        let index = format!("{{\"frames\":{},\"first_vblank\":{},\"last_vblank\":{},\"width\":{},\"height\":{}}}", chosen.len(), first, last, width, height);
        std::fs::write(dir.join("index.json"), index)?;
        Ok(Saved { dir: dir.to_path_buf(), frames: chosen.len(), first_vblank: first, last_vblank: last })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Dropping the sender closes the channel, so the writer stops.
        if let Ok(mut sender) = self.sender.lock() {
            let _ = sender.take();
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// Swaps B and R in place.
fn bgra_to_rgba(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// Averages `k`x`k` blocks (the edge remainder is dropped); returns the picture and its size.
fn shrink_box(rgba: &[u8], width: u32, height: u32, k: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width / k, height / k);
    if w == 0 || h == 0 {
        return (rgba.to_vec(), width, height);
    }
    let n = k * k;
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..k {
                let row = ((y * k + dy) * width + x * k) as usize * 4;
                for dx in 0..k as usize {
                    for c in 0..4 {
                        acc[c] += rgba[row + dx * 4 + c] as u32;
                    }
                }
            }
            out.extend(acc.iter().map(|a| ((a + n / 2) / n) as u8));
        }
    }
    (out, w, h)
}

/// Encodes RGBA (8 bit) into a PNG, fast. None when the encoding failed.
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// The 5x7 digit font, one `[u8; 7]` per digit, 5 bits per row, the leftmost column is bit 4.
const FONT: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// Draws the frame number in bright yellow on a black plaque in the top-left corner (4 px inset).
/// Safe for small buffers: it draws only what fits, and does nothing when the size is wrong.
pub fn burn_number(rgba: &mut [u8], width: u32, height: u32, vblank: u64) {
    let expected = (width as u64).checked_mul(height as u64).and_then(|n| n.checked_mul(4));
    if expected != Some(rgba.len() as u64) {
        return;
    }
    let (w, h) = (width as usize, height as usize);
    let scale = (width.min(height) / 200).max(1) as usize;
    let digits = vblank.to_string();
    let off = 4usize;
    let glyph_w = 5 * scale;
    let glyph_h = 7 * scale;
    let gap = scale;
    let text_w = digits.len() * glyph_w + digits.len().saturating_sub(1) * gap;
    let plaque_w = (off + text_w + off).min(w);
    let plaque_h = (off + glyph_h + off).min(h);
    let black = [0u8, 0, 0, 255];
    let yellow = [255u8, 230, 0, 255];
    let mut set = |x: usize, y: usize, c: [u8; 4]| {
        if x < w && y < h {
            let i = (y * w + x) * 4;
            rgba[i..i + 4].copy_from_slice(&c);
        }
    };
    for y in 0..plaque_h {
        for x in 0..plaque_w {
            set(x, y, black);
        }
    }
    for (di, ch) in digits.bytes().enumerate() {
        let glyph = FONT[(ch - b'0') as usize];
        let dx = off + di * (glyph_w + gap);
        let dy = off;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            set(dx + col * scale + sx, dy + row * scale + sy, yellow);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn frame(vblank: u64, w: u32, h: u32) -> RawFrame {
        RawFrame { vblank, width: w, height: h, pixels: Pixels::Rgba, data: vec![60; (w * h * 4) as usize], journal: String::new() }
    }

    /// A frame of pseudo-random noise, so its PNG does not shrink to nothing.
    fn noise_frame(vblank: u64, w: u32, h: u32) -> RawFrame {
        let mut data = vec![0u8; (w * h * 4) as usize];
        let mut x = (vblank as u32).wrapping_add(1) | 1;
        for b in data.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = (x >> 24) as u8;
        }
        RawFrame { vblank, width: w, height: h, pixels: Pixels::Rgba, data, journal: String::new() }
    }

    fn dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", name, std::process::id()))
    }

    fn decode(path: &Path) -> (u32, u32, Vec<u8>) {
        let f = std::fs::File::open(path).unwrap();
        let mut r = png::Decoder::new(std::io::BufReader::new(f)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf[..info.buffer_size()].to_vec())
    }

    #[test]
    fn shrink_box_averages_blocks() {
        let mut px = vec![0u8; 4 * 4 * 4];
        for p in px.chunks_exact_mut(4).take(2) {
            p.copy_from_slice(&[200, 100, 0, 255]);
        }
        let (out, w, h) = shrink_box(&px, 4, 4, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(&out[..4], &[100, 50, 0, 128]);
        assert_eq!(out.len(), 2 * 2 * 4);
    }

    #[test]
    fn burn_number_draws_and_is_safe() {
        let mut px = vec![255u8; 3 * 3 * 4];
        burn_number(&mut px, 3, 3, 123);
        assert_eq!(&px[..4], &[0u8, 0, 0, 255][..]);
        // an empty buffer and a wrong-sized one must not panic
        burn_number(&mut [], 0, 0, 0);
        let mut wrong = vec![0u8; 5];
        burn_number(&mut wrong, 1, 1, 0);
    }

    #[test]
    fn ring_discards_oldest_over_max_bytes() {
        let rec = Recorder::new(RecConfig { max_bytes: 4000, queue: 16, burn_number: false, shrink: 1 });
        for v in 0..10u64 {
            assert!(rec.push(noise_frame(v, 16, 16)));
        }
        rec.flush();
        assert!(rec.len() < 10);
        assert!(rec.bytes() <= 4000 || rec.len() == 1);
        let first = rec.ring.lock().unwrap().entries.front().unwrap().vblank;
        assert!(first > 0);
    }

    #[test]
    fn push_drops_duplicates_and_decreasing_vblank() {
        let rec = Recorder::new(RecConfig { queue: 8, shrink: 1, ..RecConfig::default() });
        assert!(rec.push(frame(5, 4, 4)));
        assert!(!rec.push(frame(5, 4, 4)));
        assert!(!rec.push(frame(3, 4, 4)));
    }

    #[test]
    fn save_last_writes_frames_and_journal() {
        let d = dir("sunpetal-rec-test-save");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { queue: 8, shrink: 1, ..RecConfig::default() });
        for v in 1..=5u64 {
            assert!(rec.push(frame(v, 8, 8)));
        }
        let saved = rec.save_last(10, &d, false).unwrap();
        assert_eq!(saved.frames, 5);
        assert!(d.join("frames").join("v00000003.png").exists());
        let jsonl = std::fs::read_to_string(d.join("frames.jsonl")).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 5);
        for l in lines {
            assert!(l.starts_with('{'));
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn saved_png_decodes_to_the_right_size() {
        let d = dir("sunpetal-rec-test-png");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { burn_number: false, shrink: 1, ..RecConfig::default() });
        assert!(rec.push(frame(7, 12, 8)));
        rec.save_last(1, &d, false).unwrap();
        let (w, h, px) = decode(&d.join("frames").join("v00000007.png"));
        assert_eq!((w, h), (12, 8));
        assert_eq!(px.len(), 12 * 8 * 4);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn bgra_is_swapped_to_rgba() {
        let d = dir("sunpetal-rec-test-bgra");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { burn_number: false, shrink: 1, ..RecConfig::default() });
        let f = RawFrame { vblank: 1, width: 1, height: 1, pixels: Pixels::Bgra, data: vec![10, 20, 30, 255], journal: String::new() };
        assert!(rec.push(f));
        rec.save_last(1, &d, false).unwrap();
        let (_, _, px) = decode(&d.join("frames").join("v00000001.png"));
        assert_eq!(px, vec![30, 20, 10, 255]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_last_marks_only_the_last_frame_when_asked() {
        let d = dir("sunpetal-rec-test-mark");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { queue: 8, shrink: 1, ..RecConfig::default() });
        for v in 1..=3u64 {
            let mut f = frame(v, 8, 8);
            f.journal = format!("{{\"vblank\":{},\"layout\":\"single\"}}", v);
            assert!(rec.push(f));
        }
        rec.save_last(10, &d, true).unwrap();
        let jsonl = std::fs::read_to_string(d.join("frames.jsonl")).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].contains("mark"));
        assert!(!lines[1].contains("mark"));
        // the mark goes into the last line, the journal's own fields stay
        assert_eq!(lines[2], "{\"vblank\":3,\"layout\":\"single\",\"mark\":true}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_last_without_a_mark_writes_no_mark_field() {
        let d = dir("sunpetal-rec-test-nomark");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { queue: 8, shrink: 1, ..RecConfig::default() });
        for v in 1..=3u64 {
            let mut f = frame(v, 8, 8);
            f.journal = format!("{{\"vblank\":{}}}", v);
            assert!(rec.push(f));
        }
        rec.save_last(10, &d, false).unwrap();
        let jsonl = std::fs::read_to_string(d.join("frames.jsonl")).unwrap();
        assert!(!jsonl.contains("mark"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_last_marks_a_frame_with_an_empty_journal() {
        let d = dir("sunpetal-rec-test-mark-empty");
        let _ = std::fs::remove_dir_all(&d);
        let rec = Recorder::new(RecConfig { queue: 8, shrink: 1, ..RecConfig::default() });
        assert!(rec.push(frame(1, 8, 8)));
        assert!(rec.push(frame(2, 8, 8)));
        rec.save_last(10, &d, true).unwrap();
        let jsonl = std::fs::read_to_string(d.join("frames.jsonl")).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines[0], "{\"vblank\":1}");
        assert_eq!(lines[1], "{\"vblank\":2,\"mark\":true}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_recording_folder_is_named_by_time_and_vblank() {
        assert_eq!(save_dir_at(1_750_000_000, 42), PathBuf::from("artifacts/rec/t1750000000_v42"));
    }
}
