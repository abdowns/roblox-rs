# Running the client and audits

How to run each viewport and diagnostic, and what each one should print.
The [README](../README.md) covers installation. Audit commands for each
feature are listed in [FEATURES.md](FEATURES.md).

## Play Solo on the Studio Baseplate

```sh
sh tools/cargo.sh run --locked -- viewport-baseplate
```

Expected: the Studio new-document place, a 512x512 dark stone grey Baseplate
with studs on top, under the default sky. The character spawns with its Head at
(0, 107, 0), falls about one second with the fall animation, lands and idles.
The Baseplate and the demo Parts are read from the place file
`places/viewport-baseplate.rbxm` (the log prints `place_loaded
path=.../places/viewport-baseplate.rbxm parts=27 joints=5`). Print its
contents with `sh tools/cargo.sh run --locked -- rbxm-dump
places/viewport-baseplate.rbxm`. Next to the Baseplate are five unanchored
Parts:
- a red 4x6x2 "Crate" 8 studs ahead of the spawn: walk forward (W) into it and
  the character pushes it until it tips over;
- two blue Parts resting on each other to the front left;
- a yellow Part dropped flat from y = 40 and a green one dropped tilted from
  y = 60, landing ahead of the spawn.

East of the spawn (+X, turn right) a showcase of the later features:
- **Balls** (x 20..44, z -20..-34): a red Ball rolls down an anchored ramp,
  a yellow one is dropped from y = 30, and a small blue one lands off-center
  on an anchored grey dome and rolls off. A green 4-stud "Kick" ball sits at
  (8, 2, -12) for the character to push.
- **Weld** (x 20, z 8): a red L-shaped pair drops from y = 10 and lands as
  one piece.
- **Glue** (x 30, z 8): a blue block hangs glued under an anchored grey one.
- **Hinges** (z 16): a yellow bar swings on a free Hinge at x 20, a green
  plate spins on a motor hinge at x 30, and a red Motor arm on a loose tower
  at x 40 turns 90 degrees.

West of the spawn (-X, turn left), the **Wedges** (x -20..-40):
- a grey Wedge ramp at (-24, 2, 6), rising towards +Z: walk up it from its
  low end at z -2;
- a red block slides down the slope of an anchored grey Wedge at x -36 and
  off its low end;
- a yellow Wedge dropped flat from y = 20 at (-26, -14), and a green one
  dropped upside down from y = 14 at (-36, -14) that lands and rolls onto a
  face.

Every 60 frames the live log gets a `loose_parts` line with each Part's kernel
list, position and connector counts. The character can stand on loose Parts,
step up onto low ones and ride them (`audit-part-stand`).
Same controls as the walking fixture below (WASD, Space, camera keys, R
respawns, Escape closes). The first camera frame is a fixture offset behind the
character until the Studio edit camera is ported; ClassicCamera takes over
immediately. Torso and Head collide with anchored Parts (`audit-part-wall`).
Unanchored Parts fall, land, stack and come to rest in the simulation
(`audit-part-drop`), the character pushes them (`audit-part-push`) and stands
on them (`audit-part-stand`), and still Parts fall asleep (SleepStage).
Welded Parts move as one assembly (`audit-part-weld`), glued Parts hang,
fall together and break off under load (`audit-part-glue`), hinged
Parts swing, spin and follow their motors (`audit-part-hinge`), and Motors
and Motor6Ds turn the Parts they link (`audit-part-motor`). Ball Parts land,
roll down a ramp and knock into each other (`audit-part-ball`), and the
character pushes a loose ball and is stopped by an anchored one
(`audit-part-ball-push`). Balls are drawn as spheres (`audit-ball-render`).
Wedge Parts land, tumble and carry blocks down their slope, and the
character walks up them (`audit-part-wedge`). They are drawn by the native
Wedge path (`geometry::workspace_wedge`).
The demo place shows these features too (see the top of this file).
Contacts come from the native broadphase (`audit-broadphase`). Not ported
yet: outlines and the Studio camera.
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
keep the torso upright and turn it toward movement. The walking fixture has no
broadphase (every collider meets every block). Maximum wall penetration is 0.114 studs in
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

## Model files

```sh
sh tools/cargo.sh run --locked -- rbxm-dump places/viewport-baseplate.rbxm
```

Prints any .rbxm/.rbxl/.rbxmx file as text: one line per instance (class
and Name, indented by depth), then one `.Property = value` line per property.
For the demo place the first lines are `Part "Baseplate"` and
`  .Anchored = Bool(true)`; the last instance is `Motor "Motor"`.

```sh
sh tools/cargo.sh run --locked -- audit-rbxm
```

Expect, for each of the two native binary models in the install
(`characterR15_2.rbxm`, `humanoidAnimateR15_2.rbxm`), a line
`rbxm_rewrite file=... bytes=7610 written=7610 identical=true
first_difference=None instances=67 classes={...}` (6353 bytes and 22
instances for the second). Then one `rbxmx_rewrite file=... identical=...
skipped={} first_difference=... context=...` line for each of the 19 XML
models, `identical=true` for PhysicsAnalyzer, TransformDragger,
VideoDocumentation, character3, humanoidAnimateLocalKeyframe2,
humanoidAnimateR15 and humanoidSoundNewLocal, and last
`rbxm_pass files=2 xml_files=19 xml_identical=7`. TerrainTools.rbxmx differs
only by its final newline (`first_difference=Some(248755)`). The audit fails
if one of the seven stops matching. The files are only read.

```sh
sh tools/cargo.sh run --locked -- rbxm-convert places/viewport-baseplate.rbxm demo.rbxmx
```

Converts a model file; the output is XML for `.rbxmx`/`.rbxlx` and binary
otherwise. Prints `rbxm_convert input=... output=demo.rbxmx instances=32`.
Converting an install file writes game data: keep that output inside
`RobloxStudio/`.

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
