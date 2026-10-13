# STATUS

## Goal and setup

Reimplement the offline Studio macOS 0.255.0.82159 client in Rust/Bevy, ultimately
matching behavior and appearance. The current result includes a tested headless
Metal renderer with tested window presentation, not the full client. Project: `/Users/abd/Programming/roblox-rs`.
The user has prioritized the core client: rendering, player, physics and camera.
Studio editor and editor GUI functionality are deferred.
Original `RobloxStudio/Contents` is read-only. Raw evidence and Ghidra databases
remain ignored under `RobloxStudio/analysis`. Rust 1.97.1, Bevy 0.20.0, Ghidra
12.1.4; no client loader installed. Player ownership is not applicable to this rewrite.

## What works (tested)

The current source has 42 passing synthetic tests; the following counts describe
earlier checkpoints, with current changes detailed below. The runtime verifies the exact executable, reads
37 meshes (10,992 vertices, 12,896 triangles), imports 26 character instances
including seven Parts and retains hierarchy/Part state through 120 Bevy updates.
All 2,606 original Contents files/symlinks match the stored fingerprint.
Native reader/writer/helper decompilation and targeted instruction capture succeed.
Live Ghidra MCP works. Native palette/head/block/projection contracts are recovered;
six draws/637 vertices/906 triangles render offscreen. Repeat pixels, depth, fog
and RGBA/BGRA checks pass. Four shader packs contain 572 records. Head face decal
loads eight native-rule mips and changes 120 pixels; synthetic blend/clamp/row
order/no-depth-write checks pass. Empty torso texture is skipped. The original
128x2048 Plastic stud atlas loads all twelve mips with the native DXT1 software
fallback. Character surface UVs and synthetic repeat/red-channel/alpha checks
pass. The final atlas pass changes 5,518 pixels and repeats identically.
Humanoid properties load explicitly. Seven Bevy command updates verify analog
fraction, normalized directions, camera projection/full rotation, zero input,
missing-camera partial writes and detached no-op. This does not yet move a rig.
Free-fall physics runs seven independent original Parts over 240 updates with
recovered native timing/gravity, angular damping, quaternion rules and tiny velocity
additions. Maximum analytic position/velocity errors are 0.000218649/0.000691672.

## What does not work

No complete graphical client, general geometry dispatch, complete physics, animation, Lua, interactive camera/input,
Studio editing/saving, mesh v1 or binary-place loading. Only explicit Part fields
are typed; other classes do not execute behavior. Native execution and full
pixel/behavior parity are **not tested**. General decals, surface subdivisions,
outlines, dynamic light grid/shadows and full material paths are incomplete.
Native compressed GPU decoding is **not tested**. Window presentation now passes after the format fix; earlier failures are preserved below.

## Earlier Ghidra MCP problem and two attempts

Initially no programs/project were open. A headless import successfully created
`RobloxStudio/analysis/ghidra/studio2016` without changing the executable.

1. MCP `open_project(...studio2016.gpr)` reported success with `headless: true`.
   The next `open_program(/RobloxStudio)` returned “No project open. Call
   /open_project first.”
2. Explicit GUI-mode `open_project(headless=false, program=/RobloxStudio)` returned
   “LockException: Current project must be closed before establishing a new active
   project.” `list_open_programs` still returned zero; `tool_running_tools` returned
   “No project open.” No more variations were attempted.

The MCP route was stopped after two attempts. Independent work used the installed
headless CLI with a separate `studio2016-batch` database under the same analysis
directory and succeeded. This was the earlier state; live instance reconnection
later restored the active project as described below.
No unrelated Ghidra project or system setting was changed to attempt a fix.

## Current live Ghidra and rendering progress

The user launched Ghidra. Instance metadata reports studio2016/RobloxStudio,
GhidraMCP 7.0.0, port 8089. The bridge retained a deleted socket; list_instances
and connect_instance restored live decompilation. No system setting changed.
Static geometry produces six draws, 637 vertices, 906 triangles with 208 native
colors. Thirteen tests pass. Camera and head deformation are recovered.
That geometry checkpoint preceded the now-tested offscreen GPU milestone.

## Other failures and limits

Broad auto-analysis timed out at 180 seconds, saving 48,563 functions. This is
partial analysis. The subsequent targeted trace succeeded. Historical external
system libraries were reported missing during import. The first Rust compile
needed an explicit f32 array and a HashMap for roxmltree NodeId; both were fixed
and the next tests/runtime passed. Remote IW4L manifest/commit lookups failed;
its public README was checked, with no copied code or dependency.

## Reproduce and inspect evidence

```sh
sh tools/cargo.sh test --locked
sh tools/cargo.sh run --locked -- audit-player
sh tools/cargo.sh run --locked -- audit-physics
sh tools/cargo.sh run --locked -- audit-render
sh tools/cargo.sh run --locked -- audit-runtime
python3 tools/verify_bundle.py check
```

Expected: twenty-six passing tests, `player_command_pass updates=7`,
`free_fall_pass bodies=7 steps=240`,
`render_probe_pass`, `gpu_contract_pass`,
`gpu_decal_contract_pass`, `gpu_plastic_contract_pass`,
`plastic_render_pass changed_pixels=5518 repeat_identical=true`,
`static_import_pass updates=120`, and `bundle_unchanged entries=2606`.
The diagnostics print new log paths. See
docs/provenance/2016-MESH.md for native addresses and targeted trace commands.

## Next boundary and important files

Next core boundaries are assemblies/joints, collision and Humanoid locomotion/state
forces; unconstrained free-fall and physics-to-render synchronization are implemented.
Player commands do not yet drive body forces, and no playable character is claimed. Studio UI work is deferred.
Remaining rendering boundaries include high-quality materials, general geometry,
outlines, light grid/shadows, sky stars/custom Sky and postprocessing. Native head
sizing, normal transforms, explicit character block dispatch, head decals and
Plastic stud surfaces are implemented. General clothing/mesh selection and
Attachment transforms remain unimplemented. The user explicitly requested a visible next build, authorizing a new bounded test
of the independently verified BGRA fix. It completed 120 presented frames. This
establishes surface presentation, not a full graphical client.

The first textured character probe changed zero pixels. Numeric CFrame evidence
showed R00=R22=-1, turning the face toward +Z. Changing the explicit probe camera
to that side passed; original model transforms and UVs were retained. Independent
PNG defilter/normal-facing numbers are in analysis/decal-facing-20261012.log.
The 2015 implementation is not evidence of equality for those functions.

- `src/bundle.rs`: exact identity and confined read-only data access.
- `src/mesh.rs`: observed mesh v2 contract.
- `src/scene.rs`: explicit XML v4 Part/instance subset.
- `src/runtime.rs`: headless import and numerical evidence.
- `src/player.rs`: explicit Humanoid properties and recovered command math.
- `src/physics.rs`: native free-fall body math and numeric fixture.
- `docs/provenance/2016-CORE.md`: core-client addresses and implemented limits.
- `src/render.rs` / `src/render.wgsl`: recovered low-quality GPU probes.
- `src/surface.rs` / `src/dds.rs`: character surface UVs and native DXT1 fallback.
- `docs/DESIGN.md`: implemented architecture/limits.
- `docs/provenance/2016-MESH.md`: native findings and local evidence paths.
- `docs/RESEARCH.md`: current tool/reference checks.
- `MODLOG.md`: successes and failures by step.

## Earlier window smoke failures — stopped after two attempts

1. Hidden surface never completed viewport-smoke; interrupted after no frame
   progress. Offscreen Metal rendering still passed.
2. Warmed up offscreen before showing the surface. GPU polling failed with
   submission index 1 / last successful submission 0. The viewport pipeline used
   BGRA while the warm-up target used RGBA, a concrete format mismatch.

Stop GUI retries per AGENTS.md. Fix/verify the format handling through an
independent offscreen BGRA contract probe: now passes and agrees with RGBA.
Window presentation after that fix
is **not tested** until a later explicit test; do not claim it works.

Latest numeric rendering evidence is in
`RobloxStudio/analysis/render-console-studs-20261012.log` and the new audit log
printed by that command. Final RGBA SHA-256:
`95e162d2e69e72c48a74ef2060a8b0f7418717006b9537805d7b459f6f15d015`.
No visual inspection or original client execution was performed.

## Window presentation recovered — current request

The user's explicit visible-build request resumed testing after the independently
verified BGRA fix. Both the unchanged renderer retest and the instrumented build
completed 120 frames at 1920×1440, six draws, 2,718 indices. Warm-up GPU readback
contains 61,363 covered pixels, SHA-256
`02f0e0ea751d7e783789f455e8836a2da79d769c080f01ddd186c8d0ad902521`.
Evidence: `RobloxStudio/analysis/viewport-retest-20261012.log` and
`RobloxStudio/analysis/viewport-live-20261012.log`. No screenshots or visual
inspection. Launch defaults to the visible viewport; incremental frame/startup
logs flush to an ignored analysis file while it runs. Tests: 26 pass.
Reproduce: `sh tools/cargo.sh run --locked -- viewport-smoke`.
Expected: `viewport_readback_pass`, `viewport_frame frame=120`, exit success.

## Physics now updates visible geometry

`motion` binds local original geometry to the seven recovered free-fall bodies;
the face shares its Head body. Initial world bytes match, parent positions/normals
are checked for 60 updates, dynamic GPU uploads match fresh posed uploads, and
reset restores identical pixels. At step 60, head minimum Y=12.767532349;
12,205 pixels change at 640×480, with 101 contributed by the moved face.
Reproduce: `sh tools/cargo.sh run --locked -- audit-motion`;
expect `motion_render_pass steps=60 fresh_upload_identical=true` (other fields
occur between the markers), plus `reset_pixels_identical=true`.

Window motion attempt 1 timed out at 45 seconds without frame 1, although warm-up
and physics progress were logged. Evidence is
`RobloxStudio/analysis/viewport-motion-console-20261012.log`.
The pinned Metal implementation returns Occluded for invisible native occlusion
state; showing a winit window does not itself activate the app. Added explicit
focus after warm-up, logged acquisition states and held automatic physics until
the first presentation. The second bounded run succeeded: an initial Occluded
retry with window_visible=true/window_focused=false, then frame 1 at step 0,
60 physics steps, 120 presentations, final coverage=48,475 pixels. Final
1920×1440 RGBA SHA-256:
`875f59fb89d4f7045f9c1102adc9708318403f40c53c596f2957fbb94edc554c`.
Evidence: `RobloxStudio/analysis/viewport-motion-focus-20261012.log`.
Focus/first-frame handling resolves this observed run; broader macOS lifecycle
behavior and manual Space/R/Escape interaction are **not tested**.
Reproduce: `sh tools/cargo.sh run --locked -- viewport-motion-smoke`;
expect `motion_probe_paused step=60`, frame 120 at physics_step=60,
`viewport_final_readback_pass`, exit success.

The quarter-second stop, explicit legacy gravity and Space/R controls are diagnostic
inputs. No floor, collisions, joints, animation or native locomotion is implied.
All prior static GPU/physics/player regressions pass and all 2,606 original bundle
entries remain unchanged. Studio editor/GUI remains deferred.

## Current R6 mesh and rig correction

The earlier block-body/stud counts above describe earlier checkpoints. Current
character geometry is six draws, 727 vertices, 1,066 triangles from the original
head, torso and four limb files. Composited body geometry disables studs/specular.
Native full texture composition remains incomplete. New audit-rig passes six
joints/seven Parts with zero rest and pivot error. Free-fall now integrates one
root and poses the native links, rather than seven independent rendered Parts.
The independent audit-physics diagnostic remains available. Initial rig motion
setup failed world-byte comparison; preserving imported rest frames until motion
resolved the second run. Body-mesh motion and linked motion both pass fresh/reset
pixel comparisons. Child physical velocities, native assembly forces, contacts,
locomotion and animation are still incomplete. Twenty-nine tests pass.

## Procedural animation checkpoint

Explicit bundled procedural animation covers six no-tool states. Zero pivot
error over 360 callbacks; GPU Running probe changes 3,950 pixels and matches
fresh uploads/exact pixel reset. Animation window smoke presents 120 frames and
60 callbacks, final coverage 56,346 at 1920x1440. The first new GPU audit compile
failed because wgpu 30 InstanceDescriptor lacks Default; the corrected existing
constructor passes. Native default keyframe playback, Lua scheduling, callback
cadence, tool animations, walking displacement and collisions remain incomplete.
Manual A/R/Space events and full parity are not tested. Reproduce with
`sh tools/cargo.sh run --locked -- viewport-animation-smoke`.

## Current walking/contact boundaries and remaining work

Live Ghidra recovered RunningBase horizontal feedback and contact-cache/velocity
contracts. audit-locomotion passes six explicit mass/floor/solver/friction cases;
audit-collision passes five threshold cases plus angular contact velocity. New
tests check radial limiting, activation selection, rotated contact positions and
reverse swap removal. Total tests: 33; Clippy passes. Original inputs remain
read-only. Raw evidence is in running-*-native/asm and collision-refresh-* JSON
files under ignored RobloxStudio/analysis.

These modules are not connected to an assembly/contact solver. Playable walking
translation and collision response are unfinished. Next dependency is native
assembly mass/inertia and broadphase/narrowphase/contact creation, then force
integration/vertical spring/balancing/state transitions. Native kernel timing and
remote Animator clip data also need recovery before default full animation parity.
The visible diagnostic now has original R6 meshes and joints with an explicit
procedural animation profile. Full native rendering (character texture atlas,
lighting/shadows/sky/material quality paths) and 1:1 equivalence are not tested.
All earlier failed routes remain recorded above. This is a progress checkpoint,
not completion of the entire player/physics/rendering rewrite.

Final linked free-fall window regression also passes: 120 presentations/60 steps,
head_min_y=12.767534256, final coverage=46,558 at 1920x1440. A stale automatic
startup joints=false log was corrected to joints=6; the rebuilt smoke passes with
the same final SHA-256 as the preceding run:
`930e15216f3e3edd139a20d437bdc45d896a9d841e9b915fbd22147101f7dff9`.
Evidence: RobloxStudio/analysis/viewport-r6-motion-logfix-20261012.log. Reproduce:
`sh tools/cargo.sh run --locked -- viewport-motion-smoke`; expect paused step=60,
frame=120, viewport_final_readback_pass, exit success. No visual inspection.


## Assembly checkpoint

Native Block mass/inertia and recursive R6 Cofm math now pass audit-assembly with
seven Parts, mass 9.9 and conserved translation inertia (error 6.74063e-8).
The root's builder override is 0.1; Flying's separate mass=10 branch is recorded,
not selected. Thirty-five tests and Clippy pass. Native assembly selection and
state-dependent overrides remain not tested. Constrained integration/contact
forces and playable walking are next dependencies. Clippy initially rejected
parentheses/native addend order; the formatting/targeted annotations resolve it.
A README patch context mismatch was corrected; no original game data changed.


## Current constrained-kernel checkpoint (2026-10-12)

39 tests pass. audit-kernel now integrates the recovered R6 assembly with native
constrained SimBody timing, momentum, Cofm offsets, impulses/force accumulators
and reset order. Constant-force discrete analytic errors are 0.003243974 position
and 0.010204064 velocity after 4,560 steps. ContactSpring implements recovered
compression/decompression/tangent histories with explicit material parameters.
First Clippy failure was excessive argument count; targeted native-boundary
annotation fixes it. Full native impulse pass, scheduler classification, contact
generation, playable walking/default keyframes and full rendering are incomplete.
Block SAT/face/vertex-plane generation is traced and raw evidence saved inside
RobloxStudio/analysis. No native differential execution or visual inspection.


## Current legacy-contact checkpoint (2026-10-12)

42 tests pass. audit-legacy-contact generates four native-rule vertex/plane
connectors in a shared-rotation/contained-face fixture, applies awake impulse
response through assembly inertia and integrates spring force. Central zero/half
restitution and two-body momentum checks pass. Native relative rotation/edge
clipping, sleeping, broadphase, scheduling and reference-impulse accumulation are
not yet integrated. Four corner impulses remove all approach speeds but produce
some linear/angular rebound even with zero restitution; that observed fixture
result is logged and has not been compared with native execution. No visual
inspection. A patch context failure was corrected before any mutation.


## Current playable-fixture checkpoint (2026-10-12)

42 tests and Clippy pass. viewport-walking adds WASD translation of the rendered
original R6 assembly with native RunningBase/SimBody math and vertex-plane wall
response. audit-walking reaches 16 studs/second, moves 15.746418 studs in one
second, stops on release and records 817 wall impulses over two seconds.
Penetration reaches 0.34594154; balancing and full scheduler work are needed.
Support, wall, materials, awake scheduling and camera remain explicit fixtures.
Limbs stay in their rest pose. Default keyframe playback, AutoRotate, jump,
slopes/stairs, native floor queries and complete rendering remain incomplete.
Matching 2016 remote keyframe clip payloads are not recovered locally.

First walking window smoke lost injected input to a focus event; the driver now
sets its scripted input each redraw. Second run walked and presented correctly;
final run added character-only readback validation (59,432 contributing pixels,
repeat-identical restoration) and presented 120 frames/466 ticks/789 impulses.
Previous free-fall window regression keeps its exact earlier 46,558-pixel hash.
No visual inspection. Manual key interaction and native equivalence are not tested.
First Clippy needless return and face-selection tie order were corrected. All
2,606 original Contents entries remain unchanged. Source changes are committed
as separate reversible steps; no assets/evidence leave RobloxStudio.


2026-10-12 walking rotation fix: live Ghidra Balancing/AutoRotate controllers are
now integrated. Sustained 30-second wall, four directions, release and angular
impulse recovery pass; originals remain unchanged. Native floor/state dispatch
and differential parity remain not tested. The next change addresses the
motionless limbs; previous notes describing absent balancing are historical.


2026-10-12 walking animation fix: four limbs now play the original bundled
procedural Lua branch with Running/Standing events and animated Cofm refresh.
44 source tests, 32-second animated wall hold, release, direction changes, pivot
constraints, Metal limb-only comparisons, exact reset and 120-frame window smoke
pass. Original files unchanged. Ghidra 00f6660c confirms the native default R6
keyframe script, whose matching remote 2016 walk/idle clip data is unavailable
locally. Therefore this fixes motionless limbs using an explicit bundled branch;
it does not establish default-keyframe parity. Native motor callback cadence is
not tested; explicit 60 Hz is retained. Clippy initially rejected test module
placement; corrected and rerun. No visual inspection or manual key test.
Source rollback before this task: 811a978; rotation fix: 9afa84b; animation step
can be reverted separately from rotation.


2026-10-12 native scheduling: Motor6D now steps once per recovered 60 Hz
PhysicsJob before its four world steps; the procedural script resumes from the
recovered wall-clock wait queue and receives 30 Hz Heartbeat time. 46 tests,
audit-scheduler, walking audits, render audit and window smoke pass; original
files unchanged. Remaining scheduling assumptions: ideal jitter-free frame clock,
explicit Heartbeat/WaitingScripts/Physics order within a frame, script start at
clock zero. Throttling, legacy non-cyclic steps, sleeping-joint skip and keyframe
Animator stepping are not implemented. Evidence summary:
RobloxStudio/analysis/scheduler-native-20261012.md. Rollback: revert this
commit; f7fd31e restores the explicit 60 Hz grid.


2026-10-12 Humanoid states: native table-driven Running/Landed/Jumping/Freefall/
RunningNoPhysics dispatch, floor rays and jump are implemented; 48 tests and all
walking/render/window audits pass. Remaining: climbing, FallingDown/GettingUp,
Seated, Swimming, slopes, moving floors, PGS, native contact "touching", native
world geometry. Jump height not compared with the original client.


2026-10-12 camera: bundled ClassicCamera/PopperCam/TransparencyController
ported; WASD is camera-relative. audit-camera and all walking audits pass.
Remaining: render character transparency, native FOV/zoom defaults, first-person
rotation, other camera modes.
