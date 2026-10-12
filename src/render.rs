//! Numeric GPU probes and a viewport with recovered free-fall pose updates.
//! Bevy owns/extracts draw components;
//! wgpu supplies Metal without modern PBR, tone mapping or automatic color-space conversion.
use crate::{
    bundle::Bundle,
    camera,
    geometry::{Draw, RenderScene, RenderVertex},
    motion::{self, MotionControl},
    runtime,
    scene::CoordinateFrame,
    surface::{self, SurfaceScene},
    texture::{self, PngTexture, TexturedDraw},
};
use bevy::prelude::*;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    fmt::Write as _,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

const SHADER: &str = include_str!("render.wgsl");
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.125,
    g: 0.15625,
    b: 0.1875,
    a: 1.0,
};

#[derive(Clone)]
pub struct RenderInputs {
    pub frame: CoordinateFrame,
    pub fov_y: f32,
    pub ambient: [f32; 3],
    pub lamp0: [f32; 3],
    pub lamp1: [f32; 3],
    pub lamp_direction: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_start: f32,
    pub fog_end: f32,
    pub light_grid: [f32; 3],
    pub shadow: f32,
}

impl RenderInputs {
    /// Explicit fixture camera/lighting, not guessed Studio defaults.
    pub fn character_probe() -> Self {
        // character3 has R00=R22=-1: its local -Z face points toward world +Z.
        let position = Vec3::new(8.0, 22.0, 35.0);
        let target = Vec3::new(0.0, 17.5, 22.5);
        let back = (position - target).normalize();
        let right = Vec3::Y.cross(back).normalize();
        let up = back.cross(right);
        Self {
            frame: CoordinateFrame {
                position: position.to_array(),
                rotation: [
                    [right.x, up.x, back.x],
                    [right.y, up.y, back.y],
                    [right.z, up.z, back.z],
                ],
            },
            fov_y: 70.0f32.to_radians(),
            ambient: [0.25; 3],
            lamp0: [0.65; 3],
            lamp1: [0.26; 3],
            lamp_direction: Vec3::new(-1.0, -2.0, -1.0).normalize().to_array(),
            fog_color: [0.125, 0.15625, 0.1875],
            fog_start: 0.0,
            fog_end: 1000.0,
            light_grid: [0.0; 3],
            shadow: 1.0,
        }
    }

    fn bytes(&self, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let vp = camera::multiply(
            camera::perspective(self.fov_y, width as f32 / height as f32, 0.5, 5000.0)?,
            camera::view(&self.frame),
        );
        let mut floats = Vec::new();
        for column in 0..4 {
            for row in &vp {
                floats.push(row[column]);
            }
        }
        for v in [
            self.frame.position,
            self.ambient,
            self.lamp0,
            self.lamp1,
            self.lamp_direction,
            self.fog_color,
        ] {
            floats.extend(v);
            floats.push(0.0);
        }
        floats.extend([
            0.0,
            self.fog_start,
            self.fog_end,
            if self.fog_start == self.fog_end {
                0.0
            } else {
                1.0 / (self.fog_end - self.fog_start)
            },
        ]);
        floats.extend(self.light_grid);
        floats.push(self.shadow);
        if floats.iter().any(|v| !v.is_finite()) {
            return Err("non-finite shader input".into());
        }
        Ok(floats.into_iter().flat_map(f32::to_le_bytes).collect())
    }
}

#[derive(Resource, Default)]
struct Extracted(Vec<Draw>);
#[derive(Resource, Default)]
struct ExtractedDecals(Vec<TexturedDraw>);
fn extract(
    draws: Query<&Draw>,
    decals: Query<&TexturedDraw>,
    mut output: ResMut<Extracted>,
    mut textures: ResMut<ExtractedDecals>,
) {
    output.0 = draws.iter().cloned().collect();
    textures.0 = decals.iter().cloned().collect();
}
fn scene_app(scene: RenderScene) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    for draw in scene.draws {
        app.world_mut().spawn(draw);
    }
    app.init_resource::<Extracted>();
    app.init_resource::<ExtractedDecals>();
    app.add_systems(Update, extract);
    app.update();
    app
}

fn install_motion(app: &mut App, bundle: &Bundle) -> Result<(), String> {
    let before = app.world().resource::<Extracted>().0.clone();
    motion::install(app, bundle)?;
    app.add_systems(
        Update,
        (motion::advance, motion::sync_bodies, motion::sync_decals)
            .chain()
            .before(extract),
    );
    app.update();
    if let Some(error) = &app.world().resource::<MotionControl>().error {
        return Err(error.clone());
    }
    let after = &app.world().resource::<Extracted>().0;
    for draw in before {
        let posed = after
            .iter()
            .find(|posed| posed.name == draw.name)
            .ok_or("pose setup lost original draw")?;
        if draw_bytes(std::slice::from_ref(&draw))? != draw_bytes(std::slice::from_ref(posed))? {
            return Err(format!("local geometry round trip changed {:?}", draw.name));
        }
    }
    Ok(())
}

struct Gpu {
    format: wgpu::TextureFormat,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bindings: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    topology: Vec<u8>,
    count: u32,
    info: String,
    decals: Option<DecalGpu>,
    studs: Option<StudGpu>,
}
struct StudGpu {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::BindGroup,
    bindings: wgpu::BindGroup,
}
#[derive(Clone, Copy, PartialEq)]
enum ShaderKind {
    Untextured,
    Plastic,
    Decal,
}
struct DecalGpu {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::BindGroup,
    draws: Vec<GpuTextureDraw>,
}
struct GpuTextureDraw {
    bindings: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    topology: Vec<u8>,
    count: u32,
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    kind: ShaderKind,
) -> wgpu::RenderPipeline {
    let decal = kind == ShaderKind::Decal;
    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4,4=>Float32x2,5=>Float32x2];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if decal {
            "recovered decal pass"
        } else {
            "static probe"
        }),
        layout: None,
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(!decal),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: if decal {
                wgpu::DepthBiasState {
                    constant: -16,
                    slope_scale: -0.5,
                    clamp: 0.0,
                }
            } else {
                Default::default()
            },
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(match kind {
                ShaderKind::Untextured => "fragment",
                ShaderKind::Plastic => "fragment_plastic",
                ShaderKind::Decal => "fragment_decal",
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: decal.then_some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Gpu {
    fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        format: wgpu::TextureFormat,
        draws: &[Draw],
    ) -> Result<(Self, wgpu::Adapter), String> {
        if draws
            .iter()
            .any(|draw| draw.material != 0x100 || draw.vertices.iter().any(|v| v.color[3] != 1.0))
        {
            return Err("static shader supports opaque Plastic only".into());
        }
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let info = format!("gpu_adapter {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Studio 2016 probe"),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("recovered low-quality equations"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline = pipeline(&device, &shader, format, ShaderKind::Untextured);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("explicit probe globals"),
            size: 192,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let (vertex_bytes, index_bytes, count) = draw_bytes(draws)?;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world vertices"),
            contents: &vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native topology"),
            contents: &index_bytes,
            usage: wgpu::BufferUsages::INDEX,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("GPU initialization: {e}"));
        }
        Ok((
            Self {
                format,
                device,
                queue,
                pipeline,
                uniform,
                bindings,
                vertices,
                indices,
                topology: index_bytes,
                count,
                info,
                decals: None,
                studs: None,
            },
            adapter,
        ))
    }

    fn texture_bindings(
        &self,
        pipeline: &wgpu::RenderPipeline,
        data: &PngTexture,
        sampler: &wgpu::Sampler,
    ) -> Result<wgpu::BindGroup, String> {
        let first = data.mips.first().ok_or("empty texture")?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("original PNG and native mips"),
            size: wgpu::Extent3d {
                width: first.width,
                height: first.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: data.mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in data.mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &mip.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip.width * 4),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Ok(bindings)
    }

    fn add_studs(&mut self, draws: &[Draw], surface: &SurfaceScene) -> Result<(), String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
            });
        let pipeline = pipeline(&self.device, &shader, self.format, ShaderKind::Plastic);
        let globals = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }],
        });
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("native low-quality studs repeat/trilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let bindings = self.texture_bindings(&pipeline, &surface.atlas, &sampler)?;
        let (vertices, _, count) = draw_bytes_with_studs(draws, Some(&surface.coordinates))?;
        if count != self.count {
            return Err("stud upload changed topology".into());
        }
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("native studs UVs"),
                contents: &vertices,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("stud upload: {error}"));
        }
        self.vertices = vertices;
        self.studs = Some(StudGpu {
            pipeline,
            globals,
            bindings,
        });
        Ok(())
    }

    fn add_decals(&mut self, draws: &[TexturedDraw]) -> Result<(), String> {
        if draws.is_empty() {
            return Ok(());
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
            });
        let pipeline = pipeline(&self.device, &shader, self.format, ShaderKind::Decal);
        let globals = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }],
        });
        // Native Decal flags 0x8: filter=2 (trilinear/anisotropic), address=1
        // (CLAMP_TO_EDGE). Anisotropy=1 is an explicit probe setting, not a
        // recovered Studio quality default.
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decal probe trilinear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let mut uploaded = Vec::new();
        for draw in draws {
            let bindings = self.texture_bindings(&pipeline, &draw.texture, &sampler)?;
            let (v, i, count) = draw_bytes(std::slice::from_ref(&draw.draw))?;
            let vertices = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: &v,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                });
            let indices = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: &i,
                    usage: wgpu::BufferUsages::INDEX,
                });
            uploaded.push(GpuTextureDraw {
                bindings,
                vertices,
                indices,
                topology: i,
                count,
            });
        }
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("decal upload: {error}"));
        }
        self.decals = Some(DecalGpu {
            pipeline,
            globals,
            draws: uploaded,
        });
        Ok(())
    }

    fn update_geometry(
        &self,
        draws: &[Draw],
        decals: &[TexturedDraw],
        surfaces: &SurfaceScene,
    ) -> Result<(), String> {
        let (vertices, indices, count) = draw_bytes_with_studs(draws, Some(&surfaces.coordinates))?;
        if count != self.count
            || indices != self.topology
            || vertices.len() as u64 != self.vertices.size()
        {
            return Err("pose update changed Part topology".into());
        }
        let uploaded = self.decals.as_ref().ok_or("motion requires decal upload")?;
        if uploaded.draws.len() != decals.len() {
            return Err("pose update changed decal count".into());
        }
        let mut decal_vertices = Vec::new();
        for (gpu, decal) in uploaded.draws.iter().zip(decals) {
            let (vertices, indices, count) = draw_bytes(std::slice::from_ref(&decal.draw))?;
            if count != gpu.count
                || indices != gpu.topology
                || vertices.len() as u64 != gpu.vertices.size()
            {
                return Err("pose update changed decal topology".into());
            }
            decal_vertices.push(vertices);
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.queue.write_buffer(&self.vertices, 0, &vertices);
        for (gpu, vertices) in uploaded.draws.iter().zip(decal_vertices) {
            self.queue.write_buffer(&gpu.vertices, 0, &vertices);
        }
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("pose vertex upload: {error}"));
        }
        Ok(())
    }

    fn texture(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }
    fn pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("static character pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if let Some(studs) = &self.studs {
            pass.set_pipeline(&studs.pipeline);
            pass.set_bind_group(0, &studs.globals, &[]);
            pass.set_bind_group(1, &studs.bindings, &[]);
        } else {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..1);
        if let Some(decals) = &self.decals {
            pass.set_pipeline(&decals.pipeline);
            pass.set_bind_group(0, &decals.globals, &[]);
            for draw in &decals.draws {
                pass.set_bind_group(1, &draw.bindings, &[]);
                pass.set_vertex_buffer(0, draw.vertices.slice(..));
                pass.set_index_buffer(draw.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..draw.count, 0, 0..1);
            }
        }
    }
    fn read_frame(
        &self,
        width: u32,
        height: u32,
        inputs: &RenderInputs,
    ) -> Result<Vec<u8>, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.queue
            .write_buffer(&self.uniform, 0, &inputs.bytes(width, height)?);
        let texture = self.texture(
            width,
            height,
            self.format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = self.texture(
            width,
            height,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("numeric readback"),
            size: row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.pass(
            &mut encoder,
            &texture.create_view(&Default::default()),
            &depth.create_view(&Default::default()),
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(15)),
            })
            .map_err(|e| e.to_string())?;
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let mut pixels: Vec<u8> = data
            .chunks_exact(row as usize)
            .flat_map(|r| r[..width as usize * 4].iter().copied())
            .collect();
        if self.format == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        drop(data);
        buffer.unmap();
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("GPU frame: {e}"));
        }
        Ok(pixels)
    }
}

fn draw_bytes(draws: &[Draw]) -> Result<(Vec<u8>, Vec<u8>, u32), String> {
    draw_bytes_with_studs(draws, None)
}
fn draw_bytes_with_studs(
    draws: &[Draw],
    coordinates: Option<&std::collections::BTreeMap<String, Vec<[f32; 2]>>>,
) -> Result<(Vec<u8>, Vec<u8>, u32), String> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut base = 0u32;
    for draw in draws {
        let uvs = coordinates
            .map(|map| map.get(&draw.name).ok_or("missing stud coordinates"))
            .transpose()?;
        if uvs.is_some_and(|uv| uv.len() != draw.vertices.len()) {
            return Err("stud coordinate count mismatch".into());
        }
        for (index, v) in draw.vertices.iter().enumerate() {
            for f in v
                .position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.color)
                .chain(&v.specular)
            {
                if !f.is_finite() {
                    return Err("non-finite vertex upload".into());
                }
                vertices.extend(f.to_le_bytes());
            }
            // Private adapter sentinel selects the recovered no-studs body
            // branch without changing public native geometry/UV structures.
            let studs = if matches!(
                draw.geometry.as_str(),
                "special_head" | "humanoid_body_mesh"
            ) {
                [-1.0; 2]
            } else {
                uvs.map_or([0.0; 2], |uvs| uvs[index])
            };
            for f in studs {
                if !f.is_finite() {
                    return Err("non-finite stud coordinate".into());
                }
                vertices.extend(f.to_le_bytes());
            }
        }
        for i in &draw.indices {
            if *i as usize >= draw.vertices.len() {
                return Err("invalid render index".into());
            }
            indices.extend(
                base.checked_add(*i)
                    .ok_or("render index overflow")?
                    .to_le_bytes(),
            );
        }
        base = base
            .checked_add(draw.vertices.len() as u32)
            .ok_or("vertex overflow")?;
    }
    let count = u32::try_from(indices.len() / 4).map_err(|e| e.to_string())?;
    if count == 0 || !count.is_multiple_of(3) {
        return Err("invalid triangle list".into());
    }
    Ok((vertices, indices, count))
}

/// Real GPU execution and readback; no screenshots or visual inspection.
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let start = Instant::now();
    let scene = RenderScene::character(bundle)?;
    let mut report = scene.report.clone();
    report.push_str(&crate::shader_pack::inventory(bundle)?);
    let decals = texture::character_decals(bundle, &scene)?;
    let surfaces = surface::character(bundle, &scene)?;
    report.push_str(&decals.report);
    report.push_str(&surfaces.report);
    let mut app = scene_app(scene);
    for draw in decals.draws {
        app.world_mut().spawn(draw);
    }
    app.update();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let draws = &app.world().resource::<Extracted>().0;
    let (mut gpu, _) = Gpu::new(&instance, None, wgpu::TextureFormat::Rgba8Unorm, draws)?;
    writeln!(report, "{}", gpu.info).unwrap();
    let inputs = RenderInputs::character_probe();
    let pixels = gpu.read_frame(640, 480, &inputs)?;
    let repeat = gpu.read_frame(640, 480, &inputs)?;
    if pixels != repeat {
        return Err("identical GPU inputs produced different frames".into());
    }
    let clear = &pixels[..4];
    let covered = pixels.chunks_exact(4).filter(|p| *p != clear).count();
    let colors: std::collections::BTreeSet<_> = pixels
        .chunks_exact(4)
        .map(|p| <[u8; 4]>::try_from(p).unwrap())
        .collect();
    if covered < 1000 || colors.len() < 10 {
        return Err(format!(
            "insufficient rasterized coverage: pixels={covered} colors={}",
            colors.len()
        ));
    }
    let digest = format!("{:x}", Sha256::digest(&pixels));
    writeln!(report,"render_probe_pass width=640 height=480 draws={} covered_pixels={covered} colors={} repeat_identical=true rgba_sha256={digest} elapsed_ms={:.3}",draws.len(),colors.len(),start.elapsed().as_secs_f64()*1000.0).unwrap();
    writeln!(
        report,
        "probe_camera position={:?} rotation={:?} fov_y={} near=0.5 far=5000",
        inputs.frame.position, inputs.frame.rotation, inputs.fov_y
    )
    .unwrap();
    report.push_str(&gpu_contract_probe(&instance)?);
    gpu.add_decals(&app.world().resource::<ExtractedDecals>().0)?;
    let textured = gpu.read_frame(640, 480, &inputs)?;
    if textured != gpu.read_frame(640, 480, &inputs)? {
        return Err("decal repeat render differs".into());
    }
    let changed = pixels
        .chunks_exact(4)
        .zip(textured.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if changed < 10 {
        writeln!(report, "decal_probe_failed changed_pixels={changed}").unwrap();
        runtime::save_log(bundle, &report)?;
        return Err(format!(
            "decal produced insufficient pixel changes: {changed}"
        ));
    }
    writeln!(report,"decal_render_pass draws={} changed_pixels={changed} repeat_identical=true rgba_sha256={:x} mip_filter=trilinear anisotropy=probe_1",app.world().resource::<ExtractedDecals>().0.len(),Sha256::digest(&textured)).unwrap();
    gpu.add_studs(draws, &surfaces)?;
    let plastic = gpu.read_frame(640, 480, &inputs)?;
    if plastic != gpu.read_frame(640, 480, &inputs)? {
        return Err("Plastic repeat render differs".into());
    }
    let changes = textured
        .chunks_exact(4)
        .zip(plastic.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if changes != 0 {
        return Err("composited character incorrectly sampled stud atlas".into());
    }
    writeln!(report,"plastic_render_pass changed_pixels={changes} repeat_identical=true rgba_sha256={:x} studs=composited_body_disabled outlines=probe_disabled compressed_decode=native_fallback",Sha256::digest(&plastic)).unwrap();
    writeln!(report,"shader_path=static_low_quality_plastic_and_decal probe_light_grid=constant probe_shadow=constant native_visual_parity=not_tested").unwrap();
    let path = runtime::save_log(bundle, &report)?;
    let frame_path = path.with_extension("ppm");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&frame_path)
        .map_err(|e| e.to_string())?;
    file.write_all(b"P6\n640 480\n255\n")
        .map_err(|e| e.to_string())?;
    for p in plastic.chunks_exact(4) {
        file.write_all(&p[..3]).map_err(|e| e.to_string())?;
    }
    writeln!(
        report,
        "frame_capture={} evidence_log={}",
        frame_path.display(),
        path.display()
    )
    .unwrap();
    Ok(report)
}

fn gpu_contract_probe(instance: &wgpu::Instance) -> Result<String, String> {
    let mut inputs = RenderInputs::character_probe();
    inputs.frame = CoordinateFrame {
        position: [0.0; 3],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    inputs.ambient = [1.0; 3];
    inputs.lamp0 = [0.0; 3];
    inputs.lamp1 = [0.0; 3];
    inputs.fog_start = 0.0;
    inputs.fog_end = 4.0;
    inputs.fog_color = [0.0, 0.0, 1.0];
    let triangle = |z: f32, color: [f32; 4]| Draw {
        name: "procedural GPU contract fixture".into(),
        vertices: [[-1.0, -1.0, z], [1.0, -1.0, z], [0.0, 1.0, z]]
            .into_iter()
            .map(|position| RenderVertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [0.0; 2],
                color,
                specular: [0.0, 1.0],
            })
            .collect(),
        indices: vec![0, 1, 2],
        material: 0x100,
        geometry: "synthetic".into(),
    };
    // Near red is drawn before far green. Disabling depth would overwrite red.
    let fixture = [
        triangle(-2.0, [1.0, 0.0, 0.0, 1.0]),
        triangle(-3.0, [0.0, 1.0, 0.0, 1.0]),
    ];
    let (gpu, _) = Gpu::new(instance, None, wgpu::TextureFormat::Rgba8Unorm, &fixture)?;
    let image = gpu.read_frame(65, 65, &inputs)?;
    let (bgra, _) = Gpu::new(instance, None, wgpu::TextureFormat::Bgra8Unorm, &fixture)?;
    if bgra.read_frame(65, 65, &inputs)? != image {
        return Err("BGRA and RGBA render targets disagree after channel conversion".into());
    }
    let center = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 4];
    if center
        .iter()
        .zip([128i32, 0, 128, 255])
        .any(|(a, b)| (*a as i32 - b).abs() > 1)
    {
        return Err(format!("GPU depth/fog contract mismatch: {center:?}"));
    }
    inputs.fog_start = 4.0;
    let image = gpu.read_frame(65, 65, &inputs)?;
    let equal = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 4];
    if equal != [0, 0, 255, 255] {
        return Err(format!("equal-range fog mismatch: {equal:?}"));
    }
    let mut report = format!(
        "gpu_contract_pass near_over_far=true bgra_matches_rgba=true center_rgba={center:?} equal_fog_rgba={equal:?}\n"
    );
    inputs.fog_start = 0.0;
    inputs.fog_end = 1.0e10;
    let (mut decal_gpu, _) = Gpu::new(
        instance,
        None,
        wgpu::TextureFormat::Rgba8Unorm,
        &[triangle(-3.5, [0.0, 1.0, 0.0, 1.0])],
    )?;
    let mut decal = triangle(-2.0, [1.0; 4]);
    for vertex in &mut decal.vertices {
        vertex.uv = [-1.0, 0.25];
    }
    let red = TexturedDraw {
        draw: decal.clone(),
        texture: PngTexture::from_rgba(
            2,
            2,
            vec![
                255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 0,
            ],
        )?,
    };
    decal_gpu.add_decals(std::slice::from_ref(&red))?;
    let image = decal_gpu.read_frame(65, 65, &inputs)?;
    let blended = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 4];
    if blended
        .iter()
        .zip([128i32, 127, 0, 255])
        .any(|(a, b)| (*a as i32 - b).abs() > 1)
    {
        return Err(format!(
            "straight-alpha/clamp/orientation contract mismatch: {blended:?}"
        ));
    }
    for vertex in &mut decal.vertices {
        vertex.position[2] = -3.0;
    }
    let blue = TexturedDraw {
        draw: decal,
        texture: PngTexture::from_rgba(1, 1, vec![0, 0, 255, 255])?,
    };
    decal_gpu.add_decals(&[red, blue])?;
    let image = decal_gpu.read_frame(65, 65, &inputs)?;
    let depth = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 4];
    if depth != [0, 0, 255, 255] {
        return Err(format!("decal depth-write contract mismatch: {depth:?}"));
    }
    writeln!(report,"gpu_decal_contract_pass clamp=true uv_row_order=true straight_alpha_rgba={blended:?} depth_write_disabled=true later_far_rgba={depth:?}").unwrap();
    let draw = triangle(-2.0, [0.25, 0.5, 0.75, 1.0]);
    let fixture = SurfaceScene {
        coordinates: std::collections::BTreeMap::from([(draw.name.clone(), vec![[1.25, 0.25]; 3])]),
        atlas: PngTexture::from_rgba(
            2,
            2,
            vec![64, 0, 0, 0, 192, 0, 0, 255, 192, 0, 0, 255, 192, 0, 0, 255],
        )?,
        report: String::new(),
    };
    let (mut stud_gpu, _) = Gpu::new(
        instance,
        None,
        wgpu::TextureFormat::Rgba8Unorm,
        std::slice::from_ref(&draw),
    )?;
    stud_gpu.add_studs(&[draw], &fixture)?;
    let image = stud_gpu.read_frame(65, 65, &inputs)?;
    let sampled = &image[(32 * 65 + 32) * 4..(32 * 65 + 32) * 4 + 4];
    if sampled
        .iter()
        .zip([32i32, 64, 96, 255])
        .any(|(a, b)| (*a as i32 - b).abs() > 1)
    {
        return Err(format!(
            "Plastic repeat/red-times-two contract mismatch: {sampled:?}"
        ));
    }
    writeln!(report,"gpu_plastic_contract_pass repeat_uv=true red_times_two=true atlas_alpha_ignored=true center_rgba={sampled:?}").unwrap();
    Ok(report)
}

/// Exercise Bevy pose extraction, dynamic vertex uploads and parented decals
/// against fresh GPU uploads. No window or visual inspection is needed.
pub fn audit_motion(bundle: &Bundle) -> Result<String, String> {
    let scene = RenderScene::character(bundle)?;
    let mut report = scene.report.clone();
    let decals = texture::character_decals(bundle, &scene)?;
    let surfaces = surface::character(bundle, &scene)?;
    let mut app = scene_app(scene);
    for decal in decals.draws {
        app.world_mut().spawn(decal);
    }
    app.update();
    install_motion(&mut app, bundle)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let (mut gpu, _) = Gpu::new(
        &instance,
        None,
        wgpu::TextureFormat::Rgba8Unorm,
        &app.world().resource::<Extracted>().0,
    )?;
    gpu.add_studs(&app.world().resource::<Extracted>().0, &surfaces)?;
    gpu.add_decals(&app.world().resource::<ExtractedDecals>().0)?;
    let inputs = RenderInputs::character_probe();
    let initial = gpu.read_frame(640, 480, &inputs)?;
    let mut final_pixels = Vec::new();
    for step in 1..=60 {
        app.world_mut()
            .resource_mut::<MotionControl>()
            .requested_steps = 1;
        app.update();
        if let Some(error) = &app.world().resource::<MotionControl>().error {
            return Err(error.clone());
        }
        let draws = &app.world().resource::<Extracted>().0;
        let decals = &app.world().resource::<ExtractedDecals>().0;
        let head = draws
            .iter()
            .find(|draw| draw.name == "Head")
            .ok_or("missing moved head")?;
        let face = decals
            .iter()
            .find(|draw| draw.draw.name == "Head/face")
            .ok_or("missing moved face")?;
        if head.vertices.len() != face.draw.vertices.len()
            || head
                .vertices
                .iter()
                .zip(&face.draw.vertices)
                .any(|(a, b)| a.position != b.position || a.normal != b.normal || a.uv != b.uv)
        {
            return Err("decal geometry detached from moving head".into());
        }
        gpu.update_geometry(draws, decals, &surfaces)?;
        if step == 1 || step == 15 || step == 30 || step == 60 {
            let pixels = gpu.read_frame(640, 480, &inputs)?;
            let repeat = gpu.read_frame(640, 480, &inputs)?;
            let changed = pixels
                .chunks_exact(4)
                .zip(initial.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            if pixels != repeat || changed == 0 {
                return Err(format!("motion readback failed at step {step}"));
            }
            let low = head
                .vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::INFINITY, f32::min);
            let high = head
                .vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::NEG_INFINITY, f32::max);
            writeln!(report,"motion_render_step step={step} head_min_y={low:.9} head_max_y={high:.9} changed_pixels={changed} repeat_identical=true rgba_sha256={:x}",Sha256::digest(&pixels)).unwrap();
            final_pixels = pixels;
        }
    }
    let draws = &app.world().resource::<Extracted>().0;
    let decals = &app.world().resource::<ExtractedDecals>().0;
    let (mut reference, _) = Gpu::new(&instance, None, wgpu::TextureFormat::Rgba8Unorm, draws)?;
    reference.add_studs(draws, &surfaces)?;
    let without_decal = reference.read_frame(640, 480, &inputs)?;
    reference.add_decals(decals)?;
    if reference.read_frame(640, 480, &inputs)? != final_pixels {
        return Err("dynamic uploads differ from freshly uploaded posed geometry".into());
    }
    let decal_changed = without_decal
        .chunks_exact(4)
        .zip(final_pixels.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if decal_changed == 0 {
        return Err("moved face has no rasterized contribution".into());
    }
    motion::reset(app.world_mut());
    app.update();
    gpu.update_geometry(
        &app.world().resource::<Extracted>().0,
        &app.world().resource::<ExtractedDecals>().0,
        &surfaces,
    )?;
    if gpu.read_frame(640, 480, &inputs)? != initial {
        return Err("motion reset did not restore original pixels".into());
    }
    writeln!(report,"motion_render_pass steps=60 initial_world_bytes_identical=true decal_parent_checks=60 moved_decal_pixels={decal_changed} fresh_upload_identical=true reset_pixels_identical=true fixture=independent_unconstrained_parts native_visual_parity=not_tested").unwrap();
    Ok(report)
}

struct Viewport {
    app: App,
    bundle_root: PathBuf,
    report: String,
    inputs: RenderInputs,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    gpu: Option<Gpu>,
    config: Option<wgpu::SurfaceConfiguration>,
    error: Option<String>,
    frames: u32,
    frame_limit: Option<u32>,
    surfaces: SurfaceScene,
    live_log: std::fs::File,
    started: Instant,
    motion_active: bool,
    motion_clock: Instant,
    motion_remainder: f64,
    uploaded_step: u32,
    acquisition_retries: u32,
}
impl Viewport {
    fn log_event(&mut self, event: String) -> Result<(), String> {
        writeln!(self.live_log, "{event}").map_err(|e| e.to_string())?;
        self.live_log.flush().map_err(|e| e.to_string())?;
        writeln!(self.report, "{event}").unwrap();
        println!("{event}");
        Ok(())
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        let _ = self.log_event(format!("viewport_failed={error}"));
        self.error = Some(error);
        event_loop.exit();
    }
}
impl ApplicationHandler for Viewport {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<(), String> {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("Roblox 2016 — Space: fall probe | R: reset | Esc: close")
                            .with_inner_size(winit::dpi::LogicalSize::new(960.0, 720.0))
                            .with_visible(false),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let surface = instance
                .create_surface(window.clone())
                .map_err(|e| e.to_string())?;
            let (mut gpu, adapter) = Gpu::new(
                &instance,
                Some(&surface),
                wgpu::TextureFormat::Bgra8Unorm,
                &self.app.world().resource::<Extracted>().0,
            )?;
            gpu.add_studs(&self.app.world().resource::<Extracted>().0, &self.surfaces)?;
            gpu.add_decals(&self.app.world().resource::<ExtractedDecals>().0)?;
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .ok_or("surface has no supported configuration")?;
            if !surface
                .get_capabilities(&adapter)
                .formats
                .contains(&wgpu::TextureFormat::Bgra8Unorm)
            {
                return Err("non-sRGB viewport format unavailable".into());
            }
            config.format = wgpu::TextureFormat::Bgra8Unorm;
            config.present_mode = wgpu::PresentMode::Fifo;
            surface.configure(&gpu.device, &config);
            // Metal can report an invisible surface as occluded. Warm up an
            // offscreen frame before showing the window, then acquire its surface.
            let pixels = gpu.read_frame(config.width, config.height, &self.inputs)?;
            let background = &pixels[..4];
            let covered = pixels.chunks_exact(4).filter(|p| *p != background).count();
            if covered == 0 {
                return Err("viewport warm-up rendered no geometry".into());
            }
            self.log_event(format!(
                "viewport_readback_pass covered_pixels={covered} rgba_sha256={:x}",
                Sha256::digest(&pixels)
            ))?;
            window.set_visible(true);
            // A shell-launched macOS application can remain inactive after its
            // hidden warm-up. This activates the requested visible viewport.
            window.focus_window();
            self.log_event(format!(
                "{}\nviewport_open width={} height={} scale_factor={} format={:?}",
                gpu.info,
                size.width,
                size.height,
                window.scale_factor(),
                config.format
            ))?;
            self.window = Some(window);
            self.surface = Some(surface);
            self.gpu = Some(gpu);
            self.config = Some(config);
            self.motion_clock = Instant::now();
            if self.motion_active {
                self.log_event("motion_probe_started limit_steps=60 contacts=false joints=false gravity=explicit_legacy".into())?;
            }
            self.window.as_ref().unwrap().request_redraw();
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit()
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let reset = matches!(&event.logical_key, Key::Character(key) if key.eq_ignore_ascii_case("r"));
                let start = event.logical_key == Key::Named(NamedKey::Space);
                if start || reset {
                    motion::reset(self.app.world_mut());
                    self.motion_active = start;
                    self.motion_remainder = 0.0;
                    self.motion_clock = Instant::now();
                    if let Err(error) = self.log_event(if start {
                        "motion_probe_started limit_steps=60 contacts=false joints=false gravity=explicit_legacy".into()
                    } else {
                        "motion_probe_reset step=0".into()
                    }) {
                        self.fail(event_loop, error);
                        return;
                    }
                    self.window.as_ref().unwrap().request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let config = self.config.as_mut().unwrap();
                config.width = size.width;
                config.height = size.height;
                self.surface
                    .as_ref()
                    .unwrap()
                    .configure(&self.gpu.as_ref().unwrap().device, config);
                if let Err(error) = self.log_event(format!(
                    "viewport_resize width={} height={}",
                    size.width, size.height
                )) {
                    self.fail(event_loop, error);
                    return;
                }
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let result = (|| -> Result<(), String> {
                    if self.motion_active && self.frames > 0 {
                        let now = Instant::now();
                        self.motion_remainder +=
                            now.duration_since(self.motion_clock).as_secs_f64();
                        self.motion_clock = now;
                        let dt = motion::timestep(self.app.world());
                        let remaining = 60 - self.app.world().resource::<MotionControl>().steps;
                        let steps = (self.motion_remainder / dt)
                            .floor()
                            .min(f64::from(remaining)) as u32;
                        self.motion_remainder -= f64::from(steps) * dt;
                        self.app
                            .world_mut()
                            .resource_mut::<MotionControl>()
                            .requested_steps = steps;
                    }
                    self.app.update();
                    let control = self.app.world().resource::<MotionControl>();
                    if let Some(error) = &control.error {
                        return Err(error.clone());
                    }
                    let step = control.steps;
                    if self.motion_active && step == 60 {
                        self.motion_active = false;
                        let head = self
                            .app
                            .world()
                            .resource::<Extracted>()
                            .0
                            .iter()
                            .find(|d| d.name == "Head")
                            .ok_or("missing rendered motion head")?;
                        let min_y = head
                            .vertices
                            .iter()
                            .map(|v| v.position[1])
                            .fold(f32::INFINITY, f32::min);
                        self.log_event(format!("motion_probe_paused step={step} simulated_seconds={:.9} head_min_y={min_y:.9} reason=unconstrained_probe_limit", f64::from(step) * motion::timestep(self.app.world())))?;
                    }
                    let config = self.config.as_ref().unwrap();
                    let gpu = self.gpu.as_ref().unwrap();
                    if self.uploaded_step != step {
                        gpu.update_geometry(
                            &self.app.world().resource::<Extracted>().0,
                            &self.app.world().resource::<ExtractedDecals>().0,
                            &self.surfaces,
                        )?;
                        self.uploaded_step = step;
                    }
                    let surface = self.surface.as_ref().unwrap();
                    let frame = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(frame)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                        state @ (wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded) => {
                            self.acquisition_retries += 1;
                            if self.acquisition_retries == 1 {
                                self.log_event(format!("viewport_acquisition_retry state={state:?} frame={} physics_step={step} window_visible={:?} window_focused={}", self.frames, self.window.as_ref().unwrap().is_visible(), self.window.as_ref().unwrap().has_focus()))?;
                            }
                            if self.frame_limit.is_some()
                                && self.started.elapsed() > Duration::from_secs(30)
                            {
                                return Err(format!(
                                    "surface acquisition timed out after {} retries: {state:?}",
                                    self.acquisition_retries
                                ));
                            }
                            self.window.as_ref().unwrap().request_redraw();
                            return Ok(());
                        }
                        wgpu::CurrentSurfaceTexture::Outdated => {
                            surface.configure(&gpu.device, config);
                            self.window.as_ref().unwrap().request_redraw();
                            return Ok(());
                        }
                        state => return Err(format!("surface acquisition failed: {state:?}")),
                    };
                    gpu.queue.write_buffer(
                        &gpu.uniform,
                        0,
                        &self.inputs.bytes(config.width, config.height)?,
                    );
                    let depth = gpu.texture(
                        config.width,
                        config.height,
                        wgpu::TextureFormat::Depth32Float,
                        wgpu::TextureUsages::RENDER_ATTACHMENT,
                    );
                    let mut encoder = gpu.device.create_command_encoder(&Default::default());
                    gpu.pass(
                        &mut encoder,
                        &frame.texture.create_view(&Default::default()),
                        &depth.create_view(&Default::default()),
                    );
                    gpu.queue.submit([encoder.finish()]);
                    gpu.queue.present(frame);
                    self.frames += 1;
                    let final_readback = if self.frame_limit == Some(self.frames) {
                        let pixels = gpu.read_frame(config.width, config.height, &self.inputs)?;
                        let covered = pixels
                            .chunks_exact(4)
                            .filter(|p| *p != &pixels[..4])
                            .count();
                        if covered == 0 {
                            return Err("final viewport frame rendered no geometry".into());
                        }
                        Some(format!(
                            "viewport_final_readback_pass physics_step={step} covered_pixels={covered} rgba_sha256={:x}",
                            Sha256::digest(&pixels)
                        ))
                    } else {
                        None
                    };
                    if self.frames == 1 {
                        self.window.as_ref().unwrap().set_visible(true);
                        self.motion_clock = Instant::now();
                    }
                    if self.frames == 1 || self.frames.is_multiple_of(60) {
                        self.log_event(format!(
                            "viewport_frame frame={} draws={} indices={} physics_step={step} elapsed_ms={:.3}",
                            self.frames,
                            self.app.world().resource::<Extracted>().0.len(),
                            gpu.count,
                            self.started.elapsed().as_secs_f64() * 1000.0
                        ))?;
                    }
                    if self.frame_limit.is_some_and(|limit| self.frames >= limit) {
                        if let Some(event) = final_readback {
                            self.log_event(event)?;
                        }
                        event_loop.exit();
                    } else if self.frame_limit.is_some() || self.motion_active {
                        self.window.as_ref().unwrap().request_redraw();
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.fail(event_loop, error);
                }
            }
            _ => (),
        }
    }
}

/// Open a diagnostic viewport. Optional frame limit supports numeric smoke tests.
pub fn viewport(bundle: &Bundle, frame_limit: Option<u32>) -> Result<String, String> {
    run_viewport(bundle, frame_limit, false)
}

/// Automatic surface smoke with the same bounded free-fall trigger as Space.
pub fn viewport_motion_smoke(bundle: &Bundle) -> Result<String, String> {
    run_viewport(bundle, Some(120), true)
}

fn run_viewport(
    bundle: &Bundle,
    frame_limit: Option<u32>,
    auto_motion: bool,
) -> Result<String, String> {
    if frame_limit.is_some_and(|n| n == 0 || n > 10000) {
        return Err("viewport frame limit must be 1..=10000".into());
    }
    let scene = RenderScene::character(bundle)?;
    let mut report = scene.report.clone();
    let decals = texture::character_decals(bundle, &scene)?;
    let surfaces = surface::character(bundle, &scene)?;
    report.push_str(&decals.report);
    report.push_str(&surfaces.report);
    let mut app = scene_app(scene);
    for draw in decals.draws {
        app.world_mut().spawn(draw);
    }
    app.update();
    install_motion(&mut app, bundle)?;
    let bodies = app
        .world_mut()
        .query::<&crate::physics::FreeFallBody>()
        .iter(app.world())
        .count();
    writeln!(report, "motion_binding_pass bodies={bodies} draws={} decals={} initial_world_bytes_identical=true contacts=false joints=false", app.world().resource::<Extracted>().0.len(), app.world().resource::<ExtractedDecals>().0.len()).unwrap();
    let live_path = runtime::save_log(bundle, &report)?;
    let live_log = OpenOptions::new()
        .append(true)
        .open(&live_path)
        .map_err(|e| e.to_string())?;
    println!("viewport_live_log={}", live_path.display());
    let mut viewport = Viewport {
        app,
        bundle_root: bundle.root().to_path_buf(),
        report,
        inputs: RenderInputs::character_probe(),
        window: None,
        surface: None,
        gpu: None,
        config: None,
        error: None,
        frames: 0,
        frame_limit,
        surfaces,
        live_log,
        started: Instant::now(),
        motion_active: auto_motion,
        motion_clock: Instant::now(),
        motion_remainder: 0.0,
        uploaded_step: 0,
        acquisition_retries: 0,
    };
    EventLoop::new()
        .map_err(|e| e.to_string())?
        .run_app(&mut viewport)
        .map_err(|e| e.to_string())?;
    viewport.log_event(format!(
        "viewport_closed frames={} native_visual_parity=not_tested",
        viewport.frames
    ))?;
    if let Some(error) = &viewport.error {
        writeln!(viewport.report, "viewport_failed={error}").unwrap();
    }
    let path = runtime::save_log(&Bundle::open(&viewport.bundle_root)?, &viewport.report)?;
    if let Some(error) = viewport.error {
        return Err(format!("{error}; evidence_log={}", path.display()));
    }
    writeln!(viewport.report, "evidence_log={}", path.display()).unwrap();
    Ok(viewport.report)
}
