//! Nintendo LZ10 (type 0x10): a 4-byte header (0x10 and the 24-bit size), then groups of a flag
//! byte and eight items, high bit first; a clear bit is a literal, a set bit a 2-byte match
//! (length 3..=18 in the high nibble minus 3, distance 1..=4096 in the other 12 bits minus 1).

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 4 || data[0] != 0x10 {
        return Err("not LZ10".into());
    }
    let mut size = u32::from_le_bytes([data[1], data[2], data[3], 0]) as usize;
    let mut pos = 4;
    // 16 MB and more: size 0 in the header and the real one in the next four bytes (as LZ11 does)
    if size == 0 && data.len() >= 8 {
        size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        pos = 8;
    }
    let mut out = Vec::with_capacity(size);
    let end = || "LZ10 data ends early".to_string();
    while out.len() < size {
        let flags = *data.get(pos).ok_or_else(end)?;
        pos += 1;
        for bit in 0..8 {
            if out.len() >= size {
                break;
            }
            if flags & (0x80 >> bit) != 0 {
                let (b1, b2) = (*data.get(pos).ok_or_else(end)? as usize, *data.get(pos + 1).ok_or_else(end)? as usize);
                pos += 2;
                let (len, disp) = ((b1 >> 4) + 3, ((b1 & 15) << 8 | b2) + 1);
                if disp > out.len() {
                    return Err("LZ10 match reaches before the start".into());
                }
                for _ in 0..len {
                    out.push(out[out.len() - disp]);
                }
            } else {
                out.push(*data.get(pos).ok_or_else(end)?);
                pos += 1;
            }
        }
    }
    out.truncate(size);
    Ok(out)
}

const WINDOW: usize = 4096;
const MAX_LEN: usize = 18;
const CHAIN: usize = 128;

/// Compresses `data`: the longest match within the window, found through chains of the positions
/// with the same first three bytes (the first of the equal ones wins, the nearest).
pub fn compress(data: &[u8]) -> Vec<u8> {
    let n = data.len();
    let mut out = if n < 1 << 24 { vec![0x10, n as u8, (n >> 8) as u8, (n >> 16) as u8] } else { [&[0x10, 0, 0, 0][..], &(n as u32).to_le_bytes()].concat() };
    let hash = |p: usize| ((data[p] as usize) << 10 ^ (data[p + 1] as usize) << 5 ^ data[p + 2] as usize) & 0x7FFF;
    let mut head = vec![usize::MAX; 0x8000];
    let mut prev = vec![usize::MAX; n];
    let mut inserted = 0;
    let mut pos = 0;
    while pos < n {
        let flag_at = out.len();
        out.push(0);
        let mut flags = 0u8;
        for bit in 0..8 {
            if pos >= n {
                break;
            }
            // positions before `pos` into the chains
            while inserted < pos && inserted + 2 < n {
                let h = hash(inserted);
                prev[inserted] = head[h];
                head[h] = inserted;
                inserted += 1;
            }
            inserted = inserted.max(pos);
            let (mut best_len, mut best_disp) = (0, 0);
            if pos + 2 < n {
                let mut cand = head[hash(pos)];
                let mut tries = 0;
                while cand != usize::MAX && pos - cand <= WINDOW && tries < CHAIN {
                    let mut l = 0;
                    while l < MAX_LEN && pos + l < n && data[cand + l] == data[pos + l] {
                        l += 1;
                    }
                    if l > best_len {
                        (best_len, best_disp) = (l, pos - cand);
                        if l == MAX_LEN {
                            break;
                        }
                    }
                    cand = prev[cand];
                    tries += 1;
                }
            }
            if best_len >= 3 {
                flags |= 0x80 >> bit;
                let d = best_disp - 1;
                out.push(((best_len - 3) << 4 | d >> 8) as u8);
                out.push(d as u8);
                pos += best_len;
            } else {
                out.push(data[pos]);
                pos += 1;
            }
        }
        out[flag_at] = flags;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..5000u32 {
            v.extend_from_slice(format!("line {} of the text, again and again\r\n", i % 37).as_bytes());
        }
        v
    }

    #[test]
    fn it_holds_more_than_16_mb() {
        let d: Vec<u8> = (0..(1usize << 24) + 1000).map(|i| (i % 251) as u8).collect();
        let c = compress(&d);
        assert_eq!(&c[..8], &[0x10, 0, 0, 0, d.len() as u8, (d.len() >> 8) as u8, (d.len() >> 16) as u8, (d.len() >> 24) as u8]);
        assert!(decompress(&c).unwrap() == d);
    }

    #[test]
    fn it_gives_back_what_it_took() {
        for data in [Vec::new(), b"a".to_vec(), b"abababababababababab".to_vec(), sample(), (0..20000u32).map(|i| (i * 7 % 251) as u8).collect()] {
            let z = compress(&data);
            assert_eq!(decompress(&z).unwrap(), data);
        }
        assert!(compress(&sample()).len() < sample().len() / 4);
    }

    #[test]
    fn a_hand_made_stream_is_read() {
        // "abc" then a match of length 6 at distance 3, then "d"
        let z = [0x10, 10, 0, 0, 0b0001_0000, b'a', b'b', b'c', 0x30, 0x02, b'd'];
        assert_eq!(decompress(&z).unwrap(), b"abcabcabcd");
    }

    #[test]
    fn broken_data_is_an_error() {
        assert!(decompress(&[0x11, 1, 0, 0, 0]).is_err());
        assert!(decompress(&[0x10, 9, 0, 0, 0xFF]).is_err());
    }
}
