# Studio 2016 rewrite

An offline Rust/Bevy recreation targeting the supplied Roblox Studio macOS
**0.255.0.82159**. The goal is identical behavior and appearance; this is the
beginning of the project and no equivalence is claimed. Unaffiliated with Roblox.

## Run the first offline diagnostic

From this project directory:

```sh
sh tools/cargo.sh run --locked -- audit-runtime
sh tools/cargo.sh test --locked
```

Expected: `mesh_totals meshes=37 vertices=10992 triangles=12896`,
`scene_loaded instances=26 parts=7 visible_parts=6`, and
`static_import_pass updates=120`. The tests report **8 passed**. No window opens.
The command prints the path of a new log under `RobloxStudio/analysis/` containing
mesh bounds, class counts, Part positions/rotations and update observations.
This imports data and checks retention; it is not game simulation.

Rust **1.97.1** and Bevy **0.20.0** are pinned. `tools/cargo.sh` keeps dependency
caches, temporary files and builds inside the project. It uses an already installed
Rust 1.97.1 toolchain; it does not install or upgrade system tools. Dependencies
are downloaded on first build; runtime data loading uses no network requests.

## What works (tested)

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

No graphical client, native geometry dispatch, rendering, camera, input,
physics, animation, Lua runtime, Studio ribbon/docks/editing, place saving,
or complete XML/binary-place loader exists yet. Mesh v1.00/v1.01 are not implemented.
Imported Attachments, Decals, SpecialMesh and Humanoid do not execute their native
behavior. Full native execution, behavioral equivalence and visual equivalence
are **not tested**. Do not treat `static_import_pass` as evidence of those.

The Ghidra MCP currently has inconsistent project/open-program state. Targeted
headless Ghidra analysis succeeded; broad auto-analysis hit its 180-second budget
and remains partial. [STATUS.md](STATUS.md) records the attempts and evidence.

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
