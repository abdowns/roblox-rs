//! Legacy constrained SimBody substep and ContactConnector's force pass.
//! Classification, contact geometry and iterative impulse scheduling are separate.
use crate::{
    assembly::{AssemblyProperties, MassProfile, Tensor, rotate, rotate_tensor},
    bundle::Bundle,
    collision::{ContactBodyState, contact_velocity},
    native_data::NativeImage,
    physics::{self, FreeFallBody, NativePhysics},
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
};
use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct KernelProfile {
    pub physics: NativePhysics,
    angular_damping: f32,
    friction_stiffness: f32,
    history_decay: f32,
    history_skin: f32,
    spring_epsilon: f64,
}
impl KernelProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            let x = f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap());
            if !x.is_finite() {
                return Err("invalid kernel constant".into());
            }
            Ok(x)
        };
        let result = Self {
            physics: NativePhysics::load(bundle)?,
            angular_damping: float(0x017a1f70)?,
            friction_stiffness: float(0x017822bc)?,
            history_decay: float(0x017bb964)?,
            history_skin: -float(0x017efdb0)?,
            spring_epsilon: f64::from_le_bytes(image.region(0x0179fcb8, 8)?.try_into().unwrap()),
        };
        if !(0.0..=1.0).contains(&result.angular_damping)
            || !(0.0..=1.0).contains(&result.history_decay)
            || result.history_skin <= 0.0
            || result.friction_stiffness <= 0.0
            || !result.spring_epsilon.is_finite()
            || result.spring_epsilon <= 0.0
        {
            return Err("unexpected constrained kernel profile".into());
        }
        Ok(result)
    }
}
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[2] * b[2] + a[1] * b[1] + a[0] * b[0]
}
fn diagonal(v: [f32; 3]) -> Tensor {
    std::array::from_fn(|r| std::array::from_fn(|c| if r == c { v[r] } else { 0.0 }))
}

#[derive(Clone, Debug)]
pub struct KernelBody {
    pub frame: CoordinateFrame,
    pub center: [f32; 3],
    /// Linear velocity of the center of mass, not the body's origin.
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub angular_momentum: [f32; 3],
    pub force: [f32; 3],
    pub torque: [f32; 3],
    pub linear_impulse: [f32; 3],
    pub angular_impulse: [f32; 3],
    pub quaternion: [f32; 4],
    /// SimBody +0xf0: the momentum change F*dt + J of the last contact
    /// pre-step (ab230a); the impulse solver's reference impulse.
    pub reference_impulse: [f32; 3],
    /// SimBody +0x12c (angular) and +0x12d (linear) resting flags, set by the
    /// Kernel::step pre-step (00a96eb0) and cleared by fast contacts.
    pub resting: [bool; 2],
    /// SimBody +0x130 / +0x13c: contact geometry displacement (00a942ee).
    pub angular_displacement: [f32; 3],
    pub linear_displacement: [f32; 3],
    center_in_body: [f32; 3],
    inverse_mass: f32,
    /// SimBody +0x78 principal inertia (body space).
    inertia_diagonal: [f32; 3],
    inverse_diagonal: [f32; 3],
    gravity_force: f32,
}
/// Row-by-vector product as the SimBody loops compile it (ab230a, ab267c,
/// ab1c28): z + (y + x).
fn row_product(m: Tensor, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| m[a][2] * v[2] + (m[a][1] * v[1] + m[a][0] * v[0]))
}
impl KernelBody {
    pub fn new(
        assembly: &AssemblyProperties,
        body: &FreeFallBody,
        profile: &KernelProfile,
    ) -> Result<Self, String> {
        if !assembly.mass.is_finite() || assembly.mass <= 0.0 {
            return Err("invalid kernel assembly mass".into());
        }
        let inertia: [f32; 3] = std::array::from_fn(|a| assembly.inertia_in_body[a][a]);
        if inertia.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("invalid kernel inertia".into());
        }
        let offset = rotate(body.frame.rotation, assembly.center_in_body);
        let center = std::array::from_fn(|a| body.frame.position[a] + offset[a]);
        let angular_offset = cross(body.angular_velocity, offset);
        let velocity = std::array::from_fn(|a| body.velocity[a] + angular_offset[a]);
        let angular_momentum = rotate(
            rotate_tensor(diagonal(inertia), body.frame.rotation),
            body.angular_velocity,
        );
        let result = Self {
            frame: body.frame.clone(),
            center,
            velocity,
            angular_velocity: body.angular_velocity,
            angular_momentum,
            force: [
                0.0,
                assembly.mass * profile.physics.legacy_acceleration,
                0.0,
            ],
            torque: [0.0; 3],
            linear_impulse: [0.0; 3],
            angular_impulse: [0.0; 3],
            quaternion: physics::matrix_quaternion(body.frame.rotation)?,
            reference_impulse: [0.0; 3],
            resting: [false; 2],
            angular_displacement: [0.0; 3],
            linear_displacement: [0.0; 3],
            center_in_body: assembly.center_in_body,
            inverse_mass: 1.0 / assembly.mass,
            inertia_diagonal: inertia,
            inverse_diagonal: inertia.map(|v| 1.0 / v),
            gravity_force: assembly.mass * profile.physics.legacy_acceleration,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), String> {
        if self
            .center
            .iter()
            .chain(&self.velocity)
            .chain(&self.angular_velocity)
            .chain(&self.angular_momentum)
            .chain(&self.force)
            .chain(&self.torque)
            .chain(&self.linear_impulse)
            .chain(&self.angular_impulse)
            .chain(&self.quaternion)
            .chain(&self.reference_impulse)
            .chain(&self.angular_displacement)
            .chain(&self.linear_displacement)
            .chain(&self.center_in_body)
            .chain(&self.inverse_diagonal)
            .chain(&self.frame.position)
            .chain(self.frame.rotation.iter().flatten())
            .chain([&self.inverse_mass, &self.gravity_force])
            .any(|v| !v.is_finite())
        {
            return Err("non-finite constrained body".into());
        }
        Ok(())
    }
    pub fn contact_state(&self) -> ContactBodyState {
        ContactBodyState {
            center: self.center,
            velocity: self.velocity,
            angular_velocity: self.angular_velocity,
        }
    }
    pub fn apply_force(&mut self, force: [f32; 3], point: [f32; 3]) -> Result<(), String> {
        if force.iter().chain(&point).any(|v| !v.is_finite()) {
            return Err("invalid applied force".into());
        }
        let arm = std::array::from_fn(|a| point[a] - self.center[a]);
        let torque = cross(arm, force);
        self.force = std::array::from_fn(|a| self.force[a] + force[a]);
        self.torque = std::array::from_fn(|a| torque[a] + self.torque[a]);
        self.validate()
    }
    pub fn inverse_inertia_world(&self) -> Tensor {
        rotate_tensor(diagonal(self.inverse_diagonal), self.frame.rotation)
    }
    /// Body::accumulateTorque, 00a6c134; no artificial orientation constraint.
    pub fn apply_torque(&mut self, torque: [f32; 3]) -> Result<(), String> {
        if torque.iter().any(|v| !v.is_finite()) {
            return Err("invalid applied torque".into());
        }
        self.torque = std::array::from_fn(|a| torque[a] + self.torque[a]);
        self.validate()
    }
    pub fn inverse_mass(&self) -> f32 {
        self.inverse_mass
    }
    /// Dirty SimBody refresh (00ab16aa): publish root origin state, rebuild
    /// Cofm/inertia and momentum from angular velocity, preserve accumulators.
    pub fn refresh_assembly(
        &mut self,
        assembly: &AssemblyProperties,
        profile: &KernelProfile,
    ) -> Result<(), String> {
        let mut refreshed = Self::new(assembly, &self.body_state(), profile)?;
        refreshed.force = self.force;
        refreshed.torque = self.torque;
        refreshed.linear_impulse = self.linear_impulse;
        refreshed.angular_impulse = self.angular_impulse;
        refreshed.validate()?;
        *self = refreshed;
        Ok(())
    }
    /// FUN_00ab267c: momentum damping occurs on each contact impulse.
    pub fn contact_impulse(
        &mut self,
        impulse: [f32; 3],
        point: [f32; 3],
        profile: &KernelProfile,
    ) -> Result<(), String> {
        if impulse.iter().chain(&point).any(|v| !v.is_finite()) {
            return Err("invalid contact impulse".into());
        }
        let angular = cross(std::array::from_fn(|a| point[a] - self.center[a]), impulse);
        self.angular_momentum = std::array::from_fn(|a| {
            self.angular_momentum[a] * profile.angular_damping + angular[a]
        });
        let m = self.inverse_inertia_world();
        self.angular_velocity = std::array::from_fn(|a| {
            m[a][2] * self.angular_momentum[2]
                + (m[a][1] * self.angular_momentum[1] + m[a][0] * self.angular_momentum[0])
                + profile.physics.anti_denormal
        });
        self.velocity = std::array::from_fn(|a| {
            (impulse[a] * self.inverse_mass + profile.physics.anti_denormal) + self.velocity[a]
        });
        self.angular_momentum = self
            .angular_momentum
            .map(|v| v + profile.physics.anti_denormal);
        self.validate()
    }
    /// FUN_00ab1d44: one constrained substep; semi-implicit center position,
    /// momentum damping, quaternion Euler update, then native accumulator reset.
    pub fn step(&mut self, profile: &KernelProfile) -> Result<(), String> {
        self.validate()?;
        let dt = profile.physics.kernel_dt;
        if !dt.is_finite() || dt <= 0.0 {
            return Err("invalid constrained dt".into());
        }
        self.angular_momentum = std::array::from_fn(|a| {
            self.torque[a] * dt
                + self.angular_impulse[a]
                + self.angular_momentum[a] * profile.angular_damping
        });
        self.angular_velocity = rotate(self.inverse_inertia_world(), self.angular_momentum)
            .map(|v| v + profile.physics.anti_denormal);
        let [wx, wy, wz] = self.angular_velocity;
        let [x, y, z, w] = self.quaternion;
        self.quaternion = physics::normalize([
            dt * ((wy * z - wz * y) + wx * w + x * 0.0) * 0.5 + x,
            dt * ((wz * x - wx * z) + wy * w + y * 0.0) * 0.5 + y,
            dt * ((wx * y - wy * x) + wz * w + z * 0.0) * 0.5 + z,
            dt * (w * 0.0 - (z * wz + wy * y + wx * x)) * 0.5 + w,
        ])?;
        self.frame.rotation = physics::quaternion_matrix(self.quaternion);
        self.velocity = std::array::from_fn(|a| {
            (self.force[a] * dt + self.linear_impulse[a]) * self.inverse_mass + self.velocity[a]
        });
        self.center = std::array::from_fn(|a| dt * self.velocity[a] + self.center[a]);
        self.force = [0.0, self.gravity_force, 0.0];
        self.torque = [0.0; 3];
        self.linear_impulse = [0.0; 3];
        self.angular_impulse = [0.0; 3];
        self.angular_momentum = self
            .angular_momentum
            .map(|v| profile.physics.anti_denormal + v);
        self.velocity = self.velocity.map(|v| v + profile.physics.anti_denormal);
        let offset = rotate(self.frame.rotation, self.center_in_body);
        self.frame.position = std::array::from_fn(|a| self.center[a] - offset[a]);
        self.validate()
    }
    /// SimBody::hasExternalForceOrImpulse: anything beyond gravity in the
    /// accumulators (exact float compares; NaN counts as external).
    pub fn has_external_force_or_impulse(&self) -> bool {
        self.force != [0.0, self.gravity_force, 0.0]
            || self.torque != [0.0; 3]
            || self.linear_impulse != [0.0; 3]
            || self.angular_impulse != [0.0; 3]
    }
    /// Publish the origin from the center after the center moved.
    fn place_origin(&mut self) {
        let offset = rotate(self.frame.rotation, self.center_in_body);
        self.frame.position = std::array::from_fn(|a| self.center[a] - offset[a]);
    }
    /// Quaternion Euler update shared by ab2d9a and ab290c (the `* 0.0`
    /// terms are exact no-ops), then the rotation matrix (006ba600).
    fn integrate_rotation(&mut self, dt: f32, w: [f32; 3]) -> Result<(), String> {
        let [wx, wy, wz] = w;
        let [x, y, z, q] = self.quaternion;
        self.quaternion = physics::normalize([
            dt * ((wy * z - wz * y) + wx * q + x * 0.0) * 0.5 + x,
            dt * ((wz * x - wx * z) + wy * q + y * 0.0) * 0.5 + y,
            dt * ((wx * y - wy * x) + wz * q + z * 0.0) * 0.5 + z,
            dt * (q * 0.0 - (z * wz + wy * y + wx * x)) * 0.5 + q,
        ])?;
        self.frame.rotation = physics::quaternion_matrix(self.quaternion);
        Ok(())
    }
    /// Free-fall SimBody step ab2d9a at freeFallDt: damps angular velocity
    /// (not momentum), legacy gravity with a half-step on Y, then the tiny
    /// additions. Only valid without external force or impulse.
    pub fn free_fall_step(&mut self, profile: &KernelProfile) -> Result<(), String> {
        self.validate()?;
        let p = &profile.physics;
        let dt = p.free_fall_dt;
        let w = self.angular_velocity.map(|v| v * p.angular_damping);
        self.angular_velocity = w;
        self.integrate_rotation(dt, w)?;
        let half = p.legacy_acceleration * dt * 0.5;
        let middle = self.velocity[1] + half;
        let full = half + middle;
        self.center = [
            dt * self.velocity[0] + self.center[0],
            middle * dt + self.center[1],
            dt * self.velocity[2] + self.center[2],
        ];
        self.angular_velocity = self.angular_velocity.map(|v| p.anti_denormal + v);
        self.velocity = [
            self.velocity[0] + p.anti_denormal,
            full + p.anti_denormal,
            self.velocity[2] + p.anti_denormal,
        ];
        self.place_origin();
        self.validate()
    }
    /// ab1c28, run when a body leaves the free-fall list: L = (R I R^T) w.
    pub fn rebuild_momentum(&mut self) {
        let world = rotate_tensor(diagonal(self.inertia_diagonal), self.frame.rotation);
        self.angular_momentum = row_product(world, self.angular_velocity);
    }
    /// Contact-body pre-step ab230a at freeFallDt: forces and impulses become
    /// momentum (stored as the reference impulse), angular momentum is damped,
    /// accumulators reset. Position is untouched until `contact_integrate`.
    pub fn contact_prestep(&mut self, profile: &KernelProfile) -> Result<(), String> {
        self.validate()?;
        let p = &profile.physics;
        let dt = p.free_fall_dt;
        self.angular_momentum = std::array::from_fn(|a| {
            (self.torque[a] * dt + self.angular_impulse[a])
                + self.angular_momentum[a] * p.angular_damping
        });
        let w = row_product(self.inverse_inertia_world(), self.angular_momentum);
        self.reference_impulse =
            std::array::from_fn(|a| self.force[a] * dt + self.linear_impulse[a]);
        self.velocity = std::array::from_fn(|a| {
            self.velocity[a] + self.reference_impulse[a] * self.inverse_mass
        });
        self.angular_velocity = w.map(|v| v + p.anti_denormal);
        self.force = [0.0, self.gravity_force, 0.0];
        self.torque = [0.0; 3];
        self.linear_impulse = [0.0; 3];
        self.angular_impulse = [0.0; 3];
        self.angular_momentum = self.angular_momentum.map(|v| v + p.anti_denormal);
        self.velocity = self.velocity.map(|v| p.anti_denormal + v);
        self.validate()
    }
    /// Contact-body integration ab290c at freeFallDt, after the impulses:
    /// orientation from w and center += v*dt; no gravity, no damping.
    pub fn contact_integrate(&mut self, profile: &KernelProfile) -> Result<(), String> {
        self.validate()?;
        let dt = profile.physics.free_fall_dt;
        self.integrate_rotation(dt, self.angular_velocity)?;
        self.center = std::array::from_fn(|a| self.velocity[a] * dt + self.center[a]);
        self.place_origin();
        self.validate()
    }
    /// Kinematic placement (Primitive::setCoordinateFrame/setVelocity while the
    /// assembly is outside the kernel, e.g. RunningNoPhysics 00c07626). Keeps the
    /// assembly properties; accumulators restart as on kernel re-entry.
    pub fn set_body_state(&mut self, body: &FreeFallBody) -> Result<(), String> {
        let offset = rotate(body.frame.rotation, self.center_in_body);
        let inertia = self.inverse_diagonal.map(|v| 1.0 / v);
        self.frame = body.frame.clone();
        self.center = std::array::from_fn(|a| body.frame.position[a] + offset[a]);
        let angular_offset = cross(body.angular_velocity, offset);
        self.velocity = std::array::from_fn(|a| body.velocity[a] + angular_offset[a]);
        self.angular_velocity = body.angular_velocity;
        self.angular_momentum = rotate(
            rotate_tensor(diagonal(inertia), body.frame.rotation),
            body.angular_velocity,
        );
        self.quaternion = physics::matrix_quaternion(body.frame.rotation)?;
        self.force = [0.0, self.gravity_force, 0.0];
        self.torque = [0.0; 3];
        self.linear_impulse = [0.0; 3];
        self.angular_impulse = [0.0; 3];
        self.validate()
    }
    /// Native Body publishing removes Cofm offset and moves center velocity back
    /// to the origin (00ab13e8); it does not overwrite velocity with center velocity.
    pub fn body_state(&self) -> FreeFallBody {
        let arm = std::array::from_fn(|a| self.frame.position[a] - self.center[a]);
        let correction = cross(self.angular_velocity, arm);
        FreeFallBody {
            frame: self.frame.clone(),
            velocity: std::array::from_fn(|a| self.velocity[a] + correction[a]),
            angular_velocity: self.angular_velocity,
            quaternion: self.quaternion,
        }
    }
}

/// Explicit material/shape-derived parameters; defaults have not been recovered.
#[derive(Clone, Copy, Debug)]
pub struct ContactParameters {
    pub compression: f32,
    pub decompression: f32,
    pub friction: f32,
}
#[derive(Clone, Debug, Default)]
pub struct ContactSpring {
    pub compression_depth: f32,
    pub decompression_depth: f32,
    pub normal_force: f32,
    pub tangent_offset: [f32; 3],
}
impl ContactSpring {
    /// FUN_00a93074. Returned vector applies to B; its negative applies to A.
    /// Geometry/normal is refreshed by the caller, as with native virtual +0x24.
    #[allow(clippy::too_many_arguments)] // Explicit native geometry, material and both bodies.
    pub fn force(
        &mut self,
        profile: &KernelProfile,
        parameters: ContactParameters,
        distance: f32,
        normal: [f32; 3],
        point: [f32; 3],
        a: &ContactBodyState,
        b: &ContactBodyState,
    ) -> Result<[f32; 3], String> {
        if [
            parameters.compression,
            parameters.decompression,
            parameters.friction,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
            || !distance.is_finite()
            || normal.iter().any(|v| !v.is_finite())
            || (dot(normal, normal) - 1.0).abs() > 0.00001
        {
            return Err("invalid explicit contact spring inputs".into());
        }
        if distance >= 0.0 {
            self.compression_depth = 0.0;
            self.decompression_depth = 0.0;
            self.normal_force = 0.0;
            // Native separation branch preserves the tangential spring history.
            return Ok([0.0; 3]);
        }
        let velocity = contact_velocity(a, b, point, normal)?;
        self.tangent_offset = std::array::from_fn(|a| {
            velocity.tangent_component[a] * profile.physics.kernel_dt + self.tangent_offset[a]
        });
        let stiffness = parameters.compression * profile.friction_stiffness;
        let force_length = dot(self.tangent_offset, self.tangent_offset).sqrt() * stiffness;
        let limit = self.normal_force * parameters.friction;
        if f64::from(force_length) > profile.spring_epsilon && limit < force_length {
            self.tangent_offset = self.tangent_offset.map(|v| v * (limit / force_length));
        }
        let component = dot(normal, self.tangent_offset);
        self.tangent_offset =
            std::array::from_fn(|a| self.tangent_offset[a] - normal[a] * component);
        let decay =
            |d: f32| (d + profile.history_skin) * profile.history_decay - profile.history_skin;
        self.decompression_depth = if self.decompression_depth != 0.0 {
            decay(self.decompression_depth)
        } else if self.compression_depth != 0.0 {
            distance
        } else {
            0.0
        };
        self.compression_depth = if self.compression_depth != 0.0 {
            decay(self.compression_depth)
        } else {
            distance
        };
        let k = if velocity.normal_speed < 0.0 {
            parameters.compression
        } else {
            parameters.decompression
        };
        self.normal_force = if self.decompression_depth < distance {
            0.0
        } else {
            (self.compression_depth - distance) * k
        };
        let force = std::array::from_fn(|a| {
            normal[a] * self.normal_force - self.tangent_offset[a] * stiffness
        });
        if force.iter().any(|v| !v.is_finite()) {
            return Err("contact spring overflow".into());
        }
        Ok(force)
    }
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = KernelProfile::load(bundle)?;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let rig = R6Rig::character(bundle, &scene)?;
    let parts = MassProfile::load(bundle)?.character_parts(bundle, &scene)?;
    let assembly = AssemblyProperties::from_rig(&parts, &rig)?;
    let original = crate::physics::character_bodies(bundle)?
        .into_iter()
        .find(|(n, _)| n == "HumanoidRootPart")
        .ok_or("missing kernel root")?
        .1;
    let mut body = KernelBody::new(&assembly, &original, &profile)?;
    let mut report = format!(
        "kernel_profile dt={} substeps={} damping={} friction_stiffness={} history_decay={} history_skin={} spring_epsilon={}\n",
        profile.physics.kernel_dt,
        profile.physics.kernel_substeps,
        profile.angular_damping,
        profile.friction_stiffness,
        profile.history_decay,
        profile.history_skin,
        profile.spring_epsilon
    );
    // Constant center force checks the integrator independently of a controller.
    let extra_force = [assembly.mass * 12.0, 0.0, 0.0];
    let start = body.center;
    let start_velocity = body.velocity;
    let count = profile.physics.kernel_substeps * 240;
    for step in 1..=count {
        body.apply_force(extra_force, body.center)?;
        body.step(&profile)?;
        if step.is_multiple_of(profile.physics.kernel_substeps * 60) {
            writeln!(
                report,
                "kernel_step step={step} center={:?} velocity={:?} root_position={:?}",
                body.center, body.velocity, body.frame.position
            )
            .unwrap();
        }
    }
    let dt = f64::from(profile.physics.kernel_dt);
    let n = f64::from(count);
    let acceleration = [12.0, f64::from(profile.physics.legacy_acceleration), 0.0];
    let position_error = (0..3)
        .map(|a| {
            (f64::from(body.center[a])
                - (f64::from(start[a])
                    + f64::from(start_velocity[a]) * n * dt
                    + acceleration[a] * dt * dt * n * (n + 1.0) * 0.5))
                .abs()
        })
        .fold(0.0, f64::max);
    let velocity_error = (0..3)
        .map(|a| {
            (f64::from(body.velocity[a])
                - (f64::from(start_velocity[a]) + acceleration[a] * dt * n))
                .abs()
        })
        .fold(0.0, f64::max);
    if position_error > 0.02 || velocity_error > 0.03 {
        return Err(format!(
            "kernel constant force mismatch position={position_error} velocity={velocity_error}"
        ));
    }
    let pose = rig.pose(&body.frame)?;
    if pose.len() != 7 {
        return Err("kernel pose lost assembly bodies".into());
    }
    let still = ContactBodyState {
        center: [0.0; 3],
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
    };
    let mut spring = ContactSpring::default();
    let parameters = ContactParameters {
        compression: 7500.0,
        decompression: 3750.0,
        friction: 0.3,
    };
    let first = spring.force(
        &profile,
        parameters,
        -0.01,
        [0.0, -1.0, 0.0],
        [0.0; 3],
        &still,
        &still,
    )?;
    let second = spring.force(
        &profile,
        parameters,
        -0.02,
        [0.0, -1.0, 0.0],
        [0.0; 3],
        &still,
        &still,
    )?;
    if first != [0.0; 3] || (second[1] + 37.5).abs() > 0.001 {
        return Err(format!(
            "kernel contact history mismatch {first:?} {second:?}"
        ));
    }
    writeln!(
        report,
        "contact_spring first_force={first:?} second_force={second:?} parameters=explicit_fixture"
    )
    .unwrap();
    writeln!(report,"kernel_pass parts=7 mass={} constrained_steps={count} position_error={position_error} velocity_error={velocity_error} pose_bodies={} contact_generation=not_implemented impulse_pass=not_implemented native_differential=not_tested",assembly.mass,pose.len()).unwrap();
    Ok(report)
}

#[cfg(test)]
impl KernelProfile {
    /// Round-number profile for unit tests in other modules.
    pub(crate) fn test_profile() -> Self {
        tests::profile()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn profile() -> KernelProfile {
        KernelProfile {
            physics: NativePhysics {
                free_fall_dt: 1.0,
                kernel_dt: 0.1,
                kernel_substeps: 10,
                angular_damping: 0.5,
                legacy_acceleration: -10.0,
                anti_denormal: 1e-20,
            },
            angular_damping: 0.9998,
            friction_stiffness: 0.2,
            history_decay: 0.999,
            history_skin: 0.01,
            spring_epsilon: 1e-8,
        }
    }
    fn body() -> KernelBody {
        let frame = CoordinateFrame {
            position: [0.0; 3],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let a = AssemblyProperties {
            mass: 2.0,
            center_world: [0.0; 3],
            center_in_body: [0.0; 3],
            inertia_in_body: [[2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 8.0]],
            body_frame: frame.clone(),
            parts: 1,
        };
        KernelBody::new(
            &a,
            &FreeFallBody::new(frame, [0.0; 3], [0.0; 3]).unwrap(),
            &profile(),
        )
        .unwrap()
    }
    #[test]
    fn constrained_step_uses_updated_velocity_and_resets_accumulators() {
        let mut b = body();
        b.apply_force([20.0, 0.0, 0.0], b.center).unwrap();
        b.linear_impulse = [2.0, 0.0, 0.0];
        b.step(&profile()).unwrap();
        assert!((b.center[0] - 0.2).abs() < 1e-7);
        assert!((b.center[1] + 0.1).abs() < 1e-7);
        assert_eq!(b.velocity[..2], [2.0, -1.0]);
        assert_eq!(b.force, [0.0, -20.0, 0.0]);
        assert_eq!(b.linear_impulse, [0.0; 3]);
        b.step(&profile()).unwrap();
        assert!((b.center[0] - 0.4).abs() < 1e-7);
    }
    #[test]
    fn free_fall_step_matches_the_free_fall_probe_for_a_centered_body() {
        let mut b = body();
        b.velocity = [1.0, 2.0, 3.0];
        b.angular_velocity = [0.5, 0.25, 0.125];
        let mut probe = FreeFallBody::new(b.frame.clone(), b.velocity, b.angular_velocity).unwrap();
        b.free_fall_step(&profile()).unwrap();
        probe
            .step(&profile().physics, physics::GravitySource::Legacy)
            .unwrap();
        assert_eq!(b.frame, probe.frame);
        assert_eq!(b.center, probe.frame.position);
        assert_eq!(b.velocity, probe.velocity);
        assert_eq!(b.angular_velocity, probe.angular_velocity);
        assert!(!b.has_external_force_or_impulse());
    }
    #[test]
    fn contact_prestep_stores_reference_impulse_and_keeps_position() {
        let mut b = body();
        b.angular_momentum = [0.0, 8.0, 0.0];
        b.linear_impulse = [1.0, 0.0, 0.0];
        assert!(b.has_external_force_or_impulse());
        b.contact_prestep(&profile()).unwrap();
        // force (0, -20, 0) * dt 1 + J (1, 0, 0).
        assert_eq!(b.reference_impulse, [1.0, -20.0, 0.0]);
        assert_eq!(b.velocity[..2], [0.5, -10.0]);
        assert_eq!(b.center, [0.0; 3]);
        assert!((b.angular_momentum[1] - 4.0).abs() < 1e-6);
        assert!((b.angular_velocity[1] - 1.0).abs() < 1e-6);
        assert!(!b.has_external_force_or_impulse());
        b.contact_integrate(&profile()).unwrap();
        assert_eq!(b.center[..2], [0.5, -10.0]);
        assert_eq!(b.frame.position, b.center);
    }
    #[test]
    fn rebuild_momentum_uses_world_inertia() {
        let mut b = body();
        b.angular_velocity = [1.0, 1.0, 1.0];
        b.rebuild_momentum();
        assert_eq!(b.angular_momentum, [2.0, 4.0, 8.0]);
    }
    #[test]
    fn force_at_contact_generates_torque_and_native_momentum_damping() {
        let mut b = body();
        b.angular_momentum = [0.0, 0.0, 8.0];
        b.apply_force([0.0, 10.0, 0.0], [2.0, 0.0, 0.0]).unwrap();
        assert_eq!(b.torque, [0.0, 0.0, 20.0]);
        b.step(&profile()).unwrap();
        assert!((b.angular_momentum[2] - (8.0 * 0.9998 + 2.0)).abs() < 1e-6);
        assert!((b.angular_velocity[2] - b.angular_momentum[2] / 8.0).abs() < 1e-6);
        assert_eq!(b.torque, [0.0; 3]);
    }
    #[test]
    fn published_origin_velocity_includes_center_offset_rotation() {
        let mut b = body();
        b.center_in_body = [1.0, 0.0, 0.0];
        b.center = [1.0, 0.0, 0.0];
        b.angular_velocity = [0.0, 0.0, 2.0];
        b.velocity = [0.0, 2.0, 0.0];
        assert_eq!(b.body_state().velocity, [0.0; 3]);
    }
    #[test]
    fn animated_cofm_refresh_preserves_origin_motion_and_pending_forces() {
        let mut b = body();
        b.angular_velocity = [0.0, 0.0, 2.0];
        b.velocity = [3.0, 4.0, 5.0];
        b.apply_force([0.0, 10.0, 0.0], [2.0, 0.0, 0.0]).unwrap();
        b.linear_impulse = [1.0, 2.0, 3.0];
        b.angular_impulse = [4.0, 5.0, 6.0];
        let origin = b.body_state();
        let assembly = AssemblyProperties {
            mass: 2.0,
            center_world: [1.0, 0.0, 0.0],
            center_in_body: [1.0, 0.0, 0.0],
            inertia_in_body: diagonal([3.0, 4.0, 10.0]),
            body_frame: b.frame.clone(),
            parts: 2,
        };
        b.refresh_assembly(&assembly, &profile()).unwrap();
        assert_eq!(b.body_state().frame, origin.frame);
        assert_eq!(b.body_state().velocity, origin.velocity);
        assert_eq!(b.angular_velocity, origin.angular_velocity);
        assert_eq!(b.center, [1.0, 0.0, 0.0]);
        assert_eq!(b.velocity, [3.0, 6.0, 5.0]);
        assert_eq!(b.angular_momentum, [0.0, 0.0, 20.0]);
        assert_eq!(b.force, [0.0, -10.0, 0.0]);
        assert_eq!(b.torque, [0.0, 0.0, 20.0]);
        assert_eq!(b.linear_impulse, [1.0, 2.0, 3.0]);
        assert_eq!(b.angular_impulse, [4.0, 5.0, 6.0]);
    }
    #[test]
    fn contact_history_limits_friction_using_previous_normal_force() {
        let a = ContactBodyState {
            center: [0.0; 3],
            velocity: [10.0, 0.0, 0.0],
            angular_velocity: [0.0; 3],
        };
        let b = ContactBodyState {
            center: [0.0; 3],
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        let p = ContactParameters {
            compression: 7500.0,
            decompression: 3750.0,
            friction: 0.3,
        };
        let mut s = ContactSpring::default();
        let f = s
            .force(&profile(), p, -0.01, [0.0, -1.0, 0.0], [0.0; 3], &a, &b)
            .unwrap();
        assert_eq!(f, [0.0; 3]);
        let f = s
            .force(&profile(), p, -0.02, [0.0, -1.0, 0.0], [0.0; 3], &a, &b)
            .unwrap();
        assert!((f[1] + 37.5).abs() < 1e-5);
        assert_eq!(f[0], 0.0);
        let f = s
            .force(&profile(), p, -0.03, [0.0, -1.0, 0.0], [0.0; 3], &a, &b)
            .unwrap();
        assert!((f[0] - 11.25).abs() < 1e-5);
        let offset = s.tangent_offset;
        assert_eq!(
            s.force(&profile(), p, 0.0, [0.0, -1.0, 0.0], [0.0; 3], &a, &b)
                .unwrap(),
            [0.0; 3]
        );
        assert_eq!(s.tangent_offset, offset);
    }
}
