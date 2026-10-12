//! Numeric GPU probe and static viewport. Bevy owns/extracts draw components;
//! wgpu supplies Metal without modern PBR, tone mapping or automatic color-space conversion.
use crate::{
    bundle::Bundle,
    camera,
    geometry::{Draw, RenderScene, RenderVertex},
    runtime,
    scene::CoordinateFrame,
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
    event::WindowEvent,
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
        let position = Vec3::new(8.0, 22.0, 10.0);
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
fn extract(draws: Query<&Draw>, mut output: ResMut<Extracted>) {
    output.0 = draws.iter().cloned().collect();
}
fn scene_app(scene: RenderScene) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    for draw in scene.draws {
        app.world_mut().spawn(draw);
    }
    app.init_resource::<Extracted>();
    app.add_systems(Update, extract);
    app.update();
    app
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
    count: u32,
    info: String,
}

impl Gpu {
    fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        format: wgpu::TextureFormat,
        draws: &[Draw],
    ) -> Result<(Self, wgpu::Adapter), String> {
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
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x4,4=>Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("static probe"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 56,
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
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
            usage: wgpu::BufferUsages::VERTEX,
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
                count,
                info,
            },
            adapter,
        ))
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..1);
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
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut base = 0u32;
    for draw in draws {
        if draw.material != 0x100 || draw.vertices.iter().any(|v| v.color[3] != 1.0) {
            return Err("static shader supports opaque Plastic only".into());
        }
        for v in &draw.vertices {
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
    let app = scene_app(scene);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let draws = &app.world().resource::<Extracted>().0;
    let (gpu, _) = Gpu::new(&instance, None, wgpu::TextureFormat::Rgba8Unorm, draws)?;
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
    writeln!(report,"shader_path=static_low_quality_untextured probe_light_grid=constant probe_shadow=constant native_visual_parity=not_tested").unwrap();
    let path = runtime::save_log(bundle, &report)?;
    let frame_path = path.with_extension("ppm");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&frame_path)
        .map_err(|e| e.to_string())?;
    file.write_all(b"P6\n640 480\n255\n")
        .map_err(|e| e.to_string())?;
    for p in pixels.chunks_exact(4) {
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
    Ok(format!(
        "gpu_contract_pass near_over_far=true bgra_matches_rgba=true center_rgba={center:?} equal_fog_rgba={equal:?}\n"
    ))
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
}
impl Viewport {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
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
                            .with_title("Studio 2016 — static rendering probe")
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
            let (gpu, adapter) = Gpu::new(
                &instance,
                Some(&surface),
                wgpu::TextureFormat::Bgra8Unorm,
                &self.app.world().resource::<Extracted>().0,
            )?;
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
            gpu.read_frame(config.width, config.height, &self.inputs)?;
            window.set_visible(true);
            writeln!(
                self.report,
                "{}\nviewport_open width={} height={} scale_factor={} format={:?}",
                gpu.info,
                size.width,
                size.height,
                window.scale_factor(),
                config.format
            )
            .unwrap();
            self.window = Some(window);
            self.surface = Some(surface);
            self.gpu = Some(gpu);
            self.config = Some(config);
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
                if event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit()
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
                writeln!(
                    self.report,
                    "viewport_resize width={} height={}",
                    size.width, size.height
                )
                .unwrap();
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let result = (|| -> Result<(), String> {
                    self.app.update();
                    let config = self.config.as_ref().unwrap();
                    let gpu = self.gpu.as_ref().unwrap();
                    let surface = self.surface.as_ref().unwrap();
                    let frame = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(frame)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                        wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded => {
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
                    if self.frames == 1 {
                        self.window.as_ref().unwrap().set_visible(true);
                    }
                    if self.frames == 1 || self.frames.is_multiple_of(60) {
                        writeln!(
                            self.report,
                            "viewport_frame frame={} draws={} indices={}",
                            self.frames,
                            self.app.world().resource::<Extracted>().0.len(),
                            gpu.count
                        )
                        .unwrap();
                    }
                    if self.frame_limit.is_some_and(|limit| self.frames >= limit) {
                        event_loop.exit();
                    } else if self.frame_limit.is_some() {
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
    if frame_limit.is_some_and(|n| n == 0 || n > 10000) {
        return Err("viewport frame limit must be 1..=10000".into());
    }
    let scene = RenderScene::character(bundle)?;
    let report = scene.report.clone();
    let mut viewport = Viewport {
        app: scene_app(scene),
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
    };
    EventLoop::new()
        .map_err(|e| e.to_string())?
        .run_app(&mut viewport)
        .map_err(|e| e.to_string())?;
    writeln!(
        viewport.report,
        "viewport_closed frames={} native_visual_parity=not_tested",
        viewport.frames
    )
    .unwrap();
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
