# MODLOG

## 2026-10-12 Integrate native constrained assembly and contact spring math

**Changed:** Added kernel.rs, audit-kernel, constrained Cofm integration and native
ContactConnector force-history branch. Existing public signatures stay stable.
**Why:** The constrained native branch uses momentum damping and semi-implicit
position, distinct from free fall; contact friction depends on the previous force.
**Tested how:** Live Ghidra decompilation/constants; 39 tests; one-second original
R6 assembly diagnostic; source formatting and Clippy with warnings denied.
**Result:** 4,560 substeps, seven posed bodies, position error 0.003243974 and
velocity error 0.010204064. Initial contact force zero, next sample -37.5 on B.
Test: `sh tools/cargo.sh run --locked -- audit-kernel`; expect `kernel_pass`.
**Still broken / not tested:** First Clippy flagged the explicit native contact
arguments; a targeted annotation documents the boundary. Native scheduling,
contact generation, iterative impulse response, walking and differential parity
are **not tested**. Viewport continues using its existing bounded probes.
**Next:** Port the traced native vertex/plane contact generation and impulse pass.


## 2026-10-12 Recover R6 assembly mass and inertia

**Changed:** Runtime-loaded Plastic density/root mass, native block inertia
polynomial, recursive Cofm aggregation and SimBody diagonal inverse-inertia path;
added audit-assembly. Existing public signatures remain stable.
**Why:** Native blocks use a surface-weighted inertia polynomial; the R6 builder
sets root mass to 0.1, so modern box formulas/default masses would differ.
**Tested how:** Live Ghidra factory/shape/mass/Cofm/SimBody decompilation and SSE
instructions; independent face-integral and rotated parallel-axis tests; 35 tests,
Clippy and original-character diagnostic.
**Result:** Seven Parts, mass 9.9, mass conserved, translation inertia error
0.0000000674063. Test: `sh tools/cargo.sh run --locked -- audit-assembly`;
expect `assembly_pass parts=7 mass_conserved=true`.
**Still broken / not tested:** First Clippy flagged parentheses and native operand
order; parentheses removed and targeted annotations preserve observed addends.
One README patch missed wrapped context; corrected. Leaf Cofm initially used a
multiply/divide roundtrip; the recovered copy branch is now retained. State-based
mass overrides, root classification and native differential behavior are **not tested**.
**Next:** Native contact force and constrained-body integration.


## 2026-10-12 Verify the final linked window probe

**Changed:** Corrected the automatic free-fall startup log to report six active
R6 joints; recorded final window regression evidence.
**Why:** The stale joints=false label contradicted the recovered linked pose path.
**Tested how:** Two bounded Metal viewport-motion-smoke runs, including a rebuilt
run after the log correction; numeric readback only; formatting check.
**Result:** 120 presentations and 60 steps; head_min_y=12.767534256; 46,558 covered
pixels at 1920x1440. Both final frames have SHA-256
`930e15216f3e3edd139a20d437bdc45d896a9d841e9b915fbd22147101f7dff9`.
Evidence: RobloxStudio/analysis/viewport-r6-motion-logfix-20261012.log. Test:
`sh tools/cargo.sh run --locked -- viewport-motion-smoke`; expect joints=6,
paused step=60, frame=120, viewport_final_readback_pass and successful exit.
**Still broken / not tested:** Manual keys and visual/native equivalence are
**not tested**. Full walking/contact response and rendering remain incomplete.
**Next:** Native assembly mass/inertia and contact generation/solver integration.

## 2026-10-12 Recover native contact persistence and velocity

**Changed:** Bullet cached-contact refresh/removal, runtime native orthogonal
factors, signed normal/tangent velocity including angular motion, audit-collision.
Existing public signatures and source whitelist are unchanged.
**Why:** Collision solving depends on native contact lifetime/separation rules
and point velocities rather than part-center velocities alone.
**Tested how:** Live Ghidra 0040294a/00402b88/00a929f6/00a92bd6 plus assembly
checks; five numeric threshold cases, rotation/removal/activation tests; 33 passing
tests, Clippy and bundle fingerprint.
**Result:** Strict threshold boundaries retain contacts; separated/drifting points
are removed; angular relative velocity fixture equals [1,1,0]. Test:
`sh tools/cargo.sh run --locked -- audit-collision`; expect
`contact_pass refresh_cases=5 angular_velocity=true`.
**Still broken / not tested:** Contact creation, shape-pair detection, native
impulse/warm-start callbacks and assembly integration remain pending. Native
differential behavior and runtime factor overrides are **not tested**. Contact
refresh does not establish playable collision response.
**Next:** Recover native assembly mass/inertia, contact generation and solver
integration; then connect floor-aware Humanoid forces to the client.


## 2026-10-12 Recover RunningBase horizontal force math

**Changed:** Runtime-loaded native feedback gains and ground/air limits; original
velocity-error, force-cancellation, radial cap, effective mass and friction order.
Added audit-locomotion. Existing public signatures remain stable.
**Why:** Walking uses native force feedback rather than directly assigning speed.
**Tested how:** Live Ghidra 00c09632 and gain/accessor traces; six numeric cases;
radial-cap synthetic test, 30 total passing tests and Clippy.
**Result:** Legacy/PGS gains 1250/150; ground/air limits 500/143. Test:
`sh tools/cargo.sh run --locked -- audit-locomotion`; expect
`running_force_pass cases=6` and cancellation force [0,0].
**Still broken / not tested:** Assembly inputs are explicit fixtures. Walking
translation, floor discovery, vertical/balancing forces, mass/inertia recovery
and complete force/contact integration remain pending; native differential
behavior is **not tested**.
**Next:** Native contact refresh and contact-point velocity math.


## 2026-10-12 Animate the recovered R6 rig

**Changed:** Runtime-loaded procedural animation literals, six no-tool poses,
Motor6D target updates and a labeled one-second Running viewport probe (A),
audit-animation, audit-animation-render and viewport-animation-smoke.
**Why:** Exercise the supplied offline procedural branch through native joints
and original body meshes while default remote keyframe clips are unavailable.
**Tested how:** Original bundled Lua + Ghidra Motor6D setter/step traces; 29 tests;
360 callback pivot checks; four Metal samples, repeat/fresh/reset comparisons;
120-frame/60-callback visible smoke; Clippy. No visual inspection.
**Result:** All six poses maintain pivots exactly; final animated frame changes
3,950 pixels at 640x480. Visible final coverage 56,346 at 1920x1440. Test:
`sh tools/cargo.sh run --locked -- audit-animation-render`; expect
`animation_render_pass callbacks=60` with fresh/reset identical. To see it:
`sh tools/cargo.sh run --locked -- viewport-animation-smoke`; expect frame 120
and paused step 60 with animation=true.
**Still broken / not tested:** GPU audit initially failed to compile with a
nonexistent InstanceDescriptor Default; the corrected constructor passes. Manual
keys, native default selection, scheduler timing and sine/visual parity are
**not tested**. Tools, keyframe clips, walking translation and contacts are pending.
**Next:** Native RunningBase force and contact processing contracts.


## 2026-10-12 Recover and bind six native R6 joints

**Changed:** Runtime decoding of native joint presets, Motor6D desired-angle dead
zone and per-callback speed limit, audit-rig, and root-to-linked-Part pose updates.
Public signatures remain stable; all original inputs stay read-only.
**Why:** The native Humanoid constructs joints absent from the serialized model.
**Tested how:** Live Ghidra builder/initializer/NormalId/Motor/link traces; 29 tests;
rig rest/pivot/reset audit; linked Metal motion fresh/reset comparisons; Clippy.
**Result:** Six joints/seven Parts, rest_error=0, pivot_error=0. Linked motion
changes 11,781 pixels, with the face attached; fresh/reset pixels match. Test:
`sh tools/cargo.sh run --locked -- audit-rig`; expect `rig_pass joints=6`.
`sh tools/cargo.sh run --locked -- audit-motion`; expect fresh/reset identical.
**Still broken / not tested:** First linked motion attempt failed the initial
byte-roundtrip check; preserving serialized rest frames until motion fixed the
second attempt. Native callback cadence, Animator blending/offset decay, physical
assembly mass/inertia/velocities and contact forces are **not tested** or pending.
**Next:** Bundled procedural animation and native walking/contact paths.


## 2026-10-12 Correct the R6 body mesh and material branch

**Changed:** Native torso/arm/leg mesh dispatch, runtime native scale table,
unscaled normals, zero body specular, and body stud bypass. Public signatures
remain stable. Original source data remains read-only.
**Why:** Ghidra fetchResources/Identifier/material traces show the supplied
Humanoid uses these five file meshes, rather than unmeshed blocks.
**Tested how:** Live Ghidra traces; 27 tests; static Metal and motion readbacks;
synthetic depth/blend/Plastic/BGRA regression checks; Clippy and bundle fingerprint.
**Result:** Six draws, 727 vertices, 1,066 triangles; 6,613 covered pixels at
640x480, 120 face pixels; 11,781 motion pixels change. Repeat/fresh/reset comparisons
pass and all 2,606 original entries are unchanged. Test:
`sh tools/cargo.sh run --locked -- audit-render`; expect geometry totals above
and `plastic_render_pass changed_pixels=0`. Motion:
`sh tools/cargo.sh run --locked -- audit-motion`; expect reset/fresh identical.
**Still broken / not tested:** Full character texture-atlas composition, clothing,
native differential/visual parity, joints and locomotion are **not tested**.
Bare fixture colors/face use the existing separate draw path.
**Next:** Native six-joint construction and Motor6D motion.


## 2026-10-12 Connect native physics frames to visible character geometry

**Changed:** Immutable Part-space geometry, seven Bevy body bindings, ordered
physics/geometry/decal extraction, guarded dynamic GPU uploads, audit-motion and
viewport-motion-smoke. Space runs 60 native free-fall steps; R resets. Added window
focus/acquisition evidence and a bounded smoke acquisition timeout. Original public
signatures/data fields remain stable; explicit body loading is shared with the
existing physics audit.
**Why:** Exercise recovered body motion through the renderer while preserving
original geometry and parented face behavior.
**Tested how:** Live Ghidra frame/bone/geometry traces; 27 synthetic tests; 60-step
GPU readback, parent checks, fresh-upload differential and exact pixel reset;
static GPU, 240-step physics and player regression diagnostics; Metal motion
window smoke; formatting/Clippy; original Contents fingerprint.
**Result:** 12,205 changed pixels and 101 moved face pixels at 640×480. Updated
uploads match fresh uploads; reset restores identical pixels. Visible motion
smoke reaches 120 frames with 60 physics steps, 48,475 final covered pixels at
1920×1440. All 2,606 original entries unchanged. Run
`sh tools/cargo.sh run --locked -- audit-motion`; expect `motion_render_pass steps=60`
and `reset_pixels_identical=true`. Run
`sh tools/cargo.sh run --locked -- viewport-motion-smoke`; expect
`motion_probe_paused step=60`, frame 120 at physics_step=60 and final readback pass.
**Still broken / not tested:** Initial compile required crate-visible internal
component types. First window motion run timed out before presentation. Cached
Metal/winit sources supported adding activation/occlusion logs and holding physics
until first presentation; the second bounded run passed. Manual key events,
remote interpolation, native GPU bone precision, joints/contacts/locomotion,
full native rendering and behavioral parity are **not tested**. Quarter-second
pause and research controls are explicit fixture behavior, not historical defaults.
**Next:** Recover native rig/assembly construction and contacts/Humanoid forces;
add native camera/input paths before claiming a playable client.

## 2026-10-12 Present the character in the default visible build

**Changed:** Default launch opens the recovered character viewport; startup GPU
coverage/hash checks, visible title and flushed live frame/timing logs. Existing
public signatures are stable. Updated the earlier window-failure status.
**Why:** The user requested something rendered in the next build.
**Tested how:** Retested the previously verified BGRA fix; instrumented Metal
window smoke completed 120 presentations, six draws/2,718 indices at 1920×1440.
Twenty-six tests pass; formatting and Clippy checks. No visual inspection.
**Result:** Warm-up coverage=61,363 pixels, SHA-256
`02f0e0ea751d7e783789f455e8836a2da79d769c080f01ddd186c8d0ad902521`.
Run `sh tools/cargo.sh run --locked -- viewport-smoke`; expect
`viewport_readback_pass`, `viewport_frame frame=120`, then close. Run
`sh tools/cargo.sh run --locked` to keep the viewport open.
**Still broken / not tested:** The two earlier failed attempts remain documented.
This checkpoint does not yet animate geometry. Native visual parity, playable
character locomotion/contacts and manual Escape interaction are **not tested**.
**Next:** Connect recovered physics frames to geometry and decal uploads.

## 2026-10-12 Recover native free-fall physics

**Changed:** Added runtime native physics constants, independent SimBody free-fall
math, native-sign matrix/quaternion conversion and Bevy audit-physics fixture.
Added timing/gravity/rotation/state-name evidence; public existing signatures stable.
**Why:** Establish native body motion before contact and Humanoid locomotion forces.
**Tested how:** Live Ghidra decompilation/instructions/memory; 26 synthetic tests;
seven original Part frames/velocities through 240 Bevy updates; analytic trajectory
comparison at every step; formatting/Clippy; whole Contents fingerprint.
**Result:** Native dt approximately 1/240, kernel dt approximately 1/4560, 19
subdivisions, damping 0.99621, legacy acceleration -196.20001 and post-step 1e-20
additions. Maximum position/velocity errors 0.000218649/0.000691672; all 2,606
original entries unchanged. Run
`sh tools/cargo.sh run --locked -- audit-physics`; expect `free_fall_pass bodies=7 steps=240`.
**Still broken / not tested:** A manual PIC address calculation initially selected
the wrong section; instruction base/offset arithmetic corrected the read before
implementation. Zero nudge would have been wrong: static initializer establishes
the observed tiny value. Its anti-denormal purpose is an inference. DynamicGravity
defaults, contacts, assemblies, sleeping and playable character/native differential
parity are **not tested**. Parts in this probe are explicitly unconstrained.
**Next:** Recover native assembly/joint construction and contact/Humanoid force
paths; integrate those before connecting interactive player motion to rendering.

## 2026-10-12 Prioritize core client and recover Humanoid commands

**Changed:** Added explicit Humanoid properties, Bevy motion component and
audit-player CLI. Recorded Ghidra command/setter/camera traces and the user's
core-client priority; Studio editing/GUI deferred. Existing public signatures
and rendering behavior retained.
**Why:** Establish native Player/Humanoid input behavior before physics forces.
**Tested how:** Live Ghidra MCP decompilation/instructions, 23 synthetic tests,
actual bundled-character command diagnostic, formatting and Clippy.
**Result:** WalkSpeed=16, JumpPower=50, HipHeight=0, MaxSlopeAngle=89, health=100.
Seven logged command updates pass including analog, camera, zero, missing-camera
partial updates and detached no-op. Run
`sh tools/cargo.sh run --locked -- audit-player`; expect `player_command_pass updates=7`.
**Still broken / not tested:** Initial test expected the wrong 4/5 float;
corrected to 0.8 with the algorithm unchanged. Degenerate ground cameras are
rejected. Locomotion, jumping, collision, signals/replication, full native
execution and behavioral parity are **not tested**. No GUI retry was performed.
**Next:** Implement the recovered free-fall branch with native timing/gravity;
recover assemblies and contact/Humanoid forces before claiming a playable rig.

## 2026-10-12 Render Plastic stud surfaces from native evidence

**Changed:** DXT1/DDS software decoder, runtime native atlas-row lookup, explicit
character surface UV stage and low-quality Plastic atlas shader. Private GPU
vertex adapter adds stud coordinates while public function/data signatures stay
stable. Updated rendering evidence, design, README and STATUS.
**Why:** Render the provided stud atlas using recovered geometry, sampler and
shader rules rather than modern engine defaults.
**Tested how:** Live native decompilation, 18 synthetic tests, actual M3 Metal
readback, repeat/red-channel/alpha contract probe, formatting and Clippy with
warnings denied; Contents fingerprint check.
**Result:** Original 128x2048 atlas, twelve mips; six draws/637 stud-coordinate
vertices. Atlas changes 5,518 pixels; final frames repeat identically with SHA-256
`95e162d2e69e72c48a74ef2060a8b0f7418717006b9537805d7b459f6f15d015`.
All 2,606 original entries unchanged. Run
`sh tools/cargo.sh run --locked -- audit-render`; expect `gpu_plastic_contract_pass`
and `plastic_render_pass changed_pixels=5518 repeat_identical=true`.
**Still broken / not tested:** First Clippy requested an iterator and removal of
a redundant borrow; fixed and checks passed. This explicitly uses native software
DXT1 decoding; original compressed GPU pixels, subdivisions, outlines, high-quality
materials and complete native parity are **not tested**. Window retries remain
stopped after the two failures recorded in STATUS; no further GUI attempt made.
**Next:** Recover the remaining rendering paths and resolve window presentation
from the recorded evidence before claiming a graphical client.

## 2026-10-12 Render the native character face decal

**Changed:** Confined RGBA8 PNG loader, recovered byte mipmap rule, separate Bevy
textured draws and low-quality decal GPU pass; front-side probe camera.
**Why:** Render the provided face from native mesh/material/sampler evidence.
**Tested how:** 15 source-only tests, real Metal readback and synthetic clamp/row
order/straight-alpha/no-depth-write checks; native decompilation plus instructions.
**Result:** One face draw, eight mips, 120 changed pixels, repeat-identical frames.
Empty torso decal skipped. Formatting and Clippy with warnings denied pass.
**Still broken / not tested:** First character probe changed zero pixels because
the probe viewed the back; CFrame evidence explained and corrected it. Two doc
patch context mismatches were fixed using exact file context. General images,
decals, native quality/anisotropy defaults and GL/Metal bias equivalence are
**not tested**. Window presentation remains **not tested** after its format fix.
**Next:** Recover block surface/stud coordinates and atlas before extending Plastic.

## 2026-10-12 Execute static rendering on Metal

**Changed:** Direct Metal renderer with Bevy extraction, independently authored
low-quality WGSL, native shader-pack reader, numeric readback and experimental
viewport commands. Source whitelist now includes WGSL.
**Why:** Execute recovered geometry, projection, lighting/fog and depth contracts.
**Tested how:** 14 source-only tests; actual M3 Metal readback; repeat comparison;
synthetic depth/fog and RGBA/BGRA checks; Contents fingerprint; formatting/Clippy.
**Result:** 6,831 covered pixels/292 colors; repeat-identical output; four packs,
572 records. Depth/fog/format checks pass; 2,606 original entries unchanged.
**Still broken / not tested:** Initial wgpu API type errors fixed. Window smoke
failed twice (hidden surface stalled, then warm-up RGBA/BGRA mismatch). Stopped
GUI retries and recorded STATUS. Offscreen format correction passes; window after
that fix is **not tested**. Texture/material completeness and native parity are
**not tested**.
**Next:** Recover decal resource/sampler/geometry contracts from native calls.

## 2026-10-12 Recover static geometry and camera

**Changed:** Native-data palette/topology reader, character head/block geometry,
material specular quantization, camera matrices, audit-geometry CLI and evidence.
**Why:** Establish the 2016 geometry/projection contract before GPU rendering.
**Tested how:** Live MCP decompilation/instructions, 13 source-only tests, actual
character diagnostic, formatting and Clippy with warnings denied.
**Result:** Live MCP restored through instance discovery. 208 native colors; six
draws, 637 vertices, 906 triangles.
**Still broken / not tested:** First palette run omitted XOR-zero; native
instructions identified the cause and regression/fix passed. First Clippy run
requested rfind; corrected. Surface UVs, decals and rendering remain absent.
Original execution and full parity are **not tested**.
**Next:** Connect static geometry to a bounded GPU probe.

## 2026-10-12 Verified 2016 mesh and character import

**Changed:** Added pinned Rust/Bevy package and lockfile; confined bundle access,
independent mesh v2 reader, explicit XML Part/instance subset, headless diagnostic,
local Ghidra utilities and Contents fingerprint check. Added design, native
provenance, current tool research and STATUS; updated README.
**Why:** Prove a data boundary from the exact 2016 binary before rendering or reuse
of 2015 behavior.
**Tested how:** Eight synthetic tests; original-bundle diagnostic over 120 Bevy
updates; native decompilation/instruction tracing; whole Contents hash comparison;
formatting and Clippy with warnings denied.
**Result:** 37 meshes / 10,992 vertices / 12,896 triangles; 26 instances, including
14 Attachments and seven Parts (six visible); `static_import_pass updates=120`.
All 2,606 original files/symlinks unchanged. Matching 2015 mesh payloads are identical,
but the old reader's acceptance differs; no source was transplanted. Logs remain
inside RobloxStudio/analysis.
**Still broken / not tested:** MCP project state failed twice; headless fallback
succeeded. Broad analysis timed out after 180 seconds, saving 48,563 functions.
First compile type/map errors were fixed and later checks passed. Rendering,
simulation, scripting, editing/saving and full native parity are not implemented /
**not tested**. Remote IW4L manifest/commit lookups failed.
**Next:** Recover exact 2016 character geometry/resource selection before rendering.

## 2026-10-12 Source baseline

**Changed:** Created the source whitelist, README and Git baseline; preserved the
provided guidance and decompilation map.
**Why:** Establish reversible source history before implementing the rewrite.
**Tested how:** Inspected Info.plist; checked Git's tracked list and ignore rule.
**Result:** Bundle version is 0.255.0.82159; installation excluded from Git.
**Still broken / not tested:** No Rust runtime; native client execution and parity
are not tested. Ghidra MCP initially reports no open programs.
**Next:** Import the exact executable into a local analysis copy and implement
the smallest data-loading boundary supported by native evidence.
