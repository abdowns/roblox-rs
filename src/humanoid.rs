//! RBX::Humanoid with its HumanoidState objects: Humanoid::onStepped state
//! dispatch, the per-state update (slot +0x14), the per-substep kernel force
//! callbacks (slot +0xc) and the Lua-visible signals fired on state edges.
use crate::{
    animation::RunningSignal,
    assembly::Tensor,
    balancing::Balancing,
    bundle::Bundle,
    humanoid_state::{self, FloorPart, FloorQuery, HumanoidStateProfile, StateType},
    kernel::{ContactParameters, KernelBody, cross},
    locomotion::{RunningInput, RunningProfile, Solver},
    native_data::NativeImage,
    physics::{self, FreeFallBody},
    player::{HumanoidMotion, HumanoidProperties, MoveContext},
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
    world::{Statics, Surroundings, World},
};
use std::collections::BTreeMap;

/// Humanoid signals as fired by 00bfda98 (state edges) and the Running
/// publishers 00c02f36 / 00be46f8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HumanoidEvent {
    Died,
    Running(f32),
    Jumping(bool),
    Climbing(f32),
    GettingUp(bool),
    FreeFalling(bool),
    FallingDown(bool),
    Seated(bool),
    PlatformStanding(bool),
    Swimming(f32),
    Ragdoll(bool),
    Strafing(bool),
}

/// Per-state object fields (HumanoidState and subclasses). A transition builds
/// a fresh object, so every field restarts from its constructor value.
#[derive(Clone)]
struct HumanoidStateData {
    kind: StateType,
    /// +0x14: type passed to the constructor; events fire when it differs.
    previous: StateType,
    /// +0xc / +0x10 / +0x90.
    timer: f32,
    elapsed: f32,
    no_floor: f32,
    floor: FloorQuery,
    /// RunningBase +0x134; INFINITY without floor.
    support_height: f32,
    /// RunningBase +0x11c.. / Freefall +0x114..: desired velocity and rotation.
    desired_velocity: [f32; 3],
    desired_rotation: [f32; 3],
    /// Freefall +0x108 carried velocity.
    carry: [f32; 3],
    /// Jumping +0x4a and +0x104 direction.
    jump_done: bool,
    jump_direction: [f32; 3],
    /// +0x98 Running event deadband.
    running_speed: f32,
    balancing: Balancing,
    /// RunningBase +0x104 / +0x110: the floor's velocity at the touch point
    /// and its angular velocity (00c02c28), zero without a floor.
    floor_velocity: [f32; 3],
    floor_angular_velocity: [f32; 3],
}

pub struct Humanoid {
    pub properties: HumanoidProperties,
    motion: HumanoidMotion,
    profile: HumanoidStateProfile,
    running: RunningProfile,
    balancing: Balancing,
    signal: RunningSignal,
    state: HumanoidStateData,
    spring_gain: f32,
    spring_damping: f32,
    spring_limit: f32,
    turn_gain: f32,
    yaw_gain: f32,
    yaw_limit: f32,
    torso_size: [f32; 3],
    head_size: [f32; 3],
    leg_height: f32,
    /// Humanoid.Jump (+0x1b9) and the ControlScript's held jump key.
    jump: bool,
    jump_held: bool,
    /// Humanoid +0x22c, stored by findFloor while flag 01d409c4 is set.
    last_floor_normal: [f32; 3],
    /// workspace.CurrentCamera for Player:Move(..., true); None = world input.
    camera: Option<CoordinateFrame>,
    pub transitions: Vec<(u32, StateType)>,
    events: Vec<HumanoidEvent>,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}

/// Humanoid torso yaw 00bdcc6c: column 2.
fn current_yaw(torso: &CoordinateFrame) -> f32 {
    f64::from(torso.rotation[0][2]).atan2(f64::from(torso.rotation[2][2])) as f32
}
fn delta_angle(target: f32, current: f32) -> f64 {
    let mut delta = f64::from(target - current);
    if !(-std::f64::consts::PI..std::f64::consts::PI).contains(&delta) {
        delta -= ((delta + std::f64::consts::PI) / std::f64::consts::TAU).floor()
            * std::f64::consts::TAU;
    }
    delta
}

impl Humanoid {
    pub fn new(bundle: &Bundle, scene: &Scene) -> Result<Self, String> {
        let mut sizes = BTreeMap::new();
        for instance in &scene.instances {
            if let Some(part) = &instance.part {
                sizes.insert(instance.name.clone(), part.size);
            }
        }
        let size = |name: &str| {
            sizes
                .get(name)
                .copied()
                .ok_or(format!("missing {name} size"))
        };
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let balancing = Balancing::load(bundle)?;
        let profile = HumanoidStateProfile::load(bundle)?;
        let gains = balancing.gains();
        let state = HumanoidStateData {
            kind: StateType::Running,
            previous: StateType::Running,
            timer: 0.0,
            elapsed: 0.0,
            no_floor: 0.0,
            floor: FloorQuery::default(),
            support_height: f32::INFINITY,
            desired_velocity: [0.0; 3],
            desired_rotation: [0.0; 3],
            carry: [0.0; 3],
            jump_done: false,
            jump_direction: [0.0, profile.jump_direction_y, 0.0],
            running_speed: 0.0,
            balancing: balancing.with_gains(gains[0], gains[1]),
            floor_velocity: [0.0; 3],
            floor_angular_velocity: [0.0; 3],
        };
        let mut humanoid = Self {
            properties: HumanoidProperties::character(bundle)?,
            motion: HumanoidMotion::default(),
            running: RunningProfile::load(bundle)?,
            signal: RunningSignal::load(bundle)?,
            state,
            spring_gain: float(0x017b05a0)?,
            spring_damping: float(0x017b05a4)?,
            spring_limit: float(0x017b05bc)?,
            turn_gain: float(0x016f7530)?,
            yaw_gain: float(0x0179fc24)?,
            yaw_limit: float(0x0177ed40)?,
            torso_size: size("Torso")?,
            head_size: size("Head")?,
            // getRightLeg (+0x29c) first, then getLeftLeg (+0x294), 00c03048.
            leg_height: size("Right Leg").or_else(|_| size("Left Leg"))?[1],
            jump: false,
            jump_held: false,
            last_floor_normal: [0.0; 3],
            camera: None,
            transitions: Vec::new(),
            events: Vec::new(),
            balancing,
            profile,
        };
        humanoid.reset();
        Ok(humanoid)
    }
    pub fn reset(&mut self) {
        self.motion = HumanoidMotion::default();
        self.balancing.reset();
        self.signal.speed = 0.0;
        // The spawned character starts in Running with no pending events.
        self.state = self.new_state(StateType::Running, StateType::Running);
        self.jump = false;
        self.jump_held = false;
        self.last_floor_normal = [0.0; 3];
        self.transitions.clear();
        self.events.clear();
    }
    /// ControlScript's jump key: while held, MasterControl sets Humanoid.Jump
    /// every render step; every new state clears it (00bfd2a2 -> setJump(0)).
    pub fn set_jump(&mut self, held: bool) {
        self.jump_held = held;
    }
    /// MasterControl calls Player:Move(moveValue, true): input is camera-relative.
    pub fn set_camera(&mut self, camera: Option<CoordinateFrame>) {
        self.camera = camera;
    }
    pub fn state_type(&self) -> StateType {
        self.state.kind
    }
    /// Running event deadband speed (+0x98).
    pub fn running_speed(&self) -> f32 {
        self.state.running_speed
    }
    pub fn support_height(&self) -> f32 {
        self.state.support_height
    }
    /// The Part under the floor rays' first primary hit (+0x60).
    pub fn floor_part(&self) -> Option<FloorPart> {
        self.state.floor.part
    }
    /// The floor motion RunningBase/RunningNoPhysics last read (00c02c28).
    pub fn floor_velocity(&self) -> [f32; 3] {
        self.state.floor_velocity
    }
    /// Signals fired since the last call, in firing order.
    pub fn take_events(&mut self) -> Vec<HumanoidEvent> {
        std::mem::take(&mut self.events)
    }
    /// Fixture contact materials. Freefall sets Head/Torso Friction to 0
    /// (00bd3ab4); the fixture then uses zero contact friction (combination
    /// not recovered).
    pub fn contact_parameters(&self) -> ContactParameters {
        ContactParameters {
            compression: 7500.0,
            decompression: 3750.0,
            friction: if self.state.kind == StateType::Freefall {
                0.0
            } else {
                0.3
            },
        }
    }
    fn new_state(&self, kind: StateType, previous: StateType) -> HumanoidStateData {
        let gains = self.balancing.gains();
        let (timer, balance) = match kind {
            StateType::Jumping => (self.profile.jump_seconds, gains),
            StateType::Landed => (self.profile.landed_seconds, self.profile.landed_balance),
            StateType::Freefall => (0.0, [self.profile.freefall_balance_gain, gains[1]]),
            _ => (0.0, gains),
        };
        HumanoidStateData {
            kind,
            previous,
            timer,
            elapsed: 0.0,
            no_floor: 0.0,
            floor: FloorQuery::default(),
            support_height: f32::INFINITY,
            desired_velocity: [0.0; 3],
            desired_rotation: [0.0; 3],
            carry: [0.0; 3],
            jump_done: false,
            jump_direction: [0.0, self.profile.jump_direction_y, 0.0],
            running_speed: 0.0,
            balancing: self.balancing.with_gains(balance[0], balance[1]),
            floor_velocity: [0.0; 3],
            floor_angular_velocity: [0.0; 3],
        }
    }
    fn torso(world: &World, rig: &R6Rig) -> Result<CoordinateFrame, String> {
        rig.pose(&world.body.frame)?
            .remove("Torso")
            .ok_or_else(|| "missing Torso".into())
    }
    /// Humanoid::calcDesiredWalkVelocity 00bdc44c, non-camera AutoRotate branch:
    /// desired velocity and rotation (vtable +0xe0).
    fn desired_motion(&self, torso: &CoordinateFrame) -> ([f32; 3], [f32; 3]) {
        let desired = self
            .motion
            .move_direction
            .map(|v| v * self.properties.walk_speed);
        let yaw = if desired[0] != 0.0 || desired[2] != 0.0 {
            let target = f64::from(-desired[0]).atan2(f64::from(-desired[2])) as f32;
            delta_angle(target, current_yaw(torso)) as f32 * self.turn_gain
        } else {
            0.0
        };
        (desired, [0.0, yaw, 0.0])
    }
    /// Humanoid::onStepped 00bde4f2 -> 00c0072c with Stepped dt.
    pub fn on_stepped(
        &mut self,
        world: &mut World,
        rig: &R6Rig,
        input: [f32; 3],
        dt: f32,
    ) -> Result<(), String> {
        self.motion.move_input(
            input,
            &MoveContext {
                attached_to_data_model: true,
                relative_to_camera: self.camera.is_some(),
                camera: self.camera.clone(),
                swimming: false,
            },
        )?;
        if self.jump_held {
            self.jump = true;
        }
        self.state_stepped(world, rig, dt)?;
        let next = {
            let torso = Self::torso(world, rig)?;
            let profile = self.profile.clone();
            let kind = self.state.kind;
            profile.transition(kind, |event| self.condition(event, &torso, world, rig))?
        };
        if let Some(next) = next {
            self.enter(next, world, rig)?;
            self.state_stepped(world, rig, dt)?;
        }
        self.state_update(world, rig, dt)?;
        self.state_events(world, rig)
    }
    /// HumanoidState::onStepped 00bff424: findFloor 00bff784, then timers.
    fn state_stepped(&mut self, world: &mut World, rig: &R6Rig, dt: f32) -> Result<(), String> {
        let had_floor = self.state.floor.floor.is_some();
        let old_part = self.state.floor.part;
        self.state.floor = FloorQuery::default();
        if self.profile.searches_floor(self.state.kind) {
            let torso = Self::torso(world, rig)?;
            let vy = world.point_velocity(torso.position)[1];
            let query = humanoid_state::find_floor_parts(
                &self.profile,
                &torso,
                self.torso_size,
                Some(self.leg_height),
                had_floor,
                vy,
                |o, d, l| world.raycast(o, d, l),
            );
            if let Some((_, normal)) = query.floor {
                self.state.no_floor = 0.0;
                self.last_floor_normal = normal;
            }
            self.state.floor = query;
        }
        // 00bff784: a new floor Part wakes its assembly (00ba7fb0, not
        // recursive).
        if let Some(part) = self.state.floor.part.filter(|p| Some(*p) != old_part) {
            world.wake_floor(part)?;
        }
        self.state.timer -= dt;
        self.state.elapsed += dt;
        self.state.no_floor += dt;
        Ok(())
    }
    /// HumanoidState 00c02c28: the floor primitive's velocity at the floor
    /// touch point (+0x6c) and its angular velocity, both times 00ba69b0 on
    /// the World's environmental throttle. PhysicsEnvironmentalThrottle
    /// DefaultAuto at level 0 gives 1 - 1/100000000, which is 1.0 in f32.
    /// Anchored floors have zero velocity. 00be32ea zeroes an unanchored
    /// floor's velocity when the character's assembly filter phase is above 2;
    /// in Play Solo the SpatialFilter mode is 0 and every moving assembly is
    /// in phase 0 (00b79c1e), so that never happens here.
    fn floor_motion(&self, world: &World) -> Result<([f32; 3], [f32; 3]), String> {
        let (Some(FloorPart::Loose(index)), Some((touch, _))) =
            (self.state.floor.part, self.state.floor.floor)
        else {
            return Ok(([0.0; 3], [0.0; 3]));
        };
        let pv = world
            .loose
            .parts
            .get(index)
            .ok_or("missing loose floor")?
            .body
            .body_state();
        let scale = 1.0f32 - 1.0 / 100_000_000.0f32;
        let p = pv.frame.position;
        let r: [f32; 3] = std::array::from_fn(|a| touch[a] - p[a]);
        let v = pv.velocity;
        let w = pv.angular_velocity;
        Ok((
            [
                ((w[1] * r[2] - r[1] * w[2]) + v[0]) * scale,
                ((w[2] * r[0] - r[2] * w[0]) + v[1]) * scale,
                ((r[1] * w[0] - r[0] * w[1]) + v[2]) * scale,
            ],
            w.map(|x| x * scale),
        ))
    }
    /// c0192c: HumanoidRootPart (non-colliding, never) or Head touching.
    fn root_or_head_touching(&mut self, world: &mut World, rig: &R6Rig) -> Result<bool, String> {
        let head = rig
            .pose(&world.body.frame)?
            .remove("Head")
            .ok_or("missing Head")?;
        if world.collider_touching("Head", &head)? {
            self.state.elapsed = 0.0;
            return Ok(true);
        }
        Ok(false)
    }
    /// HumanoidState::computeEvent 00c0132c for the events these states use.
    fn condition(
        &mut self,
        event: usize,
        torso: &CoordinateFrame,
        world: &mut World,
        rig: &R6Rig,
    ) -> Result<bool, String> {
        Ok(match event {
            0 => self.properties.health <= 0.0,
            // Head and HumanoidRootPart stay jointed in this fixture.
            1 => false,
            2 => {
                let normal = self
                    .state
                    .floor
                    .floor
                    .map_or(self.last_floor_normal, |f| f.1);
                let slope = dot(normal, [0.0, 1.0, 0.0]);
                let limit =
                    f64::from(self.properties.max_slope_angle * std::f32::consts::PI / 180.0).cos()
                        as f32;
                if slope <= limit
                    && matches!(
                        self.state.kind,
                        StateType::Running | StateType::RunningNoPhysics
                    )
                {
                    false
                } else {
                    self.jump && 0.0 < self.properties.jump_power
                }
            }
            // Sit, PlatformStand, Ragdoll flags, pending push and swimming are false.
            3 | 5 | 7 | 17 | 18 | 19 | 23 | 24 => false,
            9 => {
                if torso.rotation[1][1] < self.profile.fall_down_tilt {
                    return Err("FallingDown state is not implemented".into());
                }
                false
            }
            // Climbing detection (00c0396a/00bfeb46) is not implemented.
            11 => false,
            13 => self.state.floor.floor.is_none(),
            14 => self.profile.no_floor_seconds < self.state.no_floor,
            15 => {
                self.state.floor.floor.is_some() && world.point_velocity(torso.position)[1] <= 0.0
            }
            16 => self.root_or_head_touching(world, rig)?,
            20 => self.state.jump_done,
            21 => self.state.timer <= 0.0,
            22 => {
                self.root_or_head_touching(world, rig)?;
                // No BodyMover descendants in the character (00c01a4c).
                self.profile.running_idle_seconds < self.state.elapsed
            }
            _ => return Err(format!("Humanoid event {event} is not implemented")),
        })
    }
    fn enter(&mut self, next: StateType, world: &mut World, rig: &R6Rig) -> Result<(), String> {
        let old = self.state.kind;
        self.transitions.push((world.steps, next));
        self.state = self.new_state(next, old);
        self.jump = false;
        let torso = Self::torso(world, rig)?;
        let v = world.point_velocity(torso.position);
        let speed = (v[2] * v[2] + v[0] * v[0]).sqrt();
        match next {
            // RunningBase constructors publish through the deadband (00c02f36).
            StateType::Running | StateType::Landed => self.running_event(speed, true)?,
            // MovingNoPhysicsBase constructor: raw Running (flag 01d73b64 is BSS).
            StateType::RunningNoPhysics => self.running_event(speed, false)?,
            _ => {}
        }
        if old == StateType::RunningNoPhysics {
            // The Torso primitive returns to the dynamics engine; kernel
            // accumulators restart from the published kinematic state.
            let state = world.body.body_state();
            world.body.set_body_state(&state)?;
            world.clear_contacts();
        }
        Ok(())
    }
    /// 00c02f36 (deadband) or 00be46f8 (raw) on Humanoid.Running.
    fn running_event(&mut self, speed: f32, deadband: bool) -> Result<(), String> {
        if deadband {
            self.signal.speed = self.state.running_speed;
            if !self.signal.observe(speed)? {
                return Ok(());
            }
            self.state.running_speed = self.signal.speed;
            self.events.push(HumanoidEvent::Running(self.signal.speed));
        } else {
            self.events.push(HumanoidEvent::Running(speed));
        }
        Ok(())
    }
    /// 00bfda98: the signal for one state edge (`active` false for the old
    /// state, true for the new one).
    fn edge_event(kind: StateType, active: bool, speed: f32) -> HumanoidEvent {
        match kind {
            StateType::Jumping => HumanoidEvent::Jumping(active),
            StateType::Freefall | StateType::Landed => HumanoidEvent::FreeFalling(active),
            // Cases 8/10 fire raw Running(speed) for either edge.
            StateType::Running | StateType::RunningNoPhysics => HumanoidEvent::Running(speed),
        }
    }
    /// Slot +0x38: 00bfda14 events (old false, new true), then Running's event.
    fn state_events(&mut self, world: &World, rig: &R6Rig) -> Result<(), String> {
        let torso = Self::torso(world, rig)?;
        let v = world.point_velocity(torso.position);
        let speed = (v[2] * v[2] + v[0] * v[0]).sqrt();
        if self.state.previous != self.state.kind {
            self.events
                .push(Self::edge_event(self.state.previous, false, speed));
            self.events
                .push(Self::edge_event(self.state.kind, true, speed));
            self.state.previous = self.state.kind;
        }
        if matches!(
            self.state.kind,
            StateType::Running | StateType::RunningNoPhysics
        ) {
            self.running_event(speed, true)?;
        }
        Ok(())
    }
    /// Slot +0x14: RunningBase 00c0a498, Freefall 00bd3e14, RunningNoPhysics
    /// 00c07626; Jumping (Flying 00bd3562) does nothing.
    fn state_update(&mut self, world: &mut World, rig: &R6Rig, dt: f32) -> Result<(), String> {
        let torso = Self::torso(world, rig)?;
        match self.state.kind {
            StateType::Running | StateType::Landed => {
                let floor = self.state.floor.clone();
                self.state.support_height = match floor.average {
                    Some(average) => humanoid_state::support_height(
                        average[1],
                        self.torso_size[1],
                        Some(self.leg_height),
                    ),
                    None => f32::INFINITY,
                };
                (self.state.floor_velocity, self.state.floor_angular_velocity) =
                    self.floor_motion(world)?;
                if let Some((_, normal)) = floor.floor {
                    (self.state.desired_velocity, self.state.desired_rotation) =
                        self.desired_motion(&torso);
                    // 00c0a498: up . n summed z, y, x.
                    let up = [0.0f32, 1.0, 0.0];
                    let d = (up[2] * normal[2] + up[1] * normal[1]) + up[0] * normal[0];
                    self.state.desired_velocity =
                        self.steep_floor(self.state.desired_velocity, normal, d);
                }
            }
            StateType::Freefall => {
                let (velocity, rotation) = self.desired_motion(&torso);
                let mut desired: [f32; 3] =
                    std::array::from_fn(|a| self.state.carry[a] + velocity[a]);
                let mut rotation = rotation;
                if f64::from(dot(desired, desired).sqrt()) < self.profile.freefall_walk_threshold {
                    desired = [0.0; 3];
                    rotation = [0.0; 3];
                } else if f64::from(dot(velocity, velocity).sqrt())
                    > self.profile.freefall_walk_threshold
                {
                    let target = f64::from(-desired[0]).atan2(f64::from(-desired[2])) as f32;
                    let delta = delta_angle(target, current_yaw(&torso)) as f32;
                    // AutoRotate is true; camera-locked rotation is not modeled.
                    rotation[1] = self.profile.freefall_yaw_rate(delta);
                }
                self.state.desired_velocity = desired;
                self.state.desired_rotation = rotation;
                let decay =
                    (dt * self.profile.freefall_decay) / self.profile.freefall_decay_interval + 1.0;
                self.state.carry = self.state.carry.map(|v| v * decay);
            }
            StateType::RunningNoPhysics => self.kinematic_update(world, &torso, dt)?,
            StateType::Jumping => {}
        }
        Ok(())
    }
    /// The steep-floor rule shared by RunningBase 00c0a498 and
    /// MovingNoPhysicsBase 00c07626. With `d` = up . floor normal below
    /// cos(MaxSlopeAngle) (00bfd9b4), the velocity loses its component along
    /// the normal and its downhill component becomes WalkSpeed (+0x194).
    /// Walkable floors leave it unchanged.
    fn steep_floor(&self, velocity: [f32; 3], n: [f32; 3], d: f32) -> [f32; 3] {
        let limit =
            f64::from(self.properties.max_slope_angle * std::f32::consts::PI / 180.0).cos() as f32;
        if d >= limit {
            return velocity;
        }
        let up = [0.0f32, 1.0, 0.0];
        let mut t = [
            -(up[0] - n[0] * d),
            -(up[1] - n[1] * d),
            -(up[2] - d * n[2]),
        ];
        // G3D::Vector3::unitize(1e-6).
        let length = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
        if length <= 1e-6 {
            return velocity;
        }
        let inverse = 1.0 / length;
        t = t.map(|v| v * inverse);
        let along = (n[2] * velocity[2] + n[1] * velocity[1]) + n[0] * velocity[0];
        let p = [
            velocity[0] - n[0] * along,
            velocity[1] - n[1] * along,
            velocity[2] - along * n[2],
        ];
        let w = self.properties.walk_speed;
        let downhill = (p[2] * t[2] + p[1] * t[1]) + p[0] * t[0];
        std::array::from_fn(|a| (t[a] * w + p[a]) - t[a] * downhill)
    }
    /// MovingNoPhysicsBase 00c07626: place the Torso assembly kinematically.
    fn kinematic_update(
        &mut self,
        world: &mut World,
        torso: &CoordinateFrame,
        dt: f32,
    ) -> Result<(), String> {
        let (desired, rotation) = self.desired_motion(torso);
        // 00c07626: desired plus the floor's motion (00c02c28).
        let (floor_velocity, floor_angular) = self.floor_motion(world)?;
        let mut v: [f32; 3] = std::array::from_fn(|a| desired[a] + floor_velocity[a]);
        let rotation: [f32; 3] = std::array::from_fn(|a| rotation[a] + floor_angular[a]);
        let floor = self.state.floor.clone();
        if let Some((_, normal)) = floor.floor {
            // 00c07626: up . n summed x, y, z.
            let up = [0.0f32, 1.0, 0.0];
            let d = (up[0] * normal[0] + up[1] * normal[1]) + up[2] * normal[2];
            v = self.steep_floor(v, normal, d);
        }
        if let (Some(_), Some(average)) = (floor.floor, floor.average) {
            let height = humanoid_state::support_height(
                average[1],
                self.torso_size[1],
                Some(self.leg_height),
            );
            if height - torso.position[1] != 0.0 {
                v[1] += (height - torso.position[1]) / dt;
            }
        }
        let current = world.point_velocity(torso.position);
        let gain = self.running.legacy_gain();
        let (dx, dz) = (v[0] - current[0], v[2] - current[2]);
        let limit = self.profile.kinematic_limits[usize::from(floor.floor.is_none())];
        let (ax, az) = (dx * gain, dz * gain);
        let squared = az * az + gain * 0.0 * gain * 0.0 + ax * ax;
        if limit * limit < squared {
            let m = squared.sqrt();
            let (mut nx, mut nz) = (0.0, 0.0);
            if 1e-7 <= m {
                (nx, nz) = if 1.00001 <= m || m <= 0.99999 {
                    (ax * (1.0 / m), az * (1.0 / m))
                } else {
                    (ax, az)
                };
            }
            let (sx, sz) = (nx * limit * dt, nz * limit * dt);
            if (0.0 <= sx && sx <= dx) || (sx < 0.0 && dx <= sx) {
                v[0] = current[0] + sx;
            }
            if (0.0 <= sz && sz <= dz) || (sz < 0.0 && dz <= sz) {
                v[2] = current[2] + sz;
            }
        }
        let mut placed = torso.clone();
        placed.position = std::array::from_fn(|a| v[a] * dt + torso.position[a]);
        let yaw = rotation[1];
        // Torso placement moves the assembly rigidly. Apply the same world yaw
        // to the root and its Torso offset instead of inverting frames.
        let turn = |m: [[f32; 3]; 3]| {
            if yaw != 0.0 {
                humanoid_state::yaw_frame(m, yaw * dt)
            } else {
                m
            }
        };
        placed.rotation = turn(torso.rotation);
        let arm: [f32; 3] =
            std::array::from_fn(|a| world.body.frame.position[a] - torso.position[a]);
        let arm_turned = turn([[arm[0], 0.0, 0.0], [arm[1], 0.0, 0.0], [arm[2], 0.0, 0.0]]);
        let root = CoordinateFrame {
            position: std::array::from_fn(|a| placed.position[a] + arm_turned[a][0]),
            rotation: turn(world.body.frame.rotation),
        };
        let spin = [rotation[0], yaw, rotation[2]];
        let arm = std::array::from_fn(|a| root.position[a] - placed.position[a]);
        let offset = cross(spin, arm);
        let state = FreeFallBody {
            quaternion: physics::matrix_quaternion(root.rotation)?,
            frame: root,
            velocity: std::array::from_fn(|a| v[a] + offset[a]),
            angular_velocity: spin,
        };
        world.body.set_body_state(&state)?;
        // 00c07626: moving faster than 2 over a floor wakes the floor
        // (00ba7fb0; recursive only with the PGS solver).
        if let Some(part) = floor.part {
            let speed = (v[2] * v[2] + (v[1] * v[1] + v[0] * v[0])).sqrt();
            if world.loose.sleep_profile().kinematic_wake_speed < speed {
                world.wake_floor(part)?;
            }
        }
        Ok(())
    }
    /// Whether the current state keeps the assembly in the kernel.
    pub fn is_kinematic(&self) -> bool {
        self.state.kind == StateType::RunningNoPhysics
    }
    /// The current state's kernel force callback (vtable +0xc), once per substep.
    pub fn apply_forces(
        &mut self,
        body: &mut KernelBody,
        statics: &Statics,
        torso: &CoordinateFrame,
        inertia: Tensor,
    ) -> Result<(), String> {
        let mut surroundings = Surroundings {
            statics,
            loose: None,
        };
        self.apply_forces_in(body, &mut surroundings, torso, inertia)
    }
    /// `apply_forces` reaching the loose Parts (floor mass and reaction).
    pub fn apply_forces_in(
        &mut self,
        body: &mut KernelBody,
        surroundings: &mut Surroundings,
        torso: &CoordinateFrame,
        inertia: Tensor,
    ) -> Result<(), String> {
        match self.state.kind {
            StateType::Running | StateType::Landed => {
                self.running_force(body, surroundings, torso, inertia)
            }
            StateType::Freefall => self.freefall_force(body, torso, inertia),
            StateType::Jumping => self.jump_force(body, surroundings, torso),
            StateType::RunningNoPhysics => Err("kinematic state has no kernel force".into()),
        }
    }
    /// RunningBase 00c09632 for Running and Landed, legacy solver.
    fn running_force(
        &mut self,
        body: &mut KernelBody,
        surroundings: &mut Surroundings,
        torso: &CoordinateFrame,
        inertia: Tensor,
    ) -> Result<(), String> {
        self.state.balancing.apply(body, torso, inertia)?;
        // Yaw torque about world Y towards the floor's spin plus the desired
        // rotation (+0x114 + +0x12c); native inertia and force limit.
        let target = self.state.floor_angular_velocity[1] + self.state.desired_rotation[1];
        let yaw = ((self.yaw_gain * inertia[1][1]) * (target - body.angular_velocity[1]))
            .clamp(-self.yaw_limit, self.yaw_limit);
        body.apply_torque([0.0, yaw, 0.0])?;
        let published = body.body_state();
        let root_mass = 1.0 / body.inverse_mass();
        // Mass selection: an anchored floor (Part +0xcb, or an assembly in
        // Sim::ANCHORED, 00ad8b68) or no floor uses the root mass; an
        // unanchored floor uses min(root, floor assembly mass) and remembers
        // whether the root is the lighter one (SETBE after UCOMISS).
        let floor = self.state.floor.part;
        let (mass, root_lighter) = match floor {
            Some(part @ FloorPart::Loose(_)) => {
                let floor_mass = surroundings
                    .floor_mass(part)?
                    .ok_or("missing loose floor mass")?;
                (
                    if floor_mass < root_mass {
                        floor_mass
                    } else {
                        root_mass
                    },
                    root_mass <= floor_mass,
                )
            }
            _ => (root_mass, true),
        };
        let center = body.center;
        // Vertical support only with a floor: target from the floor rays,
        // torso height and the root's Y velocity relative to the floor's.
        if let Some(part) = floor.filter(|_| self.state.support_height.is_finite()) {
            let acceleration = (self.state.support_height - torso.position[1]) * self.spring_gain
                - (published.velocity[1] - self.state.floor_velocity[1]) * self.spring_damping;
            let existing = body.force[1] / root_mass;
            if acceleration > 0.0 && existing < acceleration {
                let delta = acceleration - existing;
                let force = if root_lighter {
                    (delta * mass).clamp(-self.spring_limit, self.spring_limit)
                } else {
                    let lift = self.profile.floor_lift_limit;
                    let capped = if lift < delta { lift } else { delta };
                    (capped * mass).clamp(-self.yaw_limit, self.yaw_limit)
                };
                body.apply_force([0.0, force, 0.0], center)?;
                // c0acec/c0abf2: the floor's share at the floor touch point.
                let touch = self.state.floor.floor.ok_or("floor without touch point")?.0;
                surroundings.push_floor(
                    part,
                    [0.0, force * self.profile.floor_reaction, 0.0],
                    touch,
                )?;
            }
        }
        let desired = self.state.desired_velocity;
        let horizontal = self.running.horizontal_force(&RunningInput {
            velocity: [published.velocity[0], published.velocity[2]],
            desired_velocity: [desired[0], desired[2]],
            floor_velocity: [self.state.floor_velocity[0], self.state.floor_velocity[2]],
            accumulated_force: [body.force[0], body.force[2]],
            root_mass,
            effective_mass: mass,
            floor_present: floor.is_some(),
            floor_friction: 1.0,
            solver: Solver::Legacy,
        })?;
        // The legacy floor reaction for this force is pushed only when its Y
        // component is positive (00c0a2ea); Running's Y component is zero
        // (+0x4d clear), so the floor gets the zero vector.
        body.apply_force([horizontal[0], 0.0, horizontal[1]], center)
    }
    /// Freefall 00bd4194: balancing, yaw torque and limited air control.
    fn freefall_force(
        &mut self,
        body: &mut KernelBody,
        torso: &CoordinateFrame,
        inertia: Tensor,
    ) -> Result<(), String> {
        self.state.balancing.apply(body, torso, inertia)?;
        let iy = inertia[1][1];
        let limit = iy * self.profile.freefall_torque_limit;
        let value = (self.state.desired_rotation[1] - body.angular_velocity[1])
            * (iy * self.profile.freefall_yaw_gain)
            - body.torque[1];
        let torque = if -limit < value {
            if limit <= value { limit } else { value }
        } else {
            -limit
        };
        body.apply_torque([0.0, torque, 0.0])?;
        let velocity = body.body_state().velocity;
        let gain = self.profile.air_gain;
        let mut a = [
            gain * (self.state.desired_velocity[0] - velocity[0]),
            gain * (self.state.desired_velocity[2] - velocity[2]),
        ];
        let squared = a[1] * a[1] + a[0] * a[0] + 0.0;
        if self.profile.air_limit_squared < squared {
            let m = squared.sqrt();
            if 1e-7 <= m {
                if 1.00001 <= m || m <= 0.99999 {
                    a = a.map(|v| v * (1.0 / m));
                }
            } else {
                a = [0.0; 2];
            }
            a = a.map(|v| v * self.profile.air_limit);
        }
        let mass = 1.0 / body.inverse_mass();
        let center = body.center;
        body.apply_force([a[0] * mass, 0.0, a[1] * mass], center)
    }
    /// Jumping 00c05e0e: no balancing (Flying 00bd3568 is empty).
    fn jump_force(
        &mut self,
        body: &mut KernelBody,
        surroundings: &mut Surroundings,
        torso: &CoordinateFrame,
    ) -> Result<(), String> {
        if self.state.jump_done {
            return Ok(());
        }
        let v = body.body_state().velocity;
        let d = self.state.jump_direction;
        let along = (v[1] * d[1] + v[0] * d[0]) + v[2] * d[2];
        let acceleration = (self.properties.jump_power - along) * self.profile.jump_gain;
        if acceleration <= 0.0 || self.ceiling(surroundings, torso) {
            self.state.jump_done = true;
            return Ok(());
        }
        let root_mass = 1.0 / body.inverse_mass();
        let center = body.center;
        let Some(part) = self.state.floor.part else {
            let force = acceleration * root_mass;
            return body.apply_force(d.map(|v| v * force), center);
        };
        // Floor branch: min(root, floor assembly mass) for any floor (no
        // anchored test here); the fixture planes stand in for a heavier one.
        let mass = match surroundings.floor_mass(part)? {
            Some(floor_mass) if floor_mass < root_mass => floor_mass,
            _ => root_mass,
        };
        let force = acceleration * mass;
        let existing = body.force[1];
        if existing < force {
            body.apply_force(d.map(|v| v * force), center)?;
            let touch = self.state.floor.floor.ok_or("floor without touch point")?.0;
            surroundings.push_floor(part, [0.0, -(force - existing), 0.0], touch)?;
        }
        Ok(())
    }
    /// 00c0624e: upward rays from the Torso; any hit ends the jump push.
    fn ceiling(&self, surroundings: &Surroundings, torso: &CoordinateFrame) -> bool {
        let [sx, sy, sz] = self.torso_size;
        let length = sy * 1.5 + self.head_size[1];
        let up = [0.0, 1.0, 0.0];
        let mut origins = vec![[0.0, -(sy * 0.4), 0.0]];
        for x in [-1.0f32, 1.0] {
            for z in [-1.0f32, 1.0] {
                origins.push([x * (sx * 0.4), sy * 0.4, z * (sz * 0.4)]);
            }
        }
        origins.into_iter().any(|local| {
            surroundings
                .raycast(torso.transform_point(local), up, length)
                .is_some()
        })
    }
}
