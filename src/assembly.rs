//! Recovered block mass and recursive Body/Cofm aggregation for the R6 tree.
use crate::{
    bundle::Bundle,
    native_data::NativeImage,
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
};
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind};
use std::{collections::BTreeMap, fmt::Write};

pub type Tensor = [[f32; 3]; 3];

pub struct MassProfile {
    plastic_density: f32,
    root_mass: f32,
    three: f32,
    /// 01834ffc: 4/3 pi (Ball::getVolume 00adc148).
    ball_volume: f32,
    /// 016f753c: 0.4 (Ball::getMoment 00adcb76).
    two_fifths: f32,
}

#[derive(Clone, Debug)]
pub struct MassPart {
    pub name: String,
    pub frame: CoordinateFrame,
    pub mass: f32,
    /// The geometry's moment (vtable +0x68), about the Part's center.
    pub inertia: Tensor,
    /// The center of mass in the Part's frame (geometry vtable +0x64, Body
    /// +0xa4): zero except for a Wedge.
    pub center: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct AssemblyProperties {
    pub mass: f32,
    pub center_world: [f32; 3],
    pub center_in_body: [f32; 3],
    pub inertia_in_body: Tensor,
    pub body_frame: CoordinateFrame,
    pub parts: usize,
}

pub(crate) fn transpose(m: Tensor) -> Tensor {
    std::array::from_fn(|r| std::array::from_fn(|c| m[c][r]))
}
pub(crate) fn multiply(a: Tensor, b: Tensor) -> Tensor {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| a[r][2] * b[2][c] + a[r][1] * b[1][c] + a[r][0] * b[0][c])
    })
}
pub(crate) fn rotate_tensor(m: Tensor, r: Tensor) -> Tensor {
    multiply(multiply(r, m), transpose(r))
}
pub(crate) fn rotate(r: Tensor, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| r[a][2] * v[2] + r[a][1] * v[1] + r[a][0] * v[0])
}
fn shift(t: Tensor, mass: f32, from: [f32; 3], to: [f32; 3]) -> Tensor {
    // FUN_006b0140: symmetric parallel-axis matrix, no diagonal-only shortcut.
    let [x, y, z] = std::array::from_fn(|a| to[a] - from[a]);
    let p = [
        [y * y + z * z, -(x * y), -(x * z)],
        [-(x * y), z * z + x * x, -(z * y)],
        [-(x * z), -(z * y), y * y + x * x],
    ];
    std::array::from_fn(|r| std::array::from_fn(|c| t[r][c] + mass * p[r][c]))
}

impl MassProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            let x = f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap());
            if !x.is_finite() || x <= 0.0 {
                return Err("invalid native mass constant".into());
            }
            Ok(x)
        };
        let instruction = Decoder::with_ip(
            32,
            image.region(0x00bdb6a6, 8)?,
            0x00bdb6a6,
            DecoderOptions::NONE,
        )
        .decode();
        if instruction.mnemonic() != Mnemonic::Mov || instruction.op1_kind() != OpKind::Immediate32
        {
            return Err("native R6 root-mass instruction changed".into());
        }
        let root_mass = f32::from_bits(instruction.immediate32());
        if !root_mass.is_finite() || root_mass <= 0.0 {
            return Err("invalid R6 root mass".into());
        }
        Ok(Self {
            plastic_density: float(0x01782284)?,
            root_mass,
            three: float(0x01834cd8)?,
            ball_volume: float(0x01834ffc)?,
            two_fifths: float(0x016f753c)?,
        })
    }

    /// Ball::getVolume 00adc148: ((4/3 pi r) r) r.
    pub fn ball_volume(&self, radius: f32) -> f32 {
        ((self.ball_volume * radius) * radius) * radius
    }

    /// Ball::getMoment 00adcb76: ((m 0.4) r) r on the diagonal.
    pub fn ball_inertia(&self, radius: f32, mass: f32) -> Result<Tensor, String> {
        if !radius.is_finite() || radius <= 0.0 || !mass.is_finite() || mass <= 0.0 {
            return Err("invalid ball mass/radius".into());
        }
        let moment = ((mass * self.two_fifths) * radius) * radius;
        if !moment.is_finite() || moment <= 0.0 {
            return Err("native ball inertia overflow".into());
        }
        Ok(std::array::from_fn(|r| {
            std::array::from_fn(|c| if r == c { moment } else { 0.0 })
        }))
    }

    /// Native Block::getMoment, FUN_00adee8a. Its surface-weighted polynomial
    /// differs from the uniform-solid-box m/12 formula. Preserve SSE order.
    #[allow(clippy::assign_op_pattern)] // Keep the observed SSE operand order.
    pub fn block_inertia(&self, size: [f32; 3], mass: f32) -> Result<Tensor, String> {
        if size.iter().any(|x| !x.is_finite() || *x <= 0.0) || !mass.is_finite() || mass <= 0.0 {
            return Err("invalid block mass/size".into());
        }
        let [x, y, z] = size;
        let area = z * x + (y * z + x * y);
        let area = area + area;
        let factor = mass / (area + area);
        let diagonal: [f32; 3] = std::array::from_fn(|axis| {
            let a = size[axis];
            let b = size[(axis + 1) % 3];
            let c = size[(axis + 2) % 3];
            let term1 = ((b * b) * b) * c / self.three;
            let term2 = ((b * c) * c) * c / self.three;
            let ab = a * b;
            let abb = b * ab;
            let mut v = ((ab * c) * c) + (term2 + term1);
            v = (b * abb) / self.three + v;
            v = abb * c + v;
            v = ((a * c) * c) * c / self.three + v;
            v * factor
        });
        if diagonal.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("native block inertia overflow".into());
        }
        Ok(std::array::from_fn(|r| {
            std::array::from_fn(|c| if r == c { diagonal[r] } else { 0.0 })
        }))
    }

    pub fn character_parts(
        &self,
        bundle: &Bundle,
        scene: &Scene,
    ) -> Result<BTreeMap<String, MassPart>, String> {
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let mut parts = BTreeMap::new();
        for instance in &scene.instances {
            let Some(part) = &instance.part else { continue };
            if part.shape != 1 {
                return Err("assembly character boundary requires legacy Block shape=1".into());
            }
            let item = doc
                .descendants()
                .find(|n| {
                    n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent)
                })
                .ok_or("missing mass Part")?;
            let props = item
                .children()
                .find(|n| n.has_tag_name("Properties"))
                .ok_or("missing mass properties")?;
            let prop = |name| {
                props
                    .children()
                    .find(|n| n.attribute("name") == Some(name))
                    .ok_or_else(|| format!("missing mass property {name}"))
            };
            let material: u32 = prop("Material")?
                .text()
                .ok_or("missing material")?
                .parse()
                .map_err(|_| "invalid material")?;
            let custom = prop("CustomPhysicalProperties")?;
            let enabled = custom
                .children()
                .find(|n| n.has_tag_name("CustomPhysics"))
                .and_then(|n| n.text())
                .ok_or("missing CustomPhysics")?;
            let density = match enabled {
                "false" if material == 256 => self.plastic_density,
                "true" => custom
                    .children()
                    .find(|n| n.has_tag_name("Density"))
                    .and_then(|n| n.text())
                    .ok_or("missing custom density")?
                    .parse::<f32>()
                    .map_err(|_| "invalid density")?,
                _ => return Err("unsupported native character mass material".into()),
            };
            let mass = if instance.name == "HumanoidRootPart" {
                self.root_mass
            } else {
                (part.size[0] * part.size[1]) * part.size[2] * density
            };
            let inertia = self.block_inertia(part.size, mass)?;
            if parts
                .insert(
                    instance.name.clone(),
                    MassPart {
                        name: instance.name.clone(),
                        frame: part.frame.clone(),
                        mass,
                        inertia,
                        center: [0.0; 3],
                    },
                )
                .is_some()
            {
                return Err("duplicate mass Part".into());
            }
        }
        Ok(parts)
    }
}

impl AssemblyProperties {
    /// A Body plus its child Cofm records, in native child order (00a90502).
    #[allow(clippy::assign_op_pattern)] // Child mass is the left native addend.
    pub fn combine(parent: &MassPart, children: &[Self]) -> Result<Self, String> {
        if !parent.mass.is_finite()
            || parent.mass <= 0.0
            || parent
                .frame
                .position
                .iter()
                .chain(parent.frame.rotation.iter().flatten())
                .chain(parent.inertia.iter().flatten())
                .any(|x| !x.is_finite())
        {
            return Err("invalid assembly parent".into());
        }
        if children.is_empty() {
            // Native no-children branch copies mass and offset and moves the
            // moment to the offset (006b0234). A Body with no offset has no
            // Cofm: its parent reads its position (00a8f4ea) and moment.
            let center_world = if parent.center == [0.0; 3] {
                parent.frame.position
            } else {
                crate::block_contact::to_world(&parent.frame, parent.center)
            };
            return Ok(Self {
                mass: parent.mass,
                center_world,
                center_in_body: parent.center,
                inertia_in_body: crate::poly::moment_about(
                    parent.inertia,
                    parent.mass,
                    parent.center,
                ),
                body_frame: parent.frame.clone(),
                parts: 1,
            });
        }
        let mut mass = parent.mass;
        // The parent's center of mass in world space times its mass.
        let mut weighted =
            crate::block_contact::to_world(&parent.frame, parent.center).map(|v| v * mass);
        let mut count = 1;
        for child in children {
            if !child.mass.is_finite() || child.mass <= 0.0 {
                return Err("invalid child Cofm mass".into());
            }
            mass = child.mass + mass;
            weighted = std::array::from_fn(|a| child.center_world[a] * child.mass + weighted[a]);
            count += child.parts;
        }
        let inverse = 1.0 / mass;
        let center_world = weighted.map(|v| v * inverse);
        let delta = std::array::from_fn(|a| center_world[a] - parent.frame.position[a]);
        let center_in_body = rotate(transpose(parent.frame.rotation), delta);
        let mut inertia = shift(
            rotate_tensor(parent.inertia, parent.frame.rotation),
            parent.mass,
            parent.frame.position,
            center_world,
        );
        for child in children {
            let t = shift(
                rotate_tensor(child.inertia_in_body, child.body_frame.rotation),
                child.mass,
                child.center_world,
                center_world,
            );
            inertia = std::array::from_fn(|r| std::array::from_fn(|c| inertia[r][c] + t[r][c]));
        }
        let inertia_in_body = rotate_tensor(inertia, transpose(parent.frame.rotation));
        if !mass.is_finite()
            || center_world
                .iter()
                .chain(&center_in_body)
                .chain(inertia_in_body.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err("assembly aggregation overflow".into());
        }
        Ok(Self {
            mass,
            center_world,
            center_in_body,
            inertia_in_body,
            body_frame: parent.frame.clone(),
            parts: count,
        })
    }
    pub fn from_rig(parts: &BTreeMap<String, MassPart>, rig: &R6Rig) -> Result<Self, String> {
        fn visit(
            name: &str,
            parts: &BTreeMap<String, MassPart>,
            rig: &R6Rig,
            path: &mut Vec<String>,
        ) -> Result<AssemblyProperties, String> {
            if path.iter().any(|n| n == name) {
                return Err("cyclic mass tree".into());
            }
            let parent = parts.get(name).ok_or("missing assembly Part")?;
            path.push(name.into());
            let children = rig
                .joints
                .iter()
                .filter(|j| j.part0 == name)
                .map(|j| visit(&j.part1, parts, rig, path))
                .collect::<Result<Vec<_>, _>>()?;
            path.pop();
            AssemblyProperties::combine(parent, &children)
        }
        let result = visit("HumanoidRootPart", parts, rig, &mut Vec::new())?;
        if result.parts != parts.len() {
            return Err("mass tree does not cover character".into());
        }
        Ok(result)
    }
    /// SimBody 00ab16aa takes Cofm's diagonal, reciprocates it, then rotates to
    /// world space. This is deliberately not a general inverse of the tensor.
    pub fn sim_inverse_inertia_world(&self) -> Result<Tensor, String> {
        let mut diagonal = [[0.0; 3]; 3];
        for (a, row) in diagonal.iter_mut().enumerate() {
            let value = self.inertia_in_body[a][a];
            if !value.is_finite() || value <= 0.0 {
                return Err("invalid SimBody inertia diagonal".into());
            }
            row[a] = 1.0 / value;
        }
        Ok(rotate_tensor(diagonal, self.body_frame.rotation))
    }
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = MassProfile::load(bundle)?;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let rig = R6Rig::character(bundle, &scene)?;
    let parts = profile.character_parts(bundle, &scene)?;
    let assembly = AssemblyProperties::from_rig(&parts, &rig)?;
    let mut report = format!(
        "native_mass_profile plastic_density={} root_override={} three={} shape=legacy_block root_branch=explicit_r6\n",
        profile.plastic_density, profile.root_mass, profile.three
    );
    for part in parts.values() {
        writeln!(
            report,
            "mass_part name={:?} mass={} inertia={:?}",
            part.name, part.mass, part.inertia
        )
        .unwrap();
    }
    writeln!(report,"assembly mass={} center_world={:?} center_in_body={:?} inertia_body={:?} inverse_inertia_world={:?}",assembly.mass,assembly.center_world,assembly.center_in_body,assembly.inertia_in_body,assembly.sim_inverse_inertia_world()?).unwrap();
    let sum: f64 = parts.values().map(|p| f64::from(p.mass)).sum();
    if assembly.parts != 7 || (f64::from(assembly.mass) - sum).abs() > 0.00001 {
        return Err("assembly mass conservation failed".into());
    }
    let shifted_parts: BTreeMap<_, _> = parts
        .iter()
        .map(|(name, part)| {
            let mut p = part.clone();
            p.frame.position = std::array::from_fn(|a| p.frame.position[a] + [8.0, -4.0, 2.0][a]);
            (name.clone(), p)
        })
        .collect();
    let translated = AssemblyProperties::from_rig(&shifted_parts, &rig)?;
    let error = assembly
        .inertia_in_body
        .iter()
        .flatten()
        .zip(translated.inertia_in_body.iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    if error > 0.0001 {
        return Err(format!("assembly translation invariant failed {error}"));
    }
    writeln!(report,"assembly_pass parts=7 mass_conserved=true translation_inertia_error={error} root_selection=explicit kernel_integration=not_implemented native_differential=not_tested").unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> MassProfile {
        MassProfile {
            plastic_density: 0.7,
            root_mass: 0.1,
            three: 3.0,
            ball_volume: f32::from_bits(0x4086_0a92),
            two_fifths: 0.4,
        }
    }
    #[test]
    fn ball_volume_and_moment() {
        let p = profile();
        // 4/3 pi 2^3 and 0.4 m r^2.
        assert!((p.ball_volume(2.0) - 33.510323).abs() < 1e-4);
        let t = p.ball_inertia(2.0, 5.0).unwrap();
        assert_eq!(t[0][0], 8.0);
        assert_eq!((t[1][1], t[2][2], t[0][1]), (8.0, 8.0, 0.0));
    }
    fn part(p: [f32; 3]) -> MassPart {
        MassPart {
            name: "synthetic".into(),
            frame: CoordinateFrame {
                position: p,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            mass: 1.0,
            inertia: [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]],
            center: [0.0; 3],
        }
    }
    #[test]
    fn block_inertia_matches_surface_integral_not_solid_box() {
        let cube = profile().block_inertia([1.0; 3], 1.0).unwrap();
        assert!((cube[0][0] - 5.0 / 18.0).abs() < 1e-7);
        let box_ = profile().block_inertia([2.0, 3.0, 4.0], 26.0).unwrap();
        // Independent thin-shell face integrals: area-weighted inertias.
        let expected = 2.0 * 3.0 * (3.0 * 3.0 / 12.0 + 4.0 * 4.0 / 4.0)
            + 2.0 * 4.0 * (4.0 * 4.0 / 12.0 + 3.0 * 3.0 / 4.0)
            + 3.0 * 4.0 * ((3.0 * 3.0 + 4.0 * 4.0) / 12.0);
        assert!((box_[0][0] - expected).abs() < 1e-5);
    }
    #[test]
    fn aggregation_keeps_parallel_axis_off_diagonal_and_rotation() {
        let root = part([-1.0, -1.0, 0.0]);
        let mut child = part([1.0, 1.0, 0.0]);
        child.frame.rotation = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let child = AssemblyProperties::combine(&child, &[]).unwrap();
        let a = AssemblyProperties::combine(&root, &[child]).unwrap();
        assert_eq!(a.mass, 2.0);
        assert_eq!(a.center_world, [0.0; 3]);
        assert_eq!(
            a.inertia_in_body,
            [[7.0, -2.0, 0.0], [-2.0, 7.0, 0.0], [0.0, 0.0, 12.0]]
        );
        assert_eq!(a.sim_inverse_inertia_world().unwrap()[0][1], 0.0);
    }
}
