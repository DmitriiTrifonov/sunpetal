//! The GPU: the two screens as separate 256x192 textures, drawn as quads wherever the layout puts
//! them, at the window's resolution (nearest or linear filtering); and the settings menu over them.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::effects::Effects;
use crate::layout::Layout;

const SHADER: &str = r#"
struct Quad {
    rect: vec4<f32>,   // x0, y0, x1, y1 in clip space
    fx: vec4<f32>,     // scanlines, grid, vignette, sharp
    color: vec4<f32>,  // saturation, contrast, brightness, gamma
    dims: vec4<f32>,   // the texture's width and height, the window's pixels per DS pixel
};
@group(0) @binding(0) var<uniform> q: Quad;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    // a strip: (0,0) (1,0) (0,1) (1,1), the texture's top row at the rect's top (y0)
    let c = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    var o: Out;
    o.pos = vec4<f32>(mix(q.rect.x, q.rect.z, c.x), mix(q.rect.y, q.rect.w, c.y), 0.0, 1.0);
    o.uv = c;
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
    var c = textureSample(tex, samp, uv).rgb;
    // the DS's own pixels and lines, only where the window has room to show them
    let room = clamp(q.dims.z - 1.0, 0.0, 1.0);
    let fr = fract(in.uv * vec2<f32>(256.0, 192.0));
    let line = 0.5 + 0.5 * cos((fr.y - 0.5) * 6.2831853);
    c = c * (1.0 - q.fx.x * room * (1.0 - line));
    let e = min(min(fr.x, 1.0 - fr.x), min(fr.y, 1.0 - fr.y));
    c = c * (1.0 - q.fx.y * room * (1.0 - smoothstep(0.0, 0.3, e)));
    let d = in.uv - vec2<f32>(0.5);
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
}

struct Screen {
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_nearest: wgpu::BindGroup,
    bind_linear: wgpu::BindGroup,
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
    screens: [Screen; 2],
    pub linear: bool,
    /// Pixels kept free at the bottom of the window (the touch controls' band).
    pub reserve: f32,
}

fn make_screen(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    nearest: &wgpu::Sampler,
    linear: &wgpu::Sampler,
    scale: usize,
    label: &str,
) -> Screen {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: 256 * scale as u32, height: 192 * scale as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: std::mem::size_of::<Quad>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = |s: &wgpu::Sampler| {
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
    Screen { bind_nearest: bind(nearest), bind_linear: bind(linear), texture, uniform }
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
        Ok(Gfx { window: w, egui_ctx, egui_state, egui_renderer, bgl, nearest, linear_sampler: linear, scale: 1, surface, device, queue, config, pipeline, screens, linear: true, reserve: 0.0 })
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
        if scale != self.scale {
            self.scale = scale;
            self.screens = [
                make_screen(&self.device, &self.bgl, &self.nearest, &self.linear_sampler, scale, "upper"),
                make_screen(&self.device, &self.bgl, &self.nearest, &self.linear_sampler, scale, "lower"),
            ];
        }
        let (w, h) = (256 * scale as u32, 192 * scale as u32);
        let n = (w * h * 4) as usize;
        for (s, data) in self.screens.iter().zip(rgba.chunks_exact(n)) {
            self.queue.write_texture(
                s.texture.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
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
        for (s, r) in self.screens.iter().zip(rects.iter()) {
            if let Some(r) = r {
                let q = Quad {
                    rect: [r.x / w * 2.0 - 1.0, 1.0 - r.y / h * 2.0, (r.x + r.w) / w * 2.0 - 1.0, 1.0 - (r.y + r.h) / h * 2.0],
                    fx: [fx.scanlines, fx.grid, fx.vignette, if fx.sharp { 1.0 } else { 0.0 }],
                    color: [fx.saturation, fx.contrast, fx.brightness, fx.gamma],
                    dims: [256.0 * self.scale as f32, 192.0 * self.scale as f32, r.w / 256.0, 0.0],
                };
                self.queue.write_buffer(&s.uniform, 0, bytemuck::bytes_of(&q));
            }
        }
        let view = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (s, r) in self.screens.iter().zip(rects.iter()) {
                if r.is_some() {
                    pass.set_bind_group(0, if self.linear { &s.bind_linear } else { &s.bind_nearest }, &[]);
                    pass.draw(0..4, 0..1);
                }
            }
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
