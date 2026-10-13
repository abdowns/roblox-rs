//! Explicit flat-floor / infinite-wall walking fixture, not native world dispatch.
use crate::{
    animation::{Pose, ProceduralAnimation, RunningSignal},
    assembly::{AssemblyProperties, MassPart, MassProfile},
    balancing::Balancing,
    bundle::Bundle,
    collision::ContactBodyState,
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile},
    legacy_contact::{AwakeContact, BlockContactProfile, ImpulseInput, ImpulseProfile},
    locomotion::{RunningInput, RunningProfile, Solver},
    native_data::NativeImage,
    physics,
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
    target_torso_y: f32,
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
        for instance in &scene.instances {
            let Some(part) = &instance.part else { continue };
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
        let target_torso_y = parts
            .get("Torso")
            .ok_or("missing walking torso")?
            .frame
            .position[1];
        if colliders.len() != 2 || !floor_y.is_finite() {
            return Err("unexpected walking fixture geometry".into());
        }
        Ok(Self {
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
            target_torso_y,
            spring_gain: float(0x017b05a0)?,
            spring_damping: float(0x017b05a4)?,
            spring_limit: float(0x017b05bc)?,
            balancing: Balancing::load(bundle)?,
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
        })
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
    }
    /// Play the bundled procedural branch while integrating walking. This is an
    /// explicit offline choice: native R6 default keyframe payloads are absent.
    pub fn step_animated(&mut self, rig: &mut R6Rig, input: [f32; 3]) -> Result<(), String> {
        // Each call is one native world step. A new ideal 60 Hz frame runs the
        // Heartbeat and WaitingScripts jobs, then PhysicsJob: gameStepped,
        // World::uiStep (Motor6D stepUi once, 00ba8c68/00b39318) and its world
        // steps (00ba9892). Job order within a frame is an explicit choice.
        while self.schedule.world_steps_remaining == 0 {
            let frame = self.schedule.begin_frame()?;
            if let Some(time) = frame.resumed_time {
                // humanoidAnimateLocal: `local _, time = wait(0.1); move(time)`.
                self.animation.apply(
                    rig,
                    if self.signal.speed > 0.0 {
                        Pose::Running
                    } else {
                        Pose::Standing
                    },
                    time,
                )?;
                self.animation_ticks += 1;
            }
            self.last_frame = Some(frame);
            if frame.world_steps == 0 {
                continue; // physicsStep returns before World::step.
            }
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
        let velocity = self.body.body_state().velocity;
        self.signal
            .observe((velocity[2] * velocity[2] + velocity[0] * velocity[0]).sqrt())?;
        self.step(rig, input)?;
        self.schedule.finish_world_step()
    }
    pub fn animation_report(&self) -> String {
        format!(
            "walking_animation profile=bundled_procedural_no_tools pose={} event_speed={} script_resumes={} joint_step_ui={} limb_angles={:?} local_inertia={:?} schedule={:?} heartbeat_time={} game_time={} schedule_source=native_jobs_ideal_clock native_default_keyframes=unavailable\n",
            if self.signal.speed > 0.0 {
                "Running"
            } else {
                "Standing"
            },
            self.signal.speed,
            self.animation_ticks,
            self.joint_callbacks,
            self.limb_angles,
            [self.inertia[0][0], self.inertia[1][1], self.inertia[2][2]],
            self.last_frame,
            self.schedule.heartbeat_elapsed,
            self.schedule.game_time
        )
    }
    pub fn step(&mut self, rig: &R6Rig, input: [f32; 3]) -> Result<(), String> {
        self.motion.move_input(
            input,
            &MoveContext {
                attached_to_data_model: true,
                relative_to_camera: false,
                camera: None,
                swimming: false,
            },
        )?;
        let desired = self
            .motion
            .move_direction
            .map(|v| v * self.humanoid.walk_speed);
        // Humanoid::calcDesiredWalkVelocity 00bdc44c, non-camera AutoRotate
        // branch; RunningBase 00c0a498 publishes this at the outer callback.
        let torso = rig
            .pose(&self.body.frame)?
            .remove("Torso")
            .ok_or("missing yaw torso")?;
        let desired_yaw_rate = if desired[0] != 0.0 || desired[2] != 0.0 {
            let target = f64::from(-desired[0]).atan2(f64::from(-desired[2])) as f32;
            let current =
                f64::from(torso.rotation[0][2]).atan2(f64::from(torso.rotation[2][2])) as f32;
            let mut delta = f64::from(target - current);
            if !(-std::f64::consts::PI..std::f64::consts::PI).contains(&delta) {
                delta -= ((delta + std::f64::consts::PI) / std::f64::consts::TAU).floor()
                    * std::f64::consts::TAU;
            }
            delta as f32 * self.turn_gain
        } else {
            0.0
        };
        let wall = CoordinateFrame {
            position: [self.wall_x, self.floor_y, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let fixed = ContactBodyState {
            center: wall.position,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        let parameters = ContactParameters {
            compression: 7500.0,
            decompression: 3750.0,
            friction: 0.3,
        }; // Explicit fixture, not native material defaults.
        let frames = rig.pose(&self.body.frame)?;
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
            self.balancing.apply(
                &mut self.body,
                frames.get("Torso").ok_or("missing balancing torso")?,
                self.inertia,
            )?;
            // 00c09632: anchored floor angular velocity is zero. Apply torque
            // about world Y; keep the native inertia and force limit.
            let yaw = ((self.yaw_gain * self.inertia[1][1])
                * (desired_yaw_rate - self.body.angular_velocity[1]))
                .clamp(-self.yaw_limit, self.yaw_limit);
            self.body.apply_torque([0.0, yaw, 0.0])?;
            let published = self.body.body_state();
            let mass = 1.0 / self.body.inverse_mass();
            let horizontal = self.running.horizontal_force(&RunningInput {
                velocity: [published.velocity[0], published.velocity[2]],
                desired_velocity: [desired[0], desired[2]],
                floor_velocity: [0.0; 2],
                accumulated_force: [self.body.force[0], self.body.force[2]],
                root_mass: mass,
                effective_mass: mass,
                floor_present: true,
                floor_friction: 1.0,
                solver: Solver::Legacy,
            })?;
            self.body
                .apply_force([horizontal[0], 0.0, horizontal[1]], self.body.center)?;
            // RunningBase legacy stationary anchored-floor vertical branch.
            // Native spring uses torso height and assembly-root Y velocity.
            let frames = rig.pose(&self.body.frame)?;
            let torso_y = frames.get("Torso").ok_or("missing support torso")?.position[1];
            let acceleration = (self.target_torso_y - torso_y) * self.spring_gain
                - published.velocity[1] * self.spring_damping;
            let existing = self.body.force[1] / mass;
            if acceleration > 0.0 && existing < acceleration {
                let force =
                    ((acceleration - existing) * mass).clamp(-self.spring_limit, self.spring_limit);
                self.body.apply_force([0.0, force, 0.0], self.body.center)?;
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
    pub fn report(&self) -> String {
        format!(
            "walking_state step={} root_position={:?} velocity={:?} contacts={} impulses={} max_wall_penetration={} floor_y={} wall_x={} upright_y={} angular_velocity={:?} fixture=flat_support_and_infinite_wall balancing=native_legacy mass_tensor={} keyframe_playback=not_implemented native_differential=not_tested\n",
            self.steps,
            self.body.frame.position,
            self.body.velocity,
            self.contacts,
            self.impulse_applications,
            self.max_penetration,
            self.floor_y,
            self.wall_x,
            self.body.frame.rotation[1][1],
            self.body.angular_velocity,
            if self.joint_callbacks > 0 {
                "animated_cofm"
            } else {
                "rest_pose"
            }
        )
    }
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
    if fixture.signal.speed != 0.0 {
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
    if fixture.signal.speed != 0.0 || fixture.body.angular_velocity.iter().any(|v| v.abs() > 0.01) {
        return Err("animated release failed".into());
    }
    fixture.reset();
    if fixture.body.frame != fixture.original.frame
        || fixture.inertia != fixture.original_inertia
        || fixture.joint_callbacks != 0
    {
        return Err("walking reset failed".into());
    }
    writeln!(report,"walking_animation_pass limbs=4 swing_radians={swing:?} pivot_error={pivot_error} min_animated_upright_y={animated_minimum_up} wall_seconds=32 release_pose=Standing animated_cofm=true reset_identical=true default_keyframes=unavailable").unwrap();
    writeln!(report,"walking_pass walk_speed={} distance_one_second={distance} input_release_stopped=true wall_impulses={} walking_animation=bundled_procedural default_keyframes=unavailable native_differential=not_tested",fixture.humanoid.walk_speed,wall_impulses).unwrap();
    Ok(report)
}
