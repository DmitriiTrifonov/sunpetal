//! The menu's sounds (K-021): the game's own cursor, select and open blips, read from the user's ROM
//! at start (their waves in the SDAT's sound bank) and played over the game's sound. Nothing from
//! the ROM is kept in the repository; with no ROM file there are no cues and the menu is silent.
//! Which waves they are: `docs/RECON-MENU.md` ("Звуки выбора").
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

/// A short sound: mono samples and their rate.
pub struct Cue {
    pub pcm: Vec<i16>,
    pub rate: f32,
}

/// The three sounds of the menu.
pub struct Cues {
    pub cursor: Arc<Cue>,
    pub select: Arc<Cue>,
    pub open: Arc<Cue>,
}

/// Where this ROM keeps them: the SDAT, the bank's file in its FAT, the waves in the bank.
const SDAT_AT: u64 = 0xC6_CE00;
const BANK_FILE: u64 = 271;
const WAVE_CURSOR: u64 = 21;
const WAVE_SELECT: u64 = 16;
const WAVE_OPEN: u64 = 307;

const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337,
    371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484,
    7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// IMA-ADPCM as the DS keeps it: a header (the first sample, the step index), then nibbles, low first.
pub fn decode_adpcm(data: &[u8]) -> Vec<i16> {
    if data.len() < 4 {
        return Vec::new();
    }
    let mut value = i16::from_le_bytes([data[0], data[1]]) as i32;
    let mut index = (u16::from_le_bytes([data[2], data[3]]) as i32).clamp(0, 88);
    let mut out = Vec::with_capacity((data.len() - 4) * 2 + 1);
    out.push(value as i16);
    for b in &data[4..] {
        for nib in [b & 15, b >> 4] {
            let (step, n) = (STEP[index as usize], nib as usize);
            let mut d = step >> 3;
            if n & 1 != 0 {
                d += step >> 2;
            }
            if n & 2 != 0 {
                d += step >> 1;
            }
            if n & 4 != 0 {
                d += step;
            }
            value = if n & 8 != 0 { (value - d).max(-0x7FFF) } else { (value + d).min(0x7FFF) };
            index = (index + INDEX[n & 7]).clamp(0, 88);
            out.push(value as i16);
        }
    }
    out
}

fn read_at<R: Read + Seek>(r: &mut R, at: u64, len: usize) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(at)).ok()?;
    let mut v = vec![0u8; len];
    r.read_exact(&mut v).ok()?;
    Some(v)
}

fn u32_at(v: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]])
}

/// One ADPCM wave of the bank, decoded. `None` if the file is not what the table says.
fn wave<R: Read + Seek>(r: &mut R, bank: u64, n: u64) -> Option<Cue> {
    let at = bank + 0x3C + 4 * n;
    let offset = u32_at(&read_at(r, at, 4)?, 0) as u64;
    let h = read_at(r, bank + offset, 12)?;
    // type 2: IMA-ADPCM; no loop; the rate; the length in words after the 12 bytes of info
    if h[0] != 2 || h[1] != 0 {
        return None;
    }
    let rate = u16::from_le_bytes([h[2], h[3]]) as f32;
    let words = u32_at(&h, 8) as usize + u16::from_le_bytes([h[6], h[7]]) as usize;
    if rate < 1000.0 || words == 0 || words > 16_000 {
        return None;
    }
    let data = read_at(r, bank + offset + 12, 4 * words)?;
    Some(Cue { pcm: decode_adpcm(&data), rate })
}

/// The cues, from a ROM file (this ROM's tables; any other file gives `None`).
pub fn load(rom: &Path) -> Option<Cues> {
    let mut f = std::fs::File::open(rom).ok()?;
    load_from(&mut f)
}

fn load_from<R: Read + Seek>(f: &mut R) -> Option<Cues> {
    let head = read_at(f, SDAT_AT, 0x30)?;
    if &head[..4] != b"SDAT" {
        return None;
    }
    let fat = u32_at(&head, 0x20) as u64;
    let rec = read_at(f, SDAT_AT + fat + 12 + 16 * BANK_FILE, 8)?;
    let bank = SDAT_AT + u32_at(&rec, 0) as u64;
    if read_at(f, bank, 4)?.as_slice() != b"SWAR" {
        return None;
    }
    Some(Cues { cursor: Arc::new(wave(f, bank, WAVE_CURSOR)?), select: Arc::new(wave(f, bank, WAVE_SELECT)?), open: Arc::new(wave(f, bank, WAVE_OPEN)?) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adpcm_starts_at_its_header_sample_and_follows_the_nibbles() {
        // header: sample 100, index 0; one byte: nibbles 0x1 (step 7: +0), 0x4 (4: +7)
        let pcm = decode_adpcm(&[100, 0, 0, 0, 0x41]);
        assert_eq!(pcm[0], 100);
        assert_eq!(pcm.len(), 3);
        assert_eq!(pcm[1], 101); // nibble 1 at step 7: 7>>3 (0) + 7>>2 (1)
        assert!(pcm[2] > pcm[1]); // nibble 4 adds the whole step
    }

    #[test]
    fn a_file_that_is_not_the_games_gives_no_cues() {
        assert!(load_from(&mut std::io::Cursor::new(vec![0u8; 100])).is_none());
    }

    /// With the ROM (`SUNPETAL_ROM`): the three waves are found and are of the lengths the recon measured.
    #[test]
    fn the_roms_three_waves_are_found() {
        let Some(rom) = std::env::var_os("SUNPETAL_ROM") else { return };
        let c = load(Path::new(&rom)).expect("cues");
        // 1 + 111 words (448 bytes): the header's sample, then two samples a byte after the 4 header bytes
        assert_eq!(c.cursor.pcm.len(), 889);
        assert!(c.select.pcm.len() > c.cursor.pcm.len());
        assert!((c.open.rate - 15000.0).abs() < 1.0);
    }
}
