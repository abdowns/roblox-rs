//! Developer panel (`--features dev`). Not part of the product: everything
//! lives here, and render.rs only calls `event`, `before_frame` and `draw`.
//! It is a child module of `render` so it can read the viewport's state.
//! F1 or the "Dev" button toggles it.
use super::{Extracted, ExtractedDecals, MotionControl, Viewport, motion};
use crate::bundle::Bundle;
use crate::lighting::LightingProfile;
use std::collections::VecDeque;
use std::time::Instant;
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{Key, NamedKey};

const HISTORY: usize = 120;

pub(super) struct DevPanel {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    open: bool,
    paused: bool,
    step_once: bool,
    time_scale: f32,
    /// Lighting.TimeOfDay override in hours; None keeps the place's value.
    time_of_day: Option<f32>,
    lighting: Option<LightingProfile>,
    intervals: VecDeque<f32>,
    cpu: VecDeque<f32>,
    frame_start: Option<Instant>,
    last_step: u32,
    steps_this_frame: u32,
    /// Open state last written to the live log.
    logged_open: Option<bool>,
}

enum Action {
    Reset,
    Snapshot,
}

/// Create the panel once the window and GPU exist.
fn panel(v: &mut Viewport) -> Option<&mut DevPanel> {
    if v.dev.is_none() {
        let (window, gpu, config) = (v.window.as_ref()?, v.gpu.as_ref()?, v.config.as_ref()?);
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &**window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(&gpu.device, config.format, Default::default());
        let lighting = v
            .walking_sky
            .is_some()
            .then(|| {
                Bundle::open(&v.bundle_root)
                    .and_then(|b| LightingProfile::load(&b))
                    .ok()
            })
            .flatten();
        v.dev = Some(DevPanel {
            ctx,
            state,
            renderer,
            open: true,
            paused: false,
            step_once: false,
            time_scale: 1.0,
            time_of_day: None,
            lighting,
            intervals: VecDeque::new(),
            cpu: VecDeque::new(),
            frame_start: None,
            last_step: 0,
            steps_this_frame: 0,
            logged_open: None,
        });
    }
    v.dev.as_mut()
}

/// Returns true when the panel used the event (the viewport should ignore it).
pub(super) fn event(v: &mut Viewport, event: &WindowEvent) -> bool {
    let Some(window) = v.window.clone() else {
        return false;
    };
    let Some(dev) = panel(v) else {
        return false;
    };
    if let WindowEvent::KeyboardInput { event: key, .. } = event
        && key.logical_key == Key::Named(NamedKey::F1)
    {
        if key.state == ElementState::Pressed && !key.repeat {
            dev.open = !dev.open;
            window.request_redraw();
        }
        return true;
    }
    let response = dev.state.on_window_event(&window, event);
    if response.repaint {
        window.request_redraw();
    }
    response.consumed
}

/// Start of a redraw: frame timing, pause / single step / time scale (by
/// moving the viewport's motion clock) and the TimeOfDay override.
pub(super) fn before_frame(v: &mut Viewport) {
    let timestep = motion::timestep(v.app.world());
    let Some(dev) = panel(v) else {
        return;
    };
    let now = Instant::now();
    if let Some(last) = dev.frame_start {
        push(&mut dev.intervals, now.duration_since(last).as_secs_f32());
    }
    dev.frame_start = Some(now);
    let (paused, step_once, scale) = (dev.paused, dev.step_once, dev.time_scale);
    dev.step_once = false;
    let sky = dev
        .time_of_day
        .zip(dev.lighting.as_ref())
        .map(|(hours, profile)| {
            let sky = profile.sky(f64::from(hours) * 3600.0);
            (profile.render_lighting(&sky), sky)
        });
    if paused {
        v.motion_clock = now;
        if step_once {
            v.motion_remainder = timestep;
        }
    } else if scale != 1.0 {
        v.motion_clock = now - now.duration_since(v.motion_clock).mul_f32(scale);
    }
    if let Some((lighting, sky)) = sky {
        v.inputs.apply_lighting(&lighting);
        v.walking_sky = Some(sky);
    }
}

/// After the scene pass: draw the panel on top of `view`.
pub(super) fn draw(
    v: &mut Viewport,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> Result<(), String> {
    let Some(window) = v.window.clone() else {
        return Ok(());
    };
    if panel(v).is_none() {
        return Ok(());
    }
    let mut dev = v.dev.take().unwrap();
    let world = v.app.world();
    let control = world.resource::<MotionControl>();
    let step = control.steps;
    if step != dev.last_step {
        dev.steps_this_frame = step.saturating_sub(dev.last_step);
        dev.last_step = step;
    } else {
        dev.steps_this_frame = 0;
    }
    let timestep = motion::timestep(world);
    let (gpu, config) = (v.gpu.as_ref().unwrap(), v.config.as_ref().unwrap());
    let stats = format!(
        "fps {:.1}  frame {:.2} ms (max {:.2})\ncpu {:.2} ms (max {:.2})\nframes {}  uptime {:.1} s\n\
         physics step {step} (+{} this frame)  {:.0} Hz  sim {:.2} s\n\
         draws {}  decals {}  indices {}  entities {}\nwindow {}x{} @{:.1}x\n\
         camera {}\ncharacter {}",
        1.0 / mean(&dev.intervals).max(1e-6),
        mean(&dev.intervals) * 1000.0,
        max(&dev.intervals) * 1000.0,
        mean(&dev.cpu) * 1000.0,
        max(&dev.cpu) * 1000.0,
        v.frames,
        v.started.elapsed().as_secs_f32(),
        dev.steps_this_frame,
        1.0 / timestep,
        f64::from(step) * timestep,
        world.resource::<Extracted>().0.len(),
        world.resource::<ExtractedDecals>().0.len(),
        gpu.count,
        world.entities().len(),
        config.width,
        config.height,
        window.scale_factor(),
        vec3(v.inputs.frame.position),
        motion::walking_frame(world).map_or("-".into(), |f| vec3(f.position)),
    );
    let report = motion::walking_report(world);
    let (walking, motion_active, info) = (control.walking_probe, v.motion_active, gpu.info.clone());
    let mut actions = Vec::new();
    let mut fov = v
        .walking_camera
        .as_ref()
        .map(|c| c.profile.field_of_view_degrees as f32);
    let input = dev.state.take_egui_input(&window);
    let mut output = dev.ctx.clone().run_ui(input, |ui| {
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::unique("dev_toggle"))
            .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
            .show(&ctx, |ui| {
                if ui
                    .button(if dev.open { "Dev ▴" } else { "Dev ▾" })
                    .clicked()
                {
                    dev.open = !dev.open;
                }
            });
        let mut open = dev.open;
        egui::Window::new("Dev panel (F1)")
            .open(&mut open)
            .default_pos([8.0, 8.0])
            .default_width(300.0)
            .show(&ctx, |ui| {
                ui.monospace(&stats);
                ui.separator();
                ui.horizontal(|ui| {
                    let label = if dev.paused {
                        "▶ Resume"
                    } else {
                        "⏸ Pause"
                    };
                    if ui.button(label).clicked() {
                        dev.paused = !dev.paused;
                    }
                    if ui
                        .add_enabled(dev.paused, egui::Button::new("Step"))
                        .clicked()
                    {
                        dev.step_once = true;
                    }
                    if ui.button("Reset").clicked() {
                        actions.push(Action::Reset);
                    }
                    if ui.button("Log snapshot").clicked() {
                        actions.push(Action::Snapshot);
                    }
                });
                ui.add(
                    egui::Slider::new(&mut dev.time_scale, 0.05..=4.0)
                        .logarithmic(true)
                        .text("time scale"),
                );
                if let Some(fov) = fov.as_mut() {
                    ui.add(egui::Slider::new(fov, 20.0..=120.0).text("field of view"));
                }
                if dev.lighting.is_some() {
                    let mut hours = dev.time_of_day.unwrap_or(14.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Slider::new(&mut hours, 0.0..=24.0).text("time of day"))
                            .changed()
                        {
                            dev.time_of_day = Some(hours);
                        }
                        if ui.small_button("place").clicked() {
                            dev.time_of_day = None;
                        }
                    });
                }
                ui.label(format!(
                    "mode: {}  motion {}",
                    if walking { "walking" } else { "probe" },
                    if motion_active { "on" } else { "off" }
                ));
                egui::CollapsingHeader::new("walking report").show(ui, |ui| {
                    ui.monospace(report.as_deref().unwrap_or("-"));
                });
                egui::CollapsingHeader::new("gpu").show(ui, |ui| {
                    ui.monospace(&info);
                });
            });
        dev.open &= open;
    });
    dev.state
        .handle_platform_output(&window, output.platform_output);
    let gpu = v.gpu.as_ref().unwrap();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [config.width, config.height],
        pixels_per_point: output.pixels_per_point,
    };
    let jobs = dev.ctx.tessellate(output.shapes, output.pixels_per_point);
    let shown = (dev.logged_open != Some(dev.open) && !(dev.open && jobs.is_empty())).then(|| {
        dev.logged_open = Some(dev.open);
        let vertices: usize = jobs
            .iter()
            .map(|job| match &job.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        format!(
            "dev_panel open={} paint_jobs={} vertices={vertices}",
            dev.open,
            jobs.len()
        )
    });
    for (id, deltas) in &output.textures_delta.set {
        for delta in deltas {
            dev.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let callbacks = dev
        .renderer
        .update_buffers(&gpu.device, &gpu.queue, encoder, &jobs, &screen);
    gpu.queue.submit(callbacks);
    let mut pass = encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dev panel"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        })
        .forget_lifetime();
    dev.renderer.render(&mut pass, &jobs, &screen);
    drop(pass);
    for id in &output.textures_delta.free {
        dev.renderer.free_texture(id);
    }
    output.textures_delta.clear();
    if let (Some(fov), Some(camera)) = (fov, v.walking_camera.as_mut())
        && fov != camera.profile.field_of_view_degrees as f32
    {
        camera.profile.field_of_view_degrees = f64::from(fov);
    }
    let repaint = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|o| o.repaint_delay.is_zero());
    if dev.open || repaint {
        window.request_redraw();
    }
    if let Some(start) = dev.frame_start {
        push(&mut dev.cpu, start.elapsed().as_secs_f32());
    }
    let snapshot = format!(
        "dev_snapshot {}",
        stats.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    v.dev = Some(dev);
    if let Some(event) = shown {
        v.log_event(event)?;
    }
    for action in actions {
        match action {
            Action::Reset => {
                motion::reset(v.app.world_mut());
                v.app
                    .world_mut()
                    .resource_mut::<MotionControl>()
                    .walking_probe = walking;
                v.walking_keys = [false; 4];
                v.motion_remainder = 0.0;
                v.motion_clock = Instant::now();
                v.log_event(format!("dev_action reset walking={walking}"))?;
            }
            Action::Snapshot => v.log_event(snapshot.clone())?,
        }
    }
    Ok(())
}

fn push(history: &mut VecDeque<f32>, value: f32) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

fn mean(history: &VecDeque<f32>) -> f32 {
    history.iter().sum::<f32>() / history.len().max(1) as f32
}

fn max(history: &VecDeque<f32>) -> f32 {
    history.iter().copied().fold(0.0, f32::max)
}

fn vec3(p: [f32; 3]) -> String {
    format!("({:.2}, {:.2}, {:.2})", p[0], p[1], p[2])
}
