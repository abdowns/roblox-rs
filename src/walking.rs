//! Explicit flat-floor / infinite-wall walking fixture: the game loop that
//! runs the native jobs (Heartbeat, WaitingScripts, PhysicsJob) over a World
//! with one character, its Humanoid and the bundled animation script. The
//! world contents (floor plane, wall half-space, materials, awake scheduling)
//! are a fixture, not native Workspace contents.
use crate::{
    animation::{Pose, ProceduralAnimation, ProceduralScript},
    bundle::Bundle,
    humanoid::Humanoid,
    humanoid_state::StateType,
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
    scheduler::{FrameEvents, NativeSchedule, SchedulerProfile},
    world::World,
};
use std::fmt::Write;

pub struct WalkingFixture {
    pub world: World,
    pub humanoid: Humanoid,
    animation: ProceduralAnimation,
    script: ProceduralScript,
    animation_ticks: u32,
    joint_callbacks: u32,
    limb_angles: [f32; 4],
    schedule: NativeSchedule,
    original_schedule: NativeSchedule,
    last_frame: Option<FrameEvents>,
}
impl WalkingFixture {
    pub fn new(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        let animation = ProceduralAnimation::load(bundle)?;
        let schedule =
            NativeSchedule::new(&SchedulerProfile::load(bundle)?, animation.wait_seconds)?;
        let mut fixture = Self {
            world: World::new(bundle, scene, rig)?,
            humanoid: Humanoid::new(bundle, scene)?,
            animation,
            script: ProceduralScript::default(),
            animation_ticks: 0,
            joint_callbacks: 0,
            limb_angles: [0.0; 4],
            original_schedule: schedule.clone(),
            schedule,
            last_frame: None,
        };
        fixture.reset();
        Ok(fixture)
    }
    pub fn reset(&mut self) {
        self.world.reset();
        self.humanoid.reset();
        self.script = ProceduralScript::default();
        self.animation_ticks = 0;
        self.joint_callbacks = 0;
        self.limb_angles = [0.0; 4];
        self.schedule = self.original_schedule.clone();
        self.last_frame = None;
    }
    /// ControlScript's jump key (Humanoid.Jump while held).
    pub fn set_jump(&mut self, held: bool) {
        self.humanoid.set_jump(held);
    }
    /// MasterControl calls Player:Move(moveValue, true): input is camera-relative.
    pub fn set_camera(&mut self, camera: Option<CoordinateFrame>) {
        self.humanoid.set_camera(camera);
    }
    /// humanoid.Torso render CFrame (the camera subject part).
    pub fn torso_frame(&self, rig: &R6Rig) -> Result<CoordinateFrame, String> {
        rig.pose(&self.world.body.frame)?
            .remove("Torso")
            .ok_or_else(|| "missing Torso".into())
    }
    /// PopperCam CastRay: segment query against the fixture world.
    pub fn camera_raycast(&self, from: [f32; 3], to: [f32; 3]) -> Option<[f32; 3]> {
        let v: [f32; 3] = std::array::from_fn(|a| to[a] - from[a]);
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if m == 0.0 {
            return None;
        }
        self.world
            .statics
            .raycast(from, v.map(|x| x / m), m.min(999.0))
            .map(|h| h.0)
    }
    pub fn state_type(&self) -> StateType {
        self.humanoid.state_type()
    }
    /// Humanoid::onStepped, then deliver its signals to the script handlers.
    fn humanoid_stepped(&mut self, rig: &R6Rig, input: [f32; 3], dt: f32) -> Result<(), String> {
        self.humanoid.on_stepped(&mut self.world, rig, input, dt)?;
        for event in self.humanoid.take_events() {
            self.script.on_event(event);
        }
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
                self.animation.apply(rig, self.script.pose, time)?;
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
                self.world.refresh_assembly(rig)?;
            }
        }
        self.world_step(rig)?;
        self.schedule.finish_world_step()
    }
    /// Humanoid::onStepped receives (float) of Stepped's double dt, which is
    /// (double)((float)steps * (float)worldStepDt).
    fn stepped_dt(&self, frame: &FrameEvents) -> f32 {
        frame.world_steps as f32 * self.world.kernel.physics.free_fall_dt
    }
    pub fn animation_report(&self) -> String {
        let inertia = self.world.inertia;
        format!(
            "walking_animation profile=bundled_procedural_no_tools pose={:?} state={:?} event_speed={} script_resumes={} joint_step_ui={} limb_angles={:?} local_inertia={:?} schedule={:?} heartbeat_time={} game_time={} schedule_source=native_jobs_ideal_clock native_default_keyframes=unavailable\n",
            self.script.pose,
            self.humanoid.state_type(),
            self.humanoid.running_speed(),
            self.animation_ticks,
            self.joint_callbacks,
            self.limb_angles,
            [inertia[0][0], inertia[1][1], inertia[2][2]],
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
    /// World::step with the current Humanoid state's force callback.
    fn world_step(&mut self, rig: &R6Rig) -> Result<(), String> {
        if self.humanoid.is_kinematic() {
            return self.world.step_kinematic(rig);
        }
        let parameters = self.humanoid.contact_parameters();
        let humanoid = &mut self.humanoid;
        self.world
            .step(rig, parameters, |body, statics, torso, inertia| {
                humanoid.apply_forces(body, statics, torso, inertia)
            })
    }
    pub fn report(&self) -> String {
        let w = &self.world;
        format!(
            "walking_state step={} state={:?} root_position={:?} velocity={:?} contacts={} impulses={} max_wall_penetration={} floor_y={} wall_x={} upright_y={} angular_velocity={:?} support_height={} fixture=flat_floor_rays_and_infinite_wall humanoid_dispatch=native_subset mass_tensor={} keyframe_playback=not_implemented native_differential=not_tested\n",
            w.steps,
            self.humanoid.state_type(),
            w.body.frame.position,
            w.body.velocity,
            w.contacts,
            w.impulse_applications,
            w.max_penetration,
            w.statics.floor_y,
            w.statics.wall_x,
            w.body.frame.rotation[1][1],
            w.body.angular_velocity,
            self.humanoid.support_height(),
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
    let start = fixture.world.body.frame.position;
    let mut report = String::new();
    for tick in 0..240 {
        fixture.step(&rig, [0.0, 0.0, -1.0])?;
        if tick == 59 || tick == 239 {
            report.push_str(&fixture.report());
        }
    }
    let distance = start[2] - fixture.world.body.frame.position[2];
    if !(15.0..16.1).contains(&distance)
        || (fixture.world.body.frame.position[1] - start[1]).abs() > 0.02
    {
        return Err(format!(
            "walking fixture speed/support failed distance={distance}"
        ));
    }
    for _ in 0..120 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if fixture.world.body.velocity[2].abs() > 0.001 {
        return Err("walking release failed to stop".into());
    }
    fixture.reset();
    for tick in 0..480 {
        fixture.step(&rig, [1.0, 0.0, 0.0])?;
        if tick == 119 || tick == 479 {
            report.push_str(&fixture.report());
        }
    }
    if fixture.world.impulse_applications == 0
        || fixture.world.max_penetration > 0.5
        || fixture.world.body.frame.position[0] > fixture.world.statics.wall_x
    {
        return Err("walking wall response failed".into());
    }
    let mut minimum_up = 1.0f32;
    let mut maximum_spin = 0.0f32;
    // Long enough to catch the accumulated tipping reported during play.
    for _ in 0..7200 {
        fixture.step(&rig, [1.0, 0.0, 0.0])?;
        minimum_up = minimum_up.min(fixture.world.body.frame.rotation[1][1]);
        maximum_spin = maximum_spin.max(fixture.world.body.angular_velocity[1].abs());
        if minimum_up < 0.99 || fixture.world.body.frame.position[0] > fixture.world.statics.wall_x
        {
            return Err(format!(
                "sustained wall balance failed: {}",
                fixture.report()
            ));
        }
    }
    report.push_str(&fixture.report());
    let wall_impulses = fixture.world.impulse_applications;
    // Release must damp angular motion as well as translation.
    for _ in 0..480 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if fixture
        .world
        .body
        .angular_velocity
        .iter()
        .any(|v| v.abs() > 0.01)
    {
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
            minimum_up = minimum_up.min(fixture.world.body.frame.rotation[1][1]);
        }
        let torso = rig
            .pose(&fixture.world.body.frame)?
            .remove("Torso")
            .ok_or("missing audit torso")?;
        let alignment = (-torso.rotation[0][2] * direction[0]
            - torso.rotation[2][2] * direction[2])
            / (direction[0] * direction[0] + direction[2] * direction[2]).sqrt();
        let speed = (fixture.world.body.velocity[0].powi(2)
            + fixture.world.body.velocity[2].powi(2))
        .sqrt();
        if fixture.state_type() != StateType::RunningNoPhysics
            || (speed - fixture.humanoid.properties.walk_speed).abs() > 0.001
            || (torso.position[1] - 18.0).abs() > 0.0001
        {
            return Err(format!(
                "open-ground run did not settle in RunningNoPhysics speed={speed}: {}",
                fixture.report()
            ));
        }
        if alignment < 0.999
            || minimum_up < 0.99
            || fixture.world.body.angular_velocity[1].abs() > 0.01
        {
            return Err(format!(
                "directional balance/yaw failed alignment={alignment}: {}",
                fixture.report()
            ));
        }
    }
    fixture.reset();
    let point = [
        fixture.world.body.center[0],
        fixture.world.body.center[1] + 1.0,
        fixture.world.body.center[2],
    ];
    fixture
        .world
        .body
        .contact_impulse([0.0, 0.0, 40.0], point, &fixture.world.kernel)?;
    let disturbed_speed = fixture.world.body.angular_velocity[0].abs();
    for _ in 0..480 {
        fixture.step(&rig, [0.0; 3])?;
    }
    if disturbed_speed < 1.0
        || fixture.world.body.frame.rotation[1][1] < 0.9999
        || fixture
            .world
            .body
            .angular_velocity
            .iter()
            .any(|v| v.abs() > 0.01)
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
        let frames = moving_rig.pose(&fixture.world.body.frame)?;
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
    if swing.iter().any(|s| *s < 1.0) || pivot_error > 0.0001 || !fixture.world.inertia_changed() {
        return Err(format!(
            "walking animation did not propagate swing={swing:?} pivot_error={pivot_error}"
        ));
    }
    report.push_str(&fixture.animation_report());
    for _ in 0..480 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    if !matches!(fixture.script.pose, Pose::Standing) {
        return Err("walking animation did not become Standing on release".into());
    }
    report.push_str(&fixture.animation_report());
    fixture.reset();
    moving_rig = rig.clone();
    for _ in 0..7680 {
        fixture.step_animated(&mut moving_rig, [1.0, 0.0, 0.0])?;
        animated_minimum_up = animated_minimum_up.min(fixture.world.body.frame.rotation[1][1]);
        if animated_minimum_up < 0.99
            || fixture.world.body.frame.position[0] > fixture.world.statics.wall_x
            || fixture.world.max_penetration > 0.5
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
            if fixture.world.body.frame.rotation[1][1] < 0.99 {
                return Err("animated direction change tipped the torso".into());
            }
        }
    }
    for _ in 0..480 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    if !matches!(fixture.script.pose, Pose::Standing)
        || fixture
            .world
            .body
            .angular_velocity
            .iter()
            .any(|v| v.abs() > 0.01)
    {
        return Err("animated release failed".into());
    }
    fixture.reset();
    if !fixture.world.is_original() || fixture.joint_callbacks != 0 {
        return Err("walking reset failed".into());
    }
    // Jump from rest: hold Space for one 60 Hz frame (MasterControl sets Jump).
    fixture.reset();
    moving_rig = rig.clone();
    let rest_y = fixture.world.body.frame.position[1];
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
        apex = apex.max(fixture.world.body.frame.position[1]);
        if poses.last() != Some(&format!("{:?}", fixture.script.pose)) {
            poses.push(format!("{:?}", fixture.script.pose));
        }
    }
    let jump_states: Vec<_> = fixture
        .humanoid
        .transitions
        .iter()
        .map(|(_, s)| *s)
        .collect();
    writeln!(
        report,
        "walking_jump transitions={:?} apex_rise={} poses={poses:?} final_y={}",
        fixture.humanoid.transitions,
        apex - rest_y,
        fixture.world.body.frame.position[1]
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
        || (fixture.world.body.frame.position[1] - rest_y).abs() > 0.01
        || !poses.iter().any(|p| p == "Jumping")
        || !poses.iter().any(|p| p == "FreeFall")
    {
        return Err(format!("native jump sequence failed: {}", fixture.report()));
    }
    let jump_rise = apex - rest_y;
    // Drop 10 studs: Running loses its floor, Freefall, Landed, Running.
    fixture.reset();
    let mut lifted = fixture.world.body.body_state();
    lifted.frame.position[1] += 10.0;
    fixture.world.body.set_body_state(&lifted)?;
    for _ in 0..960 {
        fixture.step(&rig, [0.0; 3])?;
    }
    let drop_states: Vec<_> = fixture
        .humanoid
        .transitions
        .iter()
        .map(|(_, s)| *s)
        .collect();
    writeln!(
        report,
        "walking_drop transitions={:?} final_y={} state={:?}",
        fixture.humanoid.transitions,
        fixture.world.body.frame.position[1],
        fixture.state_type()
    )
    .unwrap();
    if drop_states.get(..3)
        != Some(&[StateType::Freefall, StateType::Landed, StateType::Running][..])
        || (fixture.world.body.frame.position[1] - rest_y).abs() > 0.01
    {
        return Err(format!("native fall/landing failed: {}", fixture.report()));
    }
    writeln!(report,"walking_state_pass running_no_physics_speed=16 jump_rise={jump_rise} drop_landed=true states=Running,RunningNoPhysics,Jumping,Freefall,Landed").unwrap();
    writeln!(report,"walking_animation_pass limbs=4 swing_radians={swing:?} pivot_error={pivot_error} min_animated_upright_y={animated_minimum_up} wall_seconds=32 release_pose=Standing animated_cofm=true reset_identical=true default_keyframes=unavailable").unwrap();
    writeln!(report,"walking_pass walk_speed={} distance_one_second={distance} input_release_stopped=true wall_impulses={} walking_animation=bundled_procedural default_keyframes=unavailable native_differential=not_tested",fixture.humanoid.properties.walk_speed,wall_impulses).unwrap();
    Ok(report)
}
