//! Recovered static block and SpecialMesh Head paths. No guessed fallback shapes.
use crate::{
    bundle::Bundle,
    mesh::MeshData,
    native_data::NativeImage,
    scene::{Part, Scene},
};
use bevy::prelude::{Component, Resource};
use roxmltree::Node;
use std::{collections::BTreeMap, fmt::Write};

#[derive(Clone, Debug)]
pub struct RenderVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub specular: [f32; 2],
}

#[derive(Component, Clone, Debug)]
pub struct Draw {
    pub name: String,
    pub vertices: Vec<RenderVertex>,
    pub indices: Vec<u32>,
    pub material: u32,
    pub geometry: String,
}

#[derive(Resource)]
pub struct RenderScene {
    pub scene: Scene,
    pub draws: Vec<Draw>,
    pub report: String,
}

fn prop<'a, 'b>(item: Node<'a, 'b>, name: &str) -> Result<Node<'a, 'b>, String> {
    item.children()
        .find(|n| n.has_tag_name("Properties"))
        .and_then(|p| p.children().find(|n| n.attribute("name") == Some(name)))
        .ok_or_else(|| format!("missing rendering property {name}"))
}
fn number<T: std::str::FromStr>(item: Node<'_, '_>, name: &str) -> Result<T, String> {
    prop(item, name)?
        .text()
        .unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| format!("invalid rendering property {name}"))
}
fn vector(item: Node<'_, '_>, name: &str) -> Result<[f32; 3], String> {
    let p = prop(item, name)?;
    let mut result = [0.0; 3];
    for (axis, label) in ["X", "Y", "Z"].iter().enumerate() {
        result[axis] = p
            .children()
            .find(|n| n.has_tag_name(*label))
            .and_then(|n| n.text())
            .ok_or("missing vector component")?
            .trim()
            .parse::<f32>()
            .map_err(|e| e.to_string())?;
        if !result[axis].is_finite() {
            return Err("non-finite render vector".into());
        }
    }
    Ok(result)
}

pub fn specular(material: u32) -> Result<[f32; 2], String> {
    let (intensity, power): (u8, u8) = match material {
        0x100 => (0x66, 9),
        0x110 | 0x120 => (0xbf, 81),
        0x200 => (0x40, 32),
        0x210 => (0x47, 53),
        0x310 => (0xb3, 54),
        0x320 => (0x24, 20),
        0x330 => (0x26, 22),
        0x340 => (0x30, 24),
        0x350 => (0x21, 44),
        0x360 => (0x12, 22),
        0x370 => (0x36, 22),
        0x410 => (0x59, 103),
        0x420 => (0xe6, 160),
        0x430 => (0xee, 240),
        0x440 => (0xcc, 120),
        0x500 => (0x2b, 18),
        0x510 => (0x12, 6),
        0x520 => (8, 16),
        0x600 => (0xff, 190),
        _ => return Err(format!("unknown native material {material}")),
    };
    Ok([intensity as f32 / 255.0, power as f32])
}

/// MeshType Head: horizontal scale uses min(X,Z), with a signed Y correction.
pub fn head_point(
    position: [f32; 3],
    size: [f32; 3],
    scale: [f32; 3],
    offset: [f32; 3],
) -> [f32; 3] {
    let width = (size[0] * scale[0]).min(size[2] * scale[2]) * 0.8;
    let height = size[1] * scale[1] * 0.8;
    let shift = (height - width) * 0.625;
    [
        position[0] * width + offset[0],
        position[1] * width + offset[1] + if position[1] > 0.0 { shift } else { -shift },
        position[2] * width + offset[2],
    ]
}

fn vertex(
    part: &Part,
    local: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    material: u32,
) -> Result<RenderVertex, String> {
    let r = &part.frame.rotation;
    // The original writes world positions and rotates normals without applying
    // inverse scale; do not substitute a modern normal matrix here.
    let position = std::array::from_fn(|a| {
        part.frame.position[a] + local[1] * r[a][1] + local[0] * r[a][0] + local[2] * r[a][2]
    });
    let normal =
        std::array::from_fn(|a| r[a][2] * normal[2] + r[a][0] * normal[0] + normal[1] * r[a][1]);
    Ok(RenderVertex {
        position,
        normal,
        uv,
        color,
        specular: specular(material)?,
    })
}

impl RenderScene {
    pub fn character(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let colors = image.brick_colors()?;
        let faces = image.block_faces()?;
        let data = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        let xml = std::str::from_utf8(&data).map_err(|e| e.to_string())?;
        let scene = Scene::parse(xml)?;
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let items: BTreeMap<_, _> = doc
            .descendants()
            .filter(|n| n.has_tag_name("Item"))
            .map(|n| (n.attribute("referent").unwrap_or(""), n))
            .collect();
        let mut report = format!("native_palette_loaded colors={}\n", colors.len());
        let mut draws = Vec::new();
        for instance in &scene.instances {
            let Some(part) = &instance.part else {
                continue;
            };
            if part.transparency >= 1.0 {
                writeln!(
                    report,
                    "render_skip name={:?} reason=fully_transparent",
                    instance.name
                )
                .unwrap();
                continue;
            }
            let item = items[instance.referent.as_str()];
            let material = number(item, "Material")?;
            let reflectance: f32 = number(item, "Reflectance")?;
            if reflectance != 0.0 {
                return Err("reflective geometry path not implemented".into());
            }
            let rgb = colors.get(&part.brick_color).ok_or("invalid BrickColor")?;
            // Original alpha uses 255 - trunc(transparency*255), rather than
            // round((1-transparency)*255).
            let color = [
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
                (255 - (part.transparency * 255.0) as u32) as f32 / 255.0,
            ];
            let mesh = item.children().rfind(|n| {
                n.has_tag_name("Item")
                    && matches!(
                        n.attribute("class"),
                        Some("SpecialMesh" | "BlockMesh" | "CylinderMesh")
                    )
            });
            let mut draw = Draw {
                name: instance.name.clone(),
                vertices: Vec::new(),
                indices: Vec::new(),
                material,
                geometry: String::new(),
            };
            if let Some(mesh) = mesh {
                if mesh.attribute("class") != Some("SpecialMesh")
                    || number::<u32>(mesh, "MeshType")? != 0
                {
                    return Err(format!(
                        "unsupported DataModelMesh path for {:?}",
                        instance.name
                    ));
                }
                let scale = vector(mesh, "Scale")?;
                let offset = vector(mesh, "Offset")?;
                let mesh = MeshData::parse(&bundle.read(
                    "Contents/Resources/content/fonts/head.mesh",
                    64 * 1024 * 1024,
                )?)?;
                mesh.finite_bounds()?;
                for v in mesh.vertices {
                    draw.vertices.push(vertex(
                        part,
                        head_point(v.position, part.size, scale, offset),
                        v.normal,
                        [v.uv[0], v.uv[1]],
                        color,
                        material,
                    )?);
                }
                draw.indices = mesh.triangles.into_iter().flatten().collect();
                draw.geometry = "special_head".into();
            } else {
                if part.shape != 1 {
                    return Err(format!("unsupported Part shape {}", part.shape));
                }
                let corners: [[f32; 3]; 8] = std::array::from_fn(|i| {
                    std::array::from_fn(|a| {
                        (if i & (4 >> a) != 0 { 0.5 } else { -0.5 }) * part.size[a]
                    })
                });
                for face in faces {
                    let a = corners[face[0]];
                    let b = corners[face[1]];
                    let c = corners[face[2]];
                    let u: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
                    let v: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
                    let normal = bevy::math::Vec3::from_array(u)
                        .cross(bevy::math::Vec3::from_array(v))
                        .normalize()
                        .to_array();
                    let base = draw.vertices.len() as u32;
                    for (corner, uv) in
                        face.into_iter()
                            .zip([[1.0, 0.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
                    {
                        draw.vertices.push(vertex(
                            part,
                            corners[corner],
                            normal,
                            uv,
                            color,
                            material,
                        )?);
                    }
                    draw.indices
                        .extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
                }
                draw.geometry = "block".into();
            }
            let low: [f32; 3] = std::array::from_fn(|axis| {
                draw.vertices
                    .iter()
                    .map(|v| v.position[axis])
                    .fold(f32::INFINITY, f32::min)
            });
            let high: [f32; 3] = std::array::from_fn(|axis| {
                draw.vertices
                    .iter()
                    .map(|v| v.position[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
            });
            writeln!(report,"draw_loaded name={:?} geometry={} vertices={} triangles={} color={color:?} material={material} min={low:?} max={high:?}",draw.name,draw.geometry,draw.vertices.len(),draw.indices.len()/3).unwrap();
            draws.push(draw);
        }
        writeln!(
            report,
            "geometry_pass draws={} vertices={} triangles={}",
            draws.len(),
            draws.iter().map(|d| d.vertices.len()).sum::<usize>(),
            draws.iter().map(|d| d.indices.len() / 3).sum::<usize>()
        )
        .unwrap();
        writeln!(report,"surface_uvs=not_implemented decals=not_implemented native_behavior_parity=not_tested visual_parity=not_tested").unwrap();
        Ok(Self {
            scene,
            draws,
            report,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn head_stretches_with_sign_branch_and_preserves_offset() {
        assert_eq!(
            head_point(
                [0.5, 0.25, -0.5],
                [2.0, 3.0, 1.0],
                [1.25; 3],
                [1.0, 2.0, 3.0]
            ),
            [1.5, 3.5, 2.5]
        );
        assert_eq!(
            head_point(
                [0.5, -0.25, -0.5],
                [2.0, 3.0, 1.0],
                [1.25; 3],
                [1.0, 2.0, 3.0]
            ),
            [1.5, 0.5, 2.5]
        );
        assert_eq!(
            head_point([0.0, 0.0, 0.0], [2.0, 3.0, 1.0], [1.25; 3], [0.0; 3])[1],
            -1.25
        );
    }
    #[test]
    fn native_specular_is_byte_quantized() {
        assert_eq!(specular(0x100).unwrap(), [102.0 / 255.0, 9.0]);
        assert_eq!(specular(0x430).unwrap(), [238.0 / 255.0, 240.0]);
        assert!(specular(0).is_err());
    }
}
