//! The GPU: the two screens as separate 256x192 textures, drawn as quads wherever the layout puts
//! them, at the window's resolution (nearest or linear filtering); and the settings menu over them.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::effects::Effects;
use crate::layout::Layout;
use crate::post;

/// The glow's size: 16 across, 12 rows for each screen.
const GLOW_W: usize = 16;
const GLOW_H: usize = 24;

const SHADER: &str = r#"
struct Quad {
    rect: vec4<f32>,   // x0, y0, x1, y1 in clip space
    fx: vec4<f32>,     // scanlines, grid, vignette, sharp
    color: vec4<f32>,  // saturation, contrast, brightness, gamma
    dims: vec4<f32>,   // the texture's width and height, the window's pixels per DS pixel
    uvr: vec4<f32>,    // the part of the texture shown: u0, v0, u1, v1
};
@group(0) @binding(0) var<uniform> q: Quad;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) loc: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    // a strip: (0,0) (1,0) (0,1) (1,1), the texture's top row at the rect's top (y0)
    let c = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    var o: Out;
    o.pos = vec4<f32>(mix(q.rect.x, q.rect.z, c.x), mix(q.rect.y, q.rect.w, c.y), 0.0, 1.0);
    o.uv = mix(q.uvr.xy, q.uvr.zw, c);
    o.loc = c;
    return o;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let ts = q.dims.xy;
    var uv = in.uv;
    if (q.fx.w > 0.5) {
        // sharp pixels: nearest in the middle of a texel, a linear blend only over the edge
        let p = uv * ts;
        let pi = floor(p);
        let w = max(fwidth(p), vec2<f32>(0.0001));
        uv = (pi + 0.5 + clamp((p - pi - 0.5) / w, vec2<f32>(-0.5), vec2<f32>(0.5))) / ts;
    }
    if (q.dims.w > 0.5) {
        // a round patch: the circle in the rectangle only
        let r = (in.loc - vec2<f32>(0.5)) * 2.0;
        if (dot(r, r) > 1.0) {
            discard;
        }
    }
    var c = textureSample(tex, samp, uv).rgb;
    // the DS's own pixels and lines, only where the window has room to show them
    let room = clamp(q.dims.z - 1.0, 0.0, 1.0);
    let fr = fract(in.uv * ts / (ts.y / 192.0));
    let line = 0.5 + 0.5 * cos((fr.y - 0.5) * 6.2831853);
    c = c * (1.0 - q.fx.x * room * (1.0 - line));
    let e = min(min(fr.x, 1.0 - fr.x), min(fr.y, 1.0 - fr.y));
    c = c * (1.0 - q.fx.y * room * (1.0 - smoothstep(0.0, 0.3, e)));
    let d = in.loc - vec2<f32>(0.5);
    c = c * (1.0 - q.fx.z * dot(d, d) * 2.0);
    let luma = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    c = mix(vec3<f32>(luma), c, q.color.x);
    c = (c - vec3<f32>(0.5)) * q.color.y + vec3<f32>(0.5 + q.color.z);
    c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / q.color.w));
    return vec4<f32>(c, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Quad {
    rect: [f32; 4],
    fx: [f32; 4],
    color: [f32; 4],
    dims: [f32; 4],
    uvr: [f32; 4],
}

/// The whole texture.
const WHOLE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// What a layout's patches may be at most (one set of uniforms each).
const MAX_PATCHES: usize = 4;

/// Another quad of a screen's texture (a patch: a part of it drawn elsewhere).
struct Extra {
    uniform: wgpu::Buffer,
    bind_nearest: wgpu::BindGroup,
    bind_linear: wgpu::BindGroup,
}

struct Screen {
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_nearest: wgpu::BindGroup,
    bind_linear: wgpu::BindGroup,
    extras: Vec<Extra>,
}

pub struct Gfx {
    window: Arc<Window>,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    bgl: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    /// The screens' textures are this many times 256x192.
    scale: usize,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    post_kit: post::Kit,
    /// Made when bloom or FXAA is first on, again when the window's size changes.
    post: Option<post::Targets>,
    screens: [Screen; 2],
    /// A small picture of the screens' colours, stretched over the window behind them.
    glow: Screen,
    glow_grid: Vec<f32>,
    /// Draw the glow behind the screens (else black).
    pub background: bool,
    /// Behind a lone screen the window's bars carry its edge rows on (the texture drawn over the whole window, its
    /// edge pixels held), instead of the glow.
    pub edge_bars: bool,
    /// The colour the picture's edges all have (a logo on white): the bars take it, no glow.
    pub bar_color: Option<[f32; 3]>,
    /// The widened upper screen (3D with the view opened sideways), its scale and how many pixels
    /// it has beyond the 256 on each side.
    wide_screen: Option<(Screen, usize, usize)>,
    /// The upper screen is drawn from `wide_screen` (set by `upload_wide`, cleared by `upload`).
    pub wide_e: usize,
    pub linear: bool,
    /// Pixels kept free at the bottom of the window (the touch controls' band).
    pub reserve: f32,
}

/// The index of a patch's uniform, for the drawing pass.
struct Extra2(usize);

fn make_screen(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    nearest: &wgpu::Sampler,
    linear: &wgpu::Sampler,
    scale: usize,
    label: &str,
) -> Screen {
    make_texture(device, bgl, nearest, linear, 256 * scale as u32, 192 * scale as u32, label)
}

fn make_texture(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    nearest: &wgpu::Sampler,
    linear: &wgpu::Sampler,
    width: u32,
    height: u32,
    label: &str,
) -> Screen {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let buffer = || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Quad>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let bind = |uniform: &wgpu::Buffer, s: &wgpu::Sampler| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(s) },
            ],
        })
    };
    let uniform = buffer();
    let extras = (0..MAX_PATCHES + 1)
        .map(|_| {
            let u = buffer();
            Extra { bind_nearest: bind(&u, nearest), bind_linear: bind(&u, linear), uniform: u }
        })
        .collect();
    Screen { bind_nearest: bind(&uniform, nearest), bind_linear: bind(&uniform, linear), texture, uniform, extras }
}

impl Gfx {
    pub fn new(window: Arc<Window>) -> Result<Gfx, String> {
        let size = window.inner_size();
        let w = window.clone();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU: {}", e))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| e.to_string())?;
        let caps = surface.get_capabilities(&adapter);
        // the picture's colours are already what the screen shows: a format that does not convert
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screens"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let entries = |_: ()| {
            [
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ]
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries(()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screens"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |f: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: f, min_filter: f, ..Default::default() })
        };
        let (nearest, linear) = (sampler(wgpu::FilterMode::Nearest), sampler(wgpu::FilterMode::Linear));
        let screens = [make_screen(&device, &bgl, &nearest, &linear, 1, "upper"), make_screen(&device, &bgl, &nearest, &linear, 1, "lower")];
        let glow = make_texture(&device, &bgl, &nearest, &linear, GLOW_W as u32, GLOW_H as u32, "glow");
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*w,
            Some(w.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let post_kit = post::Kit::new(&device, format);
        Ok(Gfx { window: w, egui_ctx, egui_state, egui_renderer, bgl, nearest, linear_sampler: linear, scale: 1, surface, device, queue, config, pipeline, post_kit, post: None, screens, glow, glow_grid: vec![0.0; GLOW_W * GLOW_H * 3], background: true, edge_bars: false, bar_color: None, wide_screen: None, wide_e: 0, linear: true, reserve: 0.0 })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    /// The window's size in pixels.
    pub fn window_size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    /// The size of the part of the window the screens are fitted into.
    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, (self.config.height as f32 - self.reserve).max(1.0))
    }

    /// Uploads a picture at `scale` (both screens, RGBA, the upper first); the textures are made
    /// again when the scale changes.
    pub fn upload(&mut self, scale: usize, rgba: &[u8]) {
        self.wide_e = 0;
        if scale != self.scale {
            self.scale = scale;
            self.screens = [
                make_screen(&self.device, &self.bgl, &self.nearest, &self.linear_sampler, scale, "upper"),
                make_screen(&self.device, &self.bgl, &self.nearest, &self.linear_sampler, scale, "lower"),
            ];
        }
        let (w, h) = (256 * scale as u32, 192 * scale as u32);
        let n = (w * h * 4) as usize;
        self.update_glow(scale, rgba);
        for (s, data) in self.screens.iter().zip(rgba.chunks_exact(n)) {
            self.queue.write_texture(
                s.texture.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
    }

    /// Uploads the widened upper screen (`(256 + 2 * e) * scale` by `192 * scale`, RGBA); the upper
    /// screen is drawn from it until the next `upload`.
    pub fn upload_wide(&mut self, scale: usize, e: usize, rgba: &[u8]) {
        let (w, h) = ((256 + 2 * e as u32) * scale as u32, 192 * scale as u32);
        if rgba.len() < (w * h * 4) as usize {
            return;
        }
        if self.wide_screen.as_ref().map_or(true, |(_, s, ee)| (*s, *ee) != (scale, e)) {
            let s = make_texture(&self.device, &self.bgl, &self.nearest, &self.linear_sampler, w, h, "upper wide");
            self.wide_screen = Some((s, scale, e));
        }
        if let Some((s, _, _)) = &self.wide_screen {
            self.queue.write_texture(
                s.texture.as_image_copy(),
                &rgba[..(w * h * 4) as usize],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            self.wide_e = e;
        }
    }

    /// The glow's colours: each cell the average of a block of the picture (sampled), moved towards
    /// the new value a step at a time so a flash does not flicker the background.
    fn update_glow(&mut self, scale: usize, rgba: &[u8]) {
        let (w, h) = (256 * scale, 192 * scale);
        let screen = w * h * 4;
        if rgba.len() < 2 * screen {
            return;
        }
        let (bw, bh) = (w / GLOW_W, h / (GLOW_H / 2));
        let step = (bw / 4).max(1);
        for cy in 0..GLOW_H {
            let (s, row) = (cy / (GLOW_H / 2), cy % (GLOW_H / 2));
            for cx in 0..GLOW_W {
                let mut acc = [0u32; 3];
                let mut cnt = 0u32;
                for y in (row * bh..(row + 1) * bh).step_by(step) {
                    for x in (cx * bw..(cx + 1) * bw).step_by(step) {
                        let o = s * screen + (y * w + x) * 4;
                        acc[0] += rgba[o] as u32;
                        acc[1] += rgba[o + 1] as u32;
                        acc[2] += rgba[o + 2] as u32;
                        cnt += 1;
                    }
                }
                for c in 0..3 {
                    let v = acc[c] as f32 / cnt.max(1) as f32;
                    let slot = &mut self.glow_grid[(cy * GLOW_W + cx) * 3 + c];
                    // a cut (a logo after a logo, a scene after a fade) takes the new colour at once; a flash eases in
                    *slot = if (v - *slot).abs() > 48.0 { v } else { *slot + (v - *slot) * 0.12 };
                }
            }
        }
        let mut bytes = Vec::with_capacity(GLOW_W * GLOW_H * 4);
        for p in self.glow_grid.chunks_exact(3) {
            bytes.extend_from_slice(&[p[0] as u8, p[1] as u8, p[2] as u8, 255]);
        }
        self.queue.write_texture(
            self.glow.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(GLOW_W as u32 * 4), rows_per_image: Some(GLOW_H as u32) },
            wgpu::Extent3d { width: GLOW_W as u32, height: GLOW_H as u32, depth_or_array_layers: 1 },
        );
    }

    /// A window event for the menu. True if the menu used it (the game should not).
    pub fn ui_event(&mut self, ev: &winit::event::WindowEvent) -> bool {
        self.egui_state.on_window_event(&self.window, ev).consumed
    }

    /// Draws the screens, then, with `menu`, the menu over them.
    pub fn draw(&mut self, layout: &Layout, fx: &Effects, menu: Option<&mut dyn FnMut(&egui::Context)>) {
        use wgpu::CurrentSurfaceTexture as T;
        let frame = match self.surface.get_current_texture() {
            T::Success(f) | T::Suboptimal(f) => f,
            T::Outdated | T::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let (w, h) = self.window_size();
        let rects = layout.fit(self.size());
        let wide = self.wide_screen.as_ref().filter(|_| self.wide_e > 0);
        let pick = |i: usize| -> &Screen { if i == 0 { wide.map_or(&self.screens[0], |w| &w.0) } else { &self.screens[1] } };
        let upper_w = 256.0 + 2.0 * if wide.is_some() { self.wide_e as f32 } else { 0.0 };
        for (i, r) in rects.iter().enumerate() {
            let s = pick(i);
            let cols = if i == 0 { upper_w } else { 256.0 };
            if let Some(r) = r {
                let q = Quad {
                    rect: [r.x / w * 2.0 - 1.0, 1.0 - r.y / h * 2.0, (r.x + r.w) / w * 2.0 - 1.0, 1.0 - (r.y + r.h) / h * 2.0],
                    fx: [fx.scanlines, fx.grid, fx.vignette, if fx.sharp { 1.0 } else { 0.0 }],
                    color: [fx.saturation, fx.contrast, fx.brightness, fx.gamma],
                    dims: [cols * self.scale as f32, 192.0 * self.scale as f32, r.w / cols, 0.0],
                    uvr: WHOLE,
                };
                self.queue.write_buffer(&s.uniform, 0, bytemuck::bytes_of(&q));
            }
        }
        // the patches: a part of a screen drawn over the layout, one uniform each
        let patches: Vec<(usize, Extra2)> = layout
            .patches
            .iter()
            .take(MAX_PATCHES)
            .enumerate()
            .map(|(i, p)| {
                let d = layout.fit_rect(self.size(), p.dst);
                let q = Quad {
                    rect: [d.x / w * 2.0 - 1.0, 1.0 - d.y / h * 2.0, (d.x + d.w) / w * 2.0 - 1.0, 1.0 - (d.y + d.h) / h * 2.0],
                    fx: [0.0, 0.0, 0.0, if fx.sharp { 1.0 } else { 0.0 }],
                    color: [fx.saturation, fx.contrast, fx.brightness, fx.gamma],
                    dims: [256.0 * self.scale as f32, 192.0 * self.scale as f32, 0.0, if p.round { 1.0 } else { 0.0 }],
                    uvr: [p.src.x / 256.0, p.src.y / 192.0, (p.src.x + p.src.w) / 256.0, (p.src.y + p.src.h) / 192.0],
                };
                self.queue.write_buffer(&self.screens[p.screen].extras[i].uniform, 0, bytemuck::bytes_of(&q));
                (p.screen, Extra2(i))
            })
            .collect();
        // a lone screen with bars about it: its own picture over the whole window, the edge pixels held
        let lone = match (&rects[0], &rects[1]) {
            (Some(r), None) => Some((0, *r)),
            (None, Some(r)) => Some((1, *r)),
            _ => None,
        };
        let edge = lone.filter(|(_, r)| self.edge_bars && self.bar_color.is_none() && (r.w < w - 0.5 || r.h < h - 0.5));
        if let Some((i, r)) = edge {
            let cols = if i == 0 { upper_w } else { 256.0 };
            let q = Quad {
                rect: [-1.0, 1.0, 1.0, -1.0],
                fx: [0.0, 0.0, 0.0, if fx.sharp { 1.0 } else { 0.0 }],
                color: [fx.saturation, fx.contrast, fx.brightness, fx.gamma],
                dims: [cols * self.scale as f32, 192.0 * self.scale as f32, 0.0, 0.0],
                uvr: [-r.x / r.w, -r.y / r.h, (w - r.x) / r.w, (h - r.y) / r.h],
            };
            self.queue.write_buffer(&pick(i).extras[MAX_PATCHES].uniform, 0, bytemuck::bytes_of(&q));
        }
        if self.background && self.bar_color.is_none() && edge.is_none() {
            // the whole window, dimmed, no screen effects
            let q = Quad { rect: [-1.0, 1.0, 1.0, -1.0], fx: [0.0, 0.0, 0.55, 0.0], color: [1.1, 0.9, -0.2, 1.0], dims: [GLOW_W as f32, GLOW_H as f32, 0.0, 0.0], uvr: WHOLE };
            self.queue.write_buffer(&self.glow.uniform, 0, bytemuck::bytes_of(&q));
        }
        let view = frame.texture.create_view(&Default::default());
        let use_post = fx.bloom > 0.0 || fx.fxaa;
        if !use_post {
            self.post = None;
        } else if self.post.as_ref().map_or(true, |p| p.size != (self.config.width, self.config.height)) {
            self.post = Some(post::Targets::new(&self.device, &self.post_kit, self.config.format, self.config.width, self.config.height));
        }
        let target = match &self.post {
            Some(p) => &p.scene,
            None => &view,
        };
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(self.bar_color.map_or(wgpu::Color::BLACK, |c| wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 })), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            if let Some((i, _)) = edge {
                let e = &pick(i).extras[MAX_PATCHES];
                pass.set_bind_group(0, if self.linear { &e.bind_linear } else { &e.bind_nearest }, &[]);
                pass.draw(0..4, 0..1);
            } else if self.background && self.bar_color.is_none() {
                pass.set_bind_group(0, &self.glow.bind_linear, &[]);
                pass.draw(0..4, 0..1);
            }
            for (i, r) in rects.iter().enumerate() {
                if r.is_some() {
                    let s = pick(i);
                    pass.set_bind_group(0, if self.linear { &s.bind_linear } else { &s.bind_nearest }, &[]);
                    pass.draw(0..4, 0..1);
                }
            }
            for (s, Extra2(i)) in &patches {
                let e = &self.screens[*s].extras[*i];
                pass.set_bind_group(0, if self.linear { &e.bind_linear } else { &e.bind_nearest }, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        if let Some(p) = &self.post {
            p.run(&self.post_kit, &self.queue, &mut enc, &view, fx.bloom, fx.fxaa);
        }
        let mut buffers = Vec::new();
        let mut freed = Vec::new();
        if let Some(menu) = menu {
            let raw = self.egui_state.take_egui_input(&self.window);
            let out = self.egui_ctx.run_pass(raw, |ctx| menu(ctx));
            self.egui_state.handle_platform_output(&self.window, out.platform_output);
            let jobs = self.egui_ctx.tessellate(out.shapes, out.pixels_per_point);
            for (id, deltas) in &out.textures_delta.set {
                for delta in deltas {
                    self.egui_renderer.update_texture(&self.device, &self.queue, *id, delta);
                }
            }
            let sd = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: out.pixels_per_point,
            };
            buffers = self.egui_renderer.update_buffers(&self.device, &self.queue, &mut enc, &jobs, &sd);
            {
                let mut pass = enc
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("menu"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    })
                    .forget_lifetime();
                self.egui_renderer.render(&mut pass, &jobs, &sd);
            }
            freed = out.textures_delta.free.iter().copied().collect();
        }
        buffers.push(enc.finish());
        self.queue.submit(buffers);
        for id in &freed {
            self.egui_renderer.free_texture(id);
        }
        self.queue.present(frame);
    }
}
