//! Recovered R6 construction and Motor6DJoint: setAnimatedPose (00b37700),
//! stepUi (00b3717c) and the link transform (00b36fde).
use crate::{
    animator::CachedPose,
    bundle::Bundle,
    g3d,
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
pub(crate) fn normal_frame(id: u32) -> Result<CoordinateFrame, String> {
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
    link: MotorLink,
}
/// Motor6DJoint fields behind the link: the animated pose (+0x9c translation,
/// +0xa8 rotation vector, both already weighted), motor weight +0xb4,
/// countdown +0xb8, and the link's rotation vector x/y and translation (its
/// z component is `current_angle`, +0xd0).
#[derive(Clone, Debug, PartialEq)]
struct MotorLink {
    animated_translation: [f32; 3],
    animated_rotation: [f32; 3],
    motor_weight: f32,
    countdown: i32,
    rotation_xy: [f32; 2],
    translation: [f32; 3],
}
impl Default for MotorLink {
    fn default() -> Self {
        Self {
            animated_translation: [0.0; 3],
            animated_rotation: [0.0; 3],
            motor_weight: 1.0,
            countdown: 0,
            rotation_xy: [0.0; 2],
            translation: [0.0; 3],
        }
    }
}
impl Motor6D {
    /// A Motor6D with no animation applied; `threshold` is the
    /// DesiredAngle dead zone (`threshold()` of a loaded rig's motor).
    #[allow(clippy::too_many_arguments)] // The JointInstance's properties.
    pub fn new(
        name: &str,
        part0: &str,
        part1: &str,
        c0: CoordinateFrame,
        c1: CoordinateFrame,
        angles: [f32; 2],
        max_velocity: f32,
        threshold: f32,
    ) -> Self {
        Self {
            name: name.into(),
            part0: part0.into(),
            part1: part1.into(),
            c0,
            c1,
            current_angle: angles[0],
            desired_angle: angles[1],
            max_velocity,
            threshold,
            link: MotorLink::default(),
        }
    }
    /// The relative DesiredAngle dead zone (01767c14).
    pub fn threshold(&self) -> f32 {
        self.threshold
    }
    /// Motor6DJoint::setAnimatedPose 00b37700.
    pub fn set_animated_pose(
        &mut self,
        translation: [f32; 3],
        rotation: [f32; 3],
        weight: f32,
        motor_weight: f32,
    ) {
        self.link.animated_translation = translation.map(|v| v * weight);
        self.link.animated_rotation = [
            rotation[0] * weight,
            rotation[1] * weight,
            weight * rotation[2],
        ];
        self.link.motor_weight = motor_weight;
        self.link.countdown = 32;
    }
    /// Motor6D's IAnimatableJoint pose slot (00923fb8): the motor angle only
    /// counts when the animation weight is zero.
    pub fn animate(&mut self, pose: &CachedPose) {
        let motor_weight = if 0.0 < pose.weight { 0.0 } else { 1.0 };
        self.set_animated_pose(pose.translation, pose.rotation, pose.weight, motor_weight);
    }
    /// Link rotation vector and translation (what the joint frame uses).
    pub fn link_state(&self) -> ([f32; 3], [f32; 3]) {
        (
            [
                self.link.rotation_xy[0],
                self.link.rotation_xy[1],
                self.current_angle,
            ],
            self.link.translation,
        )
    }
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
    /// Motor6DJoint::stepUi 00b3717c. MaxVelocity is an angular limit per
    /// native joint callback, not rad/s. Without fresh animator poses for 32
    /// callbacks the animated offsets decay and the motor weight returns to 1.
    pub fn step(&mut self) -> Result<(), String> {
        if !self.max_velocity.is_finite()
            || !self.current_angle.is_finite()
            || !self.desired_angle.is_finite()
        {
            return Err("invalid motor state".into());
        }
        let change = self.desired_angle - self.current_angle;
        let limit = self.max_velocity.abs();
        let angle = if limit <= change.abs() {
            if change <= 0.0 {
                self.current_angle - limit
            } else {
                limit + self.current_angle
            }
        } else {
            self.desired_angle
        };
        let link = &mut self.link;
        if link.countdown < 1 {
            let squared = |v: [f32; 3]| v[2] * v[2] + v[1] * v[1] + v[0] * v[0];
            let settled = 1.0 <= link.motor_weight
                && g3d::fuzzy_zero(squared(link.animated_rotation))
                && g3d::fuzzy_zero(squared(link.animated_translation));
            if !settled {
                link.motor_weight = (link.motor_weight + 0.03125).min(1.0);
                for v in [&mut link.animated_rotation, &mut link.animated_translation] {
                    let s = squared(*v);
                    if s != 0.0 {
                        let mut f = s * 0.96875 + -0.01;
                        if f <= 0.0 {
                            f = 0.0;
                        }
                        let f = f / s;
                        *v = [v[0] * f, v[1] * f, f * v[2]];
                    }
                }
            }
        } else {
            link.countdown -= 1;
        }
        link.rotation_xy = [
            link.animated_rotation[0] + 0.0,
            link.animated_rotation[1] + 0.0,
        ];
        link.translation = link.animated_translation;
        self.current_angle = angle * link.motor_weight + link.animated_rotation[2];
        Ok(())
    }
    /// The link CFrame (00b36fde): fromAxisAngleFast of the rotation vector.
    pub fn transform(&self) -> CoordinateFrame {
        let (rotation, translation) = self.link_state();
        CoordinateFrame {
            position: translation,
            rotation: g3d::rotation_vector_matrix(rotation),
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
                link: MotorLink::default(),
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
            joint.link = MotorLink::default();
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
    writeln!(report,"rig_pass joints=6 parts=7 rest_error={error} pivot_error={residual} reset_identical=true native_callback_cadence=not_tested animated_pose=see_audit_animate contacts=not_implemented").unwrap();
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
            link: MotorLink::default(),
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
