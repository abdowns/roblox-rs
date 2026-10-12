# STATUS

## Goal and setup

Reimplement the offline Studio macOS 0.255.0.82159 client in Rust/Bevy, ultimately
matching behavior and appearance. The current result is a first headless import
milestone, not the full client. Project: `/Users/abd/Programming/roblox-rs`.
Original `RobloxStudio/Contents` is read-only. Raw evidence and Ghidra databases
remain ignored under `RobloxStudio/analysis`. Rust 1.97.1, Bevy 0.20.0, Ghidra
12.1.4; no client loader installed. Player ownership is not applicable to this rewrite.

## What works (tested)

Eight synthetic tests pass. The runtime verifies the exact executable, reads
37 meshes (10,992 vertices, 12,896 triangles), imports 26 character instances
including seven Parts and retains hierarchy/Part state through 120 Bevy updates.
All 2,606 original Contents files/symlinks match the stored fingerprint.
Native reader/writer/helper decompilation and targeted instruction capture succeed.

## What does not work

No graphical client, geometry dispatch, physics, animation, Lua, camera/input,
Studio editing/saving, mesh v1 or binary-place loading. Only explicit Part fields
are typed; other classes do not execute behavior. Native execution and full
pixel/behavior parity are **not tested**.

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
GPU rendering is not yet implemented at this checkpoint.

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

Expected: eight passing tests, `static_import_pass updates=120`, and
`bundle_unchanged entries=2606`. The runtime prints the new log path. See
docs/provenance/2016-MESH.md for native addresses and targeted trace commands.

## Next boundary and important files

Recover 2016 GeometryGenerator mesh selection/sizing and the character resource
path before connecting a renderer. Native head sizing, normal transforms, clothing,
Part-to-mesh dispatch and Attachment transforms are unverified for this version.
The 2015 implementation is not evidence of equality for those functions.

- `src/bundle.rs`: exact identity and confined read-only data access.
- `src/mesh.rs`: observed mesh v2 contract.
- `src/scene.rs`: explicit XML v4 Part/instance subset.
- `src/runtime.rs`: headless import and numerical evidence.
- `docs/DESIGN.md`: implemented architecture/limits.
- `docs/provenance/2016-MESH.md`: native findings and local evidence paths.
- `docs/RESEARCH.md`: current tool/reference checks.
- `MODLOG.md`: successes and failures by step.
