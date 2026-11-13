//! `WEvent` byte code of the scene events (У1.12): a stream of u16 words. The top three bits of a word pick
//! the kind: 0 an operator, 1 a method call (4 words), 2 a push of a constant (2-5 words), 3 a variable access,
//! 4 yield, 5 clear, 6 end. Event code is at `static_event_pack + EventAttr.Offset` of the database record.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A known operator: `+x -x ! ~ * / + - < <= > >= == != ^ | && || = if goto end & `; see [`op_name`].
    Simple(u16),
    Call {
        class: u16,
        method: u16,
        m: u16,
        x: u16,
    },
    Push(Value),
    Var(u16),
    Yield,
    Clear,
    End,
    /// An unknown word.
    Bad(u16),
}

pub fn op_name(code: u16) -> Option<&'static str> {
    Some(match code {
        1 => "+x",
        2 => "-x",
        3 => "!",
        4 => "~",
        5 => "*",
        6 => "/",
        7 => "+",
        8 => "-",
        9 => "<",
        10 => "<=",
        11 => ">",
        12 => ">=",
        13 => "==",
        14 => "!=",
        0x10 => "^",
        0x11 => "|",
        0x12 => "&&",
        0x13 => "||",
        0x14 => "=",
        0x16 | 0x19 => "if",
        0x18 | 0x1A => "goto",
        0x1B => "end",
        0x1F => "&",
        _ => return None,
    })
}

const CLASSES: [&str; 26] = [
    "Event",
    "self",
    "Chara",
    "Scene3D",
    "MotionPack",
    "Scene3DMgr",
    "CharaMgr",
    "Navi",
    "TaskMgr",
    "Task",
    "MotionPackMgr",
    "CameraMgr",
    "System",
    "Katharine",
    "Item",
    "VabStream",
    "VabStreamMgr",
    "Vab",
    "Se",
    "Seq",
    "Pcm",
    "PcmMgr",
    "CameraAnim",
    "Message",
    "MessageMgr",
    "SeMgr",
];

pub fn class_name(class: u16) -> Option<&'static str> {
    CLASSES.get(class as usize).copied()
}

/// Disassembles `w` from `pc`: (word index, op). With `stop` it ends at the first `end`; at most `limit` ops.
/// A word that needs more than is left ends the listing.
pub fn dis(w: &[u16], mut pc: usize, limit: usize, stop: bool) -> Vec<(usize, Op)> {
    let mut out = Vec::new();
    while pc < w.len() && out.len() < limit {
        let v = w[pc];
        let at = |k: usize| w.get(pc + k).copied();
        let (op, len) = match v & 0xE000 {
            0 => match op_name(v) {
                Some(_) => (Op::Simple(v), 1),
                None => (Op::Bad(v), 1),
            },
            0x2000 => match (at(1), at(2), at(3)) {
                (Some(a), Some(m), Some(x)) => (Op::Call { class: m & 0x1FFF, method: a, m, x }, 4),
                _ => break,
            },
            0x4000 => {
                let t = v & 7;
                let words = |n: usize| (1..=n).map(at).collect::<Option<Vec<u16>>>();
                let pushed = match t {
                    0 => words(1).map(|d| (Value::Int(i64::from(d[0] as u8 as i8)), 2)),
                    1 => words(1).map(|d| (Value::Int(i64::from(d[0] as i16)), 2)),
                    2 => words(2).map(|d| (Value::Int(i64::from((u32::from(d[0]) | u32::from(d[1]) << 16) as i32)), 3)),
                    3 => words(4).map(|d| (Value::Int(d.iter().rev().fold(0u64, |a, &x| a << 16 | u64::from(x)) as i64), 5)),
                    4 => words(2).map(|d| (Value::Float(f64::from(f32::from_bits(u32::from(d[0]) | u32::from(d[1]) << 16))), 3)),
                    _ => {
                        out.push((pc, Op::Bad(v)));
                        pc += 1;
                        continue;
                    }
                };
                match pushed {
                    Some((val, n)) => (Op::Push(val), n),
                    None => break,
                }
            }
            0x6000 => (Op::Var(v), if v & 0x18 == 0x10 { 4 } else { 2 }),
            0x8000 => (Op::Yield, 1),
            0xA000 => (Op::Clear, 1),
            0xC000 => (Op::End, 1),
            _ => (Op::Bad(v), 1),
        };
        let ends = stop && (matches!(op, Op::End) || matches!(op, Op::Simple(0x1B)));
        out.push((pc, op));
        pc += len;
        if ends {
            break;
        }
    }
    out
}

/// A method call with the constants the stack model finds in front of it: a pushed constant is itself, a
/// variable or a computed value is `None`; the arguments are the whole stack since the last barrier
/// (call, yield, clear, if, goto, `=`, end), as the method's argument count is not known.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub event: usize,
    pub pc: usize,
    pub class: u16,
    pub method: u16,
    pub m: u16,
    pub x: u16,
    pub args: Vec<Option<Value>>,
    /// Number of the call within its event.
    pub seq: usize,
}

pub fn calls(event: usize, ops: &[(usize, Op)]) -> Vec<Call> {
    let mut out = Vec::new();
    let mut st: Vec<Option<Value>> = Vec::new();
    for (pc, op) in ops {
        match op {
            Op::Push(v) => st.push(Some(*v)),
            Op::Var(_) => st.push(None),
            Op::Simple(1..=4) => {
                if st.pop().is_some() {
                    st.push(None);
                }
            }
            Op::Simple(0x16 | 0x18 | 0x19 | 0x1A | 0x14 | 0x1B) | Op::Yield | Op::Clear | Op::End => st.clear(),
            Op::Call { class, method, m, x } => {
                let seq = out.len();
                out.push(Call { event, pc: *pc, class: *class, method: *method, m: *m, x: *x, args: std::mem::take(&mut st), seq });
            }
            Op::Simple(_) => {
                if st.len() >= 2 {
                    st.truncate(st.len() - 2);
                    st.push(None);
                }
            }
            Op::Bad(_) => {}
        }
    }
    out
}

/// One `EventAttr`: where the code of an event is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attr {
    pub pack: u16,
    pub local: u16,
    pub offset: u32,
}

/// The code words of every static event (pack 0xFFFF) of a database record `b`, with the event's number.
pub fn static_events(b: &[u8], db: &super::scenedb::Db) -> Result<Vec<(usize, Vec<u16>)>, String> {
    let (fep, attr) = (db.ptrs[0] as usize, db.ptrs[1] as usize);
    let n = db.get("EventNum") as usize;
    let rd = |at: usize, len: usize| b.get(at..at + len).ok_or_else(|| format!("past the end at {at:#x}"));
    let attrs: Vec<Attr> = (0..n)
        .map(|i| {
            let a = rd(attr + 8 * i, 8)?;
            Ok(Attr { pack: u16::from_le_bytes([a[0], a[1]]), local: u16::from_le_bytes([a[2], a[3]]), offset: u32::from_le_bytes(a[4..8].try_into().unwrap()) })
        })
        .collect::<Result<_, String>>()?;
    let end = attr.checked_sub(fep).ok_or("attributes before the pack")? as u32;
    let mut offs: Vec<u32> = attrs.iter().map(|a| a.offset).chain([end]).collect();
    offs.sort_unstable();
    offs.dedup();
    let mut out = Vec::new();
    for (i, a) in attrs.iter().enumerate() {
        if a.pack != 0xFFFF {
            continue;
        }
        let next = offs.iter().copied().find(|&o| o > a.offset).unwrap_or(end);
        let bytes = rd(fep + a.offset as usize, (next.saturating_sub(a.offset) / 2 * 2) as usize)?;
        out.push((i, bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_takes_the_constants_pushed_before_it() {
        // push 5 (i8), push var, call Task.4 (m = 0x8009), end
        let w = [0x4000, 5, 0x6000, 0x7, 0x2000, 4, 0x8009, 4, 0xC000];
        let ops = dis(&w, 0, 100, true);
        assert_eq!(ops.len(), 4);
        let c = calls(3, &ops);
        assert_eq!(c.len(), 1);
        assert_eq!((class_name(c[0].class), c[0].method, c[0].pc, c[0].args.clone()), (Some("Task"), 4, 4, vec![Some(Value::Int(5)), None]));
    }

    #[test]
    fn an_unknown_word_is_marked_and_a_cut_call_ends_the_listing() {
        let ops = dis(&[0x0FFF, 0x2000, 1], 0, 10, false);
        assert_eq!(ops, vec![(0, Op::Bad(0x0FFF))]);
    }
}
