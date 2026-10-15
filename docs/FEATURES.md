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
  `part_push_camera_ray ... hit=Some(([..., -8.309371], ...)) static_only=None crate_face_z=-8.309371`
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
  - the Head touch test that ends RunningNoPhysics also checks loose Parts.

  Run `sh tools/cargo.sh run --locked -- audit-part-push`. Expect:
  - `part_push_first_contact step=101`;
  - `part_push_result ... max_character_connectors=8 lists=[FreeFall, Joint, Contact] crate_moved=[0.020631112, 3.2424927e-5, -1.3100352]`.
    The Crate is asleep when the character reaches it. The touch wakes it, and
    the character's realtime connector is its first, so 00a9be30 files it under
    jointBodies. With the native floor friction (0.3 on Plastic) the character
    pushes it about 1.31 studs. At step 780, after the character has stopped
    touching it, the Crate wakes with only its ground contacts and is filed
    under contactBodies;
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
  - `part_jump_compare pad_pushes=304 baseplate_pushes=228 pad_rise=9.4151535 baseplate_rise=8.657764`.
    The push drives the jump speed to JumpPower, so the lighter Pad only
    takes more substeps to get there. The higher rise from the Pad comes from
    the Pad springing back up (it reaches vy -118);
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

  Not ported: motors and kernel joints in the Body tree, breaking joints, and
  joints made or removed while running. A fast impact on a Part whose center
  of mass sits high above the contact gives it a large sideways kick; this
  also happens to a single tall block (see the roadmap). Native parity is
  **not tested**.

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
