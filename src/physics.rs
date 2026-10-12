//! Native unconstrained SimBody free-fall branch. No contact solver is implied.
use crate::{
    bundle::Bundle,
    native_data::NativeImage,
    scene::{CoordinateFrame, Scene},
};
use bevy::prelude::*;
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use std::fmt::Write;

#[derive(Resource, Clone, Debug)]
pub struct NativePhysics {
    pub free_fall_dt: f32,
    pub kernel_dt: f32,
    pub kernel_substeps: u32,
    pub angular_damping: f32,
    pub legacy_acceleration: f32,
    pub anti_denormal: f32,
}

impl NativePhysics {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |address| -> Result<f32, String> {
            let value = f32::from_le_bytes(image.region(address, 4)?.try_into().unwrap());
            if !value.is_finite() {
                return Err("non-finite native physics constant".into());
            }
            Ok(value)
        };
        let immediate = |address, length| -> Result<u32, String> {
            let bytes = image.region(address, length)?;
            let instruction =
                Decoder::with_ip(32, bytes, address as u64, DecoderOptions::NONE).decode();
            if instruction.mnemonic() != Mnemonic::Mov
                || instruction.op1_kind() != OpKind::Immediate32
            {
                return Err("unexpected native physics immediate".into());
            }
            Ok(instruction.immediate32())
        };
        let code = image.region(0x00a91f2c, 16)?;
        let mut decoder = Decoder::with_ip(32, code, 0x00a91f2c, DecoderOptions::NONE);
        let mut substeps = None;
        while decoder.can_decode() {
            let instruction = decoder.decode();
            if instruction.mnemonic() == Mnemonic::Ret {
                break;
            }
            if instruction.mnemonic() == Mnemonic::Mov
                && instruction.op0_register() == Register::EAX
                && instruction.op1_kind() == OpKind::Immediate32
            {
                substeps = Some(instruction.immediate32());
            }
        }
        let result = Self {
            free_fall_dt: float(0x0179fc00)?,
            kernel_dt: float(0x01760a4c)?,
            kernel_substeps: substeps.ok_or("missing kernel subdivision constant")?,
            angular_damping: float(0x017a1f74)?,
            legacy_acceleration: f32::from_bits(immediate(0x00ab30df, 7)?) * float(0x01713e34)?,
            anti_denormal: f32::from_bits(immediate(0x00ab39c6, 5)?),
        };
        if result.free_fall_dt <= 0.0
            || result.kernel_dt <= 0.0
            || result.kernel_substeps != 19
            || !(0.0..=1.0).contains(&result.angular_damping)
            || !result.legacy_acceleration.is_finite()
            || result.legacy_acceleration >= 0.0
            || !result.anti_denormal.is_finite()
            || result.anti_denormal <= 0.0
        {
            return Err("unexpected native physics profile".into());
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum GravitySource {
    Legacy,
    /// Corresponds to the DynamicGravity branch, not a recovered flag default.
    Workspace(f32),
}

#[derive(Component, Clone, Debug)]
pub struct FreeFallBody {
    pub frame: CoordinateFrame,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    /// Native x,y,z,w sign convention is retained, including negative branches.
    pub quaternion: [f32; 4],
}

fn normalize(mut q: [f32; 4]) -> Result<[f32; 4], String> {
    let squared = q[3] * q[3] + q[2] * q[2] + q[1] * q[1] + q[0] * q[0];
    if !q.iter().all(|v| v.is_finite()) || !squared.is_finite() || squared == 0.0 {
        return Err("invalid physics quaternion".into());
    }
    let inverse = 1.0 / squared.sqrt();
    q = q.map(|v| v * inverse);
    Ok(q)
}

fn matrix_quaternion(m: [[f32; 3]; 3]) -> Result<[f32; 4], String> {
    let [a, b, c] = [m[0][0], m[1][1], m[2][2]];
    let trace = a + b + c;
    let q = if trace > 0.0 {
        let root = (trace + 1.0).sqrt();
        let scale = 0.5 / root;
        [
            (m[2][1] - m[1][2]) * scale,
            (m[0][2] - m[2][0]) * scale,
            (m[1][0] - m[0][1]) * scale,
            root * 0.5,
        ]
    } else if a > b && a > c {
        let root = (((a + 1.0) - b) - c).sqrt();
        let scale = -0.5 / root;
        [
            root * -0.5,
            (m[0][1] + m[1][0]) * scale,
            (m[0][2] + m[2][0]) * scale,
            (m[2][1] - m[1][2]) * scale,
        ]
    } else if b > c {
        let root = (((b + 1.0) - a) - c).sqrt();
        let scale = -0.5 / root;
        [
            (m[0][1] + m[1][0]) * scale,
            root * -0.5,
            (m[1][2] + m[2][1]) * scale,
            (m[0][2] - m[2][0]) * scale,
        ]
    } else {
        let root = (((c + 1.0) - a) - b).sqrt();
        let scale = -0.5 / root;
        [
            (m[0][2] + m[2][0]) * scale,
            (m[1][2] + m[2][1]) * scale,
            root * -0.5,
            (m[1][0] - m[0][1]) * scale,
        ]
    };
    normalize(q)
}

fn quaternion_matrix([x, y, z, w]: [f32; 4]) -> [[f32; 3]; 3] {
    let xx = x * (x + x);
    let yy = y + y;
    let zz = z + z;
    let xw = (x + x) * w;
    [
        [1.0 - (y * yy + zz * z), x * yy - w * zz, x * zz + yy * w],
        [w * zz + x * yy, 1.0 - (zz * z + xx), y * zz - xw],
        [x * zz - yy * w, y * zz + xw, 1.0 - (y * yy + xx)],
    ]
}

impl FreeFallBody {
    pub fn new(
        frame: CoordinateFrame,
        velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Result<Self, String> {
        if !frame
            .position
            .iter()
            .chain(frame.rotation.iter().flatten())
            .chain(&velocity)
            .chain(&angular_velocity)
            .all(|x| x.is_finite())
        {
            return Err("physics probe requires finite body state".into());
        }
        let quaternion = matrix_quaternion(frame.rotation)?;
        Ok(Self {
            frame,
            velocity,
            angular_velocity,
            quaternion,
        })
    }

    /// One native freeFallDt step, for a body classified as free-fall with no
    /// external force/impulse. Assemblies and classification are not implemented.
    pub fn step(&mut self, profile: &NativePhysics, gravity: GravitySource) -> Result<(), String> {
        let acceleration = match gravity {
            GravitySource::Legacy => profile.legacy_acceleration,
            GravitySource::Workspace(g) => -g,
        };
        if !acceleration.is_finite()
            || profile.free_fall_dt <= 0.0
            || !profile.free_fall_dt.is_finite()
            || !profile.angular_damping.is_finite()
            || !profile.anti_denormal.is_finite()
        {
            return Err("invalid free-fall probe parameters".into());
        }
        let dt = profile.free_fall_dt;
        let [wx, wy, wz] = self.angular_velocity.map(|v| v * profile.angular_damping);
        let [x, y, z, w] = self.quaternion;
        let quaternion = normalize([
            dt * ((wy * z - wz * y) + wx * w + x * 0.0) * 0.5 + x,
            dt * ((wz * x - wx * z) + wy * w + y * 0.0) * 0.5 + y,
            dt * ((wx * y - wy * x) + wz * w + z * 0.0) * 0.5 + z,
            dt * (w * 0.0 - (z * wz + wy * y + wx * x)) * 0.5 + w,
        ])?;
        let half_gravity = acceleration * dt * 0.5;
        let middle_y = self.velocity[1] + half_gravity;
        let full_y = half_gravity + middle_y;
        let position = [
            self.velocity[0] * dt + self.frame.position[0],
            middle_y * dt + self.frame.position[1],
            dt * self.velocity[2] + self.frame.position[2],
        ];
        let velocity = [
            self.velocity[0] + profile.anti_denormal,
            full_y + profile.anti_denormal,
            self.velocity[2] + profile.anti_denormal,
        ];
        let angular_velocity = [
            wx + profile.anti_denormal,
            wy + profile.anti_denormal,
            wz + profile.anti_denormal,
        ];
        if !position
            .iter()
            .chain(&velocity)
            .chain(&angular_velocity)
            .all(|v| v.is_finite())
        {
            return Err("free-fall probe overflow".into());
        }
        self.quaternion = quaternion;
        self.frame.rotation = quaternion_matrix(quaternion);
        self.frame.position = position;
        self.velocity = velocity;
        self.angular_velocity = angular_velocity;
        Ok(())
    }
}

#[derive(Resource, Default)]
struct Probe {
    steps: u32,
    report: String,
    error: Option<String>,
}

fn step_probe(
    profile: Res<NativePhysics>,
    mut probe: ResMut<Probe>,
    mut bodies: Query<(&Name, &mut FreeFallBody)>,
) {
    if probe.error.is_some() {
        return;
    }
    probe.steps += 1;
    let step = probe.steps;
    for (name, mut body) in &mut bodies {
        if let Err(error) = body.step(&profile, GravitySource::Legacy) {
            probe.error = Some(error);
            return;
        }
        if step == 1 || step.is_multiple_of(60) {
            writeln!(probe.report,"free_fall_step step={step} name={name:?} position={:?} velocity={:?} angular_velocity={:?} quaternion={:?}",body.frame.position,body.velocity,body.angular_velocity,body.quaternion).unwrap();
        }
    }
}

pub(crate) fn character_bodies(bundle: &Bundle) -> Result<Vec<(String, FreeFallBody)>, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let scene = Scene::parse(xml)?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut bodies = Vec::new();
    for instance in &scene.instances {
        let Some(part) = &instance.part else {
            continue;
        };
        if part.anchored {
            return Err("free-fall character fixture contains an anchored Part".into());
        }
        let item = doc
            .descendants()
            .find(|n| n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent))
            .ok_or("missing physics XML Part")?;
        let properties = item
            .children()
            .find(|n| n.has_tag_name("Properties"))
            .ok_or("missing physics properties")?;
        let vector = |name| -> Result<[f32; 3], String> {
            let mut found = properties
                .children()
                .filter(|n| n.attribute("name") == Some(name));
            let node = found.next().ok_or("missing velocity")?;
            if found.next().is_some() || !node.has_tag_name("Vector3") {
                return Err("invalid velocity property".into());
            }
            let mut values = [0.0; 3];
            for (i, label) in ["X", "Y", "Z"].iter().enumerate() {
                let mut found = node.children().filter(|n| n.has_tag_name(*label));
                values[i] = found
                    .next()
                    .and_then(|n| n.text())
                    .ok_or("missing velocity component")?
                    .trim()
                    .parse()
                    .map_err(|_| "invalid velocity component")?;
                if found.next().is_some() {
                    return Err("duplicate velocity component".into());
                }
            }
            Ok(values)
        };
        let body = FreeFallBody::new(
            part.frame.clone(),
            vector("Velocity")?,
            vector("RotVelocity")?,
        )?;
        bodies.push((instance.name.clone(), body));
    }
    Ok(bodies)
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = NativePhysics::load(bundle)?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(profile.clone());
    app.init_resource::<Probe>();
    app.add_systems(Update, step_probe);
    let mut initial = Vec::new();
    for (name, body) in character_bodies(bundle)? {
        let entity = app.world_mut().spawn((Name::new(name), body.clone())).id();
        initial.push((entity, body));
    }
    if initial.is_empty() {
        return Err("empty free-fall fixture".into());
    }
    let mut max_position_error = 0.0f64;
    let mut max_velocity_error = 0.0f64;
    for step in 1..=240 {
        app.update();
        if let Some(error) = &app.world().resource::<Probe>().error {
            return Err(error.clone());
        }
        let t = f64::from(profile.free_fall_dt) * f64::from(step);
        for (entity, before) in &initial {
            let body = app
                .world()
                .get::<FreeFallBody>(*entity)
                .ok_or("missing physics body")?;
            let expected_y = f64::from(before.frame.position[1])
                + f64::from(before.velocity[1]) * t
                + 0.5 * f64::from(profile.legacy_acceleration) * t * t;
            max_position_error =
                max_position_error.max((f64::from(body.frame.position[1]) - expected_y).abs());
            let expected_v =
                f64::from(before.velocity[1]) + f64::from(profile.legacy_acceleration) * t;
            max_velocity_error =
                max_velocity_error.max((f64::from(body.velocity[1]) - expected_v).abs());
        }
    }
    let probe = app.world().resource::<Probe>();
    if probe.steps != 240 || max_position_error > 0.002 || max_velocity_error > 0.002 {
        return Err(format!(
            "free-fall analytic check failed position_error={max_position_error} velocity_error={max_velocity_error}"
        ));
    }
    Ok(format!(
        "native_physics free_fall_dt={} kernel_dt={} kernel_substeps={} angular_damping={} legacy_acceleration={} anti_denormal={}\n{}free_fall_pass bodies={} steps=240 max_position_error={max_position_error:.9} max_velocity_error={max_velocity_error:.9} fixture=independent_unconstrained_parts collisions=not_implemented character_rig=not_implemented native_behavior_parity=not_tested\n",
        profile.free_fall_dt,
        profile.kernel_dt,
        profile.kernel_substeps,
        profile.angular_damping,
        profile.legacy_acceleration,
        profile.anti_denormal,
        probe.report,
        initial.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> CoordinateFrame {
        CoordinateFrame {
            position: [0.0, 100.0, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
    fn profile() -> NativePhysics {
        NativePhysics {
            free_fall_dt: 1.0,
            kernel_dt: 0.125,
            kernel_substeps: 8,
            angular_damping: 1.0,
            legacy_acceleration: -10.0,
            anti_denormal: 1e-20,
        }
    }
    #[test]
    fn free_fall_uses_middle_velocity_for_position_and_nudges_after_motion() {
        let mut body = FreeFallBody::new(frame(), [2.0, 0.0, 3.0], [0.0; 3]).unwrap();
        body.step(&profile(), GravitySource::Legacy).unwrap();
        assert_eq!(body.frame.position, [2.0, 95.0, 3.0]);
        assert_eq!(body.velocity, [2.0, -10.0, 3.0]);
        assert_eq!(body.angular_velocity, [1e-20; 3]);
        body.step(&profile(), GravitySource::Workspace(0.0))
            .unwrap();
        assert_eq!(body.frame.position, [4.0, 85.0, 6.0]);
    }
    #[test]
    fn matrix_conversion_preserves_native_negative_diagonal_branches() {
        for (axis, matrix) in [
            [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
            [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
            [[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]],
        ]
        .into_iter()
        .enumerate()
        {
            let q = matrix_quaternion(matrix).unwrap();
            assert_eq!(q[axis], -1.0);
            assert_eq!(quaternion_matrix(q), matrix);
        }
    }
    #[test]
    fn angular_step_damps_then_normalizes_without_an_exponential_map() {
        let mut body = FreeFallBody::new(frame(), [0.0; 3], [0.0, 2.0, 0.0]).unwrap();
        let profile = NativePhysics {
            free_fall_dt: 0.1,
            angular_damping: 0.5,
            ..profile()
        };
        body.step(&profile, GravitySource::Legacy).unwrap();
        let angle = 2.0 * body.quaternion[1].atan2(body.quaternion[3]);
        assert!((angle - 2.0 * 0.05f32.atan()).abs() < 1e-7);
        assert!((angle - 0.1).abs() > 1e-5);
        assert_eq!(body.angular_velocity[1], 1.0);
    }
}
