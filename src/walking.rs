//! Explicit flat-floor / infinite-wall walking fixture driven by the recovered
//! Humanoid state dispatch. The world (floor plane, wall half-space, materials,
//! awake scheduling) is still an explicit fixture, not native world contents.
use crate::{
    animation::{Pose, ProceduralAnimation, RunningSignal},
    assembly::{AssemblyProperties, MassPart, MassProfile},
    balancing::Balancing,
    bundle::Bundle,
    collision::ContactBodyState,
    humanoid_state::{self, FloorQuery, Hit, HumanoidStateProfile, StateType},
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile, cross},
    legacy_contact::{AwakeContact, BlockContactProfile, ImpulseInput, ImpulseProfile},
    locomotion::{RunningInput, RunningProfile, Solver},
    native_data::NativeImage,
    physics::{self, FreeFallBody},
    player::{HumanoidMotion, HumanoidProperties, MoveContext},
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
    scheduler::{FrameEvents, NativeSchedule, SchedulerProfile},
};
use std::{collections::BTreeMap, fmt::Write};
struct ContactState {
    spring: ContactSpring,
    impulse: AwakeContact,
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
}
pub struct WalkingFixture {
    pub body: KernelBody,
    pub steps: u32,
    pub contacts: usize,
    pub impulse_applications: u64,
    pub max_penetration: f32,
    pub floor_y: f32,
    pub wall_x: f32,
    original: KernelBody,
    kernel: KernelProfile,
    running: RunningProfile,
    humanoid: HumanoidProperties,
    motion: HumanoidMotion,
    geometry: BlockContactProfile,
    impulses: ImpulseProfile,
    colliders: Vec<(String, [f32; 3])>,
    states: BTreeMap<(String, usize), ContactState>,
    spring_gain: f32,
    spring_damping: f32,
    spring_limit: f32,
    balancing: Balancing,
    inertia: crate::assembly::Tensor,
    turn_gain: f32,
    yaw_gain: f32,
    yaw_limit: f32,
    mass_parts: BTreeMap<String, MassPart>,
    original_inertia: crate::assembly::Tensor,
    animation: ProceduralAnimation,
    signal: RunningSignal,
    animation_ticks: u32,
    joint_callbacks: u32,
    limb_angles: [f32; 4],
    schedule: NativeSchedule,
    original_schedule: NativeSchedule,
    last_frame: Option<FrameEvents>,
    profile: HumanoidStateProfile,
    state: HumanoidStateData,
    torso_size: [f32; 3],
    head_size: [f32; 3],
    leg_height: f32,
    /// Humanoid.Jump (+0x1b9) and the ControlScript's held jump key.
    jump: bool,
    jump_held: bool,
    /// Humanoid +0x22c, stored by findFloor while flag 01d409c4 is set.
    last_floor_normal: [f32; 3],
    /// humanoidAnimateLocal's `pose` variable, set by Humanoid events.
    script_pose: Pose,
    pub transitions: Vec<(u32, StateType)>,
    /// workspace.CurrentCamera for Player:Move(..., true); None = world input.
    camera: Option<CoordinateFrame>,
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}
impl WalkingFixture {
    pub fn new(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        let kernel = KernelProfile::load(bundle)?;
        let parts = MassProfile::load(bundle)?.character_parts(bundle, scene)?;
        let assembly = AssemblyProperties::from_rig(&parts, rig)?;
        let root = physics::character_bodies(bundle)?
            .into_iter()
            .find(|(n, _)| n == "HumanoidRootPart")
            .ok_or("missing walking root")?
            .1;
        let body = KernelBody::new(&assembly, &root, &kernel)?;
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let mut colliders = Vec::new();
        let mut floor_y = f32::INFINITY;
        let mut sizes = BTreeMap::new();
        for instance in &scene.instances {
            let Some(part) = &instance.part else { continue };
            sizes.insert(instance.name.clone(), part.size);
            if instance.name.ends_with(" Leg") {
                floor_y = floor_y.min(part.frame.position[1] - part.size[1] * 0.5);
            }
            let item = doc
                .descendants()
                .find(|n| {
                    n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent)
                })
                .ok_or("missing collider properties")?;
            let properties = item
                .children()
                .find(|n| n.has_tag_name("Properties"))
                .ok_or("missing collider properties")?;
            match properties
                .children()
                .find(|n| n.attribute("name") == Some("CanCollide"))
                .and_then(|n| n.text())
            {
                Some("true") if part.shape == 1 => {
                    colliders.push((instance.name.clone(), part.size))
                }
                Some("false") => {}
                _ => return Err("unsupported original walking collider".into()),
            }
        }
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let animation = ProceduralAnimation::load(bundle)?;
        let schedule =
            NativeSchedule::new(&SchedulerProfile::load(bundle)?, animation.wait_seconds)?;
        let size = |name: &str| {
            sizes
                .get(name)
                .copied()
                .ok_or(format!("missing {name} size"))
        };
        // getRightLeg (+0x29c) first, then getLeftLeg (+0x294), 00c03048.
        let leg_height = size("Right Leg").or_else(|_| size("Left Leg"))?[1];
        if colliders.len() != 2 || !floor_y.is_finite() {
            return Err("unexpected walking fixture geometry".into());
        }
        let balancing = Balancing::load(bundle)?;
        let gains = balancing.gains();
        let profile = HumanoidStateProfile::load(bundle)?;
        let mut fixture = Self {
            original: body.clone(),
            body,
            steps: 0,
            contacts: 0,
            impulse_applications: 0,
            max_penetration: 0.0,
            floor_y,
            wall_x: root.frame.position[0] + 4.0,
            kernel,
            running: RunningProfile::load(bundle)?,
            humanoid: HumanoidProperties::character(bundle)?,
            motion: HumanoidMotion::default(),
            geometry: BlockContactProfile::load(bundle)?,
            impulses: ImpulseProfile::load(bundle)?,
            colliders,
            states: BTreeMap::new(),
            spring_gain: float(0x017b05a0)?,
            spring_damping: float(0x017b05a4)?,
            spring_limit: float(0x017b05bc)?,
            state: HumanoidStateData {
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
            },
            balancing,
            inertia: assembly.inertia_in_body,
            turn_gain: float(0x016f7530)?,
            yaw_gain: float(0x0179fc24)?,
            yaw_limit: float(0x0177ed40)?,
            mass_parts: parts,
            original_inertia: assembly.inertia_in_body,
            animation,
            signal: RunningSignal::load(bundle)?,
            animation_ticks: 0,
            joint_callbacks: 0,
            limb_angles: [0.0; 4],
            original_schedule: schedule.clone(),
            schedule,
            last_frame: None,
            profile,
            torso_size: size("Torso")?,
            head_size: size("Head")?,
            leg_height,
            jump: false,
            jump_held: false,
            last_floor_normal: [0.0; 3],
            script_pose: Pose::Standing,
            transitions: Vec::new(),
            camera: None,
        };
        fixture.reset();
        Ok(fixture)
    }
    pub fn reset(&mut self) {
        self.body = self.original.clone();
        self.steps = 0;
        self.contacts = 0;
        self.impulse_applications = 0;
        self.max_penetration = 0.0;
        self.states.clear();
        self.motion = HumanoidMotion::default();
        self.balancing.reset();
        self.inertia = self.original_inertia;
        self.signal.speed = 0.0;
        self.animation_ticks = 0;
        self.joint_callbacks = 0;
        self.limb_angles = [0.0; 4];
        self.schedule = self.original_schedule.clone();
        self.last_frame = None;
        // The spawned character starts in Running with no pending events.
        self.state = self.new_state(StateType::Running, StateType::Running);
        self.jump = false;
        self.jump_held = false;
        self.last_floor_normal = [0.0; 3];
        self.script_pose = Pose::Standing;
        self.transitions.clear();
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
    /// humanoid.Torso render CFrame (the camera subject part).
    pub fn torso_frame(&self, rig: &R6Rig) -> Result<CoordinateFrame, String> {
        self.torso(rig)
    }
    /// PopperCam CastRay: segment query against the fixture world.
    pub fn camera_raycast(&self, from: [f32; 3], to: [f32; 3]) -> Option<[f32; 3]> {
        let v: [f32; 3] = std::array::from_fn(|a| to[a] - from[a]);
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if m == 0.0 {
            return None;
        }
        self.raycast(from, v.map(|x| x / m), m.min(999.0))
            .map(|h| h.0)
    }
    pub fn state_type(&self) -> StateType {
        self.state.kind
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
        }
    }
    fn torso(&self, rig: &R6Rig) -> Result<CoordinateFrame, String> {
        rig.pose(&self.body.frame)?
            .remove("Torso")
            .ok_or_else(|| "missing Torso".into())
    }
    fn point_velocity(&self, point: [f32; 3]) -> [f32; 3] {
        let arm = std::array::from_fn(|a| point[a] - self.body.center[a]);
        let spin = cross(self.body.angular_velocity, arm);
        std::array::from_fn(|a| self.body.velocity[a] + spin[a])
    }
    /// Explicit fixture world: floor plane (solid below) and wall half-space.
    fn raycast(&self, origin: [f32; 3], direction: [f32; 3], length: f32) -> Option<Hit> {
        let mut best: Option<(f32, Hit)> = None;
        if direction[1] < 0.0 && origin[1] > self.floor_y {
            let t = (origin[1] - self.floor_y) / -direction[1];
            if t <= length {
                best = Some((
                    t,
                    (
                        [
                            origin[0] + direction[0] * t,
                            self.floor_y,
                            origin[2] + direction[2] * t,
                        ],
                        [0.0, 1.0, 0.0],
                    ),
                ));
            }
        }
        if direction[0] > 0.0 && origin[0] < self.wall_x {
            let t = (self.wall_x - origin[0]) / direction[0];
            if t <= length && best.as_ref().is_none_or(|(b, _)| t < *b) {
                best = Some((
                    t,
                    (
                        [
                            self.wall_x,
                            origin[1] + direction[1] * t,
                            origin[2] + direction[2] * t,
                        ],
                        [-1.0, 0.0, 0.0],
                    ),
                ));
            }
        }
        best.map(|(_, hit)| hit)
    }
    fn wall(&self) -> CoordinateFrame {
        CoordinateFrame {
            position: [self.wall_x, self.floor_y, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
    /// Fixture stand-in for a touching kernel contact: the part generates
    /// a vertex/plane connector against the wall.
    fn touching(&self, frame: &CoordinateFrame, size: [f32; 3]) -> Result<bool, String> {
        Ok(!self
            .geometry
            .plane_vertices(frame, size, &self.wall(), 3)?
            .is_empty())
    }
    /// Humanoid::calcDesiredWalkVelocity 00bdc44c, non-camera AutoRotate branch:
    /// desired velocity and rotation (vtable +0xe0).
    fn desired_motion(&self, torso: &CoordinateFrame) -> ([f32; 3], [f32; 3]) {
        let desired = self
            .motion
            .move_direction
            .map(|v| v * self.humanoid.walk_speed);
        let yaw = if desired[0] != 0.0 || desired[2] != 0.0 {
            let target = f64::from(-desired[0]).atan2(f64::from(-desired[2])) as f32;
            delta_angle(target, current_yaw(torso)) as f32 * self.turn_gain
        } else {
            0.0
        };
        (desired, [0.0, yaw, 0.0])
    }
    /// Humanoid::onStepped 00bde4f2 -> 00c0072c with Stepped dt.
    fn humanoid_stepped(&mut self, rig: &R6Rig, input: [f32; 3], dt: f32) -> Result<(), String> {
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
        self.state_stepped(rig, dt)?;
        let next = {
            let torso = self.torso(rig)?;
            let profile = self.profile.clone();
            let kind = self.state.kind;
            profile.transition(kind, |event| self.condition(event, &torso, rig))?
        };
        if let Some(next) = next {
            self.enter(next, rig)?;
            self.state_stepped(rig, dt)?;
        }
        self.state_update(rig, dt)?;
        self.state_events(rig)
    }
    /// HumanoidState::onStepped 00bff424: findFloor 00bff784, then timers.
    fn state_stepped(&mut self, rig: &R6Rig, dt: f32) -> Result<(), String> {
        let had_floor = self.state.floor.floor.is_some();
        self.state.floor = FloorQuery::default();
        if self.profile.searches_floor(self.state.kind) {
            let torso = self.torso(rig)?;
            let vy = self.point_velocity(torso.position)[1];
            let query = humanoid_state::find_floor(
                &self.profile,
                &torso,
                self.torso_size,
                Some(self.leg_height),
                had_floor,
                vy,
                |o, d, l| self.raycast(o, d, l),
            );
            if let Some((_, normal)) = query.floor {
                self.state.no_floor = 0.0;
                self.last_floor_normal = normal;
            }
            self.state.floor = query;
        }
        self.state.timer -= dt;
        self.state.elapsed += dt;
        self.state.no_floor += dt;
        Ok(())
    }
    /// c0192c: HumanoidRootPart (non-colliding, never) or Head touching.
    fn root_or_head_touching(
        &mut self,
        torso: &CoordinateFrame,
        rig: &R6Rig,
    ) -> Result<bool, String> {
        let _ = torso;
        let head = rig
            .pose(&self.body.frame)?
            .remove("Head")
            .ok_or("missing Head")?;
        if self.touching(&head, self.head_size)? {
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
        rig: &R6Rig,
    ) -> Result<bool, String> {
        Ok(match event {
            0 => self.humanoid.health <= 0.0,
            // Head and HumanoidRootPart stay jointed in this fixture.
            1 => false,
            2 => {
                let normal = self
                    .state
                    .floor
                    .floor
                    .map_or(self.last_floor_normal, |f| f.1);
                let slope = dot(normal, [0.0, 1.0, 0.0]);
                let limit = f64::from(self.humanoid.max_slope_angle * std::f32::consts::PI / 180.0)
                    .cos() as f32;
                if slope <= limit
                    && matches!(
                        self.state.kind,
                        StateType::Running | StateType::RunningNoPhysics
                    )
                {
                    false
                } else {
                    self.jump && 0.0 < self.humanoid.jump_power
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
            15 => self.state.floor.floor.is_some() && self.point_velocity(torso.position)[1] <= 0.0,
            16 => self.root_or_head_touching(torso, rig)?,
            20 => self.state.jump_done,
            21 => self.state.timer <= 0.0,
            22 => {
                self.root_or_head_touching(torso, rig)?;
                // No BodyMover descendants in the character (00c01a4c).
                self.profile.running_idle_seconds < self.state.elapsed
            }
            _ => return Err(format!("Humanoid event {event} is not implemented")),
        })
    }
    fn enter(&mut self, next: StateType, rig: &R6Rig) -> Result<(), String> {
        let old = self.state.kind;
        self.transitions.push((self.steps, next));
        self.state = self.new_state(next, old);
        self.jump = false;
        let torso = self.torso(rig)?;
        let v = self.point_velocity(torso.position);
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
            let state = self.body.body_state();
            self.body.set_body_state(&state)?;
            self.states.clear();
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
            self.script_event(StateType::Running, Some(self.signal.speed));
        } else {
            self.script_event(StateType::Running, Some(speed));
        }
        Ok(())
    }
    /// humanoidAnimateLocal handlers: onJumping, onFreeFall, onRunning.
    fn script_event(&mut self, kind: StateType, speed: Option<f32>) {
        self.script_pose = match kind {
            StateType::Jumping => Pose::Jumping,
            StateType::Freefall | StateType::Landed => Pose::FreeFall,
            StateType::Running | StateType::RunningNoPhysics => {
                if speed.unwrap_or(0.0) > 0.0 {
                    Pose::Running
                } else {
                    Pose::Standing
                }
            }
        };
    }
    /// Slot +0x38: 00bfda14 events (old false, new true), then Running's event.
    fn state_events(&mut self, rig: &R6Rig) -> Result<(), String> {
        let torso = self.torso(rig)?;
        let v = self.point_velocity(torso.position);
        let speed = (v[2] * v[2] + v[0] * v[0]).sqrt();
        if self.state.previous != self.state.kind {
            for kind in [self.state.previous, self.state.kind] {
                // 00bfda98 cases 8/10 fire raw Running(speed) for either edge.
                self.script_event(kind, Some(speed));
            }
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
    fn state_update(&mut self, rig: &R6Rig, dt: f32) -> Result<(), String> {
        let torso = self.torso(rig)?;
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
                if let Some((_, normal)) = floor.floor {
                    (self.state.desired_velocity, self.state.desired_rotation) =
                        self.desired_motion(&torso);
                    if normal != [0.0, 1.0, 0.0] {
                        return Err("sloped floor projection is not implemented".into());
                    }
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
            StateType::RunningNoPhysics => self.kinematic_update(rig, &torso, dt)?,
            StateType::Jumping => {}
        }
        Ok(())
    }
    /// MovingNoPhysicsBase 00c07626: place the Torso assembly kinematically.
    fn kinematic_update(
        &mut self,
        rig: &R6Rig,
        torso: &CoordinateFrame,
        dt: f32,
    ) -> Result<(), String> {
        let (desired, rotation) = self.desired_motion(torso);
        let mut v = [desired[0], desired[1], desired[2]];
        let floor = self.state.floor.clone();
        if let (Some((_, normal)), Some(average)) = (floor.floor, floor.average) {
            if normal != [0.0, 1.0, 0.0] {
                return Err("sloped floor projection is not implemented".into());
            }
            let height = humanoid_state::support_height(
                average[1],
                self.torso_size[1],
                Some(self.leg_height),
            );
            if height - torso.position[1] != 0.0 {
                v[1] += (height - torso.position[1]) / dt;
            }
        }
        let current = self.point_velocity(torso.position);
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
            std::array::from_fn(|a| self.body.frame.position[a] - torso.position[a]);
        let arm_turned = turn([[arm[0], 0.0, 0.0], [arm[1], 0.0, 0.0], [arm[2], 0.0, 0.0]]);
        let root = CoordinateFrame {
            position: std::array::from_fn(|a| placed.position[a] + arm_turned[a][0]),
            rotation: turn(self.body.frame.rotation),
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
        self.body.set_body_state(&state)?;
        let _ = rig;
        Ok(())
    }
    /// Play the bundled procedural branch while integrating walking. This is an
    /// explicit offline choice: native R6 default keyframe payloads are absent.
    pub fn step_animated(&mut self, rig: &mut R6Rig, input: [f32; 3]) -> Result<(), String> {
        // Each call is one native world step. A new ideal 60 Hz frame runs the
        // Heartbeat and WaitingScripts jobs, then PhysicsJob: gameStepped
        // (Humanoid::onStepped), World::uiStep (Motor6D stepUi once,
        // 00ba8c68/00b39318) and its world steps (00ba9892).
        while self.schedule.world_steps_remaining == 0 {
            let frame = self.schedule.begin_frame()?;
            if let Some(time) = frame.resumed_time {
                // humanoidAnimateLocal: `local _, time = wait(0.1); move(time)`.
                self.animation.apply(rig, self.script_pose, time)?;
                self.animation_ticks += 1;
            }
            self.last_frame = Some(frame);
            if frame.world_steps == 0 {
                continue; // physicsStep returns before World::step.
            }
            self.humanoid_stepped(rig, input, self.stepped_dt(&frame))?;
            let before: Vec<_> = rig.joints.iter().map(|j| j.current_angle).collect();
            for joint in &mut rig.joints {
                joint.step()?;
            }
            self.joint_callbacks += 1;
            self.limb_angles = std::array::from_fn(|a| rig.joints[a + 1].current_angle);
            if rig
                .joints
                .iter()
                .zip(before)
                .any(|(j, old)| j.current_angle != old)
            {
                let frames = rig.pose(&self.body.frame)?;
                let mut parts = self.mass_parts.clone();
                for (name, part) in &mut parts {
                    part.frame = frames
                        .get(name)
                        .ok_or("missing animated mass part")?
                        .clone();
                }
                let assembly = AssemblyProperties::from_rig(&parts, rig)?;
                self.body.refresh_assembly(&assembly, &self.kernel)?;
                self.inertia = assembly.inertia_in_body;
            }
        }
        self.world_step(rig)?;
        self.schedule.finish_world_step()
    }
    /// Humanoid::onStepped receives (float) of Stepped's double dt, which is
    /// (double)((float)steps * (float)worldStepDt).
    fn stepped_dt(&self, frame: &FrameEvents) -> f32 {
        frame.world_steps as f32 * self.kernel.physics.free_fall_dt
    }
    pub fn animation_report(&self) -> String {
        format!(
            "walking_animation profile=bundled_procedural_no_tools pose={:?} state={:?} event_speed={} script_resumes={} joint_step_ui={} limb_angles={:?} local_inertia={:?} schedule={:?} heartbeat_time={} game_time={} schedule_source=native_jobs_ideal_clock native_default_keyframes=unavailable\n",
            self.script_pose,
            self.state.kind,
            self.state.running_speed,
            self.animation_ticks,
            self.joint_callbacks,
            self.limb_angles,
            [self.inertia[0][0], self.inertia[1][1], self.inertia[2][2]],
            self.last_frame,
            self.schedule.heartbeat_elapsed,
            self.schedule.game_time
        )
    }
    /// One world step with the rest-pose rig: same Humanoid dispatch, no
    /// procedural script or motor stepping.
    pub fn step(&mut self, rig: &R6Rig, input: [f32; 3]) -> Result<(), String> {
        while self.schedule.world_steps_remaining == 0 {
            let frame = self.schedule.begin_frame()?;
            self.last_frame = Some(frame);
            if frame.world_steps == 0 {
                continue;
            }
            self.humanoid_stepped(rig, input, self.stepped_dt(&frame))?;
        }
        self.world_step(rig)?;
        self.schedule.finish_world_step()
    }
    /// doWorldStep: awake contact impulses, then the kernel substeps with the
    /// current state's force callback (vtable +0xc).
    fn world_step(&mut self, rig: &R6Rig) -> Result<(), String> {
        let wall = self.wall();
        let fixed = ContactBodyState {
            center: wall.position,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        // Fixture materials. Freefall sets Head/Torso Friction to 0 (00bd3ab4);
        // the fixture then uses zero contact friction (combination not recovered).
        let parameters = ContactParameters {
            compression: 7500.0,
            decompression: 3750.0,
            friction: if self.state.kind == StateType::Freefall {
                0.0
            } else {
                0.3
            },
        };
        let frames = rig.pose(&self.body.frame)?;
        if self.state.kind == StateType::RunningNoPhysics {
            // Torso assembly is outside the kernel; only touch detection runs.
            for (name, size) in &self.colliders {
                for (_, g) in self.geometry.plane_vertices(
                    frames.get(name).ok_or("missing wall collider frame")?,
                    *size,
                    &wall,
                    3,
                )? {
                    self.max_penetration = self.max_penetration.max(-g.distance);
                }
            }
            self.states.clear();
            self.contacts = 0;
            self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
            return Ok(());
        }
        let mut active = Vec::new();
        for (name, size) in &self.colliders {
            for (vertex, g) in self.geometry.plane_vertices(
                frames.get(name).ok_or("missing wall collider frame")?,
                *size,
                &wall,
                3,
            )? {
                let key = (name.clone(), vertex);
                self.states
                    .entry(key.clone())
                    .or_insert_with(|| ContactState {
                        spring: ContactSpring::default(),
                        impulse: AwakeContact::default(),
                    })
                    .impulse
                    .invalidate();
                active.push((key, g));
            }
        }
        self.states
            .retain(|key, _| active.iter().any(|(k, _)| k == key));
        self.contacts = active.len();
        // Explicit bounded awake fixture scheduling; full native adaptive/sleep
        // scheduling and +0xf0 reference-impulse accumulation remain unported.
        for _ in 0..60 {
            let mut applied = false;
            for (key, g) in &active {
                let input = ImpulseInput {
                    geometry: g.clone(),
                    parameters,
                    reference_impulse_a: [0.0; 3],
                    reference_impulse_b: [0.0; 3],
                    friction_enabled: true,
                };
                if self
                    .states
                    .get_mut(key)
                    .ok_or("missing walking contact")?
                    .impulse
                    .solve(
                        &input,
                        &mut self.body,
                        None,
                        &fixed,
                        &self.kernel,
                        &self.impulses,
                    )?
                {
                    self.impulse_applications += 1;
                    applied = true;
                }
            }
            if !applied {
                break;
            }
        }
        for _ in 0..self.kernel.physics.kernel_substeps {
            let frames = rig.pose(&self.body.frame)?;
            let torso = frames.get("Torso").ok_or("missing state torso")?.clone();
            match self.state.kind {
                StateType::Running | StateType::Landed => self.running_force(&torso)?,
                StateType::Freefall => self.freefall_force(&torso)?,
                StateType::Jumping => self.jump_force(&torso)?,
                StateType::RunningNoPhysics => unreachable!(),
            }
            for (name, size) in &self.colliders {
                for (vertex, g) in self.geometry.plane_vertices(
                    frames.get(name).ok_or("missing spring collider")?,
                    *size,
                    &wall,
                    3,
                )? {
                    self.max_penetration = self.max_penetration.max(-g.distance);
                    if let Some(state) = self.states.get_mut(&(name.clone(), vertex)) {
                        let force = state.spring.force(
                            &self.kernel,
                            parameters,
                            g.distance,
                            g.normal,
                            g.point,
                            &self.body.contact_state(),
                            &fixed,
                        )?;
                        self.body.apply_force(force.map(|v| -v), g.point)?;
                    }
                }
            }
            self.body.step(&self.kernel)?;
        }
        self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
        Ok(())
    }
    /// RunningBase 00c09632 for Running and Landed (anchored floor branch).
    fn running_force(&mut self, torso: &CoordinateFrame) -> Result<(), String> {
        self.state
            .balancing
            .apply(&mut self.body, torso, self.inertia)?;
        // 00c09632: anchored floor angular velocity is zero. Apply torque
        // about world Y; keep the native inertia and force limit.
        let yaw = ((self.yaw_gain * self.inertia[1][1])
            * (self.state.desired_rotation[1] - self.body.angular_velocity[1]))
            .clamp(-self.yaw_limit, self.yaw_limit);
        self.body.apply_torque([0.0, yaw, 0.0])?;
        let published = self.body.body_state();
        let mass = 1.0 / self.body.inverse_mass();
        let desired = self.state.desired_velocity;
        let horizontal = self.running.horizontal_force(&RunningInput {
            velocity: [published.velocity[0], published.velocity[2]],
            desired_velocity: [desired[0], desired[2]],
            floor_velocity: [0.0; 2],
            accumulated_force: [self.body.force[0], self.body.force[2]],
            root_mass: mass,
            effective_mass: mass,
            floor_present: self.state.floor.floor.is_some(),
            floor_friction: 1.0,
            solver: Solver::Legacy,
        })?;
        self.body
            .apply_force([horizontal[0], 0.0, horizontal[1]], self.body.center)?;
        // Vertical support only with a floor: target from the floor rays,
        // torso height and assembly-root Y velocity.
        if self.state.support_height.is_finite() {
            let acceleration = (self.state.support_height - torso.position[1]) * self.spring_gain
                - published.velocity[1] * self.spring_damping;
            let existing = self.body.force[1] / mass;
            if acceleration > 0.0 && existing < acceleration {
                let force =
                    ((acceleration - existing) * mass).clamp(-self.spring_limit, self.spring_limit);
                self.body.apply_force([0.0, force, 0.0], self.body.center)?;
            }
        }
        Ok(())
    }
    /// Freefall 00bd4194: balancing, yaw torque and limited air control.
    fn freefall_force(&mut self, torso: &CoordinateFrame) -> Result<(), String> {
        self.state
            .balancing
            .apply(&mut self.body, torso, self.inertia)?;
        let iy = self.inertia[1][1];
        let limit = iy * self.profile.freefall_torque_limit;
        let value = (self.state.desired_rotation[1] - self.body.angular_velocity[1])
            * (iy * self.profile.freefall_yaw_gain)
            - self.body.torque[1];
        let torque = if -limit < value {
            if limit <= value { limit } else { value }
        } else {
            -limit
        };
        self.body.apply_torque([0.0, torque, 0.0])?;
        let velocity = self.body.body_state().velocity;
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
        let mass = 1.0 / self.body.inverse_mass();
        self.body
            .apply_force([a[0] * mass, 0.0, a[1] * mass], self.body.center)
    }
    /// Jumping 00c05e0e: no balancing (Flying 00bd3568 is empty).
    fn jump_force(&mut self, torso: &CoordinateFrame) -> Result<(), String> {
        if self.state.jump_done {
            return Ok(());
        }
        let v = self.body.body_state().velocity;
        let d = self.state.jump_direction;
        let along = (v[1] * d[1] + v[0] * d[0]) + v[2] * d[2];
        let acceleration = (self.humanoid.jump_power - along) * self.profile.jump_gain;
        if acceleration <= 0.0 || self.ceiling(torso) {
            self.state.jump_done = true;
            return Ok(());
        }
        let mass = 1.0 / self.body.inverse_mass();
        let force = acceleration * mass;
        // Floor branch: the anchored floor mass is not smaller than the root's,
        // and the existing force includes gravity, so the push is the same.
        if self.state.floor.floor.is_none() || self.body.force[1] < force {
            self.body
                .apply_force(d.map(|v| v * force), self.body.center)?;
        }
        Ok(())
    }
    /// 00c0624e: upward rays from the Torso; any hit ends the jump push.
    fn ceiling(&self, torso: &CoordinateFrame) -> bool {
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
            self.raycast(torso.transform_point(local), up, length)
                .is_some()
        })
    }
    pub fn report(&self) -> String {
        format!(
            "walking_state step={} state={:?} root_position={:?} velocity={:?} contacts={} impulses={} max_wall_penetration={} floor_y={} wall_x={} upright_y={} angular_velocity={:?} support_height={} fixture=flat_floor_rays_and_infinite_wall humanoid_dispatch=native_subset mass_tensor={} keyframe_playback=not_implemented native_differential=not_tested\n",
            self.steps,
            self.state.kind,
            self.body.frame.position,
            self.body.velocity,
            self.contacts,
            self.impulse_applications,
            self.max_penetration,
            self.floor_y,
            self.wall_x,
            self.body.frame.rotation[1][1],
            self.body.angular_velocity,
            self.state.support_height,
            if self.joint_callbacks > 0 {
                "animated_cofm"
            } else {
                "rest_pose"
            }
        )
    }
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
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let rig = R6Rig::character(bundle, &scene)?;
    let mut fixture = WalkingFixture::new(bundle, &scene, &rig)?;
    let start = fixture.body.frame.position;
    let mut report = String::new();
    for tick in 0..240 {
        fixture.step(&rig, [0.0, 0.0, -1.0])?;
        if tick == 59 || tick == 239 {
            report.push_str(&fixture.report());
        }
    }
    let distance = start[2] - fixture.body.frame.position[2];
    if !(15.0..16.1).contains(&distance) || (fixture.body.frame.position[1] - start[1]).abs() > 0.02
    {
        return Err(format!(
            "walking fixture speed/support failed distance={distance}"
        ));
    }
    for _ in 0..120 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if fixture.body.velocity[2].abs() > 0.001 {
        return Err("walking release failed to stop".into());
    }
    fixture.reset();
    for tick in 0..480 {
        fixture.step(&rig, [1.0, 0.0, 0.0])?;
        if tick == 119 || tick == 479 {
            report.push_str(&fixture.report());
        }
    }
    if fixture.impulse_applications == 0
        || fixture.max_penetration > 0.5
        || fixture.body.frame.position[0] > fixture.wall_x
    {
        return Err("walking wall response failed".into());
    }
    let mut minimum_up = 1.0f32;
    let mut maximum_spin = 0.0f32;
    // Long enough to catch the accumulated tipping reported during play.
    for _ in 0..7200 {
        fixture.step(&rig, [1.0, 0.0, 0.0])?;
        minimum_up = minimum_up.min(fixture.body.frame.rotation[1][1]);
        maximum_spin = maximum_spin.max(fixture.body.angular_velocity[1].abs());
        if minimum_up < 0.99 || fixture.body.frame.position[0] > fixture.wall_x {
            return Err(format!(
                "sustained wall balance failed: {}",
                fixture.report()
            ));
        }
    }
    report.push_str(&fixture.report());
    let wall_impulses = fixture.impulse_applications;
    // Release must damp angular motion as well as translation.
    for _ in 0..480 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if fixture.body.angular_velocity.iter().any(|v| v.abs() > 0.01) {
        return Err(format!("wall release still rotating: {}", fixture.report()));
    }
    for direction in [
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [-1.0, 0.0, -1.0],
    ] {
        fixture.reset();
        for _ in 0..1920 {
            fixture.step(&rig, direction)?;
            minimum_up = minimum_up.min(fixture.body.frame.rotation[1][1]);
        }
        let torso = rig
            .pose(&fixture.body.frame)?
            .remove("Torso")
            .ok_or("missing audit torso")?;
        let alignment = (-torso.rotation[0][2] * direction[0]
            - torso.rotation[2][2] * direction[2])
            / (direction[0] * direction[0] + direction[2] * direction[2]).sqrt();
        let speed = (fixture.body.velocity[0].powi(2) + fixture.body.velocity[2].powi(2)).sqrt();
        if fixture.state_type() != StateType::RunningNoPhysics
            || (speed - fixture.humanoid.walk_speed).abs() > 0.001
            || (torso.position[1] - 18.0).abs() > 0.0001
        {
            return Err(format!(
                "open-ground run did not settle in RunningNoPhysics speed={speed}: {}",
                fixture.report()
            ));
        }
        if alignment < 0.999 || minimum_up < 0.99 || fixture.body.angular_velocity[1].abs() > 0.01 {
            return Err(format!(
                "directional balance/yaw failed alignment={alignment}: {}",
                fixture.report()
            ));
        }
    }
    fixture.reset();
    let point = [
        fixture.body.center[0],
        fixture.body.center[1] + 1.0,
        fixture.body.center[2],
    ];
    fixture
        .body
        .contact_impulse([0.0, 0.0, 40.0], point, &fixture.kernel)?;
    let disturbed_speed = fixture.body.angular_velocity[0].abs();
    for _ in 0..480 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if disturbed_speed < 1.0
        || fixture.body.frame.rotation[1][1] < 0.9999
        || fixture.body.angular_velocity.iter().any(|v| v.abs() > 0.01)
    {
        return Err("balance failed to recover from angular impulse".into());
    }
    writeln!(report,"walking_balance_pass sustained_wall_seconds=30 directions=4 min_upright_y={minimum_up} max_wall_yaw_speed={maximum_spin} angular_impulse_recovered=true").unwrap();
    fixture.reset();
    let mut moving_rig = rig.clone();
    let mut low = [f32::INFINITY; 4];
    let mut high = [f32::NEG_INFINITY; 4];
    let mut pivot_error = 0.0f32;
    let mut animated_minimum_up = 1.0f32;
    for _ in 0..480 {
        fixture.step_animated(&mut moving_rig, [0.0, 0.0, -1.0])?;
        let frames = moving_rig.pose(&fixture.body.frame)?;
        for (a, joint) in moving_rig.joints[1..5].iter().enumerate() {
            low[a] = low[a].min(joint.current_angle);
            high[a] = high[a].max(joint.current_angle);
        }
        for joint in &moving_rig.joints {
            let parent = frames[&joint.part0].transform_point(joint.c0.position);
            let child = frames[&joint.part1].transform_point(joint.c1.position);
            for a in 0..3 {
                pivot_error = pivot_error.max((parent[a] - child[a]).abs());
            }
        }
    }
    let swing: [f32; 4] = std::array::from_fn(|a| high[a] - low[a]);
    if swing.iter().any(|s| *s < 1.0)
        || pivot_error > 0.0001
        || fixture.inertia == fixture.original_inertia
    {
        return Err(format!(
            "walking animation did not propagate swing={swing:?} pivot_error={pivot_error}"
        ));
    }
    report.push_str(&fixture.animation_report());
    for _ in 0..480 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    if !matches!(fixture.script_pose, Pose::Standing) {
        return Err("walking animation did not become Standing on release".into());
    }
    report.push_str(&fixture.animation_report());
    fixture.reset();
    moving_rig = rig.clone();
    for _ in 0..7680 {
        fixture.step_animated(&mut moving_rig, [1.0, 0.0, 0.0])?;
        animated_minimum_up = animated_minimum_up.min(fixture.body.frame.rotation[1][1]);
        if animated_minimum_up < 0.99
            || fixture.body.frame.position[0] > fixture.wall_x
            || fixture.max_penetration > 0.5
        {
            return Err(format!(
                "animated wall balance failed: {}",
                fixture.report()
            ));
        }
    }
    report.push_str(&fixture.report());
    report.push_str(&fixture.animation_report());
    for direction in [
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [-1.0, 0.0, -1.0],
    ] {
        for _ in 0..480 {
            fixture.step_animated(&mut moving_rig, direction)?;
            if fixture.body.frame.rotation[1][1] < 0.99 {
                return Err("animated direction change tipped the torso".into());
            }
        }
    }
    for _ in 0..480 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    if !matches!(fixture.script_pose, Pose::Standing)
        || fixture.body.angular_velocity.iter().any(|v| v.abs() > 0.01)
    {
        return Err("animated release failed".into());
    }
    fixture.reset();
    if fixture.body.frame != fixture.original.frame
        || fixture.inertia != fixture.original_inertia
        || fixture.joint_callbacks != 0
    {
        return Err("walking reset failed".into());
    }
    // Jump from rest: hold Space for one 60 Hz frame (MasterControl sets Jump).
    fixture.reset();
    moving_rig = rig.clone();
    let rest_y = fixture.body.frame.position[1];
    for _ in 0..120 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    fixture.set_jump(true);
    let mut apex = rest_y;
    let mut poses = Vec::new();
    for tick in 0..720 {
        if tick == 4 {
            fixture.set_jump(false);
        }
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
        apex = apex.max(fixture.body.frame.position[1]);
        if poses.last() != Some(&format!("{:?}", fixture.script_pose)) {
            poses.push(format!("{:?}", fixture.script_pose));
        }
    }
    let jump_states: Vec<_> = fixture.transitions.iter().map(|(_, s)| *s).collect();
    writeln!(
        report,
        "walking_jump transitions={:?} apex_rise={} poses={poses:?} final_y={}",
        fixture.transitions,
        apex - rest_y,
        fixture.body.frame.position[1]
    )
    .unwrap();
    if jump_states.get(..4)
        != Some(
            &[
                StateType::Jumping,
                StateType::Freefall,
                StateType::Landed,
                StateType::Running,
            ][..],
        )
        || !(4.0..9.0).contains(&(apex - rest_y))
        || (fixture.body.frame.position[1] - rest_y).abs() > 0.01
        || !poses.iter().any(|p| p == "Jumping")
        || !poses.iter().any(|p| p == "FreeFall")
    {
        return Err(format!("native jump sequence failed: {}", fixture.report()));
    }
    let jump_rise = apex - rest_y;
    // Drop 10 studs: Running loses its floor, Freefall, Landed, Running.
    fixture.reset();
    let mut lifted = fixture.body.body_state();
    lifted.frame.position[1] += 10.0;
    fixture.body.set_body_state(&lifted)?;
    for _ in 0..960 {
        fixture.step(&rig, [0.0; 3])?;
    }
    let drop_states: Vec<_> = fixture.transitions.iter().map(|(_, s)| *s).collect();
    writeln!(
        report,
        "walking_drop transitions={:?} final_y={} state={:?}",
        fixture.transitions,
        fixture.body.frame.position[1],
        fixture.state_type()
    )
    .unwrap();
    if drop_states.get(..3)
        != Some(&[StateType::Freefall, StateType::Landed, StateType::Running][..])
        || (fixture.body.frame.position[1] - rest_y).abs() > 0.01
    {
        return Err(format!("native fall/landing failed: {}", fixture.report()));
    }
    writeln!(report,"walking_state_pass running_no_physics_speed=16 jump_rise={jump_rise} drop_landed=true states=Running,RunningNoPhysics,Jumping,Freefall,Landed").unwrap();
    writeln!(report,"walking_animation_pass limbs=4 swing_radians={swing:?} pivot_error={pivot_error} min_animated_upright_y={animated_minimum_up} wall_seconds=32 release_pose=Standing animated_cofm=true reset_identical=true default_keyframes=unavailable").unwrap();
    writeln!(report,"walking_pass walk_speed={} distance_one_second={distance} input_release_stopped=true wall_impulses={} walking_animation=bundled_procedural default_keyframes=unavailable native_differential=not_tested",fixture.humanoid.walk_speed,wall_impulses).unwrap();
    Ok(report)
}
