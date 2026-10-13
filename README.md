# Studio 2016 rewrite

An offline Rust/Bevy recreation targeting the supplied Roblox Studio macOS
**0.255.0.82159**. The goal is identical behavior and appearance; this is the
beginning of the project and no equivalence is claimed. Unaffiliated with Roblox.

Current priority is the offline core Roblox client: rendering, player/Humanoid,
physics, camera and appearance. Studio editor and editor GUI work is deferred.

## Play Solo on the Studio Baseplate

```sh
sh tools/cargo.sh run --locked -- viewport-baseplate
```

Expected: the Studio new-document place, a 512x512 dark stone grey Baseplate
with studs on top, under the default sky. The character spawns with its Head at
(0, 107, 0), falls about one second with the fall animation, lands and idles.
The place also holds five unanchored demo Parts (`Workspace::loose_parts_demo`,
not a native document):
- a red 4x6x2 "Crate" 8 studs ahead of the spawn: walk forward (W) into it and
  the character pushes it until it tips over;
- two blue Parts resting on each other to the front left;
- a yellow Part dropped flat from y = 40 and a green one dropped tilted from
  y = 60, landing ahead of the spawn.

Every 60 frames the live log gets a `loose_parts` line with each Part's kernel
list, position and connector counts. The character can stand on loose Parts,
step up onto low ones and ride them (`audit-part-stand`).
Same controls as the walking fixture below (WASD, Space, camera keys, R
respawns, Escape closes). The first camera frame is a fixture offset behind the
character until the Studio edit camera is ported; ClassicCamera takes over
immediately. Torso and Head collide with anchored Parts (`audit-part-wall`).
Unanchored Parts fall, land, stack and come to rest in the simulation
(`audit-part-drop`), the character pushes them (`audit-part-push`) and stands
on them (`audit-part-stand`), and still Parts fall asleep (SleepStage). Not
ported yet: the broadphase, outlines and the Studio camera.
Manual keys are **not tested**.

```sh
sh tools/cargo.sh run --locked -- audit-baseplate-render
sh tools/cargo.sh run --locked -- viewport-baseplate-smoke
```

Expect `workspace_draw name="Baseplate" quads=132 ... color=[0.3882353,
0.37254903, 0.38431373, 1.0]`, `baseplate_render ... landed_torso=[..., 3.0,
...]` and `baseplate_render_pass`. The smoke test opens the window, presses W
for 120 frames and exits after `viewport_final_readback_pass`.

## Play the experimental walking fixture

```sh
sh tools/cargo.sh run --locked -- viewport-walking
```

Expected: the original character rendered on a gray floor beside a brown wall.
**WASD** moves relative to the camera (as the original ControlScript does), **Space**
jumps, right-drag or **←/→** turns the camera, mouse wheel or **I/O** zooms, **R**
resets, **Escape** closes. The camera is the bundled 2016 ClassicCamera with
PopperCam occlusion. The fixture camera
follows translation. Movement uses native Humanoid command math, WalkSpeed=16,
RunningBase forces, R6 assembly mass/inertia and constrained SimBody substeps.
The two original collidable bodies (Head/Torso) generate vertex/plane wall
connectors. Like the original, these are realtime connectors (the Humanoid makes
its Parts non-throttling), so they get spring forces only, never awake impulses.
Arms and legs play
the August 2016 keyframe clips through the shipped Animate script (see below).
Animated poses update the assembly center of mass and inertia.

The floor support, wall, material coefficients, awake scheduling and camera are
explicit research fixtures. Native legacy balancing and AutoRotate torque now
keep the torso upright and turn it toward movement. Native floor/state queries
and broadphase remain incomplete. Maximum wall penetration is 0.114 studs in
the rest-pose regression (0.104 with animation). The 32-second animated wall
test remains upright. Native parity remains unverified. Manual keys
are **not tested**; automated input and Metal presentation are tested.
Animation timing follows the recovered native jobs:
Motor6D steps once per 60 Hz physics job before that job's four world steps, and
the script's `wait(0.1)` resumes on wall-clock time with 30 Hz Heartbeat time as
its animation input. The frame clock is ideal (no jitter) and the job order
within a frame is an explicit choice.

```sh
sh tools/cargo.sh run --locked -- audit-walking
sh tools/cargo.sh run --locked -- audit-walking-render
sh tools/cargo.sh run --locked -- viewport-walking-smoke
```

Expect `walking_balance_pass sustained_wall_seconds=30` and
`walking_pass walk_speed=16`, `input_release_stopped=true` and a small
`wall_max_penetration` (about 0.08), plus `walking_animation_pass limbs=4` and `release_pose=Standing`.
The render audit prints `walking_limb_readback` for three poses at identical root
frames and `walking_render_pass` with an exact reset comparison.
The automatic window test presses toward the wall, presents 120 frames,
prints `walking_character_readback_pass` and `viewport_final_readback_pass`, then
exits. Logs stay under RobloxStudio/analysis. Original files remain unchanged.

## Open the rendered client viewport

```sh
sh tools/cargo.sh run --locked
```

A window shows the supplied 2016 character, including its original head mesh,
face decal, colors, and built-in torso/limb meshes. **Space** starts a quarter-second
free-fall probe; **R** restores the original pose; **Escape** closes the window.
**A** runs the bundled procedural Running animation for one second, with its root
held in place and explicitly chosen 60 Hz joint callbacks. This is the current core rendering probe, with explicit fixture camera/lighting;
playable character movement and collisions are still incomplete.

For an automatic presentation test:

```sh
sh tools/cargo.sh run --locked -- viewport-smoke
```

Expected: `viewport_readback_pass covered_pixels=` with a nonzero count on this M3
window, `viewport_frame frame=120`, then exit. Coverage depends on window size.
For the automatic visible motion test, run
`sh tools/cargo.sh run --locked -- viewport-motion-smoke`. Expect
`motion_probe_paused step=60`, a frame-120 line with `physics_step=60`,
and `viewport_final_readback_pass`.

`viewport_live_log` identifies a file flushed during execution, so startup and
presentation progress remain available even before closing the window. No game
assets leave RobloxStudio. No visual inspection was performed.

## Run the first offline diagnostic

From this project directory:

```sh
sh tools/cargo.sh run --locked -- audit-runtime
sh tools/cargo.sh test --locked
```

Expected: `mesh_totals meshes=37 vertices=10992 triangles=12896`,
`scene_loaded instances=26 parts=7 visible_parts=6`, and
`static_import_pass updates=120`. The tests report **50 passed**. No window opens.
The command prints the path of a new log under `RobloxStudio/analysis/` containing
mesh bounds, class counts, Part positions/rotations and update observations.
This imports data and checks retention; it is not game simulation.

Rust **1.97.1** and Bevy **0.20.0** are pinned. `tools/cargo.sh` keeps dependency
caches, temporary files and builds inside the project. It uses an already installed
Rust 1.97.1 toolchain; it does not install or upgrade system tools. Dependencies
are downloaded on first build; runtime data loading uses no network requests.

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
  `part_push_camera_ray ... hit=Some(([..., -8.080032], ...)) static_only=None crate_face_z=-8.080033`
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
  Tumble (71), `part_drop_result ... heights=[0.58259946, 1.7632093, 0.98582256, 0.5856628] ... speeds=[0.0, 0.0, 0.0, 0.0]`,
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
  - `part_push_result ... max_character_connectors=7 lists=[FreeFall, Joint] crate_moved=[0.015719917, 0.00028276443, -1.0796337]`.
    The Crate is asleep when the character reaches it. The touch wakes it, and
    the character's realtime connector is its first, so 00a9be30 files it under
    jointBodies. With the native floor friction (0.3 on Plastic) the character
    pushes it about 1.08 studs;
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
  - `part_stand_result ... root_on_pad=[..., 3.2041068, ...] pad_top=0.18830347 lowest_between=2.9966283 step_floor_at=Some(156) root_on_step=[..., 3.9999356, ...] step_top=0.98490196`:
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
  [core evidence](docs/provenance/2016-CORE.md).

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
  See [rendering evidence](docs/provenance/2016-RENDERING.md).

- Exact bundle version, little-endian i386 Mach-O header and executable SHA-256
  validation against the supplied build.
- A mesh v2.00 reader based on the 2016 native loader and writer, including native
  newline/version matching, layout checks, counts, raw float records and faces.
  All 37 bundled meshes decode. See [native evidence](docs/provenance/2016-MESH.md).
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
180-second budget and remains partial. [STATUS.md](STATUS.md) records evidence.

## Data and rollback

The installation is read-only. Game assets and extracted files remain under the
ignored `RobloxStudio/` directory and are never committed. No game-file changes
are made, so no installation backup/restore is needed for this step. Ghidra's
analysis copies, generated traces and logs remain in `RobloxStudio/analysis/`;
the original `Contents/` is never modified.

Git tracks source and research findings through a whitelist. Each implementation
step is committed separately; use `git revert <commit>` to undo a source step.

Baseline check: `git status --short` should be empty after committing;
`git check-ignore RobloxStudio/Contents/MacOS/RobloxStudio` should print the path.

```sh
python3 tools/verify_bundle.py check
```

Expected here: `bundle_unchanged entries=2606`. The initial manifest is kept at
`RobloxStudio/analysis/bundle-before.json`. On a fresh copy, run
`python3 tools/verify_bundle.py record` once before analysis; it refuses to overwrite
an existing baseline. This manifest verifies content, not ownership or authenticity.

[Design](docs/DESIGN.md) · [Current tool research](docs/RESEARCH.md) ·
[Change log](MODLOG.md)
