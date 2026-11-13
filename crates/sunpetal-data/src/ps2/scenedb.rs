//! Scene database of an `EVENT/*.BIN` archive (У1.11): record 0 is `WDataBaseLocal` — how many records of each
//! kind follow and where they start; from it the role of every record, the `WDataChara` table and the graph of
//! "scene → record → model / motion" edges. Layouts from the game's debug types.
use super::{gig, gim, lf, sha1::sha1_hex};

/// Order of the `u16` fields after the date word.
const FIELDS: [&str; 30] = [
    "EventNum",
    "EventPackNum",
    "StartEventID",
    "LoadEventID",
    "Scene3DNum",
    "StartScene3DID",
    "CharaNum",
    "OperationChara",
    "MotionPackNum",
    "StartMotionPack",
    "MessageNum",
    "MotionNum",
    "FaceNum",
    "TaskNum",
    "LocalValCharNum",
    "LocalValShortNum",
    "LocalValIntNum",
    "LocalValFloatNum",
    "LocalValLongNum",
    "LocalFlagNum",
    "Scene3DFileID",
    "CharaFileID",
    "MotionPackFileID",
    "MessageFileID",
    "ItemNum",
    "ItemFileID",
    "ShadowNum",
    "ShadowFileID",
    "ScenarioVabID",
    "Pad",
];

/// (role, first-record field, count field): the ranges of records after record 0.
const ROLES: [(&str, &str, &str); 6] = [
    ("Scene3D", "Scene3DFileID", "Scene3DNum"),
    ("Chara", "CharaFileID", "CharaNum"),
    ("MotionPack", "MotionPackFileID", "MotionPackNum"),
    ("Message", "MessageFileID", "MessageNum"),
    ("Item", "ItemFileID", "ItemNum"),
    ("Shadow", "ShadowFileID", "ShadowNum"),
];

#[derive(Clone, Debug)]
pub struct Db {
    pub datetime: u32,
    fields: [u16; 30],
    /// Offsets from the record start: static event pack, event attributes, local data, scene 3D data, chara data.
    pub ptrs: [u32; 5],
}

impl Db {
    pub fn get(&self, name: &str) -> u16 {
        self.fields[FIELDS.iter().position(|f| *f == name).expect("known field")]
    }

    pub fn chara_data(&self) -> u32 {
        self.ptrs[4]
    }
}

pub fn parse_db(b: &[u8]) -> Result<Db, String> {
    let need = b.get(..0x54).ok_or("database record too short")?;
    let u32_at = |at: usize| u32::from_le_bytes(need[at..at + 4].try_into().unwrap());
    let mut fields = [0u16; 30];
    for (i, f) in fields.iter_mut().enumerate() {
        *f = u16::from_le_bytes([need[4 + 2 * i], need[5 + 2 * i]]);
    }
    let mut ptrs = [0u32; 5];
    for (i, p) in ptrs.iter_mut().enumerate() {
        *p = u32_at(0x40 + 4 * i);
    }
    Ok(Db { datetime: u32_at(0), fields, ptrs })
}

/// The role of each of the `n` records: "DB", a kind, or kinds joined by "+" where ranges overlap, "unassigned" otherwise.
pub fn roles(db: &Db, n: usize) -> Vec<String> {
    let mut role: Vec<Option<String>> = vec![None; n];
    role[0] = Some("DB".into());
    for (name, first, count) in ROLES {
        let (a, c) = (db.get(first) as usize, db.get(count) as usize);
        for k in a..a + c {
            if k < n {
                match &mut role[k] {
                    Some(r) => {
                        r.push('+');
                        r.push_str(name);
                    }
                    slot => *slot = Some(name.into()),
                }
            }
        }
    }
    role.into_iter().map(|r| r.unwrap_or_else(|| "unassigned".into())).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharaData {
    pub step: u16,
    pub name: u16,
    pub message_chara: u16,
    pub walk: u16,
    pub run: u16,
    pub idle: u16,
    pub quick_walk: u16,
    pub flag: u16,
    pub fast_run: u16,
}

/// `WDataChara` at `at` in the database record: `u32 num, u32 size, u32 offsets[num]`, then 0x20-byte bodies.
pub fn charas(db_rec: &[u8], at: usize) -> Result<Vec<CharaData>, String> {
    let u32_at = |p: usize| db_rec.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or("WDataChara past the end");
    let n = u32_at(at)? as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = at + u32_at(at + 8 + 4 * i)? as usize;
        let b = db_rec.get(o..o + 18).ok_or("chara body past the end")?;
        let v: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        out.push(CharaData { step: v[0], name: v[1], message_chara: v[2], walk: v[3], run: v[4], idle: v[5], quick_walk: v[6], flag: v[7], fast_run: v[8] });
    }
    Ok(out)
}

/// An edge of the asset graph: `(from, to, kind, info)`.
pub type Edge = [String; 4];

fn model_key(raw: &[u8], at: usize) -> Result<String, String> {
    Ok(sha1_hex(raw.get(at..at + gig::size(&raw[at..])?).ok_or("model past the end")?)[..10].to_string())
}

/// The graph edges of one archive named `name` (as in `edges.tsv` of the reference): scene → record (role),
/// record → model (slot), record → T-block, pack → motion, character → motion and → record.
pub fn edges(name: &str, raw: &[u8]) -> Result<Vec<Edge>, String> {
    let arc = lf::parse(raw)?;
    let n = arc.records.len();
    let db_rec = arc.data(raw, 0).ok_or("no database record")?;
    let db = parse_db(db_rec)?;
    let role = roles(&db, n);
    let mut out: Vec<Edge> = Vec::new();
    let mut e = |a: String, b: String, c: &str, d: String| out.push([a, b, c.to_string(), d]);
    for (i, r) in role.iter().enumerate() {
        e(format!("scene:{name}"), format!("rec:{name}#{i}"), r, String::new());
    }
    let u32_at = |p: usize| raw.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    for (i, rec) in arc.records.iter().enumerate() {
        let (slots, tslot): (&[(usize, &str)], usize) = match role[i].as_str() {
            "Scene3D" => (&[(0x18, "GIG"), (0x38, "GIG4"), (0x40, "GIG5")], 0x1c),
            "Chara" => (&[(0, "GIG"), (0x18, "GIG2")], 4),
            _ => continue,
        };
        for &(off, slot) in slots {
            let v = u32_at(rec.at + off).unwrap_or(0) as usize;
            if v != 0 && raw.get(rec.at + v..rec.at + v + 4) == Some(&gig::MAGIC[..]) {
                e(format!("rec:{name}#{i}"), format!("model:{}", model_key(raw, rec.at + v)?), slot, String::new());
            }
        }
        let t = u32_at(rec.at + tslot).unwrap_or(0) as usize;
        if t != 0 && t < rec.size as usize {
            let count = u32_at(rec.at + t).ok_or("T-block past the end")?;
            e(format!("rec:{name}#{i}"), format!("tblock:{name}#{i}"), "T-block", format!("count={count}"));
        }
    }
    let mut idmap = std::collections::HashMap::new();
    for (i, r) in role.iter().enumerate() {
        if r != "MotionPack" {
            continue;
        }
        let pk = arc.data(raw, i).unwrap();
        let items = gim::pack_items(pk)?;
        for &(mid, off) in &items {
            if pk.get(off..off + 8) != Some(&gim::MAGIC[..]) || gim::parse(&pk[off..]).is_err() {
                continue;
            }
            let next = items.iter().map(|x| x.1).filter(|&x| x > off).min().unwrap_or(pk.len());
            let key = sha1_hex(&pk[off..next])[..10].to_string();
            idmap.insert(mid, key.clone());
            e(format!("rec:{name}#{i}"), format!("motion:{key}"), "pack", format!("id={mid}"));
        }
    }
    if let Ok(cs) = charas(db_rec, db.chara_data() as usize) {
        let first = db.get("CharaFileID") as usize;
        for (k, c) in cs.iter().enumerate() {
            for (kind, v) in [("walk", c.walk), ("run", c.run), ("idle", c.idle), ("quickwalk", c.quick_walk), ("fastrun", c.fast_run)] {
                if let (true, Some(key)) = (v != 0xFFFF, idmap.get(&u32::from(v))) {
                    e(format!("chara:{name}#{k}"), format!("motion:{key}"), kind, format!("id={v}"));
                }
            }
            if first + k < n {
                e(format!("chara:{name}#{k}"), format!("rec:{name}#{}", first + k), "record", format!("name={}", c.name));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_ranges_join_their_roles() {
        let mut b = vec![0u8; 0x54];
        let set = |b: &mut Vec<u8>, name: &str, v: u16| {
            let i = FIELDS.iter().position(|f| *f == name).unwrap();
            b[4 + 2 * i..6 + 2 * i].copy_from_slice(&v.to_le_bytes());
        };
        set(&mut b, "Scene3DFileID", 1);
        set(&mut b, "Scene3DNum", 2);
        set(&mut b, "CharaFileID", 2);
        set(&mut b, "CharaNum", 1);
        let db = parse_db(&b).unwrap();
        assert_eq!(roles(&db, 5), ["DB", "Scene3D", "Scene3D+Chara", "unassigned", "unassigned"]);
    }
}
