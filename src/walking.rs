//! Explicit flat-floor / infinite-wall walking fixture: the game loop that
//! runs the native jobs (Heartbeat, WaitingScripts, PhysicsJob) over a World
//! with one character, its Humanoid, its Animator and the shipped R6 Animate
//! LocalScript (keyframe clips). The
//! world contents (floor plane, wall half-space, materials, awake scheduling)
//! are a fixture, not native Workspace contents.
use crate::{
    animate_script::{AnimateProfile, AnimateScript, ScriptPose},
    animator::Animator,
    block_contact::PairKind,
    bundle::Bundle,
    humanoid::Humanoid,
    humanoid_state::StateType,
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
    scheduler::{FrameEvents, NativeSchedule, SchedulerProfile},
    workspace::Workspace,
    world::World,
};
use std::fmt::Write;

pub struct WalkingFixture {
    pub world: World,
    pub humanoid: Humanoid,
    animate: AnimateProfile,
    script: AnimateScript,
    animator: Animator,
    joints: Vec<(String, String)>,
    animation_ticks: u32,
    joint_callbacks: u32,
    limb_angles: [f32; 4],
    schedule: NativeSchedule,
    original_schedule: NativeSchedule,
    last_frame: Option<FrameEvents>,
}
impl WalkingFixture {
    pub fn new(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        Self::with_world(bundle, scene, rig, World::new(bundle, scene, rig)?)
    }
    /// Play Solo in the Studio new-document place: the character spawns over
    /// the Baseplate (`World::in_workspace`).
    pub fn baseplate(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        Self::in_workspace(bundle, scene, rig, &Workspace::studio_new_document())
    }
    /// Play Solo in any Workspace of anchored Parts.
    pub fn in_workspace(
        bundle: &Bundle,
        scene: &Scene,
        rig: &R6Rig,
        workspace: &Workspace,
    ) -> Result<Self, String> {
        let world = World::in_workspace(bundle, scene, rig, workspace)?;
        Self::with_world(bundle, scene, rig, world)
    }
    fn with_world(
        bundle: &Bundle,
        scene: &Scene,
        rig: &R6Rig,
        world: World,
    ) -> Result<Self, String> {
        let animate = AnimateProfile::load(bundle)?;
        let schedule = NativeSchedule::new(&SchedulerProfile::load(bundle)?, animate.wait_seconds)?;
        let joints = rig
            .joints
            .iter()
            .map(|j| (j.part0.clone(), j.part1.clone()))
            .collect();
        let mut fixture = Self {
            world,
            humanoid: Humanoid::new(bundle, scene)?,
            animate,
            script: AnimateScript::default(),
            animator: Animator::default(),
            joints,
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
        // A respawned character gets a fresh Animator and Animate script,
        // which runs to its first wait: idle starts playing.
        self.animator = Animator::new(self.joints.clone());
        self.script = AnimateScript::default();
        self.script
            .start(&self.animate, &mut self.animator)
            .expect("Animate idle start");
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
            .camera_raycast(from, v.map(|x| x / m), m.min(999.0))
            .map(|h| h.0)
    }
    pub fn state_type(&self) -> StateType {
        self.humanoid.state_type()
    }
    /// Humanoid::onStepped (Stepped), then its signals run the Animate
    /// handlers synchronously.
    fn humanoid_stepped(&mut self, rig: &R6Rig, input: [f32; 3], dt: f32) -> Result<(), String> {
        self.humanoid.on_stepped(&mut self.world, rig, input, dt)?;
        for event in self.humanoid.take_events() {
            self.script
                .on_event(&self.animate, &mut self.animator, event)?;
        }
        Ok(())
    }
    /// Animator::onStepped on RunService's Animation signal: step tracks
    /// (KeyframeReached handlers run inside), then pose every Motor6D.
    fn animator_stepped(&mut self, rig: &mut R6Rig, dt: f32) -> Result<(), String> {
        let (script, profile) = (&mut self.script, &self.animate);
        let mut failure = None;
        let poses = self.animator.on_stepped(dt, &mut |animator, track, event| {
            if let Err(e) = script.on_track_event(profile, animator, track, event) {
                failure.get_or_insert(e);
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        for pose in poses {
            rig.joints[pose.joint].animate(&pose.pose);
        }
        Ok(())
    }
    /// One world step with the shipped Animate script and keyframe clips.
    pub fn step_animated(&mut self, rig: &mut R6Rig, input: [f32; 3]) -> Result<(), String> {
        // Each call is one native world step. A new ideal 60 Hz frame runs the
        // Heartbeat and WaitingScripts jobs, then PhysicsJob: gameStepped
        // (Stepped -> Humanoid::onStepped, Animation -> Animator::onStepped),
        // World::uiStep (breakJoints, then MovingAssemblyStage::stepUi for
        // the motor hinges and the Motor6Ds, 00ba8c68/00b39318) and its
        // world steps (00ba9892).
        while self.schedule.world_steps_remaining == 0 {
            let frame = self.schedule.begin_frame()?;
            if let Some(time) = frame.resumed_time {
                // Animate: `local _, time = wait(0.1); move(time)`.
                self.script
                    .resume(&self.animate, &mut self.animator, rig, time)?;
                self.animation_ticks += 1;
            }
            self.last_frame = Some(frame);
            if frame.world_steps == 0 {
                continue; // physicsStep returns before World::step.
            }
            let dt = self.stepped_dt(&frame);
            self.humanoid_stepped(rig, input, dt)?;
            self.animator_stepped(rig, dt)?;
            self.world.break_joints()?;
            self.world.step_ui(self.schedule.game_time)?;
            let before: Vec<_> = rig.joints.iter().map(|j| j.link_state()).collect();
            for (joint, old) in rig.joints.iter_mut().zip(&before) {
                joint.step()?;
                // 00b37550: a changed link wakes Part0's and Part1's assembly
                // (the character's) recursively.
                if joint.link_state() != *old {
                    self.world.loose.wake_character_recursive()?;
                    self.world.loose.wake_character_recursive()?;
                }
            }
            self.joint_callbacks += 1;
            self.limb_angles = std::array::from_fn(|a| rig.joints[a + 1].current_angle);
            if rig
                .joints
                .iter()
                .zip(before)
                .any(|(j, old)| j.link_state() != old)
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
            "walking_animation profile=animate_keyframe2 animation={} tracks={} loads={} clips_loaded={} clips_missing={:?} pose={:?} state={:?} event_speed={} script_resumes={} joint_step_ui={} limb_angles={:?} local_inertia={:?} schedule={:?} heartbeat_time={} game_time={} schedule_source=native_jobs_ideal_clock\n",
            self.script.current_animation(),
            self.animator.active_tracks().len(),
            self.script.loads,
            self.animate.loaded(),
            self.animate.missing,
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
    /// One world step with the rest-pose rig: same Humanoid dispatch and
    /// script handlers, no Animator or motor stepping.
    pub fn step(&mut self, rig: &R6Rig, input: [f32; 3]) -> Result<(), String> {
        while self.schedule.world_steps_remaining == 0 {
            let frame = self.schedule.begin_frame()?;
            self.last_frame = Some(frame);
            if frame.world_steps == 0 {
                continue;
            }
            self.humanoid_stepped(rig, input, self.stepped_dt(&frame))?;
            self.world.break_joints()?;
            self.world.step_ui(self.schedule.game_time)?;
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
            .step_with(rig, parameters, |body, surroundings, torso, inertia| {
                humanoid.apply_forces_in(body, surroundings, torso, inertia)
            })
    }
    pub fn report(&self) -> String {
        let w = &self.world;
        format!(
            "walking_state step={} state={:?} root_position={:?} velocity={:?} contacts={} impulses={} max_wall_penetration={} floor_y={} wall_x={} upright_y={} angular_velocity={:?} support_height={} fixture=flat_floor_rays_and_infinite_wall humanoid_dispatch=native_subset mass_tensor={} keyframe_playback=animator native_differential=not_tested\n",
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
    // Realtime springs only (no impulses on character connectors): the wall
    // must have been reached, and held to a shallow penetration.
    if fixture.world.max_penetration <= 0.0
        || fixture.world.max_penetration > 0.5
        || fixture.world.body.frame.position[0] > fixture.world.statics.wall_x
    {
        return Err(format!(
            "walking wall response failed: {}",
            fixture.report()
        ));
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
    let wall_penetration = fixture.world.max_penetration;
    // Release must damp angular motion as well as translation. The rate is
    // measured from the orientation over the last 240 ticks: against the wall
    // the per-tick angular velocity alternates sign every substep (contact
    // springs, then the Humanoid connector, 00a9768c), which is a jitter in
    // place, not rotation. Its amplitude is reported as release_jitter.
    let mut settled = fixture.world.body.frame.rotation;
    let mut jitter = 0.0f32;
    for tick in 0..480 {
        fixture.step(&rig, [0.0; 3])?;
        if tick == 239 {
            settled = fixture.world.body.frame.rotation;
        }
        if tick >= 240 {
            jitter = jitter.max(fixture.world.body.angular_velocity[1].abs());
        }
    }
    let r = fixture.world.body.frame.rotation;
    let trace: f32 = (0..3)
        .map(|i| (0..3).map(|k| settled[k][i] * r[k][i]).sum::<f32>())
        .sum();
    let rate = ((trace - 1.0) * 0.5).clamp(-1.0, 1.0).acos() / 4.0;
    writeln!(
        report,
        "walking_wall_release rotation_rate={rate} release_jitter={jitter}"
    )
    .unwrap();
    if rate > 0.01 {
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
            // The animated link may translate: child*C1 = parent*C0*link.
            let pivot = crate::rig::compose(&frames[&joint.part0], &joint.c0);
            let parent = pivot.transform_point(joint.link_state().1);
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
    if !matches!(fixture.script.pose, ScriptPose::Standing) {
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
    if !matches!(fixture.script.pose, ScriptPose::Standing)
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
    let rest_torso_y = fixture.torso_frame(&rig)?.position[1];
    for _ in 0..120 {
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
    }
    fixture.set_jump(true);
    let mut apex = rest_y;
    let mut poses = Vec::new();
    let mut clips: Vec<String> = Vec::new();
    for tick in 0..720 {
        if tick == 4 {
            fixture.set_jump(false);
        }
        fixture.step_animated(&mut moving_rig, [0.0; 3])?;
        apex = apex.max(fixture.world.body.frame.position[1]);
        if poses.last() != Some(&format!("{:?}", fixture.script.pose)) {
            poses.push(format!("{:?}", fixture.script.pose));
        }
        let clip = fixture.script.current_animation();
        if clips.last().map(String::as_str) != Some(clip) {
            clips.push(clip.to_string());
        }
    }
    let final_torso_y = fixture.torso_frame(&moving_rig)?.position[1];
    let jump_states: Vec<_> = fixture
        .humanoid
        .transitions
        .iter()
        .map(|(_, s)| *s)
        .collect();
    writeln!(
        report,
        "walking_jump transitions={:?} apex_rise={} poses={poses:?} clips={clips:?} final_root_y={} final_torso_y={final_torso_y} rest_torso_y={rest_torso_y}",
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
        || (final_torso_y - rest_torso_y).abs() > 0.01
        || clips.first().map(String::as_str) != Some("jump")
        || !clips.iter().any(|c| c == "fall")
        || clips.last().map(String::as_str) != Some("idle")
        || !poses.iter().any(|p| p == "Jumping")
        || !poses.iter().any(|p| p == "FreeFall")
    {
        return Err(format!(
            "native jump sequence failed: transitions={:?} apex_rise={} poses={poses:?} clips={clips:?} final_torso_y={final_torso_y} {}",
            fixture.humanoid.transitions,
            apex - rest_y,
            fixture.report()
        ));
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
    writeln!(report,"walking_animation_pass limbs=4 swing_radians={swing:?} pivot_error={pivot_error} min_animated_upright_y={animated_minimum_up} wall_seconds=32 release_pose=Standing animated_cofm=true reset_identical=true default_keyframes=animate_keyframe2").unwrap();
    writeln!(report,"walking_pass walk_speed={} distance_one_second={distance} input_release_stopped=true wall_max_penetration={} walking_animation=animate_keyframe2 native_differential=not_tested",fixture.humanoid.properties.walk_speed,wall_penetration).unwrap();
    Ok(report)
}

/// Play Solo in the Studio default place: spawn over the Baseplate, fall,
/// land, stand, walk and jump, with the keyframe Animate script.
pub fn audit_baseplate(bundle: &Bundle) -> Result<String, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut fixture = WalkingFixture::baseplate(bundle, &scene, &rig)?;
    let mut report = String::new();
    let head = fixture.torso_frame(&rig)?;
    writeln!(
        report,
        "baseplate_spawn root={:?} torso={:?} blocks={:?} velocity={:?}",
        fixture.world.body.frame.position,
        head.position,
        fixture
            .world
            .statics
            .blocks
            .iter()
            .map(|b| (b.name.as_str(), b.frame.position, b.size))
            .collect::<Vec<_>>(),
        fixture.world.body.velocity
    )
    .unwrap();
    let mut states = vec![fixture.state_type()];
    let mut animations = vec![fixture.script.current_animation().to_string()];
    let mut landed_step = None;
    let mut lowest = f32::INFINITY;
    let mut fastest = 0.0f32;
    // 3 seconds of world steps (240 Hz): fall, land and settle.
    for step in 0..720u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let state = fixture.state_type();
        if states.last() != Some(&state) {
            states.push(state);
            writeln!(
                report,
                "baseplate_state step={} time={:.4} state={state:?} root_y={} velocity_y={}",
                step + 1,
                (step + 1) as f32 / 240.0,
                fixture.world.body.frame.position[1],
                fixture.world.body.velocity[1]
            )
            .unwrap();
            if landed_step.is_none()
                && matches!(state, StateType::Landed | StateType::Running)
                && step > 0
            {
                landed_step = Some(step + 1);
            }
        }
        let animation = fixture.script.current_animation().to_string();
        if animations.last() != Some(&animation) {
            animations.push(animation);
        }
        lowest = lowest.min(fixture.world.body.frame.position[1]);
        fastest = fastest.min(fixture.world.body.velocity[1]);
        if step % 60 == 59 {
            report.push_str(&fixture.report());
        }
    }
    let rest = fixture.world.body.frame.position;
    writeln!(
        report,
        "baseplate_landing landed_step={landed_step:?} lowest_root_y={lowest} fastest_fall={fastest} rest_root={rest:?} upright_y={} states={states:?} animations={animations:?}",
        fixture.world.body.frame.rotation[1][1]
    )
    .unwrap();
    if landed_step.is_none()
        || !(2.9..3.1).contains(&rest[1])
        || fixture.world.body.frame.rotation[1][1] < 0.99
        || !matches!(
            fixture.state_type(),
            StateType::Running | StateType::RunningNoPhysics
        )
        || !animations.iter().any(|a| a == "fall")
        || animations.last().map(String::as_str) != Some("idle")
    {
        return Err(format!("baseplate landing failed:\n{report}"));
    }
    // One second of W (camera-less world input -Z), then release.
    let start = fixture.world.body.frame.position;
    for _ in 0..240 {
        fixture.step_animated(&mut rig, [0.0, 0.0, -1.0])?;
    }
    let walked = start[2] - fixture.world.body.frame.position[2];
    report.push_str(&fixture.report());
    if !(15.0..16.1).contains(&walked)
        || (fixture.world.body.frame.position[1] - start[1]).abs() > 0.05
    {
        return Err(format!("baseplate walk failed distance={walked}\n{report}"));
    }
    for _ in 0..240 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
    }
    let before_jump = fixture.world.body.frame.position[1];
    fixture.set_jump(true);
    let mut apex = before_jump;
    for step in 0..480 {
        if step == 2 {
            fixture.set_jump(false);
        }
        fixture.step_animated(&mut rig, [0.0; 3])?;
        apex = apex.max(fixture.world.body.frame.position[1]);
    }
    let after_jump = fixture.world.body.frame.position[1];
    writeln!(
        report,
        "baseplate_walk_jump walked={walked} jump_apex={} rest_before={before_jump} rest_after={after_jump} state={:?} animation={}",
        apex - before_jump,
        fixture.state_type(),
        fixture.script.current_animation()
    )
    .unwrap();
    if !(7.0..10.0).contains(&(apex - before_jump)) || (after_jump - before_jump).abs() > 0.05 {
        return Err(format!("baseplate jump failed\n{report}"));
    }
    writeln!(report, "baseplate_pass spawn=native_no_spawnlocation floor=baseplate_block_raycast block_contacts=block_block_contact native_differential=not_tested").unwrap();
    Ok(report)
}

/// Walk into an anchored wall Part on the Baseplate: Torso and Head meet it
/// through the BlockBlockContact narrowphase.
pub fn audit_part_wall(bundle: &Bundle) -> Result<String, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    // Explicit test Part: its +Z face is the plane z = -10, across the
    // walking path (the character faces and walks -Z).
    let mut wall = crate::workspace::PartInstance {
        name: "Wall".into(),
        anchored: true,
        ..Default::default()
    };
    wall.set_size([20.0, 10.0, 2.0]);
    wall.frame.position = [0.0, 5.0, -11.0];
    workspace.parts.push(wall);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let mut report = String::new();
    for _ in 0..720u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
    }
    let start = fixture.world.body.frame.position;
    let mut first_contact = None;
    let mut most = 0;
    for step in 0..480u32 {
        fixture.step_animated(&mut rig, [0.0, 0.0, -1.0])?;
        let contacts = fixture.world.contacts;
        if contacts > 0 && first_contact.is_none() {
            first_contact = Some(step + 1);
            writeln!(
                report,
                "part_wall_first_contact step={} root={:?} velocity={:?} {}",
                step + 1,
                fixture.world.body.frame.position,
                fixture.world.body.velocity,
                fixture.world.block_contact_report()
            )
            .unwrap();
        }
        most = most.max(contacts);
        if step % 60 == 59 {
            writeln!(
                report,
                "part_wall_walk step={} root={:?} velocity={:?} contacts={contacts} {}",
                step + 1,
                fixture.world.body.frame.position,
                fixture.world.body.velocity,
                fixture.world.block_contact_report()
            )
            .unwrap();
        }
    }
    let stop = fixture.world.body.frame.position;
    let kinds = fixture.world.block_contact_kinds();
    writeln!(
        report,
        "part_wall_result start={start:?} stop={stop:?} first_contact_step={first_contact:?} max_contacts={most} max_penetration={} impulses={} upright_y={} state={:?} kinds={kinds:?}",
        fixture.world.max_penetration,
        fixture.world.impulse_applications,
        fixture.world.body.frame.rotation[1][1],
        fixture.state_type()
    )
    .unwrap();
    // Torso front face (half depth 0.5) stops at the wall face z = -10.
    if first_contact.is_none()
        || stop[2] < -9.6
        || stop[2] > -9.0
        || fixture.world.max_penetration > 0.5
        || !kinds.contains(&PairKind::VertexPlane)
    {
        return Err(format!("part wall failed\n{report}"));
    }
    writeln!(report, "part_wall_pass blocked=true narrowphase=block_block_contact broadphase=spatial_hash native_differential=not_tested").unwrap();
    Ok(report)
}

/// Unanchored Parts on the new-document Baseplate. "Rest" starts 0.05 above
/// the Baseplate and "Stack" 0.05 above "Rest" (soft landings, asserted to
/// rest flat and stacked). "Drop" falls flat from y = 8 and "Tumble" falls
/// tilted from y = 10; their bounces are only logged, and they must end at
/// rest on some face. The character idles at the spawn, away from them.
pub fn audit_part_drop(bundle: &Bundle) -> Result<String, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let part = |name: &str, position: [f32; 3]| {
        let mut p = crate::workspace::PartInstance {
            name: name.into(),
            ..Default::default()
        };
        p.frame.position = position;
        p
    };
    let rest = part("Rest", [-20.0, 0.65, -20.0]);
    let stack = part("Stack", [-19.5, 1.9, -19.8]);
    let drop = part("Drop", [20.0, 8.0, 0.0]);
    let mut tumble = part("Tumble", [-12.0, 10.0, 12.0]);
    // 30 degrees about Z, then 20 degrees about X.
    let (sz, cz) = 30f32.to_radians().sin_cos();
    let (sx, cx) = 20f32.to_radians().sin_cos();
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    tumble.frame.rotation =
        std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| rz[r][k] * rx[k][c]).sum()));
    workspace.parts.extend([rest, stack, drop, tumble]);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let mut report = String::new();
    let count = fixture.world.loose.parts.len();
    let mut first_contact: Vec<Option<u32>> = vec![None; count];
    let mut max_iterations = 0;
    for step in 0..1440u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let loose = &fixture.world.loose;
        max_iterations = max_iterations.max(loose.stats.iterations);
        for (i, p) in loose.parts.iter().enumerate() {
            if first_contact[i].is_none() && p.list != crate::loose::BodyList::FreeFall {
                first_contact[i] = Some(step + 1);
                writeln!(
                    report,
                    "part_drop_first_contact part={} step={} {}",
                    p.name,
                    step + 1,
                    loose.report()
                )
                .unwrap();
            }
        }
        if (step + 1) % 120 == 0 {
            writeln!(
                report,
                "part_drop_state step={} iterations={} average_error={} contact_connectors={} {}",
                step + 1,
                loose.stats.iterations,
                loose.stats.average_error,
                loose.stats.contact_connectors,
                loose.report()
            )
            .unwrap();
        }
    }
    let parts = &fixture.world.loose.parts;
    let speed = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let y = |i: usize| parts[i].body.frame.position[1];
    writeln!(
        report,
        "part_drop_result first_contact={first_contact:?} heights={:?} up_y={:?} speeds={:?} spins={:?} max_iterations={max_iterations}",
        (0..count).map(y).collect::<Vec<_>>(),
        parts.iter().map(|p| p.body.frame.rotation[1][1]).collect::<Vec<_>>(),
        parts.iter().map(|p| speed(p.body.velocity)).collect::<Vec<_>>(),
        parts.iter().map(|p| speed(p.body.angular_velocity)).collect::<Vec<_>>(),
    )
    .unwrap();
    // Flat half height 0.6 on the Baseplate top (y = 0); 1.2 + 0.6 on Rest.
    // Any face of a 4 x 1.2 x 2 block: half heights 0.6, 1.0 or 2.0.
    let on_face = |h: f32| [0.6f32, 1.0, 2.0].iter().any(|f| (h - f).abs() < 0.05);
    if first_contact.iter().any(Option::is_none)
        || (y(0) - 0.6).abs() > 0.05
        || (y(1) - 1.8).abs() > 0.08
        || parts[0].body.frame.rotation[1][1] < 0.999
        || parts[1].body.frame.rotation[1][1] < 0.999
        || !on_face(y(2))
        || !on_face(y(3))
        || parts.iter().any(|p| speed(p.body.velocity) > 0.05)
    {
        return Err(format!("part drop failed\n{report}"));
    }
    writeln!(
        report,
        "{}",
        sleep_summary("part_drop_sleep", &fixture.world.loose)
    )
    .unwrap();
    // SleepStage: every Part comes to rest and leaves the kernel.
    if parts.iter().any(|p| !p.sleep.sleeping() || p.in_kernel) {
        return Err(format!("part drop: a resting Part never slept\n{report}"));
    }
    writeln!(report, "part_drop_pass rested=4 stacked=true asleep=4 sleep=sleep_stage broadphase=spatial_hash native_differential=not_tested").unwrap();
    Ok(report)
}

/// Welded Parts in the kernel (`joints.rs` assemblies, one SimBody each):
/// - "Base" (4 x 1.2 x 2) with "Arm" (1 x 4 x 1) welded upright on its +x
///   end, the Arm placed 50 studs away: the weld puts it back (the child
///   Body's pose is parent x C0 x C1^-1), then the pair falls 1.4 studs and
///   lands flat. (From higher up the impact flips it: the single-Part
///   solver does the same to a tall block, see docs/ROADMAP.md.)
/// - "Hung" welded under the anchored "Holder", half a stud off the weld's
///   pose, and "Chain" welded under Hung: Hung gets its own AnchorJoint
///   (GroundStage 00b0f7ac) and keeps its CFrame; Chain hangs from Hung's
///   Body. Both are grounded and never move.
/// - "Plank" (6 x 1 x 2) with "Weight" (2 x 2 x 2) welded under its -x end,
///   dropped so the child Part lands first and the plank tips onto its
///   other end.
pub fn audit_part_weld(bundle: &Bundle) -> Result<String, String> {
    use crate::joints::WELD;
    use crate::workspace::{JointInstance, PartInstance};
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let at = |position: [f32; 3]| CoordinateFrame {
        position,
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    fn add(
        workspace: &mut Workspace,
        name: &str,
        size: [f32; 3],
        frame: CoordinateFrame,
        anchored: bool,
    ) -> usize {
        let mut part = PartInstance {
            name: name.into(),
            anchored,
            frame,
            ..Default::default()
        };
        part.set_size(size);
        workspace.parts.push(part);
        workspace.parts.len() - 1
    }
    let weld = |workspace: &mut Workspace, part0: usize, part1: usize, c0: [f32; 3]| {
        workspace.joints.push(JointInstance {
            name: "Weld".into(),
            kind: WELD,
            part0,
            part1,
            c0: at(c0),
            c1: at([0.0; 3]),
            face: [[0.0; 3]; 4],
            base_angle: 0.0,
            current_angle: 0.0,
            desired_angle: 0.0,
            max_velocity: 0.0,
        });
    };
    let base = add(
        &mut workspace,
        "Base",
        [4.0, 1.2, 2.0],
        at([20.0, 2.0, 0.0]),
        false,
    );
    let arm = add(
        &mut workspace,
        "Arm",
        [1.0, 4.0, 1.0],
        at([50.0, 50.0, 50.0]),
        false,
    );
    weld(&mut workspace, base, arm, [1.5, 2.6, 0.0]);
    let holder = add(
        &mut workspace,
        "Holder",
        [2.0, 2.0, 2.0],
        at([-20.0, 10.0, 0.0]),
        true,
    );
    let hung = add(
        &mut workspace,
        "Hung",
        [2.0, 2.0, 2.0],
        at([-20.0, 7.5, 0.0]),
        false,
    );
    let chain = add(
        &mut workspace,
        "Chain",
        [2.0, 2.0, 2.0],
        at([0.0; 3]),
        false,
    );
    weld(&mut workspace, holder, hung, [0.0, -2.0, 0.0]);
    weld(&mut workspace, hung, chain, [0.0, -2.0, 0.0]);
    // 15 degrees about Z: the -x end (the Weight) is lowest.
    let (sz, cz) = 15f32.to_radians().sin_cos();
    let tilted = CoordinateFrame {
        position: [0.0, 10.0, 20.0],
        rotation: [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]],
    };
    let plank = add(&mut workspace, "Plank", [6.0, 1.0, 2.0], tilted, false);
    let weight = add(
        &mut workspace,
        "Weight",
        [2.0, 2.0, 2.0],
        at([0.0; 3]),
        false,
    );
    weld(&mut workspace, plank, weight, [-2.0, -1.5, 0.0]);
    let names: Vec<&str> = workspace.parts.iter().map(|p| p.name.as_str()).collect();
    let tree = workspace.joint_tree()?;
    let mut report = tree.describe(&names);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let prim = |fixture: &WalkingFixture, name: &str| {
        fixture
            .world
            .loose
            .prims
            .iter()
            .position(|p| p.name == name)
            .ok_or(format!("{name} is not a loose Part"))
    };
    let statics: Vec<&str> = fixture
        .world
        .statics
        .blocks
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    writeln!(report, "part_weld_statics {statics:?}").unwrap();
    let loose = &fixture.world.loose;
    for part in &loose.parts {
        let names: Vec<&str> = part
            .prims
            .iter()
            .map(|&i| loose.prims[i].name.as_str())
            .collect();
        let offset: [f32; 3] =
            std::array::from_fn(|a| part.body.center[a] - part.body.frame.position[a]);
        writeln!(
            report,
            "part_weld_assembly root={} prims={names:?} mass={} center_offset={offset:?}",
            part.name,
            1.0 / part.body.inverse_mass(),
        )
        .unwrap();
    }
    let frame_of = |fixture: &WalkingFixture, name: &str| -> Result<CoordinateFrame, String> {
        fixture.world.loose.prim_frame(prim(fixture, name)?)
    };
    let snapped = frame_of(&fixture, "Arm")?.position;
    writeln!(report, "part_weld_snap arm={snapped:?}").unwrap();
    let frames = |fixture: &WalkingFixture| -> String {
        fixture
            .world
            .loose
            .frames()
            .iter()
            .map(|(n, f)| format!("{n}={:?}", f.position))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut first_contact: Vec<Option<u32>> = vec![None; fixture.world.loose.parts.len()];
    for step in 0..1440u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let loose = &fixture.world.loose;
        for (i, p) in loose.parts.iter().enumerate() {
            if first_contact[i].is_none() && p.list != crate::loose::BodyList::FreeFall {
                first_contact[i] = Some(step + 1);
                writeln!(
                    report,
                    "part_weld_first_contact assembly={} step={} {} {}",
                    p.name,
                    step + 1,
                    frames(&fixture),
                    loose.report()
                )
                .unwrap();
            }
        }
        if (step + 1) % 240 == 0 {
            writeln!(
                report,
                "part_weld_state step={} {} {}",
                step + 1,
                frames(&fixture),
                loose.report()
            )
            .unwrap();
        }
    }
    let loose = &fixture.world.loose;
    let speed = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // The lowest corner of each Part (penetration into the Baseplate top).
    let lowest = |i: usize| -> Result<f32, String> {
        let f = loose.prim_frame(i)?;
        let h = loose.prims[i].size.map(|v| v * 0.5);
        Ok(f.position[1] - (0..3).map(|k| (f.rotation[1][k] * h[k]).abs()).sum::<f32>())
    };
    let lows = (0..loose.prims.len())
        .map(|i| Ok((loose.prims[i].name.clone(), lowest(i)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let base_frame = frame_of(&fixture, "Base")?;
    let arm_frame = frame_of(&fixture, "Arm")?;
    let arm_local: [f32; 3] = std::array::from_fn(|c| {
        (0..3)
            .map(|r| base_frame.rotation[r][c] * (arm_frame.position[r] - base_frame.position[r]))
            .sum()
    });
    writeln!(
        report,
        "part_weld_result first_contact={first_contact:?} lowest={lows:?} arm_in_base={arm_local:?} speeds={:?} spins={:?}",
        loose.parts.iter().map(|p| speed(p.body.velocity)).collect::<Vec<_>>(),
        loose.parts.iter().map(|p| speed(p.body.angular_velocity)).collect::<Vec<_>>(),
    )
    .unwrap();
    writeln!(report, "{}", sleep_summary("part_weld_sleep", loose)).unwrap();
    let hung_block = fixture
        .world
        .statics
        .blocks
        .iter()
        .find(|b| b.name == "Hung")
        .ok_or("Hung is not a static block")?;
    let chain_block = fixture
        .world
        .statics
        .blocks
        .iter()
        .find(|b| b.name == "Chain")
        .ok_or("Chain is not a static block")?;
    let near = |a: [f32; 3], b: [f32; 3], e: f32| (0..3).all(|k| (a[k] - b[k]).abs() <= e);
    let checks = [
        ("assemblies", loose.parts.len() == 2),
        ("snapped", near(snapped, [21.5, 4.6, 0.0], 1e-5)),
        (
            "base_flat",
            (base_frame.position[1] - 0.6).abs() < 0.05 && base_frame.rotation[1][1] > 0.999,
        ),
        ("arm_kept", near(arm_local, [1.5, 2.6, 0.0], 1e-4)),
        ("hung_kept", hung_block.frame.position == [-20.0, 7.5, 0.0]),
        (
            "chain_hangs",
            chain_block.frame.position == [-20.0, 5.5, 0.0],
        ),
        ("above_floor", lows.iter().all(|(_, y)| *y > -0.15)),
        (
            "rested",
            loose.parts.iter().all(|p| speed(p.body.velocity) < 0.05),
        ),
        (
            "asleep",
            loose
                .parts
                .iter()
                .all(|p| p.sleep.sleeping() && !p.in_kernel),
        ),
        ("plank_tipped", lowest(prim(&fixture, "Plank")?)? < 0.1),
    ];
    let failed: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    if !failed.is_empty() {
        return Err(format!("part weld failed {failed:?}\n{report}"));
    }
    let _ = (weight, chain, arm);
    writeln!(report, "part_weld_pass assemblies=2 snapped=true landed=true grounded_static=2 rested=true asleep=true native_differential=not_tested").unwrap();
    Ok(report)
}

/// Kernel joints (Glue) in a Workspace, with the character idling at the
/// origin:
/// - "Hung" (4 x 1 x 2) glued under the anchored "Holder" on their whole
///   4 x 2 face: it hangs on the four corner springs;
/// - "Left" and "Right" (2 x 1 x 2) glued side by side on a 2 x 1 face and
///   dropped onto the Baseplate: they land as one;
/// - "Heavy" (14 x 14 x 14) glued under the anchored "Beam" on a 1 x 1 face:
///   its weight is over the face's break force, so breakJoints removes the
///   Glue and it drops.
pub fn audit_part_glue(bundle: &Bundle) -> Result<String, String> {
    use crate::joints::GLUE;
    use crate::loose::JointEdge;
    use crate::workspace::{JointInstance, PartInstance};
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let at = |position: [f32; 3]| CoordinateFrame {
        position,
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let add = |workspace: &mut Workspace, name: &str, size: [f32; 3], position, anchored| {
        let mut part = PartInstance {
            name: name.into(),
            anchored,
            frame: at(position),
            ..Default::default()
        };
        part.set_size(size);
        workspace.parts.push(part);
        workspace.parts.len() - 1
    };
    // C0 and C1 put the face's z axis along the Part0 face normal.
    let glue = |workspace: &mut Workspace,
                part0: usize,
                part1: usize,
                c0: [f32; 3],
                c1: [f32; 3],
                rotation: [[f32; 3]; 3],
                half: [f32; 2]| {
        let [a, b] = half;
        workspace.joints.push(JointInstance {
            name: format!("Glue{}", workspace.joints.len()),
            kind: GLUE,
            part0,
            part1,
            c0: CoordinateFrame {
                position: c0,
                rotation,
            },
            c1: CoordinateFrame {
                position: c1,
                rotation,
            },
            face: [[-a, -b, 0.0], [a, -b, 0.0], [a, b, 0.0], [-a, b, 0.0]],
            base_angle: 0.0,
            current_angle: 0.0,
            desired_angle: 0.0,
            max_velocity: 0.0,
        });
    };
    // z along -Y (columns x, z, -y), and z along +X (columns -z, y, x).
    let down = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
    let side = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
    let holder = add(
        &mut workspace,
        "Holder",
        [4.0, 1.0, 2.0],
        [-20.0, 10.0, 0.0],
        true,
    );
    let hung = add(
        &mut workspace,
        "Hung",
        [4.0, 1.0, 2.0],
        [-20.0, 9.0, 0.0],
        false,
    );
    glue(
        &mut workspace,
        holder,
        hung,
        [0.0, -0.5, 0.0],
        [0.0, 0.5, 0.0],
        down,
        [2.0, 1.0],
    );
    let left = add(
        &mut workspace,
        "Left",
        [2.0, 1.0, 2.0],
        [10.0, 2.0, 0.0],
        false,
    );
    let right = add(
        &mut workspace,
        "Right",
        [2.0, 1.0, 2.0],
        [12.0, 2.0, 0.0],
        false,
    );
    glue(
        &mut workspace,
        left,
        right,
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        side,
        [1.0, 0.5],
    );
    let beam = add(
        &mut workspace,
        "Beam",
        [2.0, 1.0, 2.0],
        [30.0, 15.5, 0.0],
        true,
    );
    let heavy = add(
        &mut workspace,
        "Heavy",
        [14.0, 14.0, 14.0],
        [30.0, 8.0, 0.0],
        false,
    );
    glue(
        &mut workspace,
        beam,
        heavy,
        [0.0, -0.5, 0.0],
        [0.0, 7.0, 0.0],
        down,
        [0.5, 0.5],
    );
    let names: Vec<&str> = workspace.parts.iter().map(|p| p.name.as_str()).collect();
    let mut report = workspace.joint_tree()?.describe(&names);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let thresholds: Vec<f32> = workspace
        .joints
        .iter()
        .map(|j| crate::kernel_joint::glue_threshold(&j.face))
        .collect();
    let loose = &fixture.world.loose;
    writeln!(
        report,
        "part_glue_load assemblies={} points={} thresholds={thresholds:?} {} {}",
        loose.parts.len(),
        loose.point_count(),
        loose.joint_report(),
        loose.report()
    )
    .unwrap();
    let loaded = loose.joints.len() == 3
        && loose
            .joints
            .iter()
            .all(|j| j.edge == Some(JointEdge::Awake))
        && loose.point_count() == 24;
    let prim = |fixture: &WalkingFixture, name: &str| {
        fixture
            .world
            .loose
            .prims
            .iter()
            .position(|p| p.name == name)
            .ok_or(format!("{name} is not a loose Part"))
    };
    let frame_of = |fixture: &WalkingFixture, name: &str| -> Result<CoordinateFrame, String> {
        fixture.world.loose.prim_frame(prim(fixture, name)?)
    };
    let frames = |fixture: &WalkingFixture| -> String {
        fixture
            .world
            .loose
            .frames()
            .iter()
            .map(|(n, f)| format!("{n}={:?}", f.position))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut hung_range = (f32::INFINITY, f32::NEG_INFINITY);
    let mut broke_at = None;
    for step in 0..1440u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let y = frame_of(&fixture, "Hung")?.position[1];
        hung_range = (hung_range.0.min(y), hung_range.1.max(y));
        let loose = &fixture.world.loose;
        if broke_at.is_none() && !loose.broken.is_empty() {
            broke_at = Some(step + 1);
            writeln!(
                report,
                "part_glue_broke step={} broken={:?} {} {} {}",
                step + 1,
                loose.broken,
                frames(&fixture),
                loose.joint_report(),
                loose.report()
            )
            .unwrap();
        }
        if step < 8 || (step + 1) % 240 == 0 {
            writeln!(
                report,
                "part_glue_state step={} {} {} {}",
                step + 1,
                frames(&fixture),
                loose.joint_report(),
                loose.report()
            )
            .unwrap();
        }
    }
    let loose = &fixture.world.loose;
    let lowest = |name: &str| -> Result<f32, String> {
        let i = prim(&fixture, name)?;
        let f = loose.prim_frame(i)?;
        let h = loose.prims[i].size.map(|v| v * 0.5);
        Ok(f.position[1] - (0..3).map(|k| (f.rotation[1][k] * h[k]).abs()).sum::<f32>())
    };
    let hung_frame = frame_of(&fixture, "Hung")?;
    let (l, r) = (frame_of(&fixture, "Left")?, frame_of(&fixture, "Right")?);
    let gap: f32 = (0..3)
        .map(|k| (r.position[k] - l.position[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    let assembly = |name: &str| -> Result<&crate::loose::LoosePart, String> {
        Ok(&loose.parts[loose.prims[prim(&fixture, name)?].assembly])
    };
    writeln!(
        report,
        "part_glue_result hung_y={} hung_range={hung_range:?} pair_gap={gap} lowest_left={} lowest_heavy={} broke_at={broke_at:?} edges={:?}",
        hung_frame.position[1],
        lowest("Left")?,
        lowest("Heavy")?,
        loose.joints.iter().map(|j| j.edge).collect::<Vec<_>>(),
    )
    .unwrap();
    writeln!(report, "{}", sleep_summary("part_glue_sleep", loose)).unwrap();
    let checks = [
        ("loaded_in_kernel", loaded),
        (
            "hung_held",
            (hung_frame.position[1] - 9.0).abs() < 0.05 && loose.joints[0].edge.is_some(),
        ),
        (
            "pair_kept",
            (gap - 2.0).abs() < 0.01 && loose.joints[1].edge.is_some(),
        ),
        ("pair_landed", lowest("Left")?.abs() < 0.15),
        (
            "heavy_broke",
            broke_at.is_some_and(|s| s < 60) && loose.joints[2].edge.is_none(),
        ),
        ("heavy_dropped", lowest("Heavy")?.abs() < 0.15),
        (
            "asleep",
            ["Hung", "Left", "Heavy"]
                .iter()
                .map(|n| assembly(n))
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .all(|p| p.sleep.sleeping() && !p.in_kernel),
        ),
        (
            "joints_sleeping",
            loose.joints[..2]
                .iter()
                .all(|j| j.edge == Some(JointEdge::Sleeping)),
        ),
    ];
    let failed: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    if !failed.is_empty() {
        return Err(format!("part glue failed {failed:?}\n{report}"));
    }
    let _ = (right, heavy);
    writeln!(
        report,
        "part_glue_pass hung=held pair=kept heavy=broke asleep=true native_differential=not_tested"
    )
    .unwrap();
    Ok(report)
}

/// Legacy hinges in a Workspace, with the character idling at the origin.
/// Each pair is an anchored 1 x 1 x 1 block (Part0, the axle) and a plank
/// in front of it, 0.1 studs clear, on a hinge through the block's center:
/// - "Arm" (6 x 0.4 x 1) on a Rotate about Z, released level: it swings
///   down under gravity about the hinge;
/// - "Flap" (1 x 4 x 0.4) on a Rotate about Z, hanging still: it sleeps;
/// - "Wheel" (4 x 4 x 0.4) on a RotateV about Z whose axle face (Back)
///   reads Constant 0.1: it turns at 30 x 0.1 = 3 rad/s;
/// - "Paddle" (4 x 0.4 x 1) on a RotateP about Y whose axle face (Top)
///   reads Sin with ParamA 0.5 and ParamB 2: it follows 0.5 sin(2t).
pub fn audit_part_hinge(bundle: &Bundle) -> Result<String, String> {
    use crate::joints::{ROTATE, ROTATE_P, ROTATE_V};
    use crate::kernel_joint::{INPUT_CONSTANT, INPUT_SIN, SurfaceInput};
    use crate::loose::JointEdge;
    use crate::workspace::{JointInstance, PartInstance};
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // z along +Y (columns x, -z, y).
    let up = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    let add = |workspace: &mut Workspace, name: &str, size: [f32; 3], position, anchored| {
        let mut part = PartInstance {
            name: name.into(),
            anchored,
            frame: CoordinateFrame {
                position,
                rotation: IDENTITY,
            },
            ..Default::default()
        };
        part.set_size(size);
        workspace.parts.push(part);
        workspace.parts.len() - 1
    };
    let hinge = |workspace: &mut Workspace,
                 kind: u32,
                 part0: usize,
                 part1: usize,
                 c1: [f32; 3],
                 rotation: [[f32; 3]; 3]| {
        workspace.joints.push(JointInstance {
            name: format!("Hinge{}", workspace.joints.len()),
            kind,
            part0,
            part1,
            c0: CoordinateFrame {
                position: [0.0; 3],
                rotation,
            },
            c1: CoordinateFrame {
                position: c1,
                rotation,
            },
            face: [[0.0; 3]; 4],
            base_angle: 0.0,
            current_angle: 0.0,
            desired_angle: 0.0,
            max_velocity: 0.0,
        });
    };
    let block = [1.0, 1.0, 1.0];
    let pivot = add(&mut workspace, "Pivot", block, [-20.0, 12.0, 10.0], true);
    let arm = add(
        &mut workspace,
        "Arm",
        [6.0, 0.4, 1.0],
        [-17.5, 12.0, 11.1],
        false,
    );
    hinge(
        &mut workspace,
        ROTATE,
        pivot,
        arm,
        [-2.5, 0.0, -1.1],
        IDENTITY,
    );
    let hook = add(&mut workspace, "Hook", block, [-30.0, 12.0, 10.0], true);
    let flap = add(
        &mut workspace,
        "Flap",
        [1.0, 4.0, 0.4],
        [-30.0, 10.0, 10.8],
        false,
    );
    hinge(
        &mut workspace,
        ROTATE,
        hook,
        flap,
        [0.0, 2.0, -0.8],
        IDENTITY,
    );
    let axle = add(&mut workspace, "Axle", block, [-40.0, 12.0, 10.0], true);
    workspace.parts[axle].surface_inputs[2] = SurfaceInput {
        input: INPUT_CONSTANT,
        ..Default::default()
    };
    workspace.parts[axle].surface_inputs[2].param_b = 0.1;
    let wheel = add(
        &mut workspace,
        "Wheel",
        [4.0, 4.0, 0.4],
        [-40.0, 12.0, 10.8],
        false,
    );
    hinge(
        &mut workspace,
        ROTATE_V,
        axle,
        wheel,
        [0.0, 0.0, -0.8],
        IDENTITY,
    );
    let motor = add(&mut workspace, "Motor", block, [-50.0, 12.0, 10.0], true);
    workspace.parts[motor].surface_inputs[1] = SurfaceInput {
        input: INPUT_SIN,
        param_a: 0.5,
        param_b: 2.0,
    };
    let paddle = add(
        &mut workspace,
        "Paddle",
        [4.0, 0.4, 1.0],
        [-48.5, 12.8, 10.0],
        false,
    );
    hinge(
        &mut workspace,
        ROTATE_P,
        motor,
        paddle,
        [-1.5, -0.8, 0.0],
        up,
    );
    let names: Vec<&str> = workspace.parts.iter().map(|p| p.name.as_str()).collect();
    let mut report = workspace.joint_tree()?.describe(&names);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let loose = &fixture.world.loose;
    writeln!(
        report,
        "part_hinge_load assemblies={} points={} {} {}",
        loose.parts.len(),
        loose.point_count(),
        loose.joint_report(),
        loose.report()
    )
    .unwrap();
    let loaded = loose.joints.len() == 4
        && loose
            .joints
            .iter()
            .all(|j| j.edge == Some(JointEdge::Awake))
        && loose.point_count() == 16
        && loose.hinge_angle(0).is_none()
        && loose.hinge_angle(2).is_some()
        && loose.hinge_angle(3).is_some();
    let prim = |fixture: &WalkingFixture, name: &str| {
        fixture
            .world
            .loose
            .prims
            .iter()
            .position(|p| p.name == name)
            .ok_or(format!("{name} is not a loose Part"))
    };
    let frame_of = |fixture: &WalkingFixture, name: &str| -> Result<CoordinateFrame, String> {
        fixture.world.loose.prim_frame(prim(fixture, name)?)
    };
    // How far the plank's hinge point (C1) is from the block's center.
    let gap = |fixture: &WalkingFixture, name: &str, c1: [f32; 3], center: [f32; 3]| {
        let f = frame_of(fixture, name)?;
        let p = crate::g3d::frame_multiply(
            &f,
            &CoordinateFrame {
                position: c1,
                rotation: IDENTITY,
            },
        )
        .position;
        Ok::<f32, String>(
            (0..3)
                .map(|k| (p[k] - center[k]).powi(2))
                .sum::<f32>()
                .sqrt(),
        )
    };
    let frames = |fixture: &WalkingFixture| -> String {
        fixture
            .world
            .loose
            .frames()
            .iter()
            .map(|(n, f)| format!("{n}={:?}", f.position))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let steps = 1440u32;
    let mut arm_lowest = 0.0f32;
    let mut arm_tilt = 1.0f32;
    let mut max_gap = [0.0f32; 4];
    let mut wheel_turn = 0.0f64;
    let mut wheel_last = None;
    let mut wheel_turn_at_4s = 0.0f64;
    let mut paddle_range = (f32::INFINITY, f32::NEG_INFINITY);
    let mut paddle_lag = 0.0f32;
    for step in 0..steps {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let a = frame_of(&fixture, "Arm")?.rotation;
        // The Arm's x axis turned about Z.
        arm_lowest = arm_lowest.min(a[1][0].atan2(a[0][0]));
        arm_tilt = arm_tilt.min(a[2][2]);
        let gaps = [
            gap(&fixture, "Arm", [-2.5, 0.0, -1.1], [-20.0, 12.0, 10.0])?,
            gap(&fixture, "Flap", [0.0, 2.0, -0.8], [-30.0, 12.0, 10.0])?,
            gap(&fixture, "Wheel", [0.0, 0.0, -0.8], [-40.0, 12.0, 10.0])?,
            gap(&fixture, "Paddle", [-1.5, -0.8, 0.0], [-50.0, 12.0, 10.0])?,
        ];
        for k in 0..4 {
            max_gap[k] = max_gap[k].max(gaps[k]);
        }
        let loose = &fixture.world.loose;
        if let Some((current, _)) = loose.hinge_angle(2) {
            if let Some(last) = wheel_last {
                wheel_turn += f64::from(crate::kernel_joint::wrap_angle(current, last));
            }
            wheel_last = Some(current);
        }
        if step + 1 == 960 {
            wheel_turn_at_4s = wheel_turn;
        }
        let time = fixture.schedule.game_time;
        if let Some((current, _)) = loose.hinge_angle(3)
            && step >= 960
        {
            paddle_range = (paddle_range.0.min(current), paddle_range.1.max(current));
            let target = 0.5 * (f64::from(time as f32 * 2.0)).sin() as f32;
            paddle_lag = paddle_lag.max((current - target).abs());
        }
        if step < 4 || (step + 1) % 240 == 0 {
            writeln!(
                report,
                "part_hinge_state step={} game_time={time} arm_angle={} gaps={gaps:?} {} {} {}",
                step + 1,
                a[1][0].atan2(a[0][0]),
                frames(&fixture),
                loose.joint_report(),
                loose.report()
            )
            .unwrap();
        }
    }
    let loose = &fixture.world.loose;
    let assembly = |name: &str| -> Result<&crate::loose::LoosePart, String> {
        Ok(&loose.parts[loose.prims[prim(&fixture, name)?].assembly])
    };
    // Turn per second over the last 2 seconds.
    let wheel_rate = (wheel_turn - wheel_turn_at_4s) / 2.0;
    let wheel_spin = assembly("Wheel")?.body.angular_velocity;
    writeln!(
        report,
        "part_hinge_result arm_lowest_angle={arm_lowest} arm_min_z_alignment={arm_tilt} max_gaps={max_gap:?} wheel_turn={wheel_turn} wheel_rate={wheel_rate} wheel_angular_velocity={wheel_spin:?} paddle_range={paddle_range:?} paddle_max_lag={paddle_lag} edges={:?}",
        loose.joints.iter().map(|j| j.edge).collect::<Vec<_>>(),
    )
    .unwrap();
    writeln!(report, "{}", sleep_summary("part_hinge_sleep", loose)).unwrap();
    let flap_part = assembly("Flap")?;
    let checks = [
        ("loaded_in_kernel", loaded),
        ("hinges_held", max_gap.iter().all(|g| *g < 0.05)),
        ("arm_swung", arm_lowest < -1.2 && 0.999 < arm_tilt),
        (
            "flap_asleep",
            flap_part.sleep.sleeping()
                && !flap_part.in_kernel
                && loose.joints[1].edge == Some(JointEdge::Sleeping),
        ),
        ("wheel_turns", (wheel_rate - 3.0).abs() < 0.15),
        (
            "wheel_awake",
            assembly("Wheel")?.in_kernel && loose.joints[2].edge == Some(JointEdge::Awake),
        ),
        (
            "paddle_follows",
            0.4 < paddle_range.1
                && paddle_range.1 < 0.6
                && -0.6 < paddle_range.0
                && paddle_range.0 < -0.4,
        ),
    ];
    let failed: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    if !failed.is_empty() {
        return Err(format!("part hinge failed {failed:?}\n{report}"));
    }
    let _ = (arm, flap, wheel, paddle);
    writeln!(
        report,
        "part_hinge_pass arm=swung flap=asleep wheel_rate={wheel_rate:.3} paddle=follows native_differential=not_tested"
    )
    .unwrap();
    Ok(report)
}

/// Motors (MotorJoint) in a Workspace, with the character idling at the
/// origin. Each "tower" is a 2 x 1 x 2 base resting on the Baseplate with a
/// 4 x 0.4 x 0.4 arm lying on top, on a Motor about the vertical axis
/// through the base's top face; DesiredAngle pi/2, MaxVelocity 0.05 per
/// frame. In "Tower" Part0 is the base; in "Tower2" Part0 is the arm, so
/// the Body tree makes Part1 the parent and the link turns the base-side
/// frame (00b38196), the same way in the world. "Tower3" is "Tower" on a
/// Motor6D (D6Link) instead.
pub fn audit_part_motor(bundle: &Bundle) -> Result<String, String> {
    use crate::joints::{MOTOR, MOTOR_6D};
    use crate::workspace::{JointInstance, PartInstance};
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // z along +Y (columns x, -z, y).
    let up = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    let add = |workspace: &mut Workspace, name: &str, size: [f32; 3], position| {
        let mut part = PartInstance {
            name: name.into(),
            frame: CoordinateFrame {
                position,
                rotation: IDENTITY,
            },
            ..Default::default()
        };
        part.set_size(size);
        workspace.parts.push(part);
        workspace.parts.len() - 1
    };
    let base_side = CoordinateFrame {
        position: [0.0, 0.5, 0.0],
        rotation: up,
    };
    let arm_side = CoordinateFrame {
        position: [-2.0, -0.2, 0.0],
        rotation: up,
    };
    let desired = std::f32::consts::FRAC_PI_2;
    let motor = |workspace: &mut Workspace, kind, name: &str, part0, part1, c0, c1| {
        workspace.joints.push(JointInstance {
            name: name.into(),
            kind,
            part0,
            part1,
            c0,
            c1,
            face: [[0.0; 3]; 4],
            base_angle: 0.0,
            current_angle: 0.0,
            desired_angle: desired,
            max_velocity: 0.05,
        });
    };
    let base = add(&mut workspace, "Tower", [2.0, 1.0, 2.0], [-20.0, 0.5, 20.0]);
    let arm = add(&mut workspace, "Arm", [4.0, 0.4, 0.4], [-18.0, 1.2, 20.0]);
    motor(
        &mut workspace,
        MOTOR,
        "Motor",
        base,
        arm,
        base_side.clone(),
        arm_side.clone(),
    );
    let base2 = add(
        &mut workspace,
        "Tower2",
        [2.0, 1.0, 2.0],
        [-30.0, 0.5, 20.0],
    );
    let arm2 = add(&mut workspace, "Arm2", [4.0, 0.4, 0.4], [-28.0, 1.2, 20.0]);
    motor(
        &mut workspace,
        MOTOR,
        "Motor2",
        arm2,
        base2,
        arm_side.clone(),
        base_side.clone(),
    );
    let base3 = add(
        &mut workspace,
        "Tower3",
        [2.0, 1.0, 2.0],
        [-40.0, 0.5, 20.0],
    );
    let arm3 = add(&mut workspace, "Arm3", [4.0, 0.4, 0.4], [-38.0, 1.2, 20.0]);
    motor(
        &mut workspace,
        MOTOR_6D,
        "Motor6D",
        base3,
        arm3,
        base_side,
        arm_side,
    );
    let names: Vec<&str> = workspace.parts.iter().map(|p| p.name.as_str()).collect();
    let mut report = workspace.joint_tree()?.describe(&names);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let centers = |fixture: &WalkingFixture| -> Vec<[f32; 3]> {
        fixture
            .world
            .loose
            .parts
            .iter()
            .map(|p| p.body.center)
            .collect()
    };
    let loose = &fixture.world.loose;
    let start_centers = centers(&fixture);
    writeln!(
        report,
        "part_motor_load assemblies={} centers={start_centers:?} {}",
        loose.parts.len(),
        loose.report()
    )
    .unwrap();
    let loaded = loose.parts.len() == 3 && loose.motor(1).is_some() && loose.motor_6d(2).is_some();
    let prim = |fixture: &WalkingFixture, name: &str| {
        fixture
            .world
            .loose
            .prims
            .iter()
            .position(|p| p.name == name)
            .ok_or(format!("{name} is not a loose Part"))
    };
    // The arm's center in its base's frame.
    let relative = |fixture: &WalkingFixture, base: &str, arm: &str| -> Result<[f32; 3], String> {
        let loose = &fixture.world.loose;
        let b = loose.prim_frame(prim(fixture, base)?)?;
        let a = loose.prim_frame(prim(fixture, arm)?)?;
        Ok(crate::g3d::frame_multiply(&crate::g3d::frame_inverse(&b), &a).position)
    };
    let angle = |loose: &crate::loose::LooseWorld, m: usize| -> Result<f32, String> {
        match (loose.motor(m), loose.motor_6d(m)) {
            (Some(j), _) => Ok(j.current_angle),
            (_, Some(j)) => Ok(j.current_angle),
            _ => Err("missing motor".into()),
        }
    };
    let mut reached = [None; 3];
    for step in 0..960u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        let loose = &fixture.world.loose;
        for (m, at) in reached.iter_mut().enumerate() {
            let angle = angle(loose, m)?;
            if at.is_none() && angle == desired {
                *at = Some(step + 1);
            }
        }
        if step < 4 || (step + 1) % 32 == 0 && step < 160 || (step + 1) % 240 == 0 {
            writeln!(
                report,
                "part_motor_state step={} angles=[{}, {}, {}] arm={:?} arm2={:?} arm3={:?} centers={:?} {}",
                step + 1,
                angle(loose, 0)?,
                angle(loose, 1)?,
                angle(loose, 2)?,
                relative(&fixture, "Tower", "Arm")?,
                relative(&fixture, "Tower2", "Arm2")?,
                relative(&fixture, "Tower3", "Arm3")?,
                centers(&fixture),
                loose.report()
            )
            .unwrap();
        }
    }
    let loose = &fixture.world.loose;
    let arm_at = relative(&fixture, "Tower", "Arm")?;
    let arm2_at = relative(&fixture, "Tower2", "Arm2")?;
    let arm3_at = relative(&fixture, "Tower3", "Arm3")?;
    let tower = loose.prim_frame(prim(&fixture, "Tower")?)?;
    let end_centers = centers(&fixture);
    writeln!(
        report,
        "part_motor_result reached={reached:?} arm={arm_at:?} arm2={arm2_at:?} arm3={arm3_at:?} tower={:?} tower_up_y={} centers={end_centers:?}",
        tower.position, tower.rotation[1][1],
    )
    .unwrap();
    writeln!(report, "{}", sleep_summary("part_motor_sleep", loose)).unwrap();
    let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.01);
    let checks = [
        ("loaded", loaded),
        // pi/2 at 0.05 a frame: 31 frames of 0.05, then the rest; four
        // world steps a frame.
        (
            "reached_in_32_frames",
            reached.iter().all(|r| r.is_some_and(|s| s <= 32 * 4)),
        ),
        // A quarter turn about +Y takes +X to -Z.
        ("arm_turned", near(arm_at, [0.0, 0.7, -2.0])),
        ("arm2_turned_the_same_way", near(arm2_at, [0.0, 0.7, -2.0])),
        (
            "arm3_turned_on_the_motor_6d",
            near(arm3_at, [0.0, 0.7, -2.0]),
        ),
        (
            "tower_standing",
            (tower.position[1] - 0.5).abs() < 0.05 && 0.999 < tower.rotation[1][1],
        ),
        (
            "center_moved",
            end_centers[0][2] < start_centers[0][2] - 0.2
                && end_centers[0][0] < start_centers[0][0] - 0.2,
        ),
        (
            "asleep",
            loose
                .parts
                .iter()
                .all(|p| p.sleep.sleeping() && !p.in_kernel),
        ),
    ];
    let failed: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    if !failed.is_empty() {
        return Err(format!("part motor failed {failed:?}\n{report}"));
    }
    let _ = (arm, base2, arm3);
    writeln!(
        report,
        "part_motor_pass reached={reached:?} arms_turned=true asleep=true native_differential=not_tested"
    )
    .unwrap();
    Ok(report)
}

/// The character walks -Z into an unanchored 4 x 6 x 2 "Crate" resting on
/// the Baseplate (its +Z face at z = -7, tall enough to meet the Head, whose
/// touch test ends RunningNoPhysics).
/// Their contacts are realTimeConnectors (the character cannot throttle);
/// the Crate is logged through its kernel lists while it is pushed.
pub fn audit_part_push(bundle: &Bundle) -> Result<String, String> {
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let mut crate_part = crate::workspace::PartInstance {
        name: "Crate".into(),
        ..Default::default()
    };
    crate_part.set_size([4.0, 6.0, 2.0]);
    crate_part.frame.position = [0.0, 3.05, -8.0];
    workspace.parts.push(crate_part);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let mut report = String::new();
    for _ in 0..720u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
    }
    let settled = fixture.world.loose.parts[0].body.frame.position;
    writeln!(
        report,
        "part_push_settled root={:?} {}",
        fixture.world.body.frame.position,
        fixture.world.loose.report()
    )
    .unwrap();
    let mut first_contact = None;
    let mut lists = Vec::new();
    let mut most = 0;
    for step in 0..960u32 {
        let input = if step < 720 {
            [0.0, 0.0, -1.0]
        } else {
            [0.0; 3]
        };
        fixture.step_animated(&mut rig, input)?;
        let touching = fixture.world.loose.character_connector_count();
        most = most.max(touching);
        let list = fixture.world.loose.parts[0].list;
        if !lists.contains(&list) {
            lists.push(list);
        }
        if touching > 0 && first_contact.is_none() {
            first_contact = Some(step + 1);
            writeln!(
                report,
                "part_push_first_contact step={} root={:?} velocity={:?} {}",
                step + 1,
                fixture.world.body.frame.position,
                fixture.world.body.velocity,
                fixture.world.loose.report()
            )
            .unwrap();
        }
        if step % 60 == 59 {
            writeln!(
                report,
                "part_push_walk step={} root={:?} velocity={:?} character_connectors={touching} state={:?} {}",
                step + 1,
                fixture.world.body.frame.position,
                fixture.world.body.velocity,
                fixture.state_type(),
                fixture.world.loose.report()
            )
            .unwrap();
        }
    }
    let part = &fixture.world.loose.parts[0];
    let moved: [f32; 3] = std::array::from_fn(|a| part.body.frame.position[a] - settled[a]);
    writeln!(
        report,
        "part_push_result first_contact_step={first_contact:?} max_character_connectors={most} lists={lists:?} crate_moved={moved:?} crate_up_y={} root={:?} upright_y={} max_penetration={} state={:?}",
        part.body.frame.rotation[1][1],
        fixture.world.body.frame.position,
        fixture.world.body.frame.rotation[1][1],
        fixture.world.max_penetration,
        fixture.state_type()
    )
    .unwrap();
    // Mechanism checks only: the Head touch ends RunningNoPhysics, the
    // character's contacts with the Crate are realtime connectors that move
    // it out of contactBodies, and the Crate is pushed along the path.
    use crate::loose::BodyList;
    if first_contact.is_none()
        || !lists
            .iter()
            .any(|l| matches!(l, BodyList::RealTime | BodyList::Joint))
        || moved[2] > -1.0
        || fixture.world.max_penetration > 0.5
        || fixture.world.body.frame.rotation[1][1] < 0.99
    {
        return Err(format!("part push failed\n{report}"));
    }
    writeln!(
        report,
        "{}",
        sleep_summary("part_push_sleep", &fixture.world.loose)
    )
    .unwrap();
    let destroyed = fixture
        .world
        .broadphase
        .log
        .iter()
        .filter(|(_, e)| matches!(e, crate::broadphase::Event::Destroy(..)))
        .count();
    writeln!(
        report,
        "{} destroyed={destroyed}",
        broadphase_summary("part_push_broadphase", &fixture.world)
    )
    .unwrap();
    // PopperCam's ray (World::camera_raycast) sees the loose Crate: a ray
    // from 10 studs behind its centre stops on its +z face. The static-only
    // query passes through it.
    let centre = fixture.world.loose.parts[0].body.frame.position;
    let part = &fixture.world.loose.prims[0];
    let from = [centre[0], centre[1], centre[2] + 10.0];
    let camera = fixture.world.camera_raycast(from, [0.0, 0.0, -1.0], 20.0);
    let statics = fixture.world.statics.raycast(from, [0.0, 0.0, -1.0], 20.0);
    writeln!(
        report,
        "part_push_camera_ray from={from:?} hit={camera:?} static_only={statics:?} crate_face_z={} transparency={} can_collide={}",
        centre[2] + part.size[2] * 0.5,
        part.transparency(),
        part.can_collide()
    )
    .unwrap();
    if camera.is_none_or(|(point, _)| (point[2] - (centre[2] + part.size[2] * 0.5)).abs() > 0.05) {
        return Err(format!("camera ray missed the Crate\n{report}"));
    }
    writeln!(report, "part_push_pass pushed=true camera_ray=static_and_loose connectors=realtime floor_raycast=static_and_loose sleep=sleep_stage broadphase=spatial_hash native_differential=not_tested").unwrap();
    Ok(report)
}

/// The character on unanchored Parts: it falls onto a light loose "Pad"
/// (lighter than the character, so RunningBase uses the Pad's mass), stands
/// on it, walks off and up onto a heavier loose "Step" 1 stud high, and idles
/// there into RunningNoPhysics. Logs the floor Part, heights and how far the
/// Parts were pushed.
/// SleepStage assembly state changes per Part: (world step, state), and
/// the final state.
fn sleep_summary(label: &str, loose: &crate::loose::LooseWorld) -> String {
    let mut out = format!("{label} allow_sleep={}", loose.allow_sleep);
    for (i, p) in loose.parts.iter().enumerate() {
        let changes: Vec<(u32, crate::sleep::AssemblyState)> = loose
            .transitions
            .iter()
            .filter(|(_, node, _)| *node == crate::loose::Node::Loose(i))
            .map(|(step, _, state)| (*step, *state))
            .collect();
        out.push_str(&format!(
            " [{} final={:?} in_kernel={} changes={changes:?}]",
            p.name, p.sleep, p.in_kernel
        ));
    }
    out.push_str(&format!(" edges={:?}", loose.edge_counts()));
    out
}

pub fn audit_part_stand(bundle: &Bundle) -> Result<String, String> {
    use crate::humanoid_state::FloorPart;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let mut add = |name: &str, size: [f32; 3], position: [f32; 3]| {
        let mut part = crate::workspace::PartInstance {
            name: name.into(),
            ..Default::default()
        };
        part.set_size(size);
        part.frame.position = position;
        workspace.parts.push(part);
    };
    add("Pad", [6.0, 0.3, 6.0], [0.0, 0.15, 0.0]);
    add("Step", [6.0, 1.0, 6.0], [0.0, 0.5, -12.0]);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let mut report = String::new();
    let start: Vec<[f32; 3]> = fixture
        .world
        .loose
        .parts
        .iter()
        .map(|p| p.body.frame.position)
        .collect();
    let character_mass = 1.0 / fixture.world.body.inverse_mass();
    let masses: Vec<f32> = fixture
        .world
        .loose
        .parts
        .iter()
        .map(|p| 1.0 / p.body.inverse_mass())
        .collect();
    if masses[0] >= character_mass {
        return Err("the Pad must be lighter than the character".into());
    }
    writeln!(
        report,
        "part_stand_start character_mass={character_mass} part_masses={masses:?} root={:?}",
        fixture.world.body.frame.position
    )
    .unwrap();
    let line = |label: &str, step: u32, f: &WalkingFixture| {
        format!(
            "{label} step={step} root={:?} velocity={:?} state={:?} floor={:?} floor_velocity={:?} support_height={} character_connectors={} {}",
            f.world.body.frame.position,
            f.world.body.velocity,
            f.state_type(),
            f.humanoid.floor_part(),
            f.humanoid.floor_velocity(),
            f.humanoid.support_height(),
            f.world.loose.character_connector_count(),
            f.world.loose.report()
        )
    };
    // 1. Fall onto the Pad (the spawn is far above it) and stand 3 s.
    let mut pad_floor_steps = 0u32;
    let mut landed = None;
    let mut fell = false;
    for step in 0..900u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        if fixture.humanoid.floor_part() == Some(FloorPart::Loose(0)) {
            pad_floor_steps += 1;
        }
        fell |= fixture.state_type() == StateType::Freefall;
        if fell && landed.is_none() && fixture.state_type() != StateType::Freefall {
            landed = Some(step + 1);
            writeln!(report, "{}", line("part_stand_landed", step + 1, &fixture)).unwrap();
        }
        if landed.is_some_and(|l| step + 1 >= l + 180) {
            break;
        }
        if step % 30 == 29 && landed.is_some() {
            writeln!(report, "{}", line("part_stand_pad", step + 1, &fixture)).unwrap();
        }
    }
    let on_pad = fixture.world.body.frame.position;
    let pad_top = fixture.world.loose.parts[0].body.frame.position[1] + 0.15;
    // 2. Walk -Z off the Pad and up onto the Step, then stop on it.
    let mut lowest_between = f32::INFINITY;
    let mut step_floor_at = None;
    let mut walked = 0u32;
    while fixture.world.body.frame.position[2] > -12.0 {
        if walked >= 240 {
            return Err(format!("character never reached the Step\n{report}"));
        }
        fixture.step_animated(&mut rig, [0.0, 0.0, -1.0])?;
        walked += 1;
        let root = fixture.world.body.frame.position;
        if root[2] < -4.0 && root[2] > -8.0 {
            lowest_between = lowest_between.min(root[1]);
        }
        if step_floor_at.is_none() && fixture.humanoid.floor_part() == Some(FloorPart::Loose(1)) {
            step_floor_at = Some(walked);
            writeln!(
                report,
                "{}",
                line("part_stand_step_floor", walked, &fixture)
            )
            .unwrap();
        }
        if walked.is_multiple_of(15) {
            writeln!(report, "{}", line("part_stand_walk", walked, &fixture)).unwrap();
        }
    }
    // 3. Idle on the Step.
    for step in 0..180u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        if step % 30 == 29 {
            writeln!(report, "{}", line("part_stand_step", step + 1, &fixture)).unwrap();
        }
    }
    let on_step = fixture.world.body.frame.position;
    let step_top = fixture.world.loose.parts[1].body.frame.position[1] + 0.5;
    let moved: Vec<[f32; 3]> = fixture
        .world
        .loose
        .parts
        .iter()
        .zip(&start)
        .map(|(p, s)| std::array::from_fn(|a| p.body.frame.position[a] - s[a]))
        .collect();
    writeln!(
        report,
        "part_stand_result landed_step={landed:?} pad_floor_steps={pad_floor_steps} root_on_pad={on_pad:?} pad_top={pad_top} lowest_between={lowest_between} step_floor_at={step_floor_at:?} root_on_step={on_step:?} step_top={step_top} parts_moved={moved:?} state={:?} upright_y={} max_penetration={}",
        fixture.state_type(),
        fixture.world.body.frame.rotation[1][1],
        fixture.world.max_penetration
    )
    .unwrap();
    // Mechanism checks: the floor rays report the loose Parts, the support
    // height follows their tops (top + 3 for this rig), and neither Part is
    // shoved away by standing on it.
    let close = |a: f32, b: f32| (a - b).abs() < 0.1;
    if landed.is_none()
        || pad_floor_steps == 0
        || !close(on_pad[1], pad_top + 3.0)
        || !close(lowest_between, 3.0)
        || step_floor_at.is_none()
        || !close(on_step[1], step_top + 3.0)
        || moved
            .iter()
            .any(|m| m[0].abs() > 0.5 || m[2].abs() > 0.5 || m[1].abs() > 0.2)
        || fixture.world.body.frame.rotation[1][1] < 0.99
    {
        return Err(format!("part stand failed\n{report}"));
    }
    writeln!(
        report,
        "{}",
        sleep_summary("part_stand_sleep", &fixture.world.loose)
    )
    .unwrap();
    // Standing kinematically on a sleeping Part leaves it asleep.
    if fixture
        .world
        .loose
        .parts
        .iter()
        .any(|p| !p.sleep.sleeping())
    {
        return Err(format!(
            "part stand: the Parts are not asleep at the end\n{report}"
        ));
    }
    writeln!(report, "part_stand_pass floor_raycast=static_and_loose floor_velocity=c02c28 floor_reaction=running_and_jump sleep=sleep_stage native_differential=not_tested").unwrap();
    Ok(report)
}

/// The Jumping force (00c05e0e) on a floor: the character lands on a light
/// loose Pad, idles, and jumps; then the same jump from the Baseplate. Every
/// push is logged (`Humanoid::jump_pushes`) and checked against the native
/// arithmetic: force = (JumpPower - v . d) * 500 * min(root mass, floor
/// assembly mass), applied only while it exceeds the root's existing y force,
/// with force - existing pushed down on the floor at the touch point.
/// The Pad is asleep when the jump starts: the first pushes gather on its
/// SimBody outside the kernel and are dropped when it wakes (00b7439e).
pub fn audit_part_jump(bundle: &Bundle) -> Result<String, String> {
    use crate::humanoid_state::FloorPart;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut report = String::new();
    let mut results = Vec::new();
    for pad in [true, false] {
        let label = if pad { "pad" } else { "baseplate" };
        let mut rig = R6Rig::character(bundle, &scene)?;
        let mut workspace = Workspace::studio_new_document();
        if pad {
            let mut part = crate::workspace::PartInstance {
                name: "Pad".into(),
                ..Default::default()
            };
            part.set_size([6.0, 0.3, 6.0]);
            part.frame.position = [0.0, 0.15, 0.0];
            workspace.parts.push(part);
        }
        let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
        let root_mass = 1.0 / fixture.world.body.inverse_mass();
        let pad_mass = fixture
            .world
            .loose
            .parts
            .first()
            .map(|p| 1.0 / p.body.inverse_mass());
        // 1. Fall, land and idle until 240 steps after landing (the Pad
        // falls asleep at landing + about 120).
        let mut fell = false;
        let mut landed = None;
        let mut step = 0u32;
        while landed.is_none_or(|l| step < l + 240) {
            if step >= 1200 {
                return Err(format!("part jump ({label}): never landed\n{report}"));
            }
            fixture.step_animated(&mut rig, [0.0; 3])?;
            step += 1;
            fell |= fixture.state_type() == StateType::Freefall;
            if fell && landed.is_none() && fixture.state_type() != StateType::Freefall {
                landed = Some(step);
            }
        }
        let rest_y = fixture.world.body.frame.position[1];
        let floor = fixture.humanoid.floor_part();
        let pad_before = fixture
            .world
            .loose
            .parts
            .first()
            .map(|p| (p.sleep, p.body.frame.position[1]));
        writeln!(
            report,
            "part_jump_{label}_rest step={step} root_y={rest_y} floor={floor:?} root_mass={root_mass} pad_mass={pad_mass:?} pad={pad_before:?}"
        )
        .unwrap();
        // 2. Jump: hold the key for one 60 Hz frame (4 world steps), as the
        // walking audit does, then run 120 world steps.
        fixture.humanoid.jump_pushes.clear();
        fixture.set_jump(true);
        let mut apex = rest_y;
        let mut pad_lowest_vy = 0.0f32;
        let woke_from = fixture.world.loose.transitions.len();
        for frame in 0..120u32 {
            if frame == 4 {
                fixture.set_jump(false);
            }
            fixture.step_animated(&mut rig, [0.0; 3])?;
            apex = apex.max(fixture.world.body.frame.position[1]);
            if let Some(p) = fixture.world.loose.parts.first() {
                pad_lowest_vy = pad_lowest_vy.min(p.body.velocity[1]);
            }
        }
        let pushes = fixture.humanoid.jump_pushes.clone();
        for (i, p) in pushes.iter().enumerate() {
            writeln!(
                report,
                "part_jump_{label}_push index={i} floor={:?} acceleration={} root_mass={} mass={} force={} existing={} reaction={:?}",
                p.floor, p.acceleration, p.root_mass, p.mass, p.force, p.existing, p.reaction
            )
            .unwrap();
        }
        let pad_after = fixture
            .world
            .loose
            .parts
            .first()
            .map(|p| (p.sleep, p.body.frame.position[1]));
        writeln!(
            report,
            "part_jump_{label}_result pushes={} rise={} pad_lowest_vy={pad_lowest_vy} pad_after={pad_after:?} state={:?}",
            pushes.len(),
            apex - rest_y,
            fixture.state_type()
        )
        .unwrap();
        if pad {
            writeln!(
                report,
                "{}",
                sleep_summary("part_jump_pad_sleep", &fixture.world.loose)
            )
            .unwrap();
            let asleep = pad_before.is_some_and(|(s, _)| s.sleeping());
            let woke = fixture.world.loose.transitions[woke_from..]
                .iter()
                .any(|t| t.2 == crate::sleep::AssemblyState::Awake);
            if !asleep || !woke || pad_lowest_vy >= 0.0 {
                return Err(format!(
                    "part jump: the Pad does not sleep, wake and sink under the jump\n{report}"
                ));
            }
        }
        // The native arithmetic, bit for bit.
        let expected_floor = if pad {
            Some(FloorPart::Loose(0))
        } else {
            Some(FloorPart::Static(0))
        };
        let expected_mass = match pad_mass {
            Some(m) if m < root_mass => m,
            _ => root_mass,
        };
        let floor_pushes: Vec<_> = pushes.iter().filter(|p| p.floor.is_some()).collect();
        if floor != expected_floor
            || floor_pushes.is_empty()
            || floor_pushes.iter().any(|p| {
                p.floor != expected_floor
                    || p.mass != expected_mass
                    || p.force != p.acceleration * expected_mass
                    || p.existing >= p.force
                    || p.reaction != Some(p.force - p.existing)
            })
        {
            return Err(format!(
                "part jump ({label}): pushes differ from 00c05e0e\n{report}"
            ));
        }
        results.push((pushes.len(), apex - rest_y));
    }
    // Logged only: both jumps take 228 pushes. (Before the wake reset
    // the Pad took more, from a stale kick when it woke.)
    writeln!(
        report,
        "part_jump_compare pad_pushes={} baseplate_pushes={} pad_rise={} baseplate_rise={}",
        results[0].0, results[1].0, results[0].1, results[1].1
    )
    .unwrap();
    writeln!(report, "part_jump_pass floor_mass=min(root,floor) reaction=force_minus_existing native_differential=not_tested").unwrap();
    Ok(report)
}

/// The steep-floor rules on a 35-degree anchored slope, once with the
/// default MaxSlopeAngle (89) and once with MaxSlopeAngle = 30 (a script
/// can set it):
/// - RunningBase 00c0a498 / MovingNoPhysicsBase 00c07626 rewrite the desired
///   velocity when up . n < cos(MaxSlopeAngle): its normal component goes and
///   its downhill component becomes WalkSpeed (`Humanoid::steep_floors`);
/// - the jump event (00c0177e) is refused in Running / RunningNoPhysics when
///   up . n <= cos(MaxSlopeAngle).
pub fn audit_humanoid_slope(bundle: &Bundle) -> Result<String, String> {
    use crate::humanoid_state::FloorPart;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut report = String::new();
    let angle = 35.0f32.to_radians();
    let (s, c) = angle.sin_cos();
    for max_slope in [89.0f32, 30.0] {
        let mut rig = R6Rig::character(bundle, &scene)?;
        let mut workspace = Workspace::studio_new_document();
        let mut slope = crate::workspace::PartInstance {
            name: "Slope".into(),
            ..Default::default()
        };
        slope.set_size([24.0, 2.0, 24.0]);
        slope.anchored = true;
        slope.frame = crate::scene::CoordinateFrame {
            position: [0.0, 6.0, 0.0],
            // About X: the top face normal is (0, cos, sin), downhill +z.
            rotation: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        };
        workspace.parts.push(slope);
        let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
        let slope_part = FloorPart::Static(fixture.world.statics.blocks.len() - 1);
        fixture.humanoid.properties.max_slope_angle = max_slope;
        let walk_speed = fixture.humanoid.properties.walk_speed;
        // 1. Fall onto the slope.
        let mut fell = false;
        let mut landed = None;
        for step in 1..=1200u32 {
            fixture.step_animated(&mut rig, [0.0; 3])?;
            fell |= fixture.state_type() == StateType::Freefall;
            if fell && fixture.state_type() != StateType::Freefall {
                landed = Some(step);
                break;
            }
        }
        let Some(landed) = landed else {
            return Err(format!("slope {max_slope}: never landed\n{report}"));
        };
        let at_landing = fixture.world.body.frame.position;
        writeln!(
            report,
            "humanoid_slope_{max_slope}_landed step={landed} root={at_landing:?} floor={:?} state={:?}",
            fixture.humanoid.floor_part(),
            fixture.state_type()
        )
        .unwrap();
        // 2. Idle 8 steps, hold jump for one 60 Hz frame, run 40 more.
        let transitions_before = fixture.humanoid.transitions.len();
        let mut slope_steps = 0u32;
        let mut jump_on_slope = false;
        for step in 0..52u32 {
            fixture.set_jump((8..12).contains(&step));
            fixture.step_animated(&mut rig, [0.0; 3])?;
            let on_slope = fixture.humanoid.floor_part() == Some(slope_part);
            slope_steps += u32::from(on_slope);
            if step % 4 == 3 {
                writeln!(
                    report,
                    "humanoid_slope_{max_slope}_step step={} root={:?} velocity={:?} state={:?} floor={:?}",
                    step + 1,
                    fixture.world.body.frame.position,
                    fixture.world.body.velocity,
                    fixture.state_type(),
                    fixture.humanoid.floor_part()
                )
                .unwrap();
            }
            jump_on_slope |= on_slope && fixture.state_type() == StateType::Jumping;
        }
        let jumped = fixture.humanoid.transitions[transitions_before..]
            .iter()
            .any(|(_, s)| *s == StateType::Jumping);
        let steep = fixture.humanoid.steep_floors.clone();
        for (i, f) in steep.iter().enumerate().take(3) {
            writeln!(
                report,
                "humanoid_slope_{max_slope}_steep index={i} normal={:?} d={} limit={} before={:?} after={:?}",
                f.normal, f.d, f.limit, f.before, f.after
            )
            .unwrap();
        }
        // The rewritten velocity has no normal part and WalkSpeed downhill.
        let mut worst = [0.0f32; 2];
        for f in &steep {
            let n = f.normal;
            let t = [
                -(0.0 - n[0] * f.d),
                -(1.0 - n[1] * f.d),
                -(0.0 - n[2] * f.d),
            ];
            let length = dot3(t, t).sqrt();
            let t = t.map(|v| v / length);
            worst[0] = worst[0].max(dot3(f.after, n).abs());
            worst[1] = worst[1].max((dot3(f.after, t) - walk_speed).abs());
        }
        writeln!(
            report,
            "humanoid_slope_{max_slope}_result slope_steps={slope_steps} steep_rewrites={} jumped={jumped} worst_normal_speed={} worst_downhill_error={} moved={:?} state={:?}",
            steep.len(),
            worst[0],
            worst[1],
            std::array::from_fn::<f32, 3, _>(|a| fixture.world.body.frame.position[a] - at_landing[a]),
            fixture.state_type()
        )
        .unwrap();
        let ok = if max_slope > 35.0 {
            steep.is_empty() && jumped
        } else {
            !steep.is_empty()
                && !jump_on_slope
                && worst[0] < 1e-3
                && worst[1] < 1e-3
                && slope_steps > 0
        };
        if !ok {
            return Err(format!(
                "slope {max_slope}: steep-floor rules failed\n{report}"
            ));
        }
    }
    writeln!(report, "humanoid_slope_pass walkable_jumps=true steep_rewrite=walk_speed_downhill steep_jump_refused=true native_differential=not_tested").unwrap();
    Ok(report)
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}

/// One line on the broadphase: contact count, event count and the pairs.
fn broadphase_summary(label: &str, world: &crate::world::World) -> String {
    let pairs: Vec<(String, String)> = world
        .broadphase
        .contact_pairs()
        .into_iter()
        .map(|(a, b)| (world.broadphase_name(a), world.broadphase_name(b)))
        .collect();
    format!(
        "{label} prims={} contacts={} events={} pairs={pairs:?}",
        world.broadphase.prims.len(),
        world.broadphase.hash.contact_count(),
        world.broadphase.log.len()
    )
}

/// The broadphase (SpatialHashMultiRes, `broadphase.rs`) on the part-drop
/// scene, run alongside the physics: every primitive's level and cells, the
/// contact events, and two consistency checks at the end:
/// - every pair whose extents overlap has a contact (unless createContact
///   refuses it: same assembly, both anchored, CanCollide off);
/// - every contact's cell boxes overlap at the coarser level.
pub fn audit_broadphase(bundle: &Bundle) -> Result<String, String> {
    use crate::broadphase::Event;
    use crate::world::BroadPrim;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let mut workspace = Workspace::studio_new_document();
    let part = |name: &str, position: [f32; 3]| {
        let mut p = crate::workspace::PartInstance {
            name: name.into(),
            ..Default::default()
        };
        p.frame.position = position;
        p
    };
    workspace.parts.extend([
        part("Rest", [-20.0, 0.65, -20.0]),
        part("Stack", [-19.5, 1.9, -19.8]),
        part("Drop", [20.0, 8.0, 0.0]),
    ]);
    let mut fixture = WalkingFixture::in_workspace(bundle, &scene, &rig, &workspace)?;
    let mut report = String::new();
    let world = &fixture.world;
    for (id, prim) in world.broadphase.prims.iter().enumerate() {
        writeln!(
            report,
            "broadphase_prim id={id} prim={prim:?} name={} level={:?} cells={:?}",
            world.broadphase_name(*prim),
            world.broadphase.hash.level_of(id),
            world.broadphase.hash.cells_of(id)
        )
        .unwrap();
    }
    for step in 0..480u32 {
        fixture.step_animated(&mut rig, [0.0; 3])?;
        if (step + 1) % 120 == 0 {
            writeln!(
                report,
                "{}",
                broadphase_summary(
                    &format!("broadphase_step step={}", step + 1),
                    &fixture.world
                )
            )
            .unwrap();
        }
    }
    let world = &fixture.world;
    let name = |id: usize| world.broadphase_name(world.broadphase.prims[id]);
    for (step, event) in &world.broadphase.log {
        let (kind, a, b) = match event {
            Event::Create(a, b) => ("create", *a, *b),
            Event::Destroy(a, b) => ("destroy", *a, *b),
        };
        writeln!(
            report,
            "broadphase_event step={step} {kind} first={} second={}",
            name(a),
            name(b)
        )
        .unwrap();
    }
    // Checks.
    let prims = &world.broadphase.prims;
    let collides = |p: BroadPrim| match p {
        BroadPrim::Static(_) => true,
        BroadPrim::Loose(i) => world.loose.prims[i].can_collide(),
        BroadPrim::Character(_) => ["Torso", "Head"].contains(&world.broadphase_name(p).as_str()),
    };
    let mut missing = Vec::new();
    let mut inconsistent = Vec::new();
    for a in 0..prims.len() {
        for b in a + 1..prims.len() {
            let same = world.broadphase.same_assembly(a, b);
            let ea = world.broadphase_extents(&rig, prims[a])?;
            let eb = world.broadphase_extents(&rig, prims[b])?;
            let overlap = (0..3).all(|k| ea.min[k] <= eb.max[k] && eb.min[k] <= ea.max[k]);
            let contact = world.broadphase.hash.has_contact(a, b);
            if overlap && !same && collides(prims[a]) && collides(prims[b]) && !contact {
                missing.push((name(a), name(b)));
            }
            if contact {
                let (la, lb) = (
                    world.broadphase.hash.level_of(a).unwrap_or(-1),
                    world.broadphase.hash.level_of(b).unwrap_or(-1),
                );
                let (ca, cb) = (
                    world.broadphase.hash.cells_of(a).ok_or("no cells")?,
                    world.broadphase.hash.cells_of(b).ok_or("no cells")?,
                );
                let level = la.max(lb);
                let shift = |c: crate::broadphase::CellBox, l: i32| {
                    (
                        c.min.map(|v| v >> (level - l)),
                        c.max.map(|v| v >> (level - l)),
                    )
                };
                let (a0, a1) = shift(ca, la);
                let (b0, b1) = shift(cb, lb);
                if !(0..3).all(|k| a0[k] <= b1[k] && b0[k] <= a1[k]) {
                    inconsistent.push((name(a), name(b)));
                }
            }
        }
    }
    let creates_after_start = world
        .broadphase
        .log
        .iter()
        .filter(|(s, e)| *s > 0 && matches!(e, Event::Create(..)))
        .count();
    writeln!(
        report,
        "broadphase_result contacts={} events={} creates_after_start={creates_after_start} missing={missing:?} inconsistent={inconsistent:?} static_first={} loose_pairs={}",
        world.broadphase.hash.contact_count(),
        world.broadphase.log.len(),
        world.broadphase.static_first,
        world.loose.contact_keys().len()
    )
    .unwrap();
    if !missing.is_empty() || !inconsistent.is_empty() || creates_after_start == 0 {
        return Err(format!("broadphase checks failed\n{report}"));
    }
    writeln!(report, "broadphase_pass drives_contacts=true native_order=partly_assumed native_differential=not_tested").unwrap();
    Ok(report)
}
