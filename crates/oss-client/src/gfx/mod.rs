//! GPU setup and the batched sprite renderer.
//!
//! Everything is drawn as tinted quads in physical pixels. Callers place sprites in
//! "game pixels" multiplied by an integer scale and snapped to whole physical pixels,
//! which keeps pixel art crisp at any window size or DPI while letting motion between
//! game pixels stay smooth at high scales.

pub mod atlas;

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

pub use atlas::{AtlasBuilder, Region};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    camera: [f32; 2],
    scale: f32,
    time: f32,
    _pad: [f32; 2],
}

/// Handle to a texture owned by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureId {
    slot: u32,
    /// Bumped every time the slot is freed, so a handle kept past
    /// [`Renderer::free_texture`] draws nothing instead of whatever took its place.
    generation: u32,
}

impl TextureId {
    /// A handle to no texture, for tests that don't have a GPU. Drawing with it is
    /// ignored, like any handle to a slot that was never filled.
    #[cfg(test)]
    pub fn for_test(slot: u32) -> Self {
        Self {
            slot,
            generation: 0,
        }
    }
}

struct GpuTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// One texture slot. Freed slots keep their generation and are handed out again by
/// [`Renderer::create_texture`].
struct Slot {
    generation: u32,
    texture: Option<GpuTexture>,
}

struct Batch {
    texture: TextureId,
    start: u32,
    count: u32,
}

/// Parameters for the starfield background.
#[derive(Debug, Clone, Copy)]
pub struct StarParams {
    /// Camera centre in game pixels.
    pub camera: [f32; 2],
    pub scale: f32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    sprite_pipeline: wgpu::RenderPipeline,
    star_pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<Slot>,
    /// Slots whose texture was freed, reused by the next `create_texture`.
    free_slots: Vec<usize>,
    vbuf: wgpu::Buffer,
    vbuf_capacity: usize,
    vertices: Vec<Vertex>,
    batches: Vec<Batch>,
    pub present_mode: wgpu::PresentMode,
    /// When set, the next rendered frame is also saved to this PNG file.
    pub capture: Option<std::path::PathBuf>,
    /// Draws egui windows (settings) over the game, created on first use.
    ui: Option<egui_wgpu::Renderer>,
    /// egui textures to free once the frame that last used them is done.
    ui_free: Vec<egui::TextureId>,
}

/// One frame of egui output to draw over the game.
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

#[derive(Debug)]
pub enum FrameError {
    /// Skip this frame (minimised, timeout); try again next frame.
    Skip,
    /// Unrecoverable surface/device failure.
    Fatal(String),
}

impl Renderer {
    pub fn new(window: Arc<Window>, vsync: bool) -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no suitable GPU adapter: {e}"))?;
        log::info!("GPU: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("oss-client"),
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("surface has no formats")?;
        let present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };
        log::info!("surface format {format:?}, present mode {present_mode:?}");
        let config = wgpu::SurfaceConfiguration {
            usage: if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let sprite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let star_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stars"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let sprite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite"),
            layout: Some(&sprite_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let star_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stars"),
            layout: Some(&star_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_stars"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vbuf_capacity = 16 * 1024;
        let vbuf = Self::make_vbuf(&device, vbuf_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            sprite_pipeline,
            star_pipeline,
            globals_buf,
            globals_bg,
            texture_layout,
            sampler,
            textures: Vec::new(),
            free_slots: Vec::new(),
            vbuf,
            vbuf_capacity,
            vertices: Vec::new(),
            batches: Vec::new(),
            present_mode,
            capture: None,
            ui: None,
            ui_free: Vec::new(),
        })
    }

    fn make_vbuf(device: &wgpu::Device, vertices: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite vertices"),
            size: (vertices * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Surface size in physical pixels.
    /// The device's largest texture side (for egui).
    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads an RGBA8 texture and returns its handle.
    pub fn create_texture(&mut self, width: u32, height: u32, rgba: &[u8]) -> TextureId {
        let t = self.make_texture(width, height);
        let slot = match self.free_slots.pop() {
            Some(slot) => {
                self.textures[slot].texture = Some(t);
                slot
            }
            None => {
                self.textures.push(Slot {
                    generation: 0,
                    texture: Some(t),
                });
                self.textures.len() - 1
            }
        };
        let id = TextureId {
            slot: slot as u32,
            generation: self.textures[slot].generation,
        };
        self.update_texture(id, rgba);
        id
    }

    /// Drops a texture's GPU memory and frees its slot for reuse. Handles to it stop
    /// drawing anything; the arena change that freed them drops them in the same
    /// breath.
    pub fn free_texture(&mut self, id: TextureId) {
        let Some(slot) = self.textures.get_mut(id.slot as usize) else {
            return;
        };
        if slot.generation != id.generation || slot.texture.is_none() {
            return;
        }
        slot.texture = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free_slots.push(id.slot as usize);
    }

    /// Replaces a texture with a new one of a different size, keeping its handle.
    pub fn recreate_texture(&mut self, id: TextureId, width: u32, height: u32, rgba: &[u8]) {
        let t = self.make_texture(width, height);
        let Some(slot) = self.textures.get_mut(id.slot as usize) else {
            return;
        };
        if slot.generation != id.generation {
            return;
        }
        slot.texture = Some(t);
        self.update_texture(id, rgba);
    }

    fn gpu(&self, id: TextureId) -> Option<&GpuTexture> {
        self.textures
            .get(id.slot as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.texture.as_ref())
    }

    fn make_texture(&self, width: u32, height: u32) -> GpuTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        GpuTexture {
            texture,
            bind_group,
            width,
            height,
        }
    }

    /// Replaces a texture's contents (same size).
    pub fn update_texture(&mut self, id: TextureId, rgba: &[u8]) {
        let Some(t) = self.gpu(id) else {
            return;
        };
        if rgba.len() != (t.width * t.height * 4) as usize {
            log::warn!("texture update with wrong size ignored");
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(t.width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Queues a quad at physical-pixel rectangle (x, y, w, h) showing `region` of
    /// `texture`, tinted by `color` (multiplied).
    #[allow(clippy::too_many_arguments)]
    pub fn quad(
        &mut self,
        texture: TextureId,
        region: Region,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    ) {
        let Some(t) = self.gpu(texture) else {
            return;
        };
        let (tw, th) = (t.width as f32, t.height as f32);
        let u0 = region.x as f32 / tw;
        let v0 = region.y as f32 / th;
        let u1 = (region.x + region.w) as f32 / tw;
        let v1 = (region.y + region.h) as f32 / th;
        let v = |px: f32, py: f32, u: f32, vv: f32| Vertex {
            pos: [px, py],
            uv: [u, vv],
            color,
        };
        let quad = [
            v(x, y, u0, v0),
            v(x + w, y, u1, v0),
            v(x, y + h, u0, v1),
            v(x + w, y, u1, v0),
            v(x + w, y + h, u1, v1),
            v(x, y + h, u0, v1),
        ];
        let start = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&quad);
        match self.batches.last_mut() {
            Some(b) if b.texture == texture && b.start + b.count == start => b.count += 6,
            _ => self.batches.push(Batch {
                texture,
                start,
                count: 6,
            }),
        }
    }

    /// Draws everything queued since the last frame, over a starfield (or black).
    pub fn render(
        &mut self,
        stars: Option<StarParams>,
        time: f32,
        mut ui: Option<UiFrame>,
    ) -> Result<(), FrameError> {
        // egui textures first, so a skipped frame doesn't lose them.
        if let Some(frame_ui) = ui.as_mut() {
            let (device, queue, format) = (&self.device, &self.queue, self.config.format);
            let r = self.ui.get_or_insert_with(|| {
                egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default())
            });
            for id in self.ui_free.drain(..) {
                r.free_texture(&id);
            }
            let mut delta = std::mem::take(&mut frame_ui.textures);
            for (id, deltas) in &delta.set {
                for d in deltas {
                    r.update_texture(device, queue, *id, d);
                }
            }
            self.ui_free.extend(delta.free.drain());
            delta.clear();
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.configure(&self.device, &self.config);
                f
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                log::debug!("frame skipped (timeout/occluded)");
                self.clear_queue();
                return Err(FrameError::Skip);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                log::debug!("frame skipped (surface outdated/lost)");
                self.surface.configure(&self.device, &self.config);
                self.clear_queue();
                return Err(FrameError::Skip);
            }
            other => {
                self.clear_queue();
                return Err(FrameError::Fatal(format!("surface error: {other:?}")));
            }
        };

        let globals = Globals {
            screen: [self.config.width as f32, self.config.height as f32],
            camera: stars.map(|s| s.camera).unwrap_or([0.0, 0.0]),
            scale: stars.map(|s| s.scale).unwrap_or(1.0),
            time,
            _pad: [0.0; 2],
        };
        self.queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        if self.vertices.len() > self.vbuf_capacity {
            self.vbuf_capacity = self.vertices.len().next_power_of_two();
            self.vbuf = Self::make_vbuf(&self.device, self.vbuf_capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.vertices));
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);
            if stars.is_some() {
                pass.set_pipeline(&self.star_pipeline);
                pass.draw(0..3, 0..1);
            }
            if !self.batches.is_empty() {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_vertex_buffer(0, self.vbuf.slice(..));
                for b in &self.batches {
                    if let Some(t) = self.gpu(b.texture) {
                        pass.set_bind_group(1, &t.bind_group, &[]);
                        pass.draw(b.start..b.start + b.count, 0..1);
                    }
                }
            }
        }
        let mut ui_buffers = Vec::new();
        if let Some(frame_ui) = ui {
            let (device, queue, format) = (&self.device, &self.queue, self.config.format);
            let r = self.ui.get_or_insert_with(|| {
                egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default())
            });
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point: frame_ui.pixels_per_point,
            };
            ui_buffers =
                r.update_buffers(device, queue, &mut encoder, &frame_ui.primitives, &screen);
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let mut pass = pass.forget_lifetime();
                r.render(&mut pass, &frame_ui.primitives, &screen);
            }
        }
        let readback = self.capture.take().and_then(|path| {
            if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
                log::warn!("screen capture unsupported on this surface");
                return None;
            }
            let (w, h) = (self.config.width, self.config.height);
            let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: row as u64 * h as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                frame.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            Some((path, buffer, row))
        });
        self.queue
            .submit(ui_buffers.into_iter().chain([encoder.finish()]));
        if let Some((path, buffer, row)) = readback {
            self.save_capture(&path, &buffer, row);
        }
        self.queue.present(frame);
        self.clear_queue();
        Ok(())
    }

    fn save_capture(&self, path: &std::path::Path, buffer: &wgpu::Buffer, row: u32) {
        let (w, h) = (self.config.width, self.config.height);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        // Never hang the game on a capture (e.g. if the display is asleep).
        let wait = wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(2)),
        };
        if let Err(e) = self.device.poll(wait) {
            log::warn!("capture failed: {e}");
            return;
        }
        let data = match buffer.get_mapped_range(..) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("capture failed: {e:?}");
                return;
            }
        };
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let line = &data[y * row as usize..y * row as usize + w as usize * 4];
            for px in line.as_chunks::<4>().0 {
                let [a, b, c, _] = *px;
                rgba.extend_from_slice(&if bgra { [c, b, a, 255] } else { [a, b, c, 255] });
            }
        }
        drop(data);
        let result = std::fs::File::create(path)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                let mut writer = enc.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(&rgba).map_err(|e| e.to_string())
            });
        match result {
            Ok(()) => log::info!("saved screenshot {}", path.display()),
            Err(e) => log::warn!("screenshot {}: {e}", path.display()),
        }
    }

    fn clear_queue(&mut self) {
        self.vertices.clear();
        self.batches.clear();
    }
}
