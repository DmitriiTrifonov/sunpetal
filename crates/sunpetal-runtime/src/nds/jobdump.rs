//! Dump and load of one 3D frame: the `Job` as the rendering engine saw it and the software
//! renderer's picture of it. Этап 0 of the GPU backend plan (`docs/GPU-BACKEND.md`): the parity
//! harness compares a renderer against these files, so a dump must come back byte-exact.

use super::render3d::{GxPoly, GxVertex, Job, TexKey};
use super::texpack::Pack;
use super::{Extras, TexFilter};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

const MAGIC: &[u8; 4] = b"SPJ1";

/// What `SUNPETAL_JOB_DUMP` asked for: the directory and the frames to dump (`SUNPETAL_JOB_FRAMES`,
/// "350,1600-1610"; unset: every frame). Read once.
struct Cfg {
    dir: PathBuf,
    frames: Option<Vec<(u64, u64)>>,
}

fn cfg() -> &'static Option<Cfg> {
    static CFG: OnceLock<Option<Cfg>> = OnceLock::new();
    CFG.get_or_init(|| {
        let dir = std::env::var_os("SUNPETAL_JOB_DUMP")?;
        let frames = std::env::var("SUNPETAL_JOB_FRAMES").ok().map(|s| {
            s.split(',')
                .filter_map(|r| {
                    let (a, b) = match r.split_once('-') {
                        Some((a, b)) => (a, b),
                        None => (r, r),
                    };
                    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
                })
                .collect()
        });
        Some(Cfg { dir: dir.into(), frames })
    })
}

/// Writes `job_<vblank>.bin` into the dump directory when the frame is asked for. `px` is the
/// rendered picture (the words of `render`), `notes` its second return value.
pub(super) fn maybe_dump(job: &Job, scale: usize, vblank: u64, px: &[u32], notes: u32) {
    let Some(c) = cfg() else { return };
    if let Some(frames) = &c.frames {
        if !frames.iter().any(|(a, b)| (*a..=*b).contains(&vblank)) {
            return;
        }
    }
    let path = c.dir.join(format!("job_{vblank:07}.spjob"));
    match write(job, scale, vblank, px, notes, &path) {
        Ok(p) => eprintln!("job dump: {}", p.display()),
        Err(x) => eprintln!("job dump failed: {x}"),
    }
}

struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn blob(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    fn vertex(&mut self, v: &GxVertex) {
        self.u32(v.x as u32);
        self.u32(v.y as u32);
        self.u32(v.z as u32);
        self.u32(v.w as u32);
        self.f32(v.sx);
        self.f32(v.sy);
        self.u16(v.ix as u16);
        self.u16(v.iy as u16);
        self.f32(v.r);
        self.f32(v.g);
        self.f32(v.b);
        self.f32(v.s);
        self.f32(v.t);
    }
    fn poly(&mut self, p: &GxPoly) {
        self.u32(p.first);
        self.u32(p.n);
        self.u32(p.attr);
        self.u32(p.tex);
        self.u32(p.pal);
        self.u8(p.translucent as u8);
        self.u32(p.sortkey);
    }
    fn extras(&mut self, e: &Extras) {
        self.f32(e.outline);
        self.f32(e.outline_width);
        self.f32(e.shadow);
        self.f32(e.shadow_radius);
        self.u8(matches!(e.filter, TexFilter::Bilinear) as u8);
        self.u8(e.msaa);
    }
}

pub(super) fn write(job: &Job, scale: usize, vblank: u64, px: &[u32], notes: u32, path: &Path) -> io::Result<PathBuf> {
    let mut o = Out(MAGIC.to_vec());
    o.u32(scale as u32);
    o.u64(vblank);
    o.u32(notes);
    // the picture: width x 192 words at the frame's scale
    o.u32(px.len() as u32);
    for &w in px {
        o.u32(w);
    }
    o.u32(job.verts.len() as u32);
    for v in &job.verts {
        o.vertex(v);
    }
    o.u32(job.polys.len() as u32);
    for p in &job.polys {
        o.poly(p);
    }
    o.u8(job.wbuffer as u8);
    o.u16(job.disp3dcnt);
    o.u32(job.width as u32);
    o.u32(job.clear_color);
    o.u16(job.clear_depth);
    o.u16(job.clrimage_offset);
    o.u8(job.alpha_ref);
    o.extras(&job.extras);
    o.0.extend_from_slice(&job.edge.map(|e| e.to_le_bytes()).concat());
    o.u32(job.fog_color);
    o.u16(job.fog_offset);
    o.blob(&job.fog_table);
    let toon: Vec<u8> = job.toon.iter().flat_map(|t| t.to_le_bytes()).collect();
    o.blob(&toon);
    o.blob(&job.tex);
    o.blob(&job.texpal);
    // the keys sorted, so the file is stable for the same frame
    let mut keys: Vec<_> = job.keys.iter().collect();
    keys.sort_by_key(|(k, _)| (*k).clone());
    o.u32(keys.len() as u32);
    for ((tex, pal), k) in keys {
        o.u32(*tex);
        o.u32(*pal);
        o.u64(k.data);
        o.u64(k.pal);
        o.u8(k.fmt);
        o.u16(k.w);
        o.u16(k.h);
    }
    // the pack is not in the file: parity frames are captured without replacements, the flag says so
    o.u8(!job.pack.is_empty() as u8);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::File::create(&tmp)?.write_all(&o.0)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path.to_path_buf())
}

/// A loaded dump: the job, the frame it came from and the software renderer's answer to compare with.
pub(super) struct Loaded {
    pub job: Job,
    pub scale: usize,
    pub vblank: u64,
    pub px: Vec<u32>,
    pub notes: u32,
}

struct In<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> In<'a> {
    fn need(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.o + n;
        if end > self.b.len() {
            return Err(format!("dump ends at {} bytes, need {}", self.b.len(), end));
        }
        let s = &self.b[self.o..end];
        self.o = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.need(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.need(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.need(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.need(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.need(4)?.try_into().unwrap()))
    }
    fn blob(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.need(n)
    }
    fn vertex(&mut self) -> Result<GxVertex, String> {
        Ok(GxVertex {
            x: self.u32()? as i32,
            y: self.u32()? as i32,
            z: self.u32()? as i32,
            w: self.u32()? as i32,
            sx: self.f32()?,
            sy: self.f32()?,
            ix: self.u16()? as i16,
            iy: self.u16()? as i16,
            r: self.f32()?,
            g: self.f32()?,
            b: self.f32()?,
            s: self.f32()?,
            t: self.f32()?,
        })
    }
    fn poly(&mut self) -> Result<GxPoly, String> {
        Ok(GxPoly {
            first: self.u32()?,
            n: self.u32()?,
            attr: self.u32()?,
            tex: self.u32()?,
            pal: self.u32()?,
            translucent: self.u8()? != 0,
            sortkey: self.u32()?,
        })
    }
    fn extras(&mut self) -> Result<Extras, String> {
        Ok(Extras {
            outline: self.f32()?,
            outline_width: self.f32()?,
            shadow: self.f32()?,
            shadow_radius: self.f32()?,
            filter: if self.u8()? != 0 { TexFilter::Bilinear } else { TexFilter::Nearest },
            msaa: self.u8()?,
        })
    }
}

pub(super) fn read(path: &Path) -> Result<Loaded, String> {
    let b = std::fs::read(path).map_err(|x| format!("{}: {x}", path.display()))?;
    let mut i = In { b: &b, o: 0 };
    if i.need(4)? != MAGIC {
        return Err(format!("{}: not a job dump", path.display()));
    }
    let scale = i.u32()? as usize;
    let vblank = i.u64()?;
    let notes = i.u32()?;
    let n = i.u32()? as usize;
    let px: Vec<u32> = (0..n).map(|_| i.u32()).collect::<Result<_, _>>()?;
    let n = i.u32()? as usize;
    let verts: Vec<GxVertex> = (0..n).map(|_| i.vertex()).collect::<Result<_, _>>()?;
    let n = i.u32()? as usize;
    let polys: Vec<GxPoly> = (0..n).map(|_| i.poly()).collect::<Result<_, _>>()?;
    let wbuffer = i.u8()? != 0;
    let disp3dcnt = i.u16()?;
    let width = i.u32()? as usize;
    let clear_color = i.u32()?;
    let clear_depth = i.u16()?;
    let clrimage_offset = i.u16()?;
    let alpha_ref = i.u8()?;
    let extras = i.extras()?;
    let mut edge = [0u16; 8];
    for e in edge.iter_mut() {
        *e = i.u16()?;
    }
    let fog_color = i.u32()?;
    let fog_offset = i.u16()?;
    let fog_table: [u8; 32] = i.blob()?.try_into().map_err(|_| "fog table")?;
    let toon_v = i.blob()?;
    if toon_v.len() != 64 {
        return Err("toon table".into());
    }
    let mut toon = [0u16; 32];
    for (t, v) in toon.iter_mut().zip(toon_v.chunks_exact(2)) {
        *t = u16::from_le_bytes(v.try_into().unwrap());
    }
    let tex = i.blob()?.to_vec();
    let texpal = i.blob()?.to_vec();
    let n = i.u32()? as usize;
    let mut keys = HashMap::new();
    for _ in 0..n {
        let tex = i.u32()?;
        let pal = i.u32()?;
        keys.insert(
            (tex, pal),
            TexKey { data: i.u64()?, pal: i.u64()?, fmt: i.u8()?, w: i.u16()?, h: i.u16()? },
        );
    }
    let pack_active = i.u8()? != 0;
    if pack_active {
        return Err(format!("{}: captured with a texture pack, unusable for parity", path.display()));
    }
    let job = Job {
        verts,
        polys,
        wbuffer,
        disp3dcnt,
        width,
        clear_color,
        clear_depth,
        clrimage_offset,
        alpha_ref,
        extras,
        edge,
        fog_color,
        fog_offset,
        fog_table,
        toon,
        tex,
        texpal,
        keys,
        pack: Arc::new(Pack::empty()),
    };
    Ok(Loaded { job, scale, vblank, px, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small job that draws one triangle: enough to prove the roundtrip.
    fn triangle_job() -> Job {
        let verts = vec![
            GxVertex { x: 0, y: 0, z: 0, w: 4096, sx: 10.0, sy: 10.0, ix: 10, iy: 10, r: 31.0, g: 0.0, b: 0.0, s: 0.0, t: 0.0 },
            GxVertex { x: 4096, y: 0, z: 0, w: 4096, sx: 200.0, sy: 20.0, ix: 200, iy: 20, r: 0.0, g: 31.0, b: 0.0, s: 0.0, t: 0.0 },
            GxVertex { x: 4096, y: 4096, z: 4096, w: 4096, sx: 100.0, sy: 180.0, ix: 100, iy: 180, r: 0.0, g: 0.0, b: 31.0, s: 0.0, t: 0.0 },
        ];
        let polys = vec![GxPoly { first: 0, n: 3, attr: 0x28, tex: 0, pal: 0, translucent: false, sortkey: 0 }];
        Job {
            verts,
            polys,
            wbuffer: true,
            disp3dcnt: 0,
            width: 256,
            clear_color: 0x801F0000,
            clear_depth: 0x7FFF,
            clrimage_offset: 0,
            alpha_ref: 0,
            extras: Extras::default(),
            edge: [0; 8],
            fog_color: 0,
            fog_offset: 0,
            fog_table: [0; 32],
            toon: [0; 32],
            tex: vec![0; 0x20000],
            texpal: vec![0; 0x10000],
            keys: HashMap::new(),
            pack: Arc::new(Pack::empty()),
        }
    }

    #[test]
    fn a_dump_comes_back_and_renders_the_same_words() {
        let job = triangle_job();
        let (px, notes) = super::super::render3d::render(&job, 2);
        assert!(!px.is_empty());
        let dir = std::env::temp_dir().join("sunpetal_jobdump_test");
        let path = write(&job, 2, 12345, &px, notes, &dir.join("job_0012345.spjob")).unwrap();
        let l = read(&path).unwrap();
        assert_eq!(l.scale, 2);
        assert_eq!(l.vblank, 12345);
        assert_eq!(l.px, px);
        assert_eq!(l.notes, notes);
        assert_eq!(l.job.verts.len(), job.verts.len());
        assert!(l.job.verts.iter().zip(&job.verts).all(|(a, b)| a.x == b.x && a.w == b.w && a.sx == b.sx && a.r == b.r && a.s == b.s));
        assert!(l.job.polys.iter().zip(&job.polys).all(|(a, b)| a.first == b.first && a.n == b.n && a.attr == b.attr && a.tex == b.tex && a.pal == b.pal && a.translucent == b.translucent && a.sortkey == b.sortkey));
        assert_eq!(l.job.wbuffer, job.wbuffer);
        assert_eq!(l.job.width, job.width);
        assert_eq!(l.job.clear_color, job.clear_color);
        assert_eq!(l.job.extras, job.extras);
        assert_eq!(l.job.tex.len(), job.tex.len());
        assert!(l.job.pack.is_empty());
        let (px2, notes2) = super::super::render3d::render(&l.job, l.scale);
        assert_eq!(px2, px);
        assert_eq!(notes2, notes);
        std::fs::remove_dir_all(dir).ok();
    }
}
