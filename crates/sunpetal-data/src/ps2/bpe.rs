//! BPE decompressor of the PS2 disc's records (У1.3), after the game's `bpeDecode` (F-83).
//!
//! A record: `u32` unpacked size, `u32` size of the packed data, then blocks up to that size. A block: the
//! pair table (a byte above 127 skips `n-127` identity symbols, otherwise `n+1` symbols follow, each a
//! `left` byte and, when it differs from the symbol, a `right` byte; the table ends at 256 symbols), a
//! big-endian `u16` block length and the block's bytes. A byte expands by a stack: a symbol equal to its
//! `left` is a literal, any other is replaced by `left` then `right`.

fn byte(rec: &[u8], p: usize) -> Result<u8, String> {
    rec.get(p).copied().ok_or_else(|| format!("BPE record ends at {p}"))
}

/// Unpacks a record; the result is checked against the size in its header.
pub fn decode(rec: &[u8]) -> Result<Vec<u8>, String> {
    if rec.len() < 8 {
        return Err("BPE record shorter than its header".into());
    }
    let want = u32::from_le_bytes(rec[..4].try_into().unwrap()) as usize;
    let total = u32::from_le_bytes(rec[4..8].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(want);
    let mut p = 8;
    while p - 8 < total {
        let mut left: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut right = [0u8; 256];
        let mut c = 0usize;
        let mut b = byte(rec, p)?;
        loop {
            p += 1;
            let mut n = b as usize;
            if n > 127 {
                c += n - 127;
                n = 0;
            }
            if c >= 256 {
                break;
            }
            for _ in 0..=n {
                if c >= 256 {
                    return Err("BPE table past 256 symbols".into());
                }
                left[c] = byte(rec, p)?;
                p += 1;
                if left[c] as usize != c {
                    right[c] = byte(rec, p)?;
                    p += 1;
                }
                c += 1;
            }
            if c == 256 {
                break;
            }
            b = byte(rec, p)?;
        }
        let size = (byte(rec, p)? as usize) << 8 | byte(rec, p + 1)? as usize;
        p += 2;
        let end = p + size;
        if end > rec.len() {
            return Err("BPE block runs past the record".into());
        }
        let mut stack: Vec<u8> = Vec::with_capacity(256);
        while p < end {
            stack.push(rec[p]);
            p += 1;
            while let Some(x) = stack.pop() {
                if left[x as usize] == x {
                    out.push(x);
                } else {
                    stack.push(right[x as usize]);
                    stack.push(left[x as usize]);
                }
                if out.len() > want {
                    return Err("BPE output past its declared size".into());
                }
                if stack.len() > 4096 {
                    return Err("BPE symbol loop".into());
                }
            }
        }
    }
    if out.len() != want {
        return Err(format!("BPE gave {} bytes, the header says {want}", out.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One block. The table: skip 97 identities (224), then one symbol: 97 = ('b', 'c'); skip 128 (255), one
    /// identity symbol (226); skip 29 (156) to reach 256. After a skip one symbol always follows.
    fn record(body: &[u8], want: u32) -> Vec<u8> {
        let mut block = vec![224, b'b', b'c', 255, 226, 156];
        block.extend_from_slice(&(body.len() as u16).to_be_bytes());
        block.extend_from_slice(body);
        let mut rec = want.to_le_bytes().to_vec();
        rec.extend_from_slice(&(block.len() as u32).to_le_bytes());
        rec.extend(block);
        rec
    }

    #[test]
    fn a_pair_expands_and_a_literal_stays() {
        let rec = record(b"axa", 5);
        assert_eq!(decode(&rec).unwrap(), b"bcxbc");
    }

    #[test]
    fn a_wrong_declared_size_or_a_cut_record_is_an_error() {
        assert!(decode(&record(b"a", 5)).is_err());
        let rec = record(b"ax", 3);
        assert!(decode(&rec[..rec.len() - 1]).is_err());
        assert!(decode(&[1, 2, 3]).is_err());
    }

    #[test]
    fn a_symbol_that_expands_into_itself_is_stopped() {
        // symbol 'a' = (left 'a'... would be a literal); make 'a' = ('b', 'a'): a loop through 'a' itself
        let mut rec = vec![224, b'b', b'a', 255, 226, 156];
        rec.extend_from_slice(&1u16.to_be_bytes());
        rec.push(b'a');
        let mut full = 100u32.to_le_bytes().to_vec();
        full.extend_from_slice(&(rec.len() as u32).to_le_bytes());
        full.extend(rec);
        assert!(decode(&full).is_err());
    }
}
