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
            .statics
            .raycast(from, v.map(|x| x / m), m.min(999.0))
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
        // World::uiStep (Motor6D stepUi once, 00ba8c68/00b39318) and its
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
            let before: Vec<_> = rig.joints.iter().map(|j| j.link_state()).collect();
            for joint in &mut rig.joints {
                joint.step()?;
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
    writeln!(report, "part_wall_pass blocked=true narrowphase=block_block_contact broadphase_order=assumed native_differential=not_tested").unwrap();
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
    writeln!(report, "part_drop_pass rested=4 stacked=true sleep=not_implemented broadphase_order=assumed native_differential=not_tested").unwrap();
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
    writeln!(report, "part_push_pass pushed=true connectors=realtime floor_raycast=static_only sleep=not_implemented broadphase_order=assumed native_differential=not_tested").unwrap();
    Ok(report)
}
