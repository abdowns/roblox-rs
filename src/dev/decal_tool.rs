//! Dev decal tool: pick an image file, then click a block Part's surface to
//! put the image on that face as a Decal. Only the Play Solo baseplate
//! viewport (the demo place) is supported. The image is resized to a square
//! power of two here; that resize is dev tooling, not the native path.
use super::super::{Extracted, ExtractedDecals, Viewport};
use crate::bundle::Bundle;
use crate::geometry::Draw;
use crate::motion::{self, LooseDecal};
use crate::texture::{PngTexture, TexturedDraw};
use crate::workspace::{self, DecalInstance, Workspace};
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::Entity;
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct DecalTool {
    pub picking: bool,
    pub status: String,
    image: Option<Image>,
    place: Option<Place>,
    /// Decals added by this tool. They carry no marker component: a new
    /// archetype would change the extract system's iteration order, and the
    /// GPU decal list must stay in that order.
    applied: Vec<Entity>,
    /// Log line of a spawned or removed decal whose GPU upload waits for the
    /// next frame's extract (see `upload`).
    pub pending: Option<String>,
}

struct Image {
    name: String,
    texture: PngTexture,
    preview: egui::TextureHandle,
}

/// The demo place with every Part at the identity frame, the matching
/// part-local draws (same vertex order as the live draws) and the decal
/// corner table.
struct Place {
    workspace: Workspace,
    local: Workspace,
    draws: HashMap<String, Draw>,
    faces: [[usize; 4]; 6],
}

impl DecalTool {
    /// Returns true when the user asked to clear the applied decals.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut clear = false;
        ui.horizontal(|ui| {
            if ui.button("Pick image…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg"])
                    .pick_file()
            {
                match load(&path, ui.ctx()) {
                    Ok(image) => {
                        self.status = format!("loaded {}", image.name);
                        self.image = Some(image);
                    }
                    Err(error) => self.status = format!("load failed: {error}"),
                }
            }
            if ui
                .add_enabled(
                    !self.applied.is_empty(),
                    egui::Button::new(format!("Clear ({})", self.applied.len())),
                )
                .clicked()
            {
                clear = true;
            }
        });
        if let Some(image) = &self.image {
            ui.horizontal(|ui| {
                ui.image((image.preview.id(), egui::vec2(96.0, 96.0)));
                ui.vertical(|ui| {
                    ui.label(&image.name);
                    let first = &image.texture.mips[0];
                    ui.label(format!("{}x{}", first.width, first.height));
                    ui.checkbox(&mut self.picking, "Click a surface to apply");
                });
            });
        } else {
            self.picking = false;
        }
        if !self.status.is_empty() {
            ui.small(&self.status);
        }
        clear
    }
}

fn load(path: &Path, ctx: &egui::Context) -> Result<Image, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > 32 * 1024 * 1024 {
        return Err("image is larger than 32 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let side = image
        .width()
        .max(image.height())
        .next_power_of_two()
        .clamp(16, 512);
    let image = image::imageops::resize(&image, side, side, image::imageops::FilterType::Triangle);
    let preview = ctx.load_texture(
        "dev_decal_preview",
        egui::ColorImage::from_rgba_unmultiplied([side as usize; 2], image.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    Ok(Image {
        name: path
            .file_name()
            .map_or("image".into(), |n| n.to_string_lossy().into_owned()),
        texture: PngTexture::from_rgba(side, side, image.into_raw())?,
        preview,
    })
}

fn place(v: &Viewport) -> Result<Place, String> {
    let world = v.app.world();
    if !world
        .resource::<Extracted>()
        .0
        .iter()
        .any(|d| d.name == "Baseplate")
    {
        return Err("the decal tool needs viewport-baseplate".into());
    }
    let bundle = Bundle::open(&v.bundle_root)?;
    let workspace = Workspace::load(&workspace::place_path(workspace::DEMO_PLACE))?;
    let mut local = workspace.clone();
    for part in &mut local.parts {
        part.frame.position = [0.0; 3];
        part.frame.rotation = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let (draws, _, _) = crate::geometry::workspace_draws(&bundle, &local)?;
    let mut by_name = HashMap::new();
    for draw in draws {
        by_name.entry(draw.name.clone()).or_insert(draw);
    }
    Ok(Place {
        workspace,
        local,
        draws: by_name,
        faces: crate::native_data::NativeImage::load(&bundle)?.decal_faces()?,
    })
}

/// Click at `cursor` (physical pixels): put the picked image on the face of
/// the block Part under it.
pub(super) fn click(v: &mut Viewport, cursor: [f64; 2]) -> Result<String, String> {
    let mut dev = v.dev.take().unwrap();
    let result = apply(v, &mut dev.decals, cursor);
    v.dev = Some(dev);
    result
}

fn apply(v: &mut Viewport, tool: &mut DecalTool, cursor: [f64; 2]) -> Result<String, String> {
    let texture = tool
        .image
        .as_ref()
        .ok_or("no image picked")?
        .texture
        .clone();
    if tool.place.is_none() {
        tool.place = Some(place(v)?);
    }
    let place = tool.place.as_ref().unwrap();
    let config = v.config.as_ref().ok_or("no surface")?;
    let (origin, direction) = ray(v, cursor, [config.width as f32, config.height as f32]);
    let mut nearest: Option<(f32, String, usize)> = None;
    for draw in &v.app.world().resource::<Extracted>().0 {
        for (triangle, i) in draw.indices.chunks_exact(3).enumerate() {
            let [a, b, c] = [0, 1, 2].map(|k| draw.vertices[i[k] as usize].position);
            if let Some(t) = intersect(origin, direction, a, b, c)
                && nearest.as_ref().is_none_or(|n| t < n.0)
            {
                nearest = Some((t, draw.name.clone(), triangle));
            }
        }
    }
    let (_, name, triangle) = nearest.ok_or("nothing under the cursor")?;
    let index = place
        .workspace
        .parts
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| format!("{name:?} is not a Part"))?;
    let part = &place.workspace.parts[index];
    let local_part = &place.local.parts[index];
    let local = &place.draws[&name];
    if part.shape != workspace::SHAPE_BLOCK || local.geometry != "block" {
        return Err(format!("{name:?} is not a block Part"));
    }
    // The hit triangle's part-local normal picks the face (NormalId).
    let normal = local.vertices[local.indices[triangle * 3] as usize].normal;
    let mut best = (f32::MIN, 0);
    for face in 0..6 {
        let quad = crate::geometry::workspace_decal(
            &place.faces,
            local_part,
            &DecalInstance {
                face,
                ..DecalInstance::decal(index)
            },
        )?;
        let n = quad.vertices[0].normal;
        let dot = n[0] * normal[0] + n[1] * normal[1] + n[2] * normal[2];
        if dot > best.0 {
            best = (dot, face);
        }
    }
    let face = best.1;
    let decal = DecalInstance {
        name: format!("DevDecal{}", tool.applied.len() + 1),
        face,
        ..DecalInstance::decal(index)
    };
    let geometry = crate::geometry::workspace_decal(&place.faces, local_part, &decal)?;
    let loose = place
        .workspace
        .parts
        .iter()
        .filter(|p| !p.anchored)
        .position(|p| p.name == name);
    let world = v.app.world_mut();
    match loose {
        Some(loose) => {
            let entity = world
                .spawn((
                    TexturedDraw {
                        draw: geometry.clone(),
                        texture,
                    },
                    LooseDecal {
                        index: loose,
                        geometry,
                    },
                ))
                .id();
            tool.applied.push(entity);
            // Pose it now: the extract may run before this frame's sync.
            world
                .run_system_once(motion::sync_loose_decals)
                .map_err(|e| e.to_string())?;
        }
        None => {
            let draw = motion::pose_draw(&geometry, &part.frame);
            tool.applied
                .push(world.spawn(TexturedDraw { draw, texture }).id());
        }
    }
    let line = format!(
        "dev_decal_applied part={name:?} face={face} loose={}",
        loose.is_some()
    );
    tool.pending = Some(line.clone());
    Ok(line)
}

/// Scripted check for the frame-limited smokes: `DEV_DECAL_SMOKE=image,fx,fy`
/// clicks at that fraction of the window, `DEV_DECAL_SMOKE=image,@Part` at
/// the centre of that Part's current draw. The next frame's `upload` logs
/// the result.
pub(super) fn smoke(v: &mut Viewport, ctx: &egui::Context) -> Option<Result<String, String>> {
    let spec = std::env::var("DEV_DECAL_SMOKE").ok()?;
    Some((|| {
        let (path, target) = spec
            .split_once(',')
            .ok_or("DEV_DECAL_SMOKE=image,fx,fy|@Part")?;
        let image = load(Path::new(path), ctx)?;
        v.dev.as_mut().unwrap().decals.image = Some(image);
        let config = v.config.as_ref().ok_or("no surface")?;
        let size = [f64::from(config.width), f64::from(config.height)];
        let cursor = match target.strip_prefix('@') {
            Some(name) => {
                let draws = &v.app.world().resource::<Extracted>().0;
                let draw = draws
                    .iter()
                    .find(|d| d.name == name)
                    .ok_or("no such draw")?;
                let n = draw.vertices.len() as f32;
                let centre =
                    [0, 1, 2].map(|a| draw.vertices.iter().map(|x| x.position[a]).sum::<f32>() / n);
                project(v, centre, size).ok_or("Part is off screen")?
            }
            None => {
                let (fx, fy) = target.split_once(',').ok_or("expected fx,fy")?;
                let fx = fx.parse::<f64>().map_err(|e| e.to_string())?;
                let fy = fy.parse::<f64>().map_err(|e| e.to_string())?;
                [fx * size[0], fy * size[1]]
            }
        };
        click(v, cursor).map(|line| format!("dev_decal_smoke_click {line}"))
    })())
}

/// The pixel a world point projects to (the inverse of `ray`).
fn project(v: &Viewport, p: [f32; 3], size: [f64; 2]) -> Option<[f64; 2]> {
    let frame = &v.inputs.frame;
    let column = |c: usize| [0, 1, 2].map(|r| frame.rotation[r][c]);
    let d = [0, 1, 2].map(|a| p[a] - frame.position[a]);
    let dot = |c: [f32; 3]| d[0] * c[0] + d[1] * c[1] + d[2] * c[2];
    let (x, y, z) = (dot(column(0)), dot(column(1)), -dot(column(2)));
    if z <= 0.0 {
        return None;
    }
    let tan = (v.inputs.fov_y * 0.5).tan();
    let aspect = (size[0] / size[1]) as f32;
    let (nx, ny) = (x / (z * tan * aspect), y / (z * tan));
    if nx.abs() > 1.0 || ny.abs() > 1.0 {
        return None;
    }
    Some([
        f64::from(nx + 1.0) * 0.5 * size[0],
        f64::from(1.0 - ny) * 0.5 * size[1],
    ])
}

/// Remove every decal this tool added.
pub(super) fn clear(v: &mut Viewport) -> Result<String, String> {
    let added = std::mem::take(&mut v.dev.as_mut().unwrap().decals.applied);
    for entity in &added {
        v.app.world_mut().despawn(*entity);
    }
    let line = format!("dev_decal_cleared removed={}", added.len());
    v.dev.as_mut().unwrap().decals.pending = Some(line.clone());
    Ok(line)
}

/// Rebuild the GPU decals from this frame's `ExtractedDecals`. The extract
/// system's cached query does not iterate in the same order as a fresh
/// query, so the list is only taken after it ran; `panel::before_frame`
/// holds physics for that frame so `update_geometry` (which checks the
/// order) is skipped. Logs how many pixels of an offscreen frame changed.
pub(super) fn upload(v: &mut Viewport) -> Result<Option<String>, String> {
    let Some(line) = v.dev.as_mut().unwrap().decals.pending.take() else {
        return Ok(None);
    };
    let decals = v.app.world().resource::<ExtractedDecals>().0.clone();
    let config = v.config.clone().ok_or("no surface")?;
    let gpu = v.gpu.as_mut().ok_or("no GPU")?;
    let before = gpu.read_frame(config.width, config.height, &v.inputs)?;
    gpu.add_decals(&decals)?;
    let after = gpu.read_frame(config.width, config.height, &v.inputs)?;
    let changed = before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    Ok(Some(format!(
        "{line} decals={} changed_pixels={changed}",
        decals.len()
    )))
}

/// Camera ray through a pixel. The frame's columns are right, up and back;
/// the camera looks along -back (camera::view, camera::perspective).
fn ray(v: &Viewport, cursor: [f64; 2], size: [f32; 2]) -> ([f32; 3], [f32; 3]) {
    let frame = &v.inputs.frame;
    let column = |c: usize| [0, 1, 2].map(|r| frame.rotation[r][c]);
    let (right, up, back) = (column(0), column(1), column(2));
    let tan = (v.inputs.fov_y * 0.5).tan();
    let x = (2.0 * cursor[0] as f32 / size[0] - 1.0) * tan * size[0] / size[1];
    let y = (1.0 - 2.0 * cursor[1] as f32 / size[1]) * tan;
    let d = [0, 1, 2].map(|a| right[a] * x + up[a] * y - back[a]);
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (frame.position, d.map(|c| c / length))
}

/// Möller–Trumbore, both sides; the distance along the ray.
fn intersect(o: [f32; 3], d: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let cross = |p: [f32; 3], q: [f32; 3]| {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let dot = |p: [f32; 3], q: [f32; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-8 {
        return None;
    }
    let s = sub(o, a);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let w = dot(d, q) / det;
    let t = dot(e2, q) / det;
    (u >= 0.0 && w >= 0.0 && u + w <= 1.0 && t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ray_hits_triangle_in_front_only() {
        let (a, b, c) = ([-1.0, -1.0, -5.0], [1.0, -1.0, -5.0], [0.0, 1.0, -5.0]);
        let t = super::intersect([0.0; 3], [0.0, 0.0, -1.0], a, b, c).unwrap();
        assert!((t - 5.0).abs() < 1e-5);
        assert!(super::intersect([0.0; 3], [0.0, 0.0, 1.0], a, b, c).is_none());
        assert!(super::intersect([3.0, 0.0, 0.0], [0.0, 0.0, -1.0], a, b, c).is_none());
    }
}
