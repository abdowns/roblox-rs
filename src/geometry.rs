//! Recovered static block and SpecialMesh Head paths. No guessed fallback shapes.
use crate::{
    bundle::Bundle,
    mesh::MeshData,
    native_data::NativeImage,
    scene::{Part, Scene},
    workspace::{PartInstance, Workspace},
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

/// The vertex color of a Workspace Part (010ca980 without a decal).
fn part_color(colors: &BTreeMap<u32, [u8; 3]>, part: &PartInstance) -> Result<[f32; 4], String> {
    let rgb = colors.get(&part.brick_color).ok_or("invalid BrickColor")?;
    Ok([
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        (255 - (part.transparency * 255.0) as u32) as f32 / 255.0,
    ])
}

/// GeometryGenerator::addBlock (010cca80) for a Workspace block Part without
/// a decal: world-space corners from the part CFrame, one quad per Smooth
/// face, and faces with any other surface cut into 8-stud strips along Y
/// (side faces) or Z (Top/Bottom), stepping from the face's first edge. Stud
/// coordinates (types 1-5) run u = width/2 -> 0 and v = row -> row + 0.25 per
/// full strip; the last strip ends at row + clamp(rest/8, 0, 1) * 0.25.
/// Material UVs, tangents and outline edge distances are not produced (the
/// low-quality Plastic shader reads only the stud coordinates; outlines are
/// disabled). Returns the draw and its stud coordinates.
pub fn workspace_block(
    faces: &[[usize; 4]; 6],
    rows: &[f32; 6],
    colors: &BTreeMap<u32, [u8; 3]>,
    part: &PartInstance,
) -> Result<(Draw, Vec<[f32; 2]>), String> {
    let size = part.size;
    let r = &part.frame.rotation;
    let t = part.frame.position;
    let strips_y = (size[1] * 0.125 + 0.999) as i32 as i16;
    let strips_z = (size[2] * 0.125 + 0.999) as i32 as i16;
    let smooth = |face: usize| matches!(part.surfaces[face], 0 | 10);
    let mut quads = 0u32;
    for face in 0..6 {
        let strips = if face == 1 || face == 4 {
            strips_z
        } else {
            strips_y
        };
        quads += if smooth(face) {
            1
        } else {
            strips.max(0) as u32
        };
    }
    if quads > 0x2000 {
        return Err("block exceeds the native 0x2000 quad limit".into());
    }
    let half = size.map(|v| v * 0.5);
    let base: [f32; 3] = std::array::from_fn(|i| {
        -half[2] * r[i][2] + -half[1] * r[i][1] + -half[0] * r[i][0] + t[i]
    });
    let column = |c: usize| [r[0][c], r[1][c], r[2][c]];
    let corners: [[f32; 3]; 8] = std::array::from_fn(|k| {
        let mut p = base;
        for (bit, axis) in [(4, 0), (2, 1), (1, 2)] {
            if k & bit != 0 {
                let c = column(axis);
                p = std::array::from_fn(|i| p[i] + c[i] * size[axis]);
            }
        }
        p
    });
    let unit = |v: [f32; 3]| {
        let inverse = 1.0 / (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
        v.map(|x| inverse * x)
    };
    // Normals add the (disabled) per-face jitter: R + 0 and 0 - R.
    let negate = |v: [f32; 3]| v.map(|x| 0.0 - x);
    let positive = |v: [f32; 3]| v.map(|x| x + 0.0);
    let normals = [
        unit(positive(column(0))),
        unit(positive(column(1))),
        unit(positive(column(2))),
        unit(negate(column(0))),
        unit(negate(column(1))),
        unit(negate(column(2))),
    ];
    let down = column(1).map(|x| -x);
    let back = column(2).map(|x| -x);
    let steps = [down, back, down, down, column(2), down];
    let color = part_color(colors, part)?;
    let specular = specular(part.material)?;
    let mut draw = Draw {
        name: part.name.clone(),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: part.material,
        geometry: "block".into(),
    };
    let mut studs = Vec::new();
    for face in 0..6 {
        let table = faces[face];
        let (a, b) = (corners[table[0]], corners[table[1]]);
        let length = if face == 1 || face == 4 {
            size[2]
        } else {
            size[1]
        };
        let width = if face == 0 || face == 3 {
            size[2]
        } else {
            size[0]
        };
        let count = if smooth(face) {
            1
        } else {
            i32::from(if face == 1 || face == 4 {
                strips_z
            } else {
                strips_y
            })
        };
        let surface = part.surfaces[face] as usize;
        let (u, row, extent) = if (1..=5).contains(&surface) {
            (width * 0.5, rows[surface], 0.25f32)
        } else {
            (0.0, 0.0, 0.0)
        };
        let step = steps[face];
        let mut offset = [0.0f32; 3];
        let mut quad = |positions: [[f32; 3]; 4], v_end: f32| {
            let first = draw.vertices.len() as u32;
            for (position, stud) in
                positions
                    .into_iter()
                    .zip([[u, row], [0.0, row], [u, v_end], [0.0, v_end]])
            {
                draw.vertices.push(RenderVertex {
                    position,
                    normal: normals[face],
                    uv: [0.0; 2],
                    color,
                    specular,
                });
                studs.push(stud);
            }
            draw.indices
                .extend([first, first + 1, first + 2, first + 2, first + 1, first + 3]);
        };
        for _ in 1..count {
            let next: [f32; 3] = std::array::from_fn(|i| step[i] * 8.0 + offset[i]);
            quad(
                [
                    std::array::from_fn(|i| a[i] + offset[i]),
                    std::array::from_fn(|i| offset[i] + b[i]),
                    std::array::from_fn(|i| a[i] + next[i]),
                    std::array::from_fn(|i| next[i] + b[i]),
                ],
                extent + row,
            );
            offset = next;
        }
        if count > 0 {
            let rest = (count - 1) as f32 * -8.0 + length;
            let scaled = if 0.0 <= rest * 0.125 {
                rest * 0.125
            } else {
                0.0
            };
            let fraction = if scaled <= 1.0 { scaled } else { 1.0 };
            quad(
                [
                    std::array::from_fn(|i| a[i] + offset[i]),
                    std::array::from_fn(|i| b[i] + offset[i]),
                    corners[table[2]],
                    corners[table[3]],
                ],
                fraction * extent + row,
            );
        }
    }
    Ok((draw, studs))
}

/// GeometryGenerator's Wedge path (010dc290) for a Workspace WedgePart
/// without a decal. It is addBlock with these changes:
/// - the face table is 017f0744;
/// - face 1 (Top) is skipped;
/// - face 5 is the slope: normal unit(R1 z/y - R2), length |(y, z)|, and
///   strips stepping down it along unit(-(y R1 + z R2));
/// - the +X and -X faces are triangles. Both edges start at the top back
///   corner. One steps straight down (-R1); the other follows the slope
///   (-R1 - (z/y) R2). Its stud u starts at 0 (+X) or keeps width/2 (-X)
///   and moves by 4 z/y per full strip (by 4 z/y times the fraction on the
///   last one), so the studs follow the narrowing face.
///
/// As for blocks, only positions, normals and stud coordinates are
/// produced. The float association of the corner and normal sums follows
/// `workspace_block`; it is not checked against this function's
/// disassembly.
pub fn workspace_wedge(
    faces: &[[usize; 4]; 6],
    rows: &[f32; 6],
    colors: &BTreeMap<u32, [u8; 3]>,
    part: &PartInstance,
) -> Result<(Draw, Vec<[f32; 2]>), String> {
    let size = part.size;
    let r = &part.frame.rotation;
    let t = part.frame.position;
    let slope_length = (size[2] * size[2] + size[1] * size[1]).sqrt();
    // local_2c: each face's strip length.
    let lengths = [size[1], size[2], size[1], size[1], size[2], slope_length];
    let smooth = |face: usize| matches!(part.surfaces[face], 0 | 10);
    let count_of = |face: usize| {
        if smooth(face) {
            1
        } else {
            (lengths[face] * 0.125 + 0.999) as i32
        }
    };
    let quads: i64 = [0, 2, 3, 4, 5]
        .iter()
        .map(|&f| i64::from(count_of(f)))
        .sum();
    if quads > 0x2000 {
        return Err("wedge exceeds the native 0x2000 quad limit".into());
    }
    let half = size.map(|v| v * 0.5);
    let base: [f32; 3] = std::array::from_fn(|i| {
        -half[2] * r[i][2] + -half[1] * r[i][1] + -half[0] * r[i][0] + t[i]
    });
    let column = |c: usize| [r[0][c], r[1][c], r[2][c]];
    let corners: [[f32; 3]; 8] = std::array::from_fn(|k| {
        let mut p = base;
        for (bit, axis) in [(4, 0), (2, 1), (1, 2)] {
            if k & bit != 0 {
                let c = column(axis);
                p = std::array::from_fn(|i| p[i] + c[i] * size[axis]);
            }
        }
        p
    });
    let unit = |v: [f32; 3]| {
        let inverse = 1.0 / (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
        v.map(|x| inverse * x)
    };
    let k = size[2] / size[1];
    let (c0, c1, c2) = (column(0), column(1), column(2));
    let normals = [
        unit(c0.map(|x| 0.0 + x)),
        unit(c1.map(|x| 0.0 + x)),
        unit(c2.map(|x| x + 0.0)),
        unit(c0.map(|x| 0.0 - x)),
        unit(c1.map(|x| 0.0 - x)),
        unit(std::array::from_fn(|i| c1[i] * k + (0.0 - c2[i]))),
    ];
    let down = c1.map(|x| -x);
    let sloped: [f32; 3] = std::array::from_fn(|i| down[i] - c2[i] * k);
    let slope_step = unit(std::array::from_fn(|i| size[1] * down[i] - c2[i] * size[2]));
    // local_1ec: per face, the steps of corners a and b and the stud u
    // rates of a (f6) and b (f7).
    let steps: [([f32; 3], [f32; 3], f32, f32); 6] = [
        (sloped, down, k, 0.0),
        ([0.0; 3], [0.0; 3], 0.0, 0.0),
        (down, down, 0.0, 0.0),
        (down, sloped, 0.0, k),
        (c2, c2, 0.0, 0.0),
        (slope_step, slope_step, 0.0, 0.0),
    ];
    let color = part_color(colors, part)?;
    let specular = specular(part.material)?;
    let mut draw = Draw {
        name: part.name.clone(),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: part.material,
        geometry: "wedge".into(),
    };
    let mut studs = Vec::new();
    for face in [0, 2, 3, 4, 5] {
        let table = faces[face];
        let (a, b) = (corners[table[0]], corners[table[1]]);
        let (step_a, step_b, rate_a, rate_b) = steps[face];
        let width = if face == 0 || face == 3 {
            size[2]
        } else {
            size[0]
        };
        let count = count_of(face);
        let surface = part.surfaces[face] as usize;
        let (studded, row) = if (1..=5).contains(&surface) {
            (true, rows[surface])
        } else {
            (false, 0.0)
        };
        let (extent, u_rate) = if studded {
            (0.25f32, 4.0f32)
        } else {
            (0.0, 0.0)
        };
        let mut u_a = if studded { width * 0.5 } else { 0.0 };
        let mut u_b = if rate_b != 0.0 { u_a } else { 0.0 };
        if rate_a != 0.0 {
            u_a = 0.0;
        }
        let mut quad = |positions: [[f32; 3]; 4], us: [f32; 4], v_end: f32| {
            let first = draw.vertices.len() as u32;
            for (position, stud) in positions.into_iter().zip([
                [us[0], row],
                [us[1], row],
                [us[2], v_end],
                [us[3], v_end],
            ]) {
                draw.vertices.push(RenderVertex {
                    position,
                    normal: normals[face],
                    uv: [0.0; 2],
                    color,
                    specular,
                });
                studs.push(stud);
            }
            draw.indices
                .extend([first, first + 1, first + 2, first + 2, first + 1, first + 3]);
        };
        let (mut off_a, mut off_b) = ([0.0f32; 3], [0.0f32; 3]);
        for _ in 1..count {
            let (start_a, start_b) = (u_a, u_b);
            if rate_b != 0.0 {
                u_b -= u_rate * rate_b;
            }
            if rate_a != 0.0 {
                u_a += u_rate * rate_a;
            }
            let next_a: [f32; 3] = std::array::from_fn(|i| step_a[i] * 8.0 + off_a[i]);
            let next_b: [f32; 3] = std::array::from_fn(|i| step_b[i] * 8.0 + off_b[i]);
            quad(
                [
                    std::array::from_fn(|i| off_a[i] + a[i]),
                    std::array::from_fn(|i| off_b[i] + b[i]),
                    std::array::from_fn(|i| a[i] + next_a[i]),
                    std::array::from_fn(|i| b[i] + next_b[i]),
                ],
                [start_a, start_b, u_a, u_b],
                extent + row,
            );
            off_a = next_a;
            off_b = next_b;
        }
        if count > 0 {
            let rest = (count - 1) as f32 * -8.0 + lengths[face];
            let scaled = if 0.0 <= rest * 0.125 {
                rest * 0.125
            } else {
                0.0
            };
            let fraction = if scaled <= 1.0 { scaled } else { 1.0 };
            let (start_a, start_b) = (u_a, u_b);
            if rate_b != 0.0 {
                u_b -= rate_b * fraction * u_rate;
            }
            if rate_a != 0.0 {
                u_a += u_rate * fraction * rate_a;
            }
            quad(
                [
                    std::array::from_fn(|i| a[i] + off_a[i]),
                    std::array::from_fn(|i| b[i] + off_b[i]),
                    corners[table[2]],
                    corners[table[3]],
                ],
                [start_a, start_b, u_a, u_b],
                extent * fraction + row,
            );
        }
    }
    Ok((draw, studs))
}

/// addSphere's grid for a Ball Part: `tiles` strips per cube face, each
/// `rows` quads tall, and `tiles * rows` quads across. A Part whose six
/// surfaces are all Smooth (isPartSmooth 010cae50) gets one tile; otherwise
/// tiles = ceil(min(size.x, size.y, size.z, 350) / 7) and
/// rows = (tiles + 5) / tiles. Zero tiles draw nothing.
pub fn sphere_grid(part: &PartInstance, tiling: [f32; 2]) -> (u32, u32) {
    let (_, clamped) = sphere_side(part, tiling);
    let tiles = if part.surfaces.iter().all(|s| *s == 0) {
        1.0
    } else {
        (clamped / tiling[1]).ceil()
    };
    let tiles = tiles as i64 as u32;
    if tiles == 0 {
        return (0, 0);
    }
    (tiles, (tiles + 5) / tiles)
}

/// MINSS dest, src.
fn minss(dest: f32, src: f32) -> f32 {
    if dest < src { dest } else { src }
}

/// addSphere's smallest side min(z, min(y, x)) and that side clamped to 350.
fn sphere_side(part: &PartInstance, tiling: [f32; 2]) -> (f32, f32) {
    let size = part.size;
    let side = minss(size[2], minss(size[1], size[0]));
    (side, minss(tiling[0], side))
}

/// GeometryGenerator::addSphere (010caee0) for a Workspace Ball Part without
/// a decal: a cube sphere. Each cube face (corners from the block face table,
/// stepping along the column directions 0224e0f0 and the row directions
/// 0224e0a0) is cut into `tiles` strips of `rows + 1` vertex rows, each row
/// `tiles * rows + 1` vertices across; a strip's last row repeats as the next
/// strip's first. Every grid point d on the [-1, 1] cube is projected to
/// size * 0.5 * d / |d|, with the ellipsoid normal (d / |d|) / (size * 0.5).
/// Faces with surface types 1-5 get stud coordinates: u runs from
/// side / (2 tiles) down by side / (2 tiles rows) per vertex, v starts each
/// strip at frac(strip * side / tiles) / 8 * 0.25 + row and grows by
/// side * 0.25 / (8 tiles rows) per row (side = the clamped smallest size).
/// Material UVs, tangents and outline edge distances are not produced, as
/// for blocks. Returns the draw and its stud coordinates.
pub fn workspace_sphere(
    faces: &[[usize; 4]; 6],
    rows: &[f32; 6],
    tiling: [f32; 2],
    colors: &BTreeMap<u32, [u8; 3]>,
    part: &PartInstance,
) -> Result<(Draw, Vec<[f32; 2]>), String> {
    // The cube corners 0224e02c: bit 4 = +X, bit 2 = +Y, bit 1 = +Z.
    let corner = |k: usize| -> [f32; 3] {
        [
            if k & 4 != 0 { 1.0 } else { -1.0 },
            if k & 2 != 0 { 1.0 } else { -1.0 },
            if k & 1 != 0 { 1.0 } else { -1.0 },
        ]
    };
    const ROW_STEP: [[f32; 3]; 6] = [
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, -1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
    ];
    const COLUMN_STEP: [[f32; 3]; 6] = [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let mut draw = Draw {
        name: part.name.clone(),
        vertices: Vec::new(),
        indices: Vec::new(),
        material: part.material,
        geometry: "sphere".into(),
    };
    let mut studs = Vec::new();
    let (tiles, strip_rows) = sphere_grid(part, tiling);
    if tiles == 0 {
        return Ok((draw, studs));
    }
    let size = part.size;
    let (side, clamped) = sphere_side(part, tiling);
    let tiles_f = tiles as f32;
    let rows_f = strip_rows as f32;
    let across = tiles * strip_rows + 1;
    let total = 6 * (strip_rows + 1) * tiles * across;
    if total > 0x10000 {
        return Err("sphere exceeds 16-bit vertex indices".into());
    }
    let fraction = 1.0 / (tiles_f * rows_f);
    let half = (side * 0.5) / side;
    let color = part_color(colors, part)?;
    let specular = specular(part.material)?;
    let r = &part.frame.rotation;
    let t = part.frame.position;
    for face in 0..6 {
        let mut start = corner(faces[face][0]);
        // The half steps are doubled once and added once per vertex or row.
        let row_step = ROW_STEP[face].map(|v| fraction * v).map(|v| v + v);
        let column_step = COLUMN_STEP[face].map(|v| fraction * v).map(|v| v + v);
        let surface = part.surfaces[face] as usize;
        let (row, extent, u_step) = if (1..=5).contains(&surface) {
            (rows[surface], 0.25f32, clamped * fraction * 0.5)
        } else {
            (0.0, 0.0, 0.0)
        };
        for strip in 0..tiles {
            let first = draw.vertices.len() as u32;
            let x = (strip as f32 * clamped) / tiles_f;
            let mut v = (x - x.floor()) * 0.125 * extent + row;
            for row_index in 0..=strip_rows {
                let mut d = start;
                let mut u = rows_f * u_step;
                for _ in 0..across {
                    // Sums run x, y, z as the disassembly shows (010cc130..).
                    let inverse = 1.0 / ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]).sqrt();
                    let unit = d.map(|v| v * inverse);
                    let scale = size.map(|v| v * half);
                    let local: [f32; 3] = std::array::from_fn(|i| scale[i] * unit[i] + 0.0);
                    let n: [f32; 3] = std::array::from_fn(|i| unit[i] / scale[i]);
                    let position: [f32; 3] = std::array::from_fn(|i| {
                        ((local[0] * r[i][0] + local[1] * r[i][1]) + local[2] * r[i][2]) + t[i]
                    });
                    let w: [f32; 3] =
                        std::array::from_fn(|i| (r[i][0] * n[0] + r[i][1] * n[1]) + r[i][2] * n[2]);
                    let length = 1.0 / ((w[0] * w[0] + w[1] * w[1]) + w[2] * w[2]).sqrt();
                    draw.vertices.push(RenderVertex {
                        position,
                        normal: w.map(|v| v * length),
                        uv: [0.0; 2],
                        color,
                        specular,
                    });
                    studs.push([u, v]);
                    d = std::array::from_fn(|i| d[i] + column_step[i]);
                    u -= u_step;
                }
                if row_index < strip_rows {
                    start = std::array::from_fn(|i| start[i] + row_step[i]);
                }
                v += clamped * extent * fraction * 0.125;
            }
            for row_index in 0..strip_rows {
                for column in 0..across - 1 {
                    let a = first + row_index * across + column;
                    let b = a + across;
                    draw.indices.extend([a, a + 1, b, b, a + 1, b + 1]);
                }
            }
        }
    }
    if draw.vertices.len() as u32 != total
        || draw.indices.len() as u32 != 36 * (across - 1) * (across - 1)
    {
        return Err("sphere vertex or quad count mismatch".into());
    }
    Ok((draw, studs))
}

/// The mesh half of `audit-ball-render`: Ball draws from `workspace_sphere`
/// checked against addSphere's own counts (its asserts "faceCounter ==
/// quads" and "vertexCounter == totalVerts") and against the sphere they
/// approximate: vertex distance from the center, normals along the radius,
/// outward winding, closed seams (6 N^2 + 2 distinct points for N quads
/// across a face) and the stud coordinate ranges per face.
pub fn sphere_mesh_report(bundle: &Bundle) -> Result<String, String> {
    use crate::workspace::{SHAPE_BALL, SURFACE_SMOOTH};
    let image = NativeImage::load(bundle)?;
    let (faces, rows, colors, tiling) = (
        image.block_faces()?,
        image.surface_rows()?,
        image.brick_colors()?,
        image.sphere_tiling()?,
    );
    // 30 degrees about (1, 1, 0) / sqrt(2).
    let (c, s) = (30.0f32.to_radians().cos(), 30.0f32.to_radians().sin());
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let axis = [k, k, 0.0];
    let tilted: [[f32; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let cross = [
                [0.0, -axis[2], axis[1]],
                [axis[2], 0.0, -axis[0]],
                [-axis[1], axis[0], 0.0],
            ];
            let identity = if i == j { 1.0 } else { 0.0 };
            c * identity + s * cross[i][j] + (1.0 - c) * axis[i] * axis[j]
        })
    });
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let cases = [
        ("Pebble", 1.0, false, identity),
        ("Boulder", 4.0, false, identity),
        ("Smooth", 20.0, true, identity),
        ("Big", 20.0, false, identity),
        ("Huge", 400.0, false, identity),
        ("Tilted", 6.0, false, tilted),
    ];
    let mut report = String::new();
    let mut failures = Vec::new();
    for (name, diameter, smooth, rotation) in cases {
        let mut part = PartInstance {
            name: name.into(),
            ..Default::default()
        };
        part.set_shape(SHAPE_BALL);
        part.set_size([diameter; 3]);
        part.frame.position = [3.0, 7.0, -5.0];
        part.frame.rotation = rotation;
        if smooth {
            part.surfaces = [SURFACE_SMOOTH; 6];
        }
        let (draw, studs) = workspace_sphere(&faces, &rows, tiling, &colors, &part)?;
        let (tiles, strip_rows) = sphere_grid(&part, tiling);
        let across = tiles * strip_rows;
        let vertices = 6 * (strip_rows + 1) * tiles * (across + 1);
        let quads = 6 * across * across;
        let radius = diameter * 0.5;
        let center = part.frame.position;
        let offset = |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| p[i] - center[i]) };
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut radius_error = 0.0f32;
        let mut normal_error = 0.0f32;
        let mut least_alignment = 1.0f32;
        for v in &draw.vertices {
            let d = offset(v.position);
            let l = length(d);
            radius_error = radius_error.max((l / radius - 1.0).abs());
            normal_error = normal_error.max((length(v.normal) - 1.0).abs());
            let along = (v.normal[0] * d[0] + v.normal[1] * d[1] + v.normal[2] * d[2]) / l;
            least_alignment = least_alignment.min(along);
        }
        let mut inward = 0;
        for t in draw.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|i| draw.vertices[t[i] as usize].position);
            let e1: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let e2: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let mid = offset(std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0));
            if n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] <= 0.0 {
                inward += 1;
            }
        }
        // Distinct points (within 1e-4 of the radius), for small meshes.
        let distinct = if draw.vertices.len() <= 2000 {
            let mut points: Vec<[f32; 3]> = Vec::new();
            for v in &draw.vertices {
                let near = points.iter().any(|p| {
                    length(std::array::from_fn(|i| p[i] - v.position[i])) <= radius * 1e-4
                });
                if !near {
                    points.push(v.position);
                }
            }
            Some(points.len() as u32)
        } else {
            None
        };
        let per_face = vertices as usize / 6;
        let mut ranges = Vec::new();
        for face in 0..6 {
            let range = studs[face * per_face..(face + 1) * per_face].iter().fold(
                [f32::MAX, f32::MIN, f32::MAX, f32::MIN],
                |r, s| {
                    [
                        r[0].min(s[0]),
                        r[1].max(s[0]),
                        r[2].min(s[1]),
                        r[3].max(s[1]),
                    ]
                },
            );
            ranges.push(range);
        }
        writeln!(
            report,
            "ball_mesh name={name:?} diameter={diameter} smooth={smooth} tiles={tiles} rows={strip_rows} vertices={} expected_vertices={vertices} quads={} expected_quads={quads} radius_error={radius_error} normal_length_error={normal_error} least_normal_alignment={least_alignment} inward_triangles={inward} distinct_points={distinct:?} expected_distinct={}",
            draw.vertices.len(),
            draw.indices.len() / 6,
            6 * across * across + 2
        )
        .unwrap();
        writeln!(
            report,
            "ball_mesh_studs name={name:?} surfaces={:?} u_min_max_v_min_max={ranges:?}",
            part.surfaces
        )
        .unwrap();
        let closed = distinct.is_none_or(|d| d == 6 * across * across + 2);
        if draw.vertices.len() as u32 != vertices
            || draw.indices.len() as u32 != quads * 6
            || radius_error > 1e-5
            || normal_error > 1e-5
            || least_alignment < 0.99999
            || inward != 0
            || !closed
        {
            failures.push(name);
        }
    }
    if !failures.is_empty() {
        return Err(format!("ball mesh failed {failures:?}\n{report}"));
    }
    Ok(report)
}

/// Draws and stud coordinates for every Workspace Part (Blocks and Balls).
#[allow(clippy::type_complexity)] // Draws plus their per-vertex stud coordinates.
pub fn workspace_draws(
    bundle: &Bundle,
    workspace: &Workspace,
) -> Result<(Vec<Draw>, BTreeMap<String, Vec<[f32; 2]>>, String), String> {
    let image = NativeImage::load(bundle)?;
    let (faces, rows, colors, tiling) = (
        image.block_faces()?,
        image.surface_rows()?,
        image.brick_colors()?,
        image.sphere_tiling()?,
    );
    let wedge_faces = image.wedge_faces()?;
    let mut draws = Vec::new();
    let mut coordinates = BTreeMap::new();
    let mut report = String::new();
    for part in &workspace.parts {
        if part.shape == crate::workspace::SHAPE_BALL {
            let (draw, studs) = workspace_sphere(&faces, &rows, tiling, &colors, part)?;
            let (tiles, strip_rows) = sphere_grid(part, tiling);
            let center = part.frame.position;
            let (near, far) = draw.vertices.iter().fold((f32::MAX, 0.0f32), |(n, f), v| {
                let d: [f32; 3] = std::array::from_fn(|i| v.position[i] - center[i]);
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                (n.min(l), f.max(l))
            });
            writeln!(
                report,
                "workspace_draw name={:?} geometry=sphere tiles={tiles} rows={strip_rows} vertices={} triangles={} color={:?} radius_range=[{near}, {far}] stud_extent={:?}",
                draw.name,
                draw.vertices.len(),
                draw.indices.len() / 3,
                draw.vertices.first().map(|v| v.color),
                studs.iter().fold([0.0f32; 2], |m, s| [m[0].max(s[0]), m[1].max(s[1])])
            )
            .unwrap();
            if coordinates.insert(draw.name.clone(), studs).is_some() {
                return Err(format!("duplicate Workspace Part name {:?}", draw.name));
            }
            draws.push(draw);
            continue;
        }
        if part.shape == crate::workspace::SHAPE_WEDGE {
            let (draw, studs) = workspace_wedge(&wedge_faces, &rows, &colors, part)?;
            writeln!(
                report,
                "workspace_draw name={:?} geometry=wedge quads={} vertices={} triangles={} color={:?} stud_extent={:?}",
                draw.name,
                draw.vertices.len() / 4,
                draw.vertices.len(),
                draw.indices.len() / 3,
                draw.vertices.first().map(|v| v.color),
                studs.iter().fold([0.0f32; 2], |m, s| [m[0].max(s[0]), m[1].max(s[1])])
            )
            .unwrap();
            if coordinates.insert(draw.name.clone(), studs).is_some() {
                return Err(format!("duplicate Workspace Part name {:?}", draw.name));
            }
            draws.push(draw);
            continue;
        }
        if part.shape != crate::workspace::SHAPE_BLOCK {
            return Err(format!(
                "Part {:?}: only Block, Ball and Wedge Parts render",
                part.name
            ));
        }
        let (draw, studs) = workspace_block(&faces, &rows, &colors, part)?;
        writeln!(
            report,
            "workspace_draw name={:?} quads={} vertices={} triangles={} color={:?} top_stud_extent={:?}",
            draw.name,
            draw.vertices.len() / 4,
            draw.vertices.len(),
            draw.indices.len() / 3,
            draw.vertices[0].color,
            studs.iter().fold([0.0f32; 2], |m, s| [m[0].max(s[0]), m[1].max(s[1])])
        )
        .unwrap();
        let mut start = 0;
        for face in 0..6 {
            // Quads of this face: consecutive vertices with the same normal.
            let normal = draw.vertices[start].normal;
            let mut end = start;
            while end < draw.vertices.len() && draw.vertices[end].normal == normal {
                end += 4;
            }
            let quad: Vec<_> = (start..start + 4)
                .map(|i| (draw.vertices[i].position, studs[i]))
                .collect();
            writeln!(
                report,
                "workspace_face name={:?} face={face} surface={} quads={} normal={normal:?} first_quad={quad:?} last_v_end={}",
                draw.name,
                part.surfaces[face],
                (end - start) / 4,
                studs[end - 1][1]
            )
            .unwrap();
            start = end;
        }
        if coordinates.insert(draw.name.clone(), studs).is_some() {
            return Err(format!("duplicate Workspace Part name {:?}", draw.name));
        }
        draws.push(draw);
    }
    Ok((draws, coordinates, report))
}

impl RenderScene {
    pub fn character(bundle: &Bundle) -> Result<Self, String> {
        Self::character_geometry(bundle, false)
    }

    /// Keep source positions in Part space for later frame updates. Building
    /// these from original inputs avoids a world-to-local rounding round trip.
    pub(crate) fn character_local(bundle: &Bundle) -> Result<Self, String> {
        Self::character_geometry(bundle, true)
    }

    fn character_geometry(bundle: &Bundle, local: bool) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let colors = image.brick_colors()?;
        let faces = image.block_faces()?;
        let body_sizes = image.humanoid_mesh_sizes()?;
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
            let mut geometry_part = part.clone();
            if local {
                geometry_part.frame.position = [0.0; 3];
                geometry_part.frame.rotation = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            }
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
            // HumanoidIdentifier scans the Humanoid's model children, rather
            // than recognizing similarly named Parts elsewhere in the scene.
            let humanoid_sibling = scene
                .instances
                .iter()
                .any(|other| other.class == "Humanoid" && other.parent == instance.parent);
            let body_mesh = if humanoid_sibling {
                match instance.name.as_str() {
                    "Torso" => Some(("torso.mesh", 1)),
                    "Left Leg" => Some(("leftleg.mesh", 2)),
                    "Right Leg" => Some(("rightleg.mesh", 2)),
                    "Left Arm" => Some(("leftarm.mesh", 3)),
                    "Right Arm" => Some(("rightarm.mesh", 3)),
                    _ => None,
                }
            } else {
                None
            };
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
                        &geometry_part,
                        head_point(v.position, part.size, scale, offset),
                        v.normal,
                        [v.uv[0], v.uv[1]],
                        color,
                        material,
                    )?);
                }
                draw.indices = mesh.triangles.into_iter().flatten().collect();
                draw.geometry = "special_head".into();
            } else if let Some((path, kind)) = body_mesh {
                if part.shape != 1 || !matches!(material, 0x100 | 0x110) {
                    return Err("unsupported Humanoid body composition predicate".into());
                }
                let mesh = MeshData::parse(&bundle.read(
                    &format!("Contents/Resources/content/fonts/{path}"),
                    64 * 1024 * 1024,
                )?)?;
                mesh.finite_bounds()?;
                let scale: [f32; 3] = std::array::from_fn(|a| part.size[a] / body_sizes[kind][a]);
                for v in mesh.vertices {
                    draw.vertices.push(vertex(
                        &geometry_part,
                        std::array::from_fn(|a| v.position[a] * scale[a]),
                        v.normal,
                        [v.uv[0], v.uv[1]],
                        color,
                        material,
                    )?);
                }
                draw.indices = mesh.triangles.into_iter().flatten().collect();
                draw.geometry = "humanoid_body_mesh".into();
                writeln!(report, "body_mesh_selected name={:?} path={path} native_size={:?} scale={scale:?} flags=0x64", instance.name, body_sizes[kind]).unwrap();
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
                            &geometry_part,
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
            // isBodyPartComposited accepts these opaque Plastic R6 parts at
            // zero reflectance. addFileMesh's 0x40 branch clears specular data.
            if humanoid_sibling
                && matches!(
                    draw.geometry.as_str(),
                    "special_head" | "humanoid_body_mesh"
                )
            {
                for v in &mut draw.vertices {
                    v.specular = [0.0; 2];
                }
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
        writeln!(report,"geometry_only_audit surface_uvs=separate_surface_stage decals=separate_texture_stage native_behavior_parity=not_tested visual_parity=not_tested").unwrap();
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
    fn wedge_draw_lies_on_the_wedge_mesh() {
        // The 017f0744 corner columns (see docs/RESEARCH.md).
        let faces = [
            [7, 7, 4, 5],
            [3, 7, 3, 7],
            [7, 3, 5, 1],
            [3, 3, 1, 0],
            [0, 4, 1, 5],
            [3, 7, 0, 4],
        ];
        let colors = BTreeMap::from([(194, [163, 162, 165])]);
        let mut part = PartInstance {
            name: "W".into(),
            ..Default::default()
        };
        part.set_shape(crate::workspace::SHAPE_WEDGE);
        part.set_size([4.0, 10.0, 20.0]);
        part.surfaces = [3; 6];
        let (draw, studs) = workspace_wedge(&faces, &[0.0; 6], &colors, &part).unwrap();
        let mesh = crate::poly::Mesh::wedge(part.size);
        // Strips: +X, Back, -X 2 each (10 tall), Bottom 3, slope 3 (22.4).
        assert_eq!(draw.vertices.len(), 4 * 12);
        for v in &draw.vertices {
            let face = mesh
                .faces
                .iter()
                .find(|f| {
                    let n = f.plane.normal;
                    (n[0] - v.normal[0]).abs() < 1e-5
                        && (n[1] - v.normal[1]).abs() < 1e-5
                        && (n[2] - v.normal[2]).abs() < 1e-5
                })
                .expect("normal of a wedge face");
            let p = v.position;
            let d = |f: &crate::poly::Face| {
                (f.plane.normal[0] * p[0] + f.plane.normal[1] * p[1]) + f.plane.normal[2] * p[2]
                    - f.plane.distance
            };
            assert!(d(face).abs() < 1e-4, "{p:?} off its face");
            assert!(mesh.faces.iter().all(|f| d(f) < 1e-4), "{p:?} outside");
        }
        // The +X triangle's studs: u grows from 0 to width/2 = 10.
        assert_eq!(studs[0], [0.0, 0.0]);
        assert!((studs[6][0] - 10.0).abs() < 1e-5);
    }
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
