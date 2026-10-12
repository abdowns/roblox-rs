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

Twenty-seven synthetic tests pass. The runtime verifies the exact executable, reads
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
outlines, lighting/grid/shadows, sky and postprocessing. Native head
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
