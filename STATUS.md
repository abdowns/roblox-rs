# STATUS

## Goal and setup

Reimplement the offline Studio macOS 0.255.0.82159 client in Rust/Bevy, ultimately
matching behavior and appearance. The current result includes a tested headless
static Metal renderer, not the full client. Project: `/Users/abd/Programming/roblox-rs`.
Original `RobloxStudio/Contents` is read-only. Raw evidence and Ghidra databases
remain ignored under `RobloxStudio/analysis`. Rust 1.97.1, Bevy 0.20.0, Ghidra
12.1.4; no client loader installed. Player ownership is not applicable to this rewrite.

## What works (tested)

Fifteen synthetic tests pass. The runtime verifies the exact executable, reads
37 meshes (10,992 vertices, 12,896 triangles), imports 26 character instances
including seven Parts and retains hierarchy/Part state through 120 Bevy updates.
All 2,606 original Contents files/symlinks match the stored fingerprint.
Native reader/writer/helper decompilation and targeted instruction capture succeed.
Live Ghidra MCP works. Native palette/head/block/projection contracts are recovered;
six draws/637 vertices/906 triangles render offscreen. Repeat pixels, depth, fog
and RGBA/BGRA checks pass. Four shader packs contain 572 records. Head face decal
loads eight native-rule mips and changes 120 pixels; synthetic blend/clamp/row
order/no-depth-write checks pass. Empty torso texture is skipped.

## What does not work

No complete graphical client, general geometry dispatch, physics, animation, Lua, interactive camera/input,
Studio editing/saving, mesh v1 or binary-place loading. Only explicit Part fields
are typed; other classes do not execute behavior. Native execution and full
pixel/behavior parity are **not tested**. General decals, surface textures, dynamic light
grid/shadows and full material paths are incomplete. Window presentation after
the format fix is **not tested** (two failed attempts below).

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
directory and succeeded. MCP active-project/CodeBrowser state remains unresolved.
Its frontend tool binding and loaded extension/version may need inspection.
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
sh tools/cargo.sh run --locked -- audit-runtime
python3 tools/verify_bundle.py check
```

Expected: fifteen passing tests, `static_import_pass updates=120`, and
`bundle_unchanged entries=2606`. The runtime prints the new log path. See
docs/provenance/2016-MESH.md for native addresses and targeted trace commands.

## Next boundary and important files

Continue the recovered decal/material and surface paths. Native head sizing,
normal transforms and explicit character block dispatch are implemented. General
clothing/mesh selection and Attachment transforms remain unimplemented.

The first textured character probe changed zero pixels. Numeric CFrame evidence
showed R00=R22=-1, turning the face toward +Z. Changing the explicit probe camera
to that side passed; original model transforms and UVs were retained. Independent
PNG defilter/normal-facing numbers are in analysis/decal-facing-20261012.log.
The 2015 implementation is not evidence of equality for those functions.

- `src/bundle.rs`: exact identity and confined read-only data access.
- `src/mesh.rs`: observed mesh v2 contract.
- `src/scene.rs`: explicit XML v4 Part/instance subset.
- `src/runtime.rs`: headless import and numerical evidence.
- `docs/DESIGN.md`: implemented architecture/limits.
- `docs/provenance/2016-MESH.md`: native findings and local evidence paths.
- `docs/RESEARCH.md`: current tool/reference checks.
- `MODLOG.md`: successes and failures by step.

## Window smoke failures — stopped after two attempts

1. Hidden surface never completed viewport-smoke; interrupted after no frame
   progress. Offscreen Metal rendering still passed.
2. Warmed up offscreen before showing the surface. GPU polling failed with
   submission index 1 / last successful submission 0. The viewport pipeline used
   BGRA while the warm-up target used RGBA, a concrete format mismatch.

Stop GUI retries per AGENTS.md. Fix/verify the format handling through an
independent offscreen BGRA contract probe: now passes and agrees with RGBA.
Window presentation after that fix
is **not tested** until a later explicit test; do not claim it works.
