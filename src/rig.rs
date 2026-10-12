//! Recovered R6 construction and the unsmoothed Motor6D angle branch.
//! Assembly selection, animated pose blending and contact forces are separate.
use crate::{
    bundle::Bundle,
    native_data::NativeImage,
    scene::{CoordinateFrame, Scene},
};
use bevy::prelude::Resource;
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write,
};

#[cfg(test)]
fn identity() -> CoordinateFrame {
    CoordinateFrame {
        position: [0.0; 3],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}
pub(crate) fn compose(a: &CoordinateFrame, b: &CoordinateFrame) -> CoordinateFrame {
    CoordinateFrame {
        position: a.transform_point(b.position),
        rotation: std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                a.rotation[r][0] * b.rotation[0][c]
                    + a.rotation[r][1] * b.rotation[1][c]
                    + a.rotation[r][2] * b.rotation[2][c]
            })
        }),
    }
}
pub(crate) fn inverse(a: &CoordinateFrame) -> CoordinateFrame {
    let rotation = std::array::from_fn(|r| std::array::from_fn(|c| a.rotation[c][r]));
    CoordinateFrame {
        position: std::array::from_fn(|r| {
            -(rotation[r][0] * a.position[0]
                + rotation[r][1] * a.position[1]
                + rotation[r][2] * a.position[2])
        }),
        rotation,
    }
}
fn normal_frame(id: u32) -> Result<CoordinateFrame, String> {
    // NormalId::uvwToObject 006b6c54, evaluated on the three unit axes.
    let column = |v: [f32; 3]| -> Result<[f32; 3], String> {
        Ok(match id {
            0 => [v[2], v[1], -v[0]],
            1 => [-v[0], v[2], v[1]],
            2 => v,
            3 => [-v[2], v[1], v[0]],
            4 => [v[0], -v[2], v[1]],
            5 => [-v[0], v[1], -v[2]],
            _ => return Err("invalid native NormalId".into()),
        })
    };
    let columns = [
        column([1.0, 0.0, 0.0])?,
        column([0.0, 1.0, 0.0])?,
        column([0.0, 0.0, 1.0])?,
    ];
    Ok(CoordinateFrame {
        position: [0.0; 3],
        rotation: std::array::from_fn(|r| std::array::from_fn(|c| columns[c][r])),
    })
}

fn presets(image: &NativeImage) -> Result<Vec<CoordinateFrame>, String> {
    // BSS is not file-backed. Decode only the traced initializer's normal IDs
    // and translation writes, rather than treating BSS zeros as the presets.
    let mut decoder = Decoder::with_ip(
        32,
        image.region(0x00bfa03f, 0x3bb)?,
        0x00bfa03f,
        DecoderOptions::NONE,
    );
    let mut registers = HashMap::from([(Register::EBX, 0u32)]); // XOR at 00bf9b73.
    let mut argument = None;
    let mut rotations = Vec::new();
    let mut translations = BTreeMap::new();
    while decoder.can_decode() {
        let i = decoder.decode();
        if i.is_invalid() {
            return Err("invalid rig initializer instruction".into());
        }
        if i.mnemonic() == Mnemonic::Mov {
            let value = match i.op1_kind() {
                OpKind::Immediate32 => Some(i.immediate32()),
                OpKind::Register => registers.get(&i.op1_register()).copied(),
                _ => None,
            };
            if i.op0_kind() == OpKind::Register {
                registers.remove(&i.op0_register());
                if let Some(value) = value {
                    registers.insert(i.op0_register(), value);
                }
            } else if i.op0_kind() == OpKind::Memory && i.memory_index() == Register::None {
                if i.memory_base() == Register::ESP && i.memory_displacement64() == 0 {
                    argument = value;
                }
                if i.memory_base() == Register::EDI {
                    let address = 0x00bf9aabu64 + i.memory_displacement64();
                    if (0x0223e904..0x0223eae4).contains(&address)
                        && (address - 0x0223e904) % 48 >= 36
                    {
                        translations.insert(address, value.ok_or("unresolved rig translation")?);
                    }
                }
            }
        } else if i.mnemonic() == Mnemonic::Call {
            if i.op0_kind() == OpKind::NearBranch32 && i.near_branch_target() == 0x006b745c {
                rotations.push(normal_frame(argument.ok_or("unresolved rig NormalId")?)?);
            }
            argument = None;
            for r in [Register::EAX, Register::ECX, Register::EDX] {
                registers.remove(&r);
            }
        }
    }
    if rotations.len() != 10 || translations.len() != 30 {
        return Err("incomplete native R6 presets".into());
    }
    for (index, frame) in rotations.iter_mut().enumerate() {
        for axis in 0..3 {
            frame.position[axis] = f32::from_bits(
                *translations
                    .get(&(0x0223e904 + index as u64 * 48 + 36 + axis as u64 * 4))
                    .ok_or("missing rig translation component")?,
            );
        }
    }
    Ok(rotations)
}

#[derive(Clone, Debug)]
pub struct Motor6D {
    pub name: String,
    pub part0: String,
    pub part1: String,
    pub c0: CoordinateFrame,
    pub c1: CoordinateFrame,
    pub current_angle: f32,
    pub desired_angle: f32,
    pub max_velocity: f32,
    threshold: f32,
}
impl Motor6D {
    /// SetDesiredAngle has a relative two-degree dead zone in this version.
    pub fn set_desired_angle(&mut self, value: f32) -> Result<bool, String> {
        if !value.is_finite() {
            return Err("non-finite motor angle".into());
        }
        if value != self.desired_angle
            && (value.abs() + 1.0) * self.threshold < (value - self.desired_angle).abs()
        {
            self.desired_angle = value;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// MaxVelocity is an angular limit per native joint callback, not rad/s.
    /// This covers the constructor's zero-offset, unit-weight branch only.
    pub fn step(&mut self) -> Result<(), String> {
        if !self.max_velocity.is_finite()
            || !self.current_angle.is_finite()
            || !self.desired_angle.is_finite()
        {
            return Err("invalid motor state".into());
        }
        let change = self.desired_angle - self.current_angle;
        let limit = self.max_velocity.abs();
        self.current_angle = if limit <= change.abs() {
            if change <= 0.0 {
                self.current_angle - limit
            } else {
                limit + self.current_angle
            }
        } else {
            self.desired_angle
        };
        Ok(())
    }
    pub fn transform(&self) -> CoordinateFrame {
        let (s, c) = self.current_angle.sin_cos();
        CoordinateFrame {
            position: [0.0; 3],
            rotation: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        }
    }
}

#[derive(Resource, Clone)]
pub struct R6Rig {
    pub joints: Vec<Motor6D>,
}
impl R6Rig {
    pub fn character(bundle: &Bundle, scene: &Scene) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let values = presets(&image)?;
        let threshold = f32::from_le_bytes(image.region(0x01767c14, 4)?.try_into().unwrap());
        let i = Decoder::with_ip(
            32,
            image.region(0x00bdaa94, 8)?,
            0x00bdaa94,
            DecoderOptions::NONE,
        )
        .decode();
        if i.mnemonic() != Mnemonic::Mov || i.op1_kind() != OpKind::Immediate32 {
            return Err("unexpected R6 motor speed instruction".into());
        }
        let max_velocity = f32::from_bits(i.immediate32());
        let humanoid = scene
            .instances
            .iter()
            .find(|i| i.class == "Humanoid")
            .ok_or("missing rig Humanoid")?;
        let part_names: Vec<_> = scene
            .instances
            .iter()
            .filter(|i| i.part.is_some() && i.parent == humanoid.parent)
            .map(|i| i.name.as_str())
            .collect();
        let root = normal_frame(1)?;
        let mut joints = Vec::new();
        for (name, part0, part1, indices) in [
            ("RootJoint", "HumanoidRootPart", "Torso", None),
            ("Right Shoulder", "Torso", "Right Arm", Some(0)),
            ("Left Shoulder", "Torso", "Left Arm", Some(1)),
            ("Right Hip", "Torso", "Right Leg", Some(2)),
            ("Left Hip", "Torso", "Left Leg", Some(3)),
            ("Neck", "Torso", "Head", Some(4)),
        ] {
            if !part_names.contains(&part0) || !part_names.contains(&part1) {
                return Err("unsupported incomplete R6 rig".into());
            }
            joints.push(Motor6D {
                name: name.into(),
                part0: part0.into(),
                part1: part1.into(),
                c0: indices.map_or_else(|| root.clone(), |n| values[n].clone()),
                c1: indices.map_or_else(|| root.clone(), |n| values[n + 5].clone()),
                current_angle: 0.0,
                desired_angle: 0.0,
                max_velocity,
                threshold,
            });
        }
        Ok(Self { joints })
    }
    pub fn pose(
        &self,
        root: &CoordinateFrame,
    ) -> Result<BTreeMap<String, CoordinateFrame>, String> {
        let mut frames = BTreeMap::from([("HumanoidRootPart".into(), root.clone())]);
        for joint in &self.joints {
            let parent = frames
                .get(&joint.part0)
                .ok_or("non-topological native rig")?;
            let child = compose(
                &compose(&compose(parent, &joint.c0), &joint.transform()),
                &inverse(&joint.c1),
            );
            if frames.insert(joint.part1.clone(), child).is_some() {
                return Err("multiple rig parents".into());
            }
        }
        Ok(frames)
    }
    pub fn reset(&mut self) {
        for joint in &mut self.joints {
            joint.current_angle = 0.0;
            joint.desired_angle = 0.0;
        }
    }
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let scene = crate::geometry::RenderScene::character(bundle)?.scene;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let root = &scene
        .instances
        .iter()
        .find(|i| i.name == "HumanoidRootPart")
        .and_then(|i| i.part.as_ref())
        .ok_or("missing rig root")?
        .frame;
    let mut report = String::new();
    for joint in &rig.joints {
        writeln!(report,"rig_joint name={:?} part0={:?} part1={:?} c0={:?} c1={:?} max_velocity={} angle_threshold={}",joint.name,joint.part0,joint.part1,joint.c0,joint.c1,joint.max_velocity,joint.threshold).unwrap();
    }
    let frames = rig.pose(root)?;
    let mut error = 0.0f32;
    for instance in &scene.instances {
        if let Some(part) = &instance.part {
            let frame = frames.get(&instance.name).ok_or("unposed rig Part")?;
            for (a, b) in frame
                .position
                .iter()
                .chain(frame.rotation.iter().flatten())
                .zip(
                    part.frame
                        .position
                        .iter()
                        .chain(part.frame.rotation.iter().flatten()),
                )
            {
                error = error.max((a - b).abs());
            }
        }
    }
    if error > 0.000001 {
        return Err(format!(
            "native rig rest pose differs from original: {error}"
        ));
    }
    // Independent pivot invariant: an animated shoulder must rotate about C0,
    // not about the limb center or mesh bounds.
    rig.joints[1].set_desired_angle(0.8)?;
    for _ in 0..10 {
        rig.joints[1].step()?;
    }
    let posed = rig.pose(root)?;
    let shoulder = &rig.joints[1];
    let a = compose(&posed[&shoulder.part0], &shoulder.c0).position;
    let b = compose(&posed[&shoulder.part1], &shoulder.c1).position;
    let residual = a
        .into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    if residual > 0.00001 || posed["Right Arm"] == frames["Right Arm"] {
        return Err("animated shoulder pivot invariant failed".into());
    }
    rig.reset();
    if rig.pose(root)? != frames {
        return Err("rig reset changed rest frames".into());
    }
    writeln!(report,"rig_pass joints=6 parts=7 rest_error={error} pivot_error={residual} reset_identical=true native_callback_cadence=not_tested blend_offsets=not_implemented contacts=not_implemented").unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverse_compose_rotated_offset_preserves_pivot() {
        let a = CoordinateFrame {
            position: [2.0, 3.0, 4.0],
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        };
        assert_eq!(compose(&a, &inverse(&a)), identity());
        assert_eq!(compose(&inverse(&a), &a), identity());
    }
    #[test]
    fn motor_setter_dead_zone_and_per_callback_velocity() {
        let mut m = Motor6D {
            name: String::new(),
            part0: String::new(),
            part1: String::new(),
            c0: identity(),
            c1: identity(),
            current_angle: 0.0,
            desired_angle: 0.0,
            max_velocity: -0.1,
            threshold: 0.034906585,
        };
        assert!(!m.set_desired_angle(0.01).unwrap());
        assert!(m.set_desired_angle(0.25).unwrap());
        m.step().unwrap();
        assert_eq!(m.current_angle, 0.1);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.current_angle, 0.25);
        assert!(m.set_desired_angle(-0.5).unwrap());
        m.step().unwrap();
        assert_eq!(m.current_angle, 0.15);
        assert!(m.set_desired_angle(f32::NAN).is_err());
    }
}
