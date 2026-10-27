//! The geometry engine, in software.
//!
//! Commands come packed through GXFIFO (0x04000400: a word of up to four command bytes, then
//! their parameters) or one at a time through the command ports (0x04000440-0x040005CC: each write
//! a parameter). They run at once, in fixed point as the DS computes: the matrix stacks
//! (projection, position and direction, texture), vertices in their five forms, texture
//! coordinates and their transforms, lighting at NORMAL, polygons from triangles, quads and their
//! strips, face culling, clipping against the view volume (the far plane by POLYGON_ATTR bit 12),
//! the screen position through the viewport, BOX_TEST, POS_TEST and VEC_TEST with their results.
//!
//! SWAP_BUFFERS waits for the VBlank: the commands that follow it wait in a queue (GXSTAT shows the
//! engine busy and the FIFO filling), then the polygon and vertex lists go to the rendering engine
//! and the queue runs. The rendering engine takes its lists, the 3D registers and the texture
//! memory at line 215 (`render3d`): what it draws is shown in the next frame.
//!
//! Not done: the 1-dot polygons' depth test, the lists' limits beyond the DS's own (2 048
//! polygons, 6 144 vertices: past them a polygon is dropped, as on the DS, and DISP3DCNT's
//! overflow bit set).
use std::collections::VecDeque;

use super::render3d::{GxPoly, GxVertex, Job};
use super::Nds;

const PARAMS: [u8; 128] = {
    let mut t = [0u8; 128];
    // 0x10: MTX_MODE PUSH POP STORE RESTORE IDENTITY LOAD44 LOAD43 MULT44 MULT43 MULT33 SCALE TRANS
    let a = [1u8, 0, 1, 1, 1, 0, 16, 12, 16, 12, 9, 3, 3];
    let mut i = 0;
    while i < a.len() {
        t[0x10 + i] = a[i];
        i += 1;
    }
    // 0x20: COLOR NORMAL TEXCOORD VTX16 VTX10 XY XZ YZ DIFF POLYGON_ATTR TEXIMAGE_PARAM PLTT_BASE
    let b = [1u8, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1];
    i = 0;
    while i < b.len() {
        t[0x20 + i] = b[i];
        i += 1;
    }
    // 0x30: DIF_AMB SPE_EMI LIGHT_VECTOR LIGHT_COLOR SHININESS
    let c = [1u8, 1, 1, 1, 32];
    i = 0;
    while i < c.len() {
        t[0x30 + i] = c[i];
        i += 1;
    }
    t[0x40] = 1; // BEGIN_VTXS
    t[0x50] = 1; // SWAP_BUFFERS
    t[0x60] = 1; // VIEWPORT
    t[0x70] = 3; // BOX_TEST
    t[0x71] = 2; // POS_TEST
    t[0x72] = 1; // VEC_TEST
    t
};

fn known(cmd: u32) -> bool {
    cmd == 0 || cmd == 0x11 || cmd == 0x15 || cmd == 0x41 || (cmd < 0x80 && PARAMS[cmd as usize] != 0)
}

type Mtx = [i32; 16];

const IDENTITY: Mtx = [0x1000, 0, 0, 0, 0, 0x1000, 0, 0, 0, 0, 0x1000, 0, 0, 0, 0, 0x1000];

/// m = n x m (n given, row-major, rows of four)
fn mult44(m: &mut Mtx, n: &Mtx) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] = ((n[r * 4] as i64 * t[c] as i64
                + n[r * 4 + 1] as i64 * t[4 + c] as i64
                + n[r * 4 + 2] as i64 * t[8 + c] as i64
                + n[r * 4 + 3] as i64 * t[12 + c] as i64)
                >> 12) as i32;
        }
    }
}

fn mult43(m: &mut Mtx, n: &[i32]) {
    let mut f = [0i32; 16];
    for r in 0..4 {
        f[r * 4] = n[r * 3];
        f[r * 4 + 1] = n[r * 3 + 1];
        f[r * 4 + 2] = n[r * 3 + 2];
        f[r * 4 + 3] = if r == 3 { 0x1000 } else { 0 };
    }
    mult44(m, &f);
}

fn mult33(m: &mut Mtx, n: &[i32]) {
    let t = *m;
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = ((n[r * 3] as i64 * t[c] as i64
                + n[r * 3 + 1] as i64 * t[4 + c] as i64
                + n[r * 3 + 2] as i64 * t[8 + c] as i64)
                >> 12) as i32;
        }
    }
}

fn scale(m: &mut Mtx, s: &[i32]) {
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = ((m[r * 4 + c] as i64 * s[r] as i64) >> 12) as i32;
        }
    }
}

fn translate(m: &mut Mtx, v: &[i32]) {
    for c in 0..4 {
        m[12 + c] = ((m[c] as i64 * v[0] as i64
            + m[4 + c] as i64 * v[1] as i64
            + m[8 + c] as i64 * v[2] as i64
            + ((m[12 + c] as i64) << 12))
            >> 12) as i32;
    }
}

/// A vertex in clip coordinates with its attributes.
#[derive(Clone, Copy, Default)]
struct V {
    p: [i32; 4],
    c: [i32; 3],
    t: [i32; 2],
}

fn clip_point(inp: &V, out: &V, comp: usize, plane: i64) -> V {
    let num = inp.p[3] as i64 - plane * inp.p[comp] as i64;
    let den = num - (out.p[3] as i64 - plane * out.p[comp] as i64);
    let lerp = |a: i32, b: i32| -> i32 {
        if den != 0 { (a as i64 + ((b as i64 - a as i64) * num) / den) as i32 } else { a }
    };
    let mut m = V::default();
    for i in 0..4 {
        m.p[i] = if i == comp { 0 } else { lerp(inp.p[i], out.p[i]) };
    }
    m.p[comp] = (plane * m.p[3] as i64) as i32;
    for i in 0..3 {
        m.c[i] = lerp(inp.c[i], out.c[i]);
    }
    for i in 0..2 {
        m.t[i] = lerp(inp.t[i], out.t[i]);
    }
    m
}

/// Sutherland-Hodgman against comp <= w (plane +1) then comp >= -w (plane -1).
fn clip_axis(v: &mut [V; 10], n: usize, comp: usize) -> usize {
    let mut tmp = [V::default(); 10];
    let mut m = 0;
    for i in 0..n {
        let (a, b) = (v[i], v[(i + 1) % n]);
        let (ain, bin) = (a.p[comp] <= a.p[3], b.p[comp] <= b.p[3]);
        if ain && m < 10 {
            tmp[m] = a;
            m += 1;
        }
        if ain != bin && m < 10 {
            tmp[m] = if ain { clip_point(&a, &b, comp, 1) } else { clip_point(&b, &a, comp, 1) };
            m += 1;
        }
    }
    let mut n = 0;
    for i in 0..m {
        let (a, b) = (tmp[i], tmp[(i + 1) % m]);
        let (ain, bin) = (a.p[comp] >= -a.p[3], b.p[comp] >= -b.p[3]);
        if ain && n < 10 {
            v[n] = a;
            n += 1;
        }
        if ain != bin && n < 10 {
            v[n] = if ain { clip_point(&a, &b, comp, -1) } else { clip_point(&b, &a, comp, -1) };
            n += 1;
        }
    }
    n
}

#[derive(Default)]
struct Lists {
    verts: Vec<GxVertex>,
    polys: Vec<GxPoly>,
    nverts_ds: u32,
}

struct Queued {
    cmd: u32,
    p: Vec<u32>,
}

pub(super) struct Gx {
    /// 0x04000320-0x0400071F, as written.
    regs: Vec<u8>,
    mtx_proj: Mtx,
    mtx_pos: Mtx,
    mtx_vec: Mtx,
    mtx_tex: Mtx,
    mtx_clip: Mtx,
    stk_proj: Mtx,
    stk_tex: Mtx,
    stk_pos: [Mtx; 32],
    stk_vec: [Mtx; 32],
    sp_proj: u32,
    sp_tex: u32,
    sp_pos: u32,
    clip_dirty: bool,
    mtx_mode: u32,
    /// GXSTAT's own bits: the test result (1), the stack error (15), IRQ mode (30-31).
    stat: u32,
    // vertex state
    vtx: [i16; 3],
    raw_tc: [i16; 2],
    tc: [i16; 2],
    normal: [i16; 3],
    vcolor: [u8; 3],
    attr_pending: u32,
    attr: u32,
    tex_param: u32,
    pltt_base: u32,
    /// BEGIN_VTXS's type, -1 outside.
    prim: i32,
    /// x1, y1 (top), x2, y2 (bottom), as the screen counts.
    viewport: [u8; 4],
    /// Pixels the view is widened by each side (`nds::set_wide`).
    wide: usize,
    // lighting
    light_vec: [[i32; 3]; 4],
    light_col: [[u8; 3]; 4],
    diffuse: [u8; 3],
    ambient: [u8; 3],
    specular: [u8; 3],
    emission: [u8; 3],
    shine_table_on: bool,
    shine_table: [u8; 128],
    // the lists being built, and the rendering engine's
    build: Lists,
    render: Lists,
    render_wbuf: bool,
    render_manual: bool,
    swaps: u64,
    swap_pending: bool,
    swap_param: u32,
    // a strip's or quad's vertices so far
    strip: [V; 4],
    nstrip: usize,
    strip_count: u32,
    // the FIFO's decoder and the queue behind a swap
    cmds: u32,
    ncmds: i32,
    cur: i32,
    need: usize,
    got: usize,
    params: [u32; 32],
    queue: VecDeque<Queued>,
    port_got: Vec<usize>,
    port_params: Vec<[u32; 32]>,
    pos_result: [i32; 4],
    vec_result: [i16; 3],
}

impl Gx {
    pub(super) fn set_wide(&mut self, each_side: usize) {
        self.wide = each_side;
    }

    pub(super) fn new() -> Gx {
        Gx {
            regs: vec![0; 0x400],
            mtx_proj: IDENTITY,
            mtx_pos: IDENTITY,
            mtx_vec: IDENTITY,
            mtx_tex: IDENTITY,
            mtx_clip: IDENTITY,
            stk_proj: IDENTITY,
            stk_tex: IDENTITY,
            stk_pos: [IDENTITY; 32],
            stk_vec: [IDENTITY; 32],
            sp_proj: 0,
            sp_tex: 0,
            sp_pos: 0,
            clip_dirty: true,
            mtx_mode: 0,
            stat: 0,
            vtx: [0; 3],
            raw_tc: [0; 2],
            tc: [0; 2],
            normal: [0; 3],
            vcolor: [0; 3],
            attr_pending: 0,
            attr: 0,
            tex_param: 0,
            pltt_base: 0,
            prim: -1,
            viewport: [0, 0, 255, 191],
            wide: 0,
            light_vec: [[0; 3]; 4],
            light_col: [[0; 3]; 4],
            diffuse: [0; 3],
            ambient: [0; 3],
            specular: [0; 3],
            emission: [0; 3],
            shine_table_on: false,
            shine_table: [0; 128],
            build: Lists::default(),
            render: Lists::default(),
            render_wbuf: false,
            render_manual: false,
            swaps: 0,
            swap_pending: false,
            swap_param: 0,
            strip: [V::default(); 4],
            nstrip: 0,
            strip_count: 0,
            cmds: 0,
            ncmds: 0,
            cur: -1,
            need: 0,
            got: 0,
            params: [0; 32],
            queue: VecDeque::new(),
            port_got: vec![0; 0x80],
            port_params: vec![[0; 32]; 0x80],
            pos_result: [0; 4],
            vec_result: [0; 3],
        }
    }

    fn update_clip(&mut self) {
        if !self.clip_dirty {
            return;
        }
        self.mtx_clip = self.mtx_proj;
        let pos = self.mtx_pos;
        mult44(&mut self.mtx_clip, &pos);
        self.clip_dirty = false;
    }

    /// Applies `f` to the current matrix (or, in mode 2, both position and direction when
    /// `both_in_mode2`).
    fn on_current(&mut self, both_in_mode2: bool, f: impl Fn(&mut Mtx)) {
        match self.mtx_mode {
            0 => {
                f(&mut self.mtx_proj);
                self.clip_dirty = true;
            }
            1 => {
                f(&mut self.mtx_pos);
                self.clip_dirty = true;
            }
            2 => {
                f(&mut self.mtx_pos);
                if both_in_mode2 {
                    f(&mut self.mtx_vec);
                }
                self.clip_dirty = true;
            }
            _ => f(&mut self.mtx_tex),
        }
    }

    fn lighting(&mut self) {
        let m = self.mtx_vec;
        let nm = self.normal;
        let mut n = [0i32; 3];
        for i in 0..3 {
            n[i] = ((nm[0] as i64 * m[i] as i64 + nm[1] as i64 * m[4 + i] as i64 + nm[2] as i64 * m[8 + i] as i64) >> 12) as i32;
        }
        let mut c = [self.emission[0] as i32, self.emission[1] as i32, self.emission[2] as i32];
        for l in 0..4 {
            if self.attr & (1 << l) == 0 {
                continue;
            }
            let lv = self.light_vec[l];
            let mut dl = -(((lv[0] as i64 * n[0] as i64 + lv[1] as i64 * n[1] as i64 + lv[2] as i64 * n[2] as i64) >> 10) as i32);
            dl = dl.clamp(0, 255);
            let mut sl = -((((lv[0] >> 1) as i64 * n[0] as i64
                + (lv[1] >> 1) as i64 * n[1] as i64
                + ((lv[2] - 0x200) >> 1) as i64 * n[2] as i64)
                >> 10) as i32);
            if sl < 0 {
                sl = 0;
            } else if sl > 255 {
                sl = (0x100 - sl) & 0xFF;
            }
            sl = ((sl * sl) >> 7) - 0x100;
            if sl < 0 {
                sl = 0;
            }
            if self.shine_table_on {
                sl = self.shine_table[(sl >> 1) as usize] as i32;
            }
            for i in 0..3 {
                let lc = self.light_col[l][i] as i32;
                c[i] += (self.specular[i] as i32 * lc * sl) >> 13;
                c[i] += (self.diffuse[i] as i32 * lc * dl) >> 13;
                c[i] += (self.ambient[i] as i32 * lc) >> 5;
            }
        }
        for i in 0..3 {
            self.vcolor[i] = c[i].min(31) as u8;
        }
    }

    // ---- polygons ----------------------------------------------------------------------------
    /// Returns true if the polygon list overflowed (the caller sets DISP3DCNT's bit).
    fn submit_poly(&mut self, inp: &[V], n: usize) -> bool {
        // face culling, on the clip coordinates (melonDS's form: the normal of the triangle
        // (x, y, w) of the first three vertices, against the eye)
        let (v0, v1, v2) = (&inp[0], &inp[1], &inp[2]);
        let p = |a: i32, b: i32| (a - b) as i64;
        let mut nx = p(v0.p[1], v1.p[1]) * p(v2.p[3], v1.p[3]) - p(v0.p[3], v1.p[3]) * p(v2.p[1], v1.p[1]);
        let mut ny = p(v0.p[3], v1.p[3]) * p(v2.p[0], v1.p[0]) - p(v0.p[0], v1.p[0]) * p(v2.p[3], v1.p[3]);
        let mut nz = p(v0.p[0], v1.p[0]) * p(v2.p[1], v1.p[1]) - p(v0.p[1], v1.p[1]) * p(v2.p[0], v1.p[0]);
        let big = |x: i64| (x >> 31) ^ (x >> 63) != 0;
        while big(nx) || big(ny) || big(nz) {
            nx >>= 4;
            ny >>= 4;
            nz >>= 4;
        }
        let dot = v1.p[0] as i64 * nx + v1.p[1] as i64 * ny + v1.p[3] as i64 * nz;
        let front = dot < 0;
        let attr = self.attr;
        let drop = if front {
            attr & 0x80 == 0
        } else if dot > 0 {
            attr & 0x40 == 0
        } else {
            attr & 0xC0 == 0
        };
        if drop {
            return false;
        }
        let mut v = [V::default(); 10];
        v[..n].copy_from_slice(&inp[..n]);
        // a widened view: x is taken in a frustum that much wider, so the clipping lets the sides in
        let big_w = 256 + 2 * self.wide as i64;
        if self.wide > 0 {
            // the game's own panels and rules (flat, w the same all over, running to the screen's
            // edge) run to the edge of the wider view too; everything else keeps its place
            let flat = v[..n].iter().all(|x| x.p[3] == v[0].p[3]);
            let (left, right) = (v[..n].iter().any(|x| x.p[0] == -x.p[3]), v[..n].iter().any(|x| x.p[0] == x.p[3]));
            // only the untextured ones (a picture would be stretched), and a translucent one only when
            // it is a band across most of the screen (a panel), not a narrow strip (it would veil the sides)
            let untextured = (self.tex_param >> 26) & 7 == 0;
            let wide_enough = v[..n].iter().map(|x| x.p[0]).max() > v[..n].iter().map(|x| x.p[0] + v[0].p[3]).min();
            let stretch = flat && untextured && (left || right) && ((attr >> 16) & 31 == 31 || wide_enough);
            // a band across most of the screen goes to both edges, even if it starts a little inside
            // (the dialogue panel starts beside the portrait)
            let (min_x, max_x) = (v[..n].iter().map(|x| x.p[0]).min().unwrap_or(0), v[..n].iter().map(|x| x.p[0]).max().unwrap_or(0));
            let (min_y, max_y) = (v[..n].iter().map(|x| x.p[1]).min().unwrap_or(0), v[..n].iter().map(|x| x.p[1]).max().unwrap_or(0));
            let w0 = v[0].p[3];
            let tall = (max_y - min_y) as i64 * 100 >= w0 as i64 * 2 * 90;
            let (on_left, on_right) = (max_x as i64 * 10 <= -(w0 as i64) * 2, min_x as i64 * 10 >= w0 as i64 * 2);
            let edge_strip = flat && untextured && !stretch && tall && (on_left != on_right);
            for x in &mut v[..n] {
                if stretch {
                    if x.p[0].abs() == x.p[3] {
                        continue;
                    }
                    if wide_enough && (x.p[0] == min_x || x.p[0] == max_x) {
                        x.p[0] = if x.p[0] == min_x { -x.p[3] } else { x.p[3] };
                        continue;
                    }
                }
                x.p[0] = (x.p[0] as i64 * 256 / big_w) as i32;
                // a strip the game lays against one edge of its picture (the flickering bars of the "on air"
                // frame, the wipe of the dialogue panel) stays against that edge of the wider view
                if edge_strip {
                    let w = x.p[3] as i64;
                    x.p[0] -= (w - w * 256 / big_w) as i32 * if on_left { 1 } else { -1 };
                }
            }
        }
        if attr & 0x1000 == 0 && v[..n].iter().any(|x| x.p[2] > x.p[3]) {
            return false; // past the far plane, and not to be drawn cut
        }
        let mut n = clip_axis(&mut v, n, 2);
        if n != 0 {
            n = clip_axis(&mut v, n, 0);
        }
        if n != 0 {
            n = clip_axis(&mut v, n, 1);
        }
        if n < 3 {
            return false;
        }
        if self.build.polys.len() >= 2048 || self.build.nverts_ds + n as u32 > 6144 {
            return true;
        }
        let fmt = (self.tex_param >> 26) & 7;
        let alpha = (attr >> 16) & 31;
        let mut poly = GxPoly {
            first: self.build.verts.len() as u32,
            n: n as u32,
            attr,
            tex: self.tex_param,
            pal: self.pltt_base,
            translucent: (alpha > 0 && alpha < 31) || fmt == 1 || fmt == 6,
            sortkey: 0,
        };
        let (vx, vy) = (self.viewport[0] as i64, self.viewport[1] as i64);
        let vw0 = ((self.viewport[2] as i64 - self.viewport[0] as i64 + 1) & 0x1FF) as i64;
        // widened: the viewport as wide again as the frustum; the picture keeps its place, with the
        // new columns each side of it
        let vw = vw0 * big_w / 256;
        let vx = vx + self.wide as i64 - (vw - vw0) / 2;
        let vh = ((self.viewport[3] as i64 - self.viewport[1] as i64 + 1) & 0xFF) as i64;
        let (mut ytop, mut ybot) = (192i32, 0i32);
        for x in &v[..n] {
            let (px, py, pz, w) = (x.p[0], x.p[1], x.p[2], x.p[3]);
            let wl = w as i64;
            let mut g = GxVertex { x: px, y: py, z: pz, w, ..Default::default() };
            if wl > 0 {
                g.ix = ((px as i64 + wl) * vw / (wl * 2) + vx) as i16;
                g.iy = ((-(py as i64) + wl) * vh / (wl * 2) + vy) as i16;
                g.sx = ((px as f64 + wl as f64) * vw as f64 / (wl as f64 * 2.0) + vx as f64) as f32;
                g.sy = ((-(py as f64) + wl as f64) * vh as f64 / (wl as f64 * 2.0) + vy as f64) as f32;
            } else {
                g.ix = vx as i16;
                g.iy = vy as i16;
                g.sx = vx as f32;
                g.sy = vy as f32;
            }
            g.r = x.c[0] as f32 / 16.0;
            g.g = x.c[1] as f32 / 16.0;
            g.b = x.c[2] as f32 / 16.0;
            g.s = x.t[0] as f32 / 16.0;
            g.t = x.t[1] as f32 / 16.0;
            ytop = ytop.min(g.iy as i32);
            ybot = ybot.max(g.iy as i32);
            self.build.verts.push(g);
        }
        let (ytop, ybot) = (ytop.clamp(0, 255) as u32, ybot.clamp(0, 255) as u32);
        poly.sortkey = ybot << 8 | ytop;
        self.build.polys.push(poly);
        self.build.nverts_ds += n as u32;
        false
    }
}

impl Nds {
    fn gx_submit(&mut self, v: &[V], n: usize) {
        if self.gx.submit_poly(v, n) {
            let d = self.io_reg16(9, 0x0400_0060) | 0x2000;
            self.io_set16(9, 0x0400_0060, d);
            self.once("gx", 1, || "3D: the polygon or vertex list is full (DS limits): polygons dropped".to_string());
        }
    }

    fn gx_add_vertex(&mut self) {
        let g = &mut self.gx;
        if g.prim < 0 {
            return;
        }
        if g.tex_param >> 30 == 3 {
            // texture coordinates from the vertex
            let m = g.mtx_tex;
            for i in 0..2 {
                g.tc[i] = ((g.vtx[0] as i64 * m[i] as i64
                    + g.vtx[1] as i64 * m[4 + i] as i64
                    + g.vtx[2] as i64 * m[8 + i] as i64
                    + ((g.raw_tc[i] as i64) << 24))
                    >> 24) as i16;
            }
        }
        g.update_clip();
        let m = g.mtx_clip;
        let mut nv = V::default();
        for i in 0..4 {
            nv.p[i] = ((g.vtx[0] as i64 * m[i] as i64
                + g.vtx[1] as i64 * m[4 + i] as i64
                + g.vtx[2] as i64 * m[8 + i] as i64
                + 0x1000 * m[12 + i] as i64)
                >> 12) as i32;
        }
        for i in 0..3 {
            let c = g.vcolor[i] as i32;
            nv.c[i] = c << 4 | if c != 0 { 0xF } else { 0 }; // 9 bits, as the DS keeps them
        }
        nv.t = [g.tc[0] as i32, g.tc[1] as i32];
        g.strip_count += 1;
        let strip = &mut g.strip;
        match g.prim {
            0 => {
                // triangles
                strip[g.nstrip] = nv;
                g.nstrip += 1;
                if g.nstrip == 3 {
                    let s = *strip;
                    g.nstrip = 0;
                    self.gx_submit(&s, 3);
                }
            }
            1 => {
                // quads
                strip[g.nstrip] = nv;
                g.nstrip += 1;
                if g.nstrip == 4 {
                    let s = *strip;
                    g.nstrip = 0;
                    self.gx_submit(&s, 4);
                }
            }
            2 => {
                // triangle strip: every other one turned round
                if g.nstrip < 2 {
                    strip[g.nstrip] = nv;
                    g.nstrip += 1;
                    return;
                }
                strip[2] = nv;
                let odd = g.strip_count & 1 != 0;
                let t = if odd { [strip[0], strip[1], strip[2]] } else { [strip[1], strip[0], strip[2]] };
                strip[0] = strip[1];
                strip[1] = strip[2];
                self.gx_submit(&t, 3);
            }
            3 => {
                // quad strip: 0 1 3 2, then on in pairs
                if g.nstrip < 3 {
                    strip[g.nstrip] = nv;
                    g.nstrip += 1;
                    return;
                }
                strip[3] = nv;
                let q = [strip[0], strip[1], strip[3], strip[2]];
                strip[0] = strip[2];
                strip[1] = strip[3];
                g.nstrip = 2;
                self.gx_submit(&q, 4);
            }
            _ => {}
        }
    }

    // ---- the tests ---------------------------------------------------------------------------
    fn gx_box_test(&mut self, p: &[u32]) {
        let (x, y, z) = (p[0] as i16 as i32, (p[0] >> 16) as i16 as i32, p[1] as i16 as i32);
        let (w, h, d) = ((p[1] >> 16) as i16 as i32, p[2] as i16 as i32, (p[2] >> 16) as i16 as i32);
        let g = &mut self.gx;
        g.update_clip();
        let m = g.mtx_clip;
        let mut c = [V::default(); 8];
        for (i, ci) in c.iter_mut().enumerate() {
            let px = x + if i & 1 != 0 { w } else { 0 };
            let py = y + if i & 2 != 0 { h } else { 0 };
            let pz = z + if i & 4 != 0 { d } else { 0 };
            for k in 0..4 {
                ci.p[k] = ((px as i64 * m[k] as i64 + py as i64 * m[4 + k] as i64 + pz as i64 * m[8 + k] as i64 + 0x1000 * m[12 + k] as i64) >> 12) as i32;
            }
        }
        const FACES: [[usize; 4]; 6] = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
        let mut inside = false;
        for f in FACES.iter() {
            if inside {
                break;
            }
            let mut v = [V::default(); 10];
            for k in 0..4 {
                v[k] = c[f[k]];
            }
            let mut n = clip_axis(&mut v, 4, 2);
            if n != 0 {
                n = clip_axis(&mut v, n, 0);
            }
            if n != 0 {
                n = clip_axis(&mut v, n, 1);
            }
            inside = n > 0;
        }
        g.stat = (g.stat & !2) | if inside { 2 } else { 0 };
    }

    // ---- the commands ------------------------------------------------------------------------
    fn gx_exec(&mut self, cmd: u32, p: &[u32]) {
        match cmd {
            0x00 => {}
            0x10 => self.gx.mtx_mode = p[0] & 3,
            0x11 => {
                // MTX_PUSH
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    if g.sp_proj != 0 {
                        g.stat |= 0x8000;
                    }
                    g.stk_proj = g.mtx_proj;
                    g.sp_proj = 1;
                } else if g.mtx_mode == 3 {
                    if g.sp_tex != 0 {
                        g.stat |= 0x8000;
                    }
                    g.stk_tex = g.mtx_tex;
                    g.sp_tex = 1;
                } else {
                    if g.sp_pos > 30 {
                        g.stat |= 0x8000;
                    }
                    let i = (g.sp_pos & 31) as usize;
                    g.stk_pos[i] = g.mtx_pos;
                    g.stk_vec[i] = g.mtx_vec;
                    g.sp_pos = (g.sp_pos + 1) & 0x3F;
                }
            }
            0x12 => {
                // MTX_POP
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    if g.sp_proj == 0 {
                        g.stat |= 0x8000;
                    }
                    g.sp_proj = 0;
                    g.mtx_proj = g.stk_proj;
                    g.clip_dirty = true;
                } else if g.mtx_mode == 3 {
                    if g.sp_tex == 0 {
                        g.stat |= 0x8000;
                    }
                    g.sp_tex = 0;
                    g.mtx_tex = g.stk_tex;
                } else {
                    let off = ((p[0] << 26) as i32) >> 26;
                    g.sp_pos = (g.sp_pos as i32 - off) as u32 & 0x3F;
                    if g.sp_pos > 30 {
                        g.stat |= 0x8000;
                    }
                    let i = (g.sp_pos & 31) as usize;
                    g.mtx_pos = g.stk_pos[i];
                    g.mtx_vec = g.stk_vec[i];
                    g.clip_dirty = true;
                }
            }
            0x13 => {
                // MTX_STORE
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    g.stk_proj = g.mtx_proj;
                } else if g.mtx_mode == 3 {
                    g.stk_tex = g.mtx_tex;
                } else {
                    let a = (p[0] & 31) as usize;
                    if a == 31 {
                        g.stat |= 0x8000;
                    }
                    g.stk_pos[a] = g.mtx_pos;
                    g.stk_vec[a] = g.mtx_vec;
                }
            }
            0x14 => {
                // MTX_RESTORE
                let g = &mut self.gx;
                if g.mtx_mode == 0 {
                    g.mtx_proj = g.stk_proj;
                    g.clip_dirty = true;
                } else if g.mtx_mode == 3 {
                    g.mtx_tex = g.stk_tex;
                } else {
                    let a = (p[0] & 31) as usize;
                    if a == 31 {
                        g.stat |= 0x8000;
                    }
                    g.mtx_pos = g.stk_pos[a];
                    g.mtx_vec = g.stk_vec[a];
                    g.clip_dirty = true;
                }
            }
            0x15 => self.gx.on_current(true, |m| *m = IDENTITY),
            0x16 => {
                let q: Vec<i32> = p[..16].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| m.copy_from_slice(&q));
            }
            0x17 => {
                let q: Vec<i32> = p[..12].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| {
                    for r in 0..4 {
                        for c in 0..3 {
                            m[r * 4 + c] = q[r * 3 + c];
                        }
                        m[r * 4 + 3] = if r == 3 { 0x1000 } else { 0 };
                    }
                });
            }
            0x18 => {
                let mut n: Mtx = [0; 16];
                for i in 0..16 {
                    n[i] = p[i] as i32;
                }
                self.gx.on_current(true, |m| mult44(m, &n));
            }
            0x19 => {
                let q: Vec<i32> = p[..12].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| mult43(m, &q));
            }
            0x1A => {
                let q: Vec<i32> = p[..9].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| mult33(m, &q));
            }
            0x1B => {
                let q: Vec<i32> = p[..3].iter().map(|x| *x as i32).collect();
                self.gx.on_current(false, |m| scale(m, &q));
            }
            0x1C => {
                let q: Vec<i32> = p[..3].iter().map(|x| *x as i32).collect();
                self.gx.on_current(true, |m| translate(m, &q));
            }
            0x20 => {
                let g = &mut self.gx;
                g.vcolor = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
            }
            0x21 => {
                // NORMAL
                let g = &mut self.gx;
                for i in 0..3 {
                    g.normal[i] = (((p[0] >> (10 * i) << 22) as i32) >> 22) as i16;
                }
                if g.tex_param >> 30 == 2 {
                    let m = g.mtx_tex;
                    for i in 0..2 {
                        g.tc[i] = ((((g.normal[0] as i64) << 3) * m[i] as i64
                            + ((g.normal[1] as i64) << 3) * m[4 + i] as i64
                            + ((g.normal[2] as i64) << 3) * m[8 + i] as i64
                            + ((g.raw_tc[i] as i64) << 24))
                            >> 24) as i16;
                    }
                }
                g.lighting();
            }
            0x22 => {
                // TEXCOORD
                let g = &mut self.gx;
                g.raw_tc = [p[0] as i16, (p[0] >> 16) as i16];
                if g.tex_param >> 30 == 1 {
                    let m = g.mtx_tex;
                    for i in 0..2 {
                        g.tc[i] = ((g.raw_tc[0] as i64 * m[i] as i64
                            + g.raw_tc[1] as i64 * m[4 + i] as i64
                            + m[8 + i] as i64
                            + m[12 + i] as i64)
                            >> 12) as i16;
                    }
                } else {
                    g.tc = g.raw_tc;
                }
            }
            0x23 => {
                self.gx.vtx = [p[0] as i16, (p[0] >> 16) as i16, p[1] as i16];
                self.gx_add_vertex();
            }
            0x24 => {
                for i in 0..3 {
                    self.gx.vtx[i] = ((((p[0] >> (10 * i)) & 0x3FF) << 6) as u16) as i16;
                }
                self.gx_add_vertex();
            }
            0x25 => {
                self.gx.vtx[0] = p[0] as i16;
                self.gx.vtx[1] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x26 => {
                self.gx.vtx[0] = p[0] as i16;
                self.gx.vtx[2] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x27 => {
                self.gx.vtx[1] = p[0] as i16;
                self.gx.vtx[2] = (p[0] >> 16) as i16;
                self.gx_add_vertex();
            }
            0x28 => {
                for i in 0..3 {
                    let d = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                    self.gx.vtx[i] = (self.gx.vtx[i] as i32 + d) as i16;
                }
                self.gx_add_vertex();
            }
            0x29 => self.gx.attr_pending = p[0],
            0x2A => self.gx.tex_param = p[0],
            0x2B => self.gx.pltt_base = p[0] & 0x1FFF,
            0x30 => {
                let g = &mut self.gx;
                g.diffuse = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
                g.ambient = [((p[0] >> 16) & 31) as u8, ((p[0] >> 21) & 31) as u8, ((p[0] >> 26) & 31) as u8];
                if p[0] & 0x8000 != 0 {
                    g.vcolor = g.diffuse;
                }
            }
            0x31 => {
                let g = &mut self.gx;
                g.specular = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
                g.emission = [((p[0] >> 16) & 31) as u8, ((p[0] >> 21) & 31) as u8, ((p[0] >> 26) & 31) as u8];
                g.shine_table_on = p[0] & 0x8000 != 0;
            }
            0x32 => {
                // LIGHT_VECTOR
                let g = &mut self.gx;
                let l = (p[0] >> 30) as usize;
                let mut d = [0i32; 3];
                for i in 0..3 {
                    d[i] = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                }
                let m = g.mtx_vec;
                for i in 0..3 {
                    g.light_vec[l][i] = ((d[0] as i64 * m[i] as i64 + d[1] as i64 * m[4 + i] as i64 + d[2] as i64 * m[8 + i] as i64) >> 12) as i32;
                }
            }
            0x33 => {
                let g = &mut self.gx;
                let l = (p[0] >> 30) as usize;
                g.light_col[l] = [(p[0] & 31) as u8, ((p[0] >> 5) & 31) as u8, ((p[0] >> 10) & 31) as u8];
            }
            0x34 => {
                for i in 0..32 {
                    for k in 0..4 {
                        self.gx.shine_table[i * 4 + k] = (p[i] >> (8 * k)) as u8;
                    }
                }
            }
            0x40 => {
                // BEGIN_VTXS
                let g = &mut self.gx;
                g.prim = (p[0] & 3) as i32;
                g.attr = g.attr_pending;
                g.nstrip = 0;
                g.strip_count = 0;
            }
            0x41 => {} // END_VTXS: nothing on the DS either
            0x50 => {
                // SWAP_BUFFERS: at the VBlank
                self.gx.swap_pending = true;
                self.gx.swap_param = p[0];
            }
            0x60 => {
                // VIEWPORT (y counted from the bottom)
                let (x1, y1, x2, y2) = (p[0] as u8, (p[0] >> 8) as u8, (p[0] >> 16) as u8, (p[0] >> 24) as u8);
                self.gx.viewport = [x1, 191u8.wrapping_sub(y2), x2, 191u8.wrapping_sub(y1)];
            }
            0x70 => self.gx_box_test(p),
            0x71 => {
                // POS_TEST
                let g = &mut self.gx;
                g.vtx = [p[0] as i16, (p[0] >> 16) as i16, p[1] as i16];
                g.update_clip();
                let m = g.mtx_clip;
                for i in 0..4 {
                    g.pos_result[i] = ((g.vtx[0] as i64 * m[i] as i64 + g.vtx[1] as i64 * m[4 + i] as i64 + g.vtx[2] as i64 * m[8 + i] as i64 + 0x1000 * m[12 + i] as i64) >> 12) as i32;
                }
            }
            0x72 => {
                // VEC_TEST
                let g = &mut self.gx;
                let mut n = [0i32; 3];
                for i in 0..3 {
                    n[i] = ((p[0] >> (10 * i) << 22) as i32) >> 22;
                }
                let m = g.mtx_vec;
                for i in 0..3 {
                    let r = ((n[0] as i64 * m[i] as i64 + n[1] as i64 * m[4 + i] as i64 + n[2] as i64 * m[8 + i] as i64) >> 9) as i32;
                    g.vec_result[i] = (if r & 0x1000 != 0 { r | 0xF000 } else { r & 0x0FFF }) as i16;
                }
            }
            _ => self.once("gx cmd", cmd, || format!("3D: command {:02X} not known", cmd)),
        }
    }

    /// A whole command: now, or behind a swap.
    fn gx_command(&mut self, cmd: u32, p: &[u32]) {
        if self.gx.swap_pending {
            let n = if cmd < 0x80 { PARAMS[cmd as usize] as usize } else { 0 };
            self.gx.queue.push_back(Queued { cmd, p: p[..n].to_vec() });
            return;
        }
        self.gx_exec(cmd, p);
    }

    fn gx_fifo_word(&mut self, v: u32) {
        if self.gx.cur >= 0 {
            // a parameter of the command being gathered
            let g = &mut self.gx;
            g.params[g.got] = v;
            g.got += 1;
            if g.got < g.need {
                return;
            }
            let (cmd, params) = (g.cur as u32, g.params);
            g.cur = -1;
            self.gx_command(cmd, &params);
        } else {
            self.gx.cmds = v;
            self.gx.ncmds = 4;
        }
        // the packed commands that need no parameters run now; the next one with parameters waits
        while self.gx.ncmds > 0 {
            let c = self.gx.cmds & 0xFF;
            self.gx.cmds >>= 8;
            self.gx.ncmds -= 1;
            if !known(c) {
                self.once("gx cmd", c, || format!("3D: packed command {:02X} not known", c));
            }
            let need = if c < 0x80 { PARAMS[c as usize] as usize } else { 0 };
            if need == 0 {
                if c != 0 {
                    let params = self.gx.params;
                    self.gx_command(c, &params);
                }
                if self.gx.cmds == 0 {
                    self.gx.ncmds = 0; // only NOPs left in the word
                }
                continue;
            }
            self.gx.cur = c as i32;
            self.gx.need = need;
            self.gx.got = 0;
            return;
        }
    }

    fn gx_port_write(&mut self, cmd: u32, v: u32) {
        let need = PARAMS[cmd as usize & 0x7F] as usize;
        let c = cmd as usize & 0x7F;
        if need == 0 {
            let p = self.gx.port_params[c];
            return self.gx_command(cmd, &p);
        }
        let g = &mut self.gx;
        g.port_params[c][g.port_got[c]] = v;
        g.port_got[c] += 1;
        if g.port_got[c] < need {
            return;
        }
        g.port_got[c] = 0;
        let p = g.port_params[c];
        self.gx_command(cmd, &p);
    }

    // ---- the registers -----------------------------------------------------------------------
    fn gxstat(&self) -> u32 {
        let g = &self.gx;
        let mut v = g.stat & 0xC000_8002;
        v |= (g.sp_pos & 31) << 8 | (g.sp_proj & 1) << 13;
        let n = g.queue.len().min(256) as u32;
        v |= n << 16;
        if n < 128 {
            v |= 1 << 25;
        }
        if n == 0 {
            v |= 1 << 26;
        }
        if g.swap_pending {
            v |= 1 << 27;
        }
        v
    }

    fn gx_fifo_irq(&mut self) {
        let (m, n) = (self.gx.stat >> 30, self.gx.queue.len());
        if (m == 1 && n < 128) || (m == 2 && n == 0) {
            self.irq_raise(9, 21);
        }
    }

    pub(super) fn gx_write(&mut self, a: u32, v: u32, size: usize) {
        let off = a - 0x0400_0320;
        if (0x0400_0400..0x0400_0440).contains(&a) {
            if size != 4 {
                self.once("gx", 2, || format!("3D: a {}-byte write to GXFIFO", size));
            }
            return self.gx_fifo_word(v);
        }
        if (0x0400_0440..0x0400_0600).contains(&a) {
            if size != 4 {
                self.once("gx", 3, || format!("3D: a {}-byte write to a command port", size));
            }
            return self.gx_port_write((a - 0x0400_0400) >> 2, v);
        }
        if a == 0x0400_0600 || a == 0x0400_0602 {
            // GXSTAT: the FIFO interrupt's mode, the error bit acknowledged
            let w = if a == 0x0400_0602 && size == 2 { v << 16 } else { v };
            if size == 4 || a == 0x0400_0602 {
                self.gx.stat = (self.gx.stat & !0xC000_0000) | (w & 0xC000_0000);
            }
            if (size == 4 || a == 0x0400_0600) && w & 0x8000 != 0 {
                self.gx.stat &= !0x8000;
                self.gx.sp_proj = 0;
                self.gx.sp_tex = 0;
            }
            return self.gx_fifo_irq();
        }
        let o = off as usize;
        if o + size <= self.gx.regs.len() {
            for i in 0..size {
                self.gx.regs[o + i] = (v >> (8 * i)) as u8;
            }
        }
    }

    pub(super) fn gx_read(&mut self, a: u32, _size: usize) -> u32 {
        let off = a & !3;
        let g = &mut self.gx;
        if off == 0x0400_0600 {
            return self.gxstat();
        }
        if off == 0x0400_0604 {
            return g.build.polys.len() as u32 | g.build.nverts_ds << 16;
        }
        if (0x0400_0620..0x0400_0630).contains(&off) {
            return g.pos_result[((off - 0x0400_0620) >> 2) as usize] as u32;
        }
        if (0x0400_0630..0x0400_0636).contains(&off) {
            let i = ((off - 0x0400_0630) >> 1) as usize;
            return g.vec_result[i] as u16 as u32 | if i < 2 { (g.vec_result[i + 1] as u16 as u32) << 16 } else { 0 };
        }
        if (0x0400_0640..0x0400_0680).contains(&off) {
            g.update_clip();
            return g.mtx_clip[((off - 0x0400_0640) >> 2) as usize] as u32;
        }
        if (0x0400_0680..0x0400_06A4).contains(&off) {
            let i = ((off - 0x0400_0680) >> 2) as usize;
            return g.mtx_vec[(i / 3) * 4 + i % 3] as u32;
        }
        let o = (off - 0x0400_0320) as usize;
        if o + 4 <= g.regs.len() {
            return u32::from_le_bytes([g.regs[o], g.regs[o + 1], g.regs[o + 2], g.regs[o + 3]]);
        }
        0
    }

    // ---- the swap, and the rendering engine's job ----------------------------------------------
    /// At the VBlank: a SWAP_BUFFERS waiting sends the lists to the rendering engine and the
    /// queue behind it runs.
    pub(super) fn gx_vblank(&mut self) {
        if !self.gx.swap_pending {
            return;
        }
        let g = &mut self.gx;
        g.swap_pending = false;
        std::mem::swap(&mut g.render, &mut g.build);
        g.build = Lists::default();
        g.render_manual = g.swap_param & 1 != 0;
        g.render_wbuf = g.swap_param & 2 != 0;
        g.swaps += 1;
        while !self.gx.queue.is_empty() && !self.gx.swap_pending {
            let q = self.gx.queue.pop_front().unwrap();
            self.gx_exec(q.cmd, &q.p);
        }
        self.gx_fifo_irq();
    }

    pub(super) fn gx_line(&mut self) {
        self.gx_fifo_irq();
    }

    fn gx_r16(&self, a: u32) -> u16 {
        let o = (a - 0x0400_0320) as usize;
        u16::from_le_bytes([self.gx.regs[o], self.gx.regs[o + 1]])
    }

    /// The rendering engine starts a frame: its job is taken (lists, 3D registers, texture
    /// memory) and drawn, to be shown in the next frame.
    pub(super) fn gx_render_start(&mut self) {
        if !self.gpu.on {
            return; // a headless run with no picture of the next frame
        }
        let g = &self.gx;
        let mut polys: Vec<GxPoly> = g.render.polys.iter().filter(|p| !p.translucent).copied().collect();
        let nopaque = polys.len();
        polys.extend(g.render.polys.iter().filter(|p| p.translucent).copied());
        // the order of drawing: opaque polygons, then translucent ones; each sorted by y unless
        // SWAP_BUFFERS asked for the translucent ones in their own order
        polys[..nopaque].sort_by_key(|p| p.sortkey);
        if !g.render_manual {
            polys[nopaque..].sort_by_key(|p| p.sortkey);
        }
        let mut fog_table = [0u8; 32];
        fog_table.copy_from_slice(&g.regs[0x360 - 0x320..0x380 - 0x320]);
        let mut toon = [0u16; 32];
        for (i, t) in toon.iter_mut().enumerate() {
            *t = self.gx_r16(0x0400_0380 + 2 * i as u32);
        }
        let mut edge = [0u16; 8];
        for (i, e) in edge.iter_mut().enumerate() {
            *e = self.gx_r16(0x0400_0330 + 2 * i as u32);
        }
        let (tex_mem, texpal_mem) = (self.vram_copy(super::video::VRAM_TEX, 0x80000), self.vram_copy(super::video::VRAM_TEXPAL, 0x18000));
        let texture_on = self.io_reg16(9, 0x0400_0060) & 1 != 0;
        let keys = self.texture_keys(&polys, &super::render3d::Mem { tex: &tex_mem, pal: &texpal_mem }, texture_on);
        let g = &self.gx;
        let reg32 = |o: usize| u32::from_le_bytes([g.regs[o], g.regs[o + 1], g.regs[o + 2], g.regs[o + 3]]);
        let job = Job {
            verts: g.render.verts.clone(),
            polys,
            wbuffer: g.render_wbuf,
            disp3dcnt: self.io_reg16(9, 0x0400_0060),
            width: 256 + 2 * g.wide,
            clear_color: reg32(0x350 - 0x320),
            clear_depth: self.gx_r16(0x0400_0354) & 0x7FFF,
            clrimage_offset: self.gx_r16(0x0400_0356),
            alpha_ref: g.regs[0x340 - 0x320] & 31,
            extras: self.extras,
            edge,
            fog_color: reg32(0x358 - 0x320),
            fog_offset: self.gx_r16(0x0400_035C),
            fog_table,
            toon,
            tex: tex_mem,
            texpal: texpal_mem,
            keys,
            pack: self.texpack.pack.clone(),
        };
        if std::env::var_os("SUNPETAL_3D_DEBUG").is_some() {
            eprintln!("3D frame: {} polys, wbuf {}, disp3dcnt {:04X}, clear {:08X}", job.polys.len(), job.wbuffer, job.disp3dcnt, job.clear_color);
            for p in job.polys.iter().take(std::env::var("SUNPETAL_3D_DEBUG").ok().and_then(|v| v.parse().ok()).unwrap_or(6)) {
                eprintln!("  poly attr {:08X} tex {:08X} n {}", p.attr, p.tex, p.n);
                for v in &job.verts[p.first as usize..(p.first + p.n) as usize] {
                    eprintln!("    clip {} {} {} {}  scr {:.1} {:.1}  rgb {:.1} {:.1} {:.1}  st {:.1} {:.1}", v.x, v.y, v.z, v.w, v.sx, v.sy, v.r, v.g, v.b, v.s, v.t);
                }
            }
        }
        let (px, notes) = super::render3d::render(&job, self.scale);
        self.frame3d_wide = (job.width - 256) / 2;
        self.frame3d_scale = self.scale;
        self.frame3d = px;
        for (s, text) in super::render3d::NOTE.iter().enumerate() {
            if notes >> s & 1 != 0 {
                self.once("3d", s as u32, || text.to_string());
            }
        }
    }
}
