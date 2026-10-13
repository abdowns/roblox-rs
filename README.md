# Studio 2016 rewrite

An offline Rust/Bevy recreation targeting the supplied Roblox Studio macOS
**0.255.0.82159**. The goal is identical behavior and appearance; this is the
beginning of the project and no equivalence is claimed. Unaffiliated with Roblox.

Current priority is the offline core Roblox client: rendering, player/Humanoid,
physics, camera and appearance. Studio editor and editor GUI work is deferred.

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
connectors and receive native awake impulses/spring forces. Arms and legs now
play the bundled procedural animation while walking, then its Standing pose on
release. Animated poses update the assembly center of mass and inertia.

The floor support, wall, material coefficients, awake scheduling and camera are
explicit research fixtures. Native legacy balancing and AutoRotate torque now
keep the torso upright and turn it toward movement. Native floor/state queries
and broadphase remain incomplete. Maximum wall penetration is 0.114 studs in
the rest-pose regression (0.104 with animation). The 32-second animated wall
test remains upright. Native parity remains unverified. Manual keys
are **not tested**; automated input and Metal presentation are tested.
The original R6 default loader uses remote keyframe clips absent from this bundle.
Walking explicitly uses the shipped procedural script instead; default-keyframe
equivalence is **not tested**. Its timing now follows the recovered native jobs:
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
`walking_pass walk_speed=16`, `input_release_stopped=true` and nonzero wall
impulses, plus `walking_animation_pass limbs=4` and `release_pose=Standing`.
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
  not drawn yet; Torso/Head contacts with Parts are not ported.

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
