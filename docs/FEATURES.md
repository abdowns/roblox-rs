# Features: what works and what does not

The detailed status list. Each entry names the command that tests it and the
output to expect. Anything not covered by an audit is marked **not tested**.

## What works (tested)

- Studio default world data: Part defaults, the new-document Baseplate and the
  Play Solo spawn point, recovered from the binary. Run
  `sh tools/cargo.sh run --locked -- audit-workspace`; expect
  `workspace_part name="Baseplate" ... size=[512.0, 20.0, 512.0]
  brick_color=199 anchored=true locked=true surfaces=[0, 3, 0, 0, 4, 0]` and
  `spawn_location position=[0.0, 100.0, 0.0]`.

- Play Solo on the Baseplate (headless): the character spawns with its Head
  at (0, 107, 0), falls, lands on the Baseplate through the native floor
  raycast and stands, walks and jumps with the keyframe animations. Run
  `sh tools/cargo.sh run --locked -- audit-baseplate`; expect
  `baseplate_landing landed_step=Some(241) lowest_root_y=3 ...
  animations=["idle", "fall", "idle"]` and `baseplate_pass`. The Baseplate is
  drawn by `viewport-baseplate`; Torso/Head contacts with Parts use the
  BlockBlockContact port (below).

- Native Lighting defaults and day cycle: Roblox's LightingParameters::setTime,
  presetLighting and setFog, recovered from the binary. Run
  `sh tools/cargo.sh run --locked -- audit-lighting`; expect
  `lighting_pass time_of_day=14:00:00` with lamp0 0.675 and ambient 0.5, and
  `lighting_render_pass changed_pixels=6613 repeat_identical=true`. The voxel
  light grid, shadows and fog quality culling are not implemented.

- August 2016 R6 keyframe animations: the shipped Animate script drives a
  port of the native Animator/AnimationTrack with the original clips. Fetch
  the clips once with `python3 tools/fetch_animations.py` (downloads 23 small
  files into `RobloxStudio/assets/`), then run
  `sh tools/cargo.sh run --locked -- audit-animate`; expect `animate_walk
  speed=1.1034483 ... arm_link_rotation` equal to `arm_sample_rotation`, and
  `animate_pass`. Without the fetched clips the character stays in the rest
  pose (like a failed asset load). Native pixel parity is not tested; the Lua
  random seed is assumed.

- Default sky: the original 512px skybox faces, sun and moon billboards
  (Roblox's Sky::render). Run `sh tools/cargo.sh run --locked -- audit-sky`;
  expect `sky_spawn_view ... clear_after=0 repeat_identical=true` and
  `sky_pass faces=6 sun_draws=3 moon=1 skybox_full_coverage=true`. Stars and
  custom Sky instances are not implemented; native pixel parity is not tested.

- Bundled PlayerScripts camera (ClassicCamera, PopperCam, TransparencyController).
  Run `sh tools/cargo.sh run --locked -- audit-camera`; expect `camera_spawn`
  distance 12.5, `camera_relative_walk`, a clamped pitch of -80, the RK4 wheel
  zoom, `camera_popper` with the near plane at the wall and
  `camera_pass classic=true popper=true transparency_fade=true`. Character
  transparency is computed but not rendered; FOV 70 is explicit.
  PopperCam's ray sees static and unanchored Parts, and pierces Parts that are
  CanCollide off or have Transparency above 0.95. Run
  `sh tools/cargo.sh run --locked -- audit-part-push`; expect
  `part_push_camera_ray ... hit=Some(([..., -8.426032], ...)) static_only=None crate_face_z=-8.426032`
  and `part_push_pass pushed=true camera_ray=static_and_loose`. Ties between
  Parts at exactly the same distance are not ordered like native (the spatial
  grid order is not ported).

- Native Humanoid state dispatch: Running, Landed, Jumping, Freefall and
  RunningNoPhysics with the original transition table, floor rays, timers and
  events. Run `sh tools/cargo.sh run --locked -- audit-walking`; expect
  `walking_jump transitions=[...Jumping...Freefall...Landed...Running...]`,
  `walking_drop`, and `walking_state_pass running_no_physics_speed=16`.
  Climbing, FallingDown, Seated, Swimming, slopes and moving floors are not
  implemented. Native jump height is **not tested** (this model rises 8.66 studs).

- Native job scheduling for joints and Lua waits. Run
  `sh tools/cargo.sh run --locked -- audit-scheduler`; expect a
  `scheduler_profile` with physics 60 Hz, dt 0.016666668, heartbeat 30 Hz,
  minimum wait 0.03, then `scheduler_pass seconds=2 physics_jobs=120
  world_steps=480 joint_step_ui=120 heartbeats=60 script_resumes=18`. Wall-clock
  jitter, thread interleaving, throttling and sleeping-joint skips are not modeled.

- Native BlockBlockContact narrowphase (box/box separating axes, face clipping,
  edge connectors, connector reuse). Run
  `sh tools/cargo.sh run --locked -- audit-block-contact`; expect
  `block_contact case=resting colliding=true features=[4, -1] ... connectors=4`,
  four `length=-0.050000` connectors and
  `block_contact_audit resting_ok=true separated_ok=true tilted_ok=true`.
  Torso and Head use it against anchored Parts: run
  `sh tools/cargo.sh run --locked -- audit-part-wall`; expect
  `part_wall_first_contact step=145`, `VertexPlane` connectors and
  `part_wall_result ... stop=[..., -9.51014] ... max_penetration=0.067126274 impulses=0`
  then `part_wall_pass blocked=true`. Native parity is **not tested**.
- Unanchored Parts as their own kernel bodies: free fall, contact bodies with
  native awake impulses, resting flags and contact displacement, Part/Part and
  Part/Baseplate contacts. Run
  `sh tools/cargo.sh run --locked -- audit-part-drop`; expect
  `part_drop_first_contact` lines for Rest (step 7), Stack (10), Drop (67) and
  Tumble (71), `part_drop_result ... heights=[0.58254874, 1.7640274, 0.9883815, 0.58554494] ... speeds=[0.0, 0.0, 0.0, 0.0]`,
  `part_drop_sleep` with every Part `final=SleepingDeeply in_kernel=false`
  (Rest and Stack at world step 120, Tumble 271, Drop 303), and
  `part_drop_pass rested=4 stacked=true asleep=4`. High drops bounce and spin.
  Whether native does the same is **not tested**.
- SleepStage: still Parts leave the kernel (SLEEPING_CHECKING or
  SLEEPING_DEEPLY), contacts follow the native edge states, and touching,
  separating or Humanoid floor events wake them. The `part_*_sleep` lines of
  the three loose audits list each Part's state changes by world step.
  Native parity is **not tested**.
- The character against unanchored Parts:
  - realtime contact springs between Torso/Head and loose Parts;
  - loose jointConnectors springs;
  - the Torso/Head touch test that ends RunningNoPhysics also checks loose
    Parts.

  Run `sh tools/cargo.sh run --locked -- audit-part-push`. Expect:
  - `part_push_first_contact step=101`;
  - `part_push_result ... max_character_connectors=7 lists=[FreeFall, Joint, Contact] crate_moved=[0.02115126, -0.00053167343, -1.4266963]`.
    The Crate is asleep when the character reaches it. The touch wakes it, and
    the character's realtime connector is its first, so 00a9be30 files it under
    jointBodies. With the native floor friction (0.3 on Plastic) the character
    pushes it about 1.43 studs. While the character animates, its Motor6Ds
    wake it recursively every frame (00b37550), and the wake reaches the
    Crate through their touching contact, so the Crate stays awake as long
    as the Torso touches it (to the end of the run);
  - `part_push_broadphase prims=9 contacts=5 ... destroyed=0`: the
    broadphase contacts (Baseplate-Crate, Baseplate-Torso, Baseplate-Head,
    Crate-Torso, Crate-Head);
  - `part_push_pass pushed=true`.

  Native parity is **not tested**.
- The character standing on unanchored Parts:
  - the Humanoid floor and ceiling rays see loose Parts (the HumanoidState
    filter skips only CanCollide-off, dragged and own-assembly Parts);
  - the floor's point velocity and spin are added to the walk (Running and
    RunningNoPhysics);
  - RunningBase's support force uses min(character, floor) mass on a loose
    floor and pushes the floor down with half of it; the jump push does the
    same with the full difference;
  - floors steeper than MaxSlopeAngle get the native downhill rule.

  Run `sh tools/cargo.sh run --locked -- audit-part-stand`. Expect:
  - `part_stand_start character_mass=9.9 part_masses=[7.56, 25.199999]`: the
    Pad is lighter than the character, so the support force uses its mass;
  - `part_stand_landed step=241 ... floor=Some(Loose(0))`: it lands on the Pad;
  - `part_stand_result ... root_on_pad=[..., 3.2040696, ...] pad_top=0.18826233 lowest_between=2.9966283 step_floor_at=Some(156) root_on_step=[..., 3.999967, ...] step_top=0.98490196`:
    it stands at Part top + 3 on both Parts and walks up the 1-stud Step;
  - `part_stand_sleep ... [Pad final=SleepingDeeply ... (239, WakePending), (240, Awake), (367, SleepingDeeply)] [Step final=SleepingDeeply ...]`:
    landing wakes the sleeping Pad, and both Parts sleep again under the idle
    character;
  - `part_stand_pass ... sleep=sleep_stage`.

  The Pad sinks about 0.11 into the Baseplate, settles (y 0.011, 0.037,
  0.044, then 0.0383) and sleeps there. The Humanoid pushes it every
  substep, and the kernel pre-step (00a96eb0) moves any contact body with a
  force into realTimeBodies and has no path back. So the Pad rests as a
  realtime body against its contactConnectors. Whether native sinks the same
  amount is **not tested**. Native parity is **not tested**.

- The Jumping push on a floor (00c05e0e). Run
  `sh tools/cargo.sh run --locked -- audit-part-jump`. Expect:
  - `part_jump_pad_push index=0 floor=Some(Loose(0)) acceleration=25004.637 root_mass=9.9 mass=7.56 force=189035.05 existing=-1942.38 reaction=Some(190977.42)`:
    the force uses min(character, Pad) mass, and the Pad is pushed down with
    force - existing;
  - `part_jump_baseplate_push index=0 floor=Some(Static(0)) ... mass=9.9 force=247542.61`;
  - `part_jump_pad_result pushes=228 rise=8.590376 pad_lowest_vy=-65.589676 ...`:
    the Pad is asleep when the jump starts, so the first pushes gather on its
    SimBody outside the kernel. They are dropped when it wakes (00b7439e);
  - `part_jump_compare pad_pushes=228 baseplate_pushes=228 pad_rise=8.590376 baseplate_rise=8.657764`;
  - `part_jump_pass`.

  Each push is checked against the native arithmetic bit for bit. Native jump
  height and parity are **not tested**.

- Steep floors (RunningBase 00c0a498, MovingNoPhysicsBase 00c07626, jump
  event 00c0177e) on a 35-degree anchored slope. Run
  `sh tools/cargo.sh run --locked -- audit-humanoid-slope`. Expect:
  - `humanoid_slope_89_result ... steep_rewrites=0 jumped=true`: with the
    default MaxSlopeAngle the slope is walkable and the jump fires;
  - `humanoid_slope_30_steep index=0 normal=[0.0, 0.81915206, 0.57357645] d=0.81915206 limit=0.8660254 before=[0.0, 0.0, 0.0] after=[0.0, -9.177222, 13.106433]`:
    with MaxSlopeAngle 30 the desired velocity becomes WalkSpeed (16) straight
    downhill;
  - `humanoid_slope_30_result slope_steps=48 steep_rewrites=13 jumped=false ... moved=[..., -4.304735, 2.3139892]`:
    the jump is refused and the character slides down;
  - `humanoid_slope_pass`.

  Native parity is **not tested**.

- Broadphase (SpatialHashMultiRes, `broadphase.rs`). It decides which Parts
  have a contact: the loose-Part, character-loose and character-static
  contacts exist only while their cell boxes overlap, and are created in its
  order. Run
  `sh tools/cargo.sh run --locked -- audit-broadphase`. Expect:
  - `broadphase_prim id=0 prim=Static(0) name=Baseplate level=Some(2) cells=Some(CellBox { min: [-9, -1, -9], max: [8, 0, 8] })`
    and the bricks at level 0;
  - `broadphase_event step=0 create first=Baseplate second=Stack`, then
    Rest, Drop and `first=Rest second=Stack` (findAllContacts at the start);
  - `broadphase_event step=206 create first=Torso second=Baseplate` and
    `step=209 ... first=Head`: the falling character reaches the
    Baseplate's coarse cells;
  - `broadphase_result contacts=6 events=6 creates_after_start=2 missing=[] inconsistent=[]`
    and `broadphase_pass`.

  Assumed: the order in which assemblies run findAllContacts at the start.
  The character-static block contact always keeps the character Part as
  body 0; `Broadphase::static_first` counts the contacts native would create
  the other way round (0 in the audits). Native parity is **not tested**.

- Joint spanning tree (`joints.rs`): which Part is an assembly's root and
  how welded, snapped and motor-jointed Parts hang from it. Run
  `sh tools/cargo.sh run --locked -- audit-joints`. Expect:
  - the R6 character from the shipped rig: `HumanoidRootPart priority=2001`
    hangs from the ground (`tree_joint_kind=Some(20)`, a FreeJoint), the
    Torso under it, and the Torso's Body children
    `["Right Arm", "Left Arm", "Right Leg", "Left Leg", "Head"]`, then
    `joint_tree_r6 root=HumanoidRootPart parts=7 checks=[]` (the same tree
    and child order the character mass code uses);
  - a 1-stud block welded to a 4x1x4 one: `root=Large`, offset
    `[0.0, -1.0, 0.0]`;
  - a block welded to an anchored base and a block snapped on top: both
    `grounded=true` (the welded block's FreeJoint became an AnchorJoint);
  - a chain A-B-C that re-roots at C, then
    `joint_tree_pass r6_root=HumanoidRootPart pair_root=Large stack_grounded=true chain_root=C`.

  Assumed: GUID order is Part creation order and address ties follow joint
  creation order. Native parity is **not tested**.

- Welded Parts in the physics: Workspace Weld/ManualWeld/Snap joints
  (`Workspace::joints`) group unanchored Parts into one assembly with one
  SimBody (mass, center and inertia over the Body tree). Each Part keeps its
  own block for contacts and follows its parent Body. Assemblies tied to an
  anchored Part never move and become static blocks. Run
  `sh tools/cargo.sh run --locked -- audit-part-weld`. Expect:
  - the tree lines (`joint_tree_part name=Arm ... body_parent=Base ...`,
    `name=Hung ... tree_joint_kind=Some(0) ... grounded=true`), then
    `part_weld_statics ["Baseplate", "Holder", "Hung", "Chain"]`;
  - `part_weld_assembly root=Base prims=["Base", "Arm"] mass=9.52` and
    `root=Plank prims=["Plank", "Weight"] mass=13.999999`;
  - `part_weld_snap arm=[21.5, 4.6, 0.0]` (the Arm, placed 50 studs away,
    sits where the weld puts it);
  - `part_weld_result ... arm_in_base=[1.5000002, 2.5999997, 2.2351742e-8]`
    with every Part's lowest corner above -0.02 and zero speeds;
  - `part_weld_pass assemblies=2 snapped=true landed=true grounded_static=2 rested=true asleep=true native_differential=not_tested`.

  Not ported: motors in the Body tree, and rigid joints made, broken or
  removed while running. A fast impact on a Part whose center
  of mass sits high above the contact gives it a large sideways kick; this
  also happens to a single tall block (see the roadmap). Native parity is
  **not tested**.

- Glue joints in the physics: a Workspace Glue (`joints::GLUE`, face F0..F3
  in `JointInstance::face`) puts four corner Points per side and four
  NormalBreakConnectors in the kernel. It holds two Parts together with
  springs, sleeps with them, and breaks when the pull on a corner passes
  20 x the face strength. Run
  `sh tools/cargo.sh run --locked -- audit-part-glue`. Expect:
  - `joint_tree_part name=Hung priority=401 tree_joint_kind=Some(9) tree_parent=Holder ... grounded=false`
    (glued under an anchored Part, but not grounded);
  - `part_glue_load assemblies=4 points=24 thresholds=[478650.0, 160050.0, 82350.0] [Glue0 edge=Some(Awake) connectors=4 ...`:
    the joints are in the kernel before the first step;
  - `part_glue_broke step=9 broken=[(7, 2)] ...`: the 14-stud cube's 1 x 1
    Glue breaks at the third frame's breakJoints;
  - `part_glue_result hung_y=9.000007 hung_range=(8.99987, 9.000012) pair_gap=2 lowest_left=-0.010354698 lowest_heavy=-0.021046638 broke_at=Some(9) edges=[Some(Sleeping), Some(Sleeping), None]`:
    the hung Part holds within 0.00013 studs, the glued pair lands 2 studs
    apart, the cube lands, and the two intact Glues sleep with their Parts;
  - `part_glue_pass hung=held pair=kept heavy=broke asleep=true native_differential=not_tested`.

  Only Glues present at load. Not ported: ManualGlue, joints to the
  character, and Glue between two anchored Parts (refused). The
  JointInstance removal after a break is assumed to happen at once. Native
  parity is **not tested**.

- Hinge joints in the physics: a Workspace Rotate, RotateP or RotateV
  (`joints::ROTATE`, `ROTATE_P`, `ROTATE_V`) holds both ends of C0's z axis
  with two never-breaking springs, so the Parts turn freely about it.
  RotateP and RotateV add a RotateConnector that drives the angle from the
  axle Part's surface input (`PartInstance::surface_inputs`, Constant or
  Sin) and `JointInstance::base_angle`: RotateP turns to the value, RotateV
  turns at 30 x value rad/s. While the value is not 0 the motor keeps both
  assemblies awake. Run
  `sh tools/cargo.sh run --locked -- audit-part-hinge`. Expect:
  - `part_hinge_load assemblies=4 points=16 [Hinge0 edge=Some(Awake) connectors=2 ...`:
    every hinge is in the kernel before the first step, the two motors with
    their angles at 0;
  - `part_hinge_result arm_lowest_angle=-3.003305 arm_min_z_alignment=0.9999999 max_gaps=[0.00042617292, 0.00026417468, 0.0009584427, 0.00027128975] wheel_turn=18.002966428321088 wheel_rate=2.9988605701364577 ... paddle_range=(-0.5067923, 0.5029547) paddle_max_lag=0.016227335 edges=[Some(Awake), Some(Sleeping), Some(Awake), Some(Awake)]`:
    the level arm swings down and nearly up the other side, every hinge
    point stays within 0.001 studs of its axle, the RotateV wheel (Constant
    0.1) turns at 3 rad/s, and the RotateP paddle (Sin 0.5, 2) stays within
    0.017 rad of 0.5 sin(2t);
  - `part_hinge_sleep ... [Flap final=SleepingDeeply in_kernel=false changes=[(0, Awake), (119, SleepingDeeply)]] ...`:
    the still flap sleeps and its hinge leaves the kernel; the wheel goes
    RECURSIVE_WAKE_PENDING and back every frame without leaving it;
  - `part_hinge_pass arm=swung flap=asleep wheel_rate=2.999 paddle=follows native_differential=not_tested`.

  Only hinges present at load, and surface inputs as loaded (no changes
  while running). Not ported: hinges to the character, the controller
  inputs (they give 0), and hinges between two anchored Parts (refused).
  Native parity is **not tested**.

- Motor joints in the physics: a Workspace Motor or Motor6D
  (`joints::MOTOR`, `MOTOR_6D`, with `JointInstance::current_angle`,
  `desired_angle`, `max_velocity`) is a Body link whose offset turns with
  CurrentAngle (RevoluteLink for Motor, D6Link for Motor6D). Each frame the
  angle moves toward DesiredAngle by at most |MaxVelocity|, the child Part
  follows, the assembly wakes, and its SimBody's center of mass and inertia
  are rebuilt. Run `sh tools/cargo.sh run --locked -- audit-part-motor`.
  Expect:
  - `joint_tree_part name=Arm ... tree_joint_kind=Some(4) tree_parent=Tower body_parent=Tower ... offset=motor7`
    (Arm2 under Tower2, whose Motor has the arm as Part0, and Arm3 under
    Tower3 on a Motor6D, are the same);
  - `part_motor_result reached=[Some(125), Some(125), Some(125)] arm=[1.9073486e-6, 0.70000005, -2.000002] arm2=[1.9073486e-6, 0.6999999, -2.0] arm3=[-3.8146973e-6, 0.6999999, -2.0] tower=[-19.99919, 0.48368213, 19.998695] tower_up_y=0.9999946 ...`:
    all three motors reach pi/2 on frame 32, every arm ends a quarter turn
    from +X to -Z in its tower's frame, the towers stay upright, and the
    center of mass follows the arm;
  - `part_motor_sleep ... [Tower final=SleepingDeeply in_kernel=false ...`:
    the towers wake every frame while their motors turn, then sleep at
    step 239;
  - `part_motor_pass reached=[Some(125), Some(125), Some(125)] arms_turned=true asleep=true native_differential=not_tested`.

  Not ported: VelocityMotor, animating a Workspace motor (the Animator
  drives only the character), and a motor in an anchored assembly
  (refused). Native parity is **not tested**.

- Ball Parts in the physics: a Part with Shape Ball (`set_shape(SHAPE_BALL)`,
  size forced to (x, x, x)), loose or anchored, has the Ball radius, volume,
  moment and extents. It collides through BallBallContact and
  BallBlockContact (`ball_contact.rs`), and the Humanoid ray query hits it as
  a sphere. Run
  `sh tools/cargo.sh run --locked -- audit-part-ball`. Expect:
  - `part_ball_load masses=[("Drop", 2.9321532), ...] radii=[("Drop", Ball, 1.0), ...]`:
    a diameter-2 Plastic ball weighs 4/3 pi x 0.7;
  - `part_ball_first_contact part=Top step=36 contacts=[... [Bottom/Top BallBall length=-0.03341818] ...`,
    `part_ball_first_contact part=Pebble step=32 contacts=[... [static3/Pebble BallBall length=-0.0997417]`
    (the anchored Dome), and BallPlane connectors for balls on the
    Baseplate, the Ramp and the Crate (`Rider/Crate BallPlane`);
  - `part_ball_result ... drop_y=0.9799997 rider_y=2.1593502 crate_y=0.5793512 roll_acceleration=23.315145 expected=24.335554 ratio=0.9580692 spin_ratio=1.0102892 min_gap=2.4665818 ...`:
    - a dropped ball rests on the Baseplate, 0.02 studs in;
    - a ball rests on top of a loose block;
    - a ball rolls down a 10 degree ramp at 0.96 of the rolling-without-slipping
      acceleration 5/7 g sin 10 (sliding would be 1.4), turning at v / r;
    - two balls that land on each other push apart;
    - a small ball dropped off-center on an anchored ball rolls off it and
      lands on the Baseplate (`pebble=[6.904241, 0.49823838, 15.0]`);
  - `part_ball_sleep ... [Drop final=SleepingDeeply ... (191, SleepingDeeply)] ... [Rider final=SleepingDeeply ...`;
  - `part_ball_pass rested=true rides=true rolls=0.958 parted=true rolled_off=true native_differential=not_tested`.

  The narrowphase also has unit tests (`sh tools/cargo.sh test --locked ball`).
  Balls rolling on flat ground slow down: Top goes from 8.5 to 4.1 studs/s
  between steps 300 and 900 (exponential decay at about 0.29/s). The kernel's
  angular damping (0.9998 per pre-step and per impulse) explains only part of
  that. Whether native balls slow the same way is **not tested**. Not
  ported: Cylinders (refused), Balls in the character. Native parity is
  **not tested**.

- Wedge Parts in the physics: a WedgePart (`set_shape(SHAPE_WEDGE)`), loose
  or anchored, has the Wedge mass, center of mass, moment and extents. It
  collides with Blocks and Wedges through PolyPolyContact and with Balls
  through BallPolyContact (`poly_contact.rs`), and the Humanoid ray query
  hits its faces. Run
  `sh tools/cargo.sh run --locked -- audit-part-wedge`. Expect:
  - `part_wedge_load masses=[22.4, 22.4, 2.8] centers_in_body=[[0.0, -0.33333334, 0.6666667], ...]`:
    a 4 x 2 x 4 Plastic Wedge weighs the whole box (x y z x 0.7), with its
    center of mass at (0, -y/6, z/6);
  - `part_wedge_first_contact part=Slider step=8 contacts=[static1/Slider FaceVertex length=-0.024642944]...`
    and `part=Drop step=56 contacts=[static0/Drop FaceVertex length=-0.1519562]...`:
    FaceVertex connectors from PolyPolyContact (static0 is the Baseplate,
    static1 the anchored Slope);
  - `part_wedge_marble_first step=7 contacts=[... [Marble/static1 PolyBallPlane length=-0.00483...]`:
    a Ball on the anchored slope gets a BallPlane connector;
  - `part_wedge_result ... lowest=[-0.020275969, -0.019395053, -0.0157507] speeds=[0.0, 0.0, 0.0] drop=[29.997843, 0.98619145, -30.002726] ... slider=[-29.915707, 0.48694745, 24.166956] slider_slope_steps=150 ray=Some((([-33.0, 2.0, 30.0], [0.0, 0.8944272, -0.4472136]), Static(1))) ramp_steps=148 worst_ramp_height_error=0.07017875 walk_end=[5.068154e-5, 5.4002967, -15.872218] marble_slope_steps=107 marble_end=[-33.0, 0.9841863, -36.22281]`:
    - a dropped Wedge rests flat on the Baseplate, 0.02 studs in;
    - an upside-down Wedge tumbles and comes to rest on its slope;
    - a block set on a 27 degree anchored slope slides off its low end;
    - a ray hits the slope with the slope's normal;
    - the character walks up a 14 degree Wedge ramp, its root 3 studs over
      the slope (within 0.07);
    - a Ball rolls down the slope and off onto the Baseplate;
  - `part_wedge_sleep ... [Drop final=SleepingDeeply ...] [Flip final=SleepingDeeply ...] [Slider final=SleepingDeeply ...]`;
  - `part_wedge_pass rested=true flat=true slid=true hit=true climbed=true rolled=true native_differential=not_tested`.

  Unit tests: `sh tools/cargo.sh test --locked poly` (mesh topology, the
  face tests, the ray hit test, three Poly contact cases, and a Ball
  against a Wedge's face, edge and corner). Not tested: the
  character's Torso or Head touching a Wedge (in the audit they never do),
  a Wedge welded into a multi-Part assembly, and Wedge against Wedge.
  Not ported:
  - CornerWedge and Cylinder.

  Native parity is **not tested**.

- Wedge Parts are drawn by the native Wedge path (010dc290,
  `geometry::workspace_wedge`): the Block's strips, without the Top face,
  with the slope as face 5 and the +X/-X triangles drawn as narrowing
  strips whose studs follow them. Run
  `sh tools/cargo.sh test --locked wedge_draw`: every vertex of a studded
  4 x 10 x 20 Wedge lies on its face of the physics mesh with that face's
  normal. Run `sh tools/cargo.sh run --locked -- viewport-baseplate-smoke`
  and expect, among the draws:
  - `workspace_draw name="WedgeRamp" geometry=wedge quads=6 vertices=24 triangles=12 ... stud_extent=[4.0, 0.75]`;
  - `workspace_draw name="Chute" geometry=wedge quads=5 vertices=20 triangles=10 ...`;
  - `workspace_draw name="Doorstop" geometry=wedge quads=5 ...` and
    `name="Tumbler" geometry=wedge quads=5 ...`.

  The float association of the corner and normal sums is copied from
  `workspace_block`; it is **not checked** against 010dc290's disassembly.
  How it looks is **not tested** (log, don't look). Material UVs, tangents
  and outline distances are not produced, as for blocks.

- Ball Parts are drawn as addSphere's cube sphere (`geometry::workspace_sphere`).
  Run `sh tools/cargo.sh run --locked -- audit-ball-render`. Expect:
  - `ball_mesh name="Boulder" diameter=4 smooth=false tiles=1 rows=6 vertices=294 expected_vertices=294 quads=216 expected_quads=216 radius_error=0.00000023841858 ... least_normal_alignment=0.9999999 inward_triangles=0 distinct_points=Some(218) expected_distinct=218`,
    and the same checks for diameters 1, 20 (Smooth and studded), 400 and a
    tilted 6:
    - `Big` (diameter 20) has `tiles=3 rows=2 vertices=378`;
    - `Huge` (diameter 400, clamped to 350) has `tiles=50 rows=1 vertices=30600 ... quads=15000`;
  - `ball_mesh_studs name="Boulder" ... [-1.7881393e-7, 2.0, 0.0, 0.125] ... [-1.7881393e-7, 2.0, 0.5, 0.6249999] ...`:
    Top and Bottom stud coordinates, matching a 4-stud block's top face;
  - `ball_render_part name="Ball" pixels=3730 bounds=[216, 206, 285, 273] width=70 height=68 fill=0.78361344`
    (a disc fills pi/4 = 0.785 of its box) and
    `ball_render_part name="Cube" pixels=6266 ... fill=0.9933418`;
  - `ball_render_pass round=true repeat_identical=true native_differential=not_tested`.

  The pixel counts depend on the GPU. No viewport shows a Ball yet (the demo
  place has none). Not ported: decals on Balls, material UVs, tangents and
  outline distances (as for blocks). Native parity is **not tested**.

- The character against Ball Parts. A ball 4 studs across stays below the
  Head, so the Torso's touch ends RunningNoPhysics. Run
  `sh tools/cargo.sh run --locked -- audit-part-ball-push`. Expect:
  - `part_ball_push_first_contact step=89 root=[..., 3.2054923, -5.7723727] contacts=[Boulder/static0 BallPlane length=-0.003461957][Boulder/collider1 BallEdge length=-0.23461568]`:
    the Torso's bottom front edge meets the loose Boulder (it went about
    0.23 deep while still kinematic);
  - `part_ball_push_result ... boulder_moved=[-0.106956504, -0.00883019, -39.435963] boulder_velocity=[..., ..., -15.936434] root=[0.009192032, 3.1812725, -44.956074] upright_y=0.9999999 max_penetration=0.24823523 state=Running`:
    the character pushes the ball ahead of it at walking speed;
  - `part_ball_post_result kinds=[BallEdge] closest_root_distance=2.232411 ... state=Running`:
    an anchored ball stops the character, the Torso edge 0.25 deep;
  - `part_ball_push_pass kicked=true blocked=true native_differential=not_tested`.

  Native parity is **not tested**.

- Legacy block vertex/plane contact generation for shared box orientation and
  complete face containment, plus awake impulse response through assembly mass,
  inertia, angular momentum, restitution and friction. Run
  `sh tools/cargo.sh run --locked -- audit-legacy-contact`; expect
  `legacy_contact_pass generated=4`, `momentum_conserved=true` and
  `spring_integrated=true`. Relative rotation, partial face clipping, sleeping,
  broadphase and complete kernel scheduling remain pending.

- Native constrained SimBody substeps and contact spring/friction history. Run
  `sh tools/cargo.sh run --locked -- audit-kernel`; expect
  `kernel_pass parts=7 mass=9.9 constrained_steps=4560`. The diagnostic integrates
  the R6 assembly under an explicit constant force and tests two contact-history
  samples. The walking fixture integrates these paths; general contacts and the
  full native scheduler remain pending.

- Native block mass/inertia and recursive R6 assembly aggregation. Run
  `sh tools/cargo.sh run --locked -- audit-assembly`; expect
  `assembly_pass parts=7 mass_conserved=true`. The explicit builder branch uses
  root mass 0.1 and total mass 9.9. Native block inertia follows its recovered
  surface-weighted polynomial. State-dependent mass overrides and complete
  constrained-body integration remain pending.

- Native contact-cache refresh and angular contact-point velocity math. Run
  `sh tools/cargo.sh run --locked -- audit-collision`; expect
  `contact_pass refresh_cases=5 angular_velocity=true`. This does not yet generate
  collisions or solve impulses in the viewport.

- RunningBase horizontal-force math, with explicit root/floor masses, solver
  selection, velocity and friction inputs. Run
  `sh tools/cargo.sh run --locked -- audit-locomotion`; expect
  `running_force_pass cases=6`. The explicit walking fixture now integrates this
  legacy force calculation with original WalkSpeed and assembly mass.

- Bundled procedural no-tool animation: Standing, Running, Jumping, FreeFall,
  Climbing and Seated. Run `sh tools/cargo.sh run --locked -- audit-animation`;
  expect six poses/360 callbacks with zero pivot error. Run
  `sh tools/cargo.sh run --locked -- audit-animation-render`; expect 60 callbacks,
  3,950 final changed pixels and exact fresh/reset pixel comparisons.
  `sh tools/cargo.sh run --locked -- viewport-animation-smoke` opens the automatic
  one-second animation probe and exits at frame 120. Default keyframe clips,
  script-selection flags and native callback scheduling remain **not tested**.

- Native R6 six-joint construction and the zero-offset Motor6D angle branch. Run
  `sh tools/cargo.sh run --locked -- audit-rig`; expect `rest_error=0`,
  `pivot_error=0` and `reset_identical=true`. The visible free-fall probe now
  integrates one root and poses its linked children. The walking fixture adds
  recovered assembly mass/inertia and wall contact response; full physics remains
  incomplete.

- Native R6 body-mesh selection and scaling: six draws, 727 vertices and 1,066
  triangles. Run `sh tools/cargo.sh run --locked -- audit-render`; expect
  `plastic_render_pass changed_pixels=0` because composited body meshes bypass
  studs. Face rendering changes 120 pixels at 640×480. Full native character
  atlas composition and clothing are **not tested** and remain incomplete.

- Physics frames update base geometry and its face decal. Run
  `sh tools/cargo.sh run --locked -- audit-motion`; expect
  `motion_render_pass steps=60`, `fresh_upload_identical=true`, and
  `reset_pixels_identical=true`. Moved frames change 11,781 pixels at 640×480;
  the moved face contributes 101 pixels. This is explicitly independent,
  unconstrained root motion with linked R6 Parts; walking and contacts use the
  separate experimental fixture.

- Native unconstrained free-fall branch. Run
  `sh tools/cargo.sh run --locked -- audit-physics`; expect
  `free_fall_pass bodies=7 steps=240`. Reads native timing/gravity/damping values
  and original Part velocities, preserves quaternion rules and tiny velocity
  additions. Maximum analytic position error is about 0.000219 studs. This is a
  fixture of independent Parts. R6 joints and horizontal locomotion forces are
  audited separately; complete assembly/contact integration is pending.

- Recovered Humanoid movement command math. Run
  `sh tools/cargo.sh run --locked -- audit-player`; expect the explicit character
  properties (WalkSpeed=16, JumpPower=50) and `player_command_pass updates=7`.
  Numeric checks cover analog strength, normalization, zero input, camera ground
  projection/full rotation, missing camera and detached handling. This does not
  independently simulate locomotion; the walking fixture connects its command
  math to assembly forces. See
  [core evidence](provenance/2016-CORE.md).

- Headless Metal rendering on this M3. Run
  `sh tools/cargo.sh run --locked -- audit-render`; expect
  `render_probe_pass`, `repeat_identical=true`, and `gpu_contract_pass` with
  `bgra_matches_rgba=true`. This renders the static character with explicit probe
  lighting/camera; checks depth and fog numerically; inventories 572 shader records
  across four native packs. Logs and a PPM capture stay in RobloxStudio/analysis.
  It covers opaque Plastic low-quality shading with constant grid/shadow inputs.

- Bundled head face decal: straight-alpha RGBA8 PNG, eight native-rule mip levels,
  trilinear clamp, recovered blending and disabled decal depth writes. The same
  `audit-render` command reports `decal_import_pass draws=1`,
  `decal_render_pass draws=1 changed_pixels=120`, and `gpu_decal_contract_pass`.
  The empty torso decal is skipped. Other decal geometry and image formats are
  not implemented.

- Original Plastic stud atlas and recovered block surface coordinates. The same
  `audit-render` command reports `stud_atlas_loaded width=128 height=2048 mips=12`,
  `gpu_plastic_contract_pass`, and `plastic_render_pass changed_pixels=0` for the corrected R6 body branch.
  DXT1 colors follow the native software fallback; all stored mip levels upload.
  Repeat/trilinear sampling and red-channel modulation pass numeric checks.
  Outlines are disabled for this probe. Original compressed GPU decoding and full
  native pixel parity are **not tested**.

- Recovered static character geometry and palette import. Run
  `sh tools/cargo.sh run --locked -- audit-geometry`; expect
  `native_palette_loaded colors=208` and
  `geometry_pass draws=6 vertices=727 triangles=1066`. Includes native head
  deformation, block face ordering, material specular quantization and camera math.
  See [rendering evidence](provenance/2016-RENDERING.md).

- Exact bundle version, little-endian i386 Mach-O header and executable SHA-256
  validation against the supplied build.
- A mesh v2.00 reader based on the 2016 native loader and writer, including native
  newline/version matching, layout checks, counts, raw float records and faces.
  All 37 bundled meshes decode. See [native evidence](provenance/2016-MESH.md).
- XML v4 instance hierarchy and a typed subset of explicit Part properties.
  The character contains 26 instances, including 14 Attachments, seven Parts,
  one Humanoid, two Decals and one SpecialMesh. Only Part properties are currently
  interpreted; other classes retain identity/hierarchy and property counts.
- Headless Bevy import of mesh records and instance/Part components, retaining
  hierarchy and bitwise Part state through 120 updates.
- Source-only tests and a whole-Contents fingerprint check: 2,606 files/symlinks
  unchanged after analysis and diagnostic execution.

## What does not work / not tested

Window presentation now passes a 120-frame Metal smoke test after the BGRA
format fix. Earlier failures remain recorded in STATUS.md. Pixel equivalence
with the original running client is **not tested**.

No full graphical client, general native geometry dispatch, complete native camera/input,
complete physics, keyframe animation, Lua runtime, Studio ribbon/docks/editing, place saving,
or complete XML/binary-place loader exists yet. Mesh v1.00/v1.01 are not implemented.
General surface subdivisions/UVs, decals/textures, high-quality materials, outlines,
dynamic light grid, shadows, sky stars and postprocessing remain unimplemented. Imported Attachments and
Humanoid do not execute their native behavior. Full native execution, behavioral equivalence and visual equivalence
are **not tested**. Do not treat `static_import_pass` as evidence of those.

The Ghidra MCP now connects to the running CodeBrowser in project studio2016.
Targeted live decompilation succeeds. Earlier broad auto-analysis hit its
180-second budget and remains partial. [STATUS.md](../STATUS.md) records evidence.
