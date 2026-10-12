# Design

This is a standalone offline reimplementation. The supplied Studio 0.255.0.82159
installation supplies data at runtime. The original executable is never patched
or launched. Bevy supplies application scheduling and entity storage at this stage.
Each historical behavior needs evidence; modern engine defaults do not establish it.

## Data boundary

`Bundle::open` checks Info.plist, the i386 executable header and exact SHA-256.
Data paths are canonicalized and must remain inside the bundle. Reads are bounded:
64 MiB for meshes, 16 MiB for character XML. These are research limits, not native
restrictions. The Apple plist DOCTYPE is accepted by a non-networking XML parser.

`MeshData::parse` implements the observed v2 storage format. It preserves nine
float words per vertex and three unsigned indices per face. Version-line handling
follows the 2016 reader. `finite_bounds` is a separate diagnostic restriction;
NaN checks do not silently change the decoder's native acceptance contract.
Mesh v1 and native cached bounding-box computation are not implemented.

`Scene::parse` walks XML v4 instances in document order, retaining referents,
parents, class, name and property counts. It interprets explicit Part CFrame,
size, shape, BrickColor, transparency, anchored and collision flags. Matrices
retain row order and signed zero. Missing required properties fail rather than
inventing defaults. Other properties are not interpreted or saved; non-Part
classes do not run behavior. This is a diagnostic subset, not a native deserializer.

## Bevy boundary

`runtime::audit` loads all fonts-directory meshes and character3.rbxm into a
headless App. Mesh records and Instance/Part components retain original data.
`ChildOf` records ownership hierarchy; original world CFrames are not converted
to a Bevy Transform hierarchy. Part names do not yet select render geometry.

Manual `App::update` calls check counts, parent relationships, identity, bitwise
Part state and mesh bounds/counts. Observations log at update 1 and every 40
updates. The CLI uses 120 updates. This is import retention, with no recovered
timestep, simulation or rendering implied by that count.

Successes and failures after identity validation create new logs in
`RobloxStudio/analysis`. Output directories cannot be symlinks; files use
create-new semantics. All original data, raw decompilations, databases, extraction
outputs and numeric evidence stay ignored inside RobloxStudio. Human-written
functional findings go in tracked docs. Public function signatures are contracts
to preserve as implementation grows.

## Tooling and verification

Rust 1.97.1 and Bevy 0.20.0 are pinned with a lockfile. Only Bevy's `std` feature
is enabled. Direct wgpu 30.0.1/Metal and winit 0.30.13 provide GPU/window plumbing;
Bevy supplies scheduling/storage rather than modern material defaults.
`tools/cargo.sh` directs dependency,
temporary and build output into this project. The reference guides remain their
pre-existing independent checkout, excluded from the outer repository.

Fourteen source-only tests use invented inputs: version-line quirks, raw float words,
malformed layouts/indices, matrix row order and nested referents. The actual bundle
diagnostic establishes import. `tools/verify_bundle.py` fingerprints every original
Contents file and symlink. Neither establishes original game behavior or pixels.
Compatible reference execution and differential comparisons remain necessary.

## Recovered static rendering data

`native_data::NativeImage` reads file-backed Mach-O regions from the verified
binary. A bounded x86 constant-argument decoder recovers 208 historical colors;
it never executes code and refuses unresolved arguments/count changes. Block
face topology is read at runtime. No extracted tables are embedded in source.
`geometry::RenderScene` produces world-space character head/block draws from
explicit XML. Head deformation is recovered; normals rotate without inverse
scaling. Unsupported shapes and reflectance return errors. Surface UVs/decals
are not implemented. `camera` supplies recovered view/perspective matrices.
Diagnostic poses are explicit probe inputs, not recovered historical defaults.

## GPU boundary

`render` extracts Draw components through a Bevy update, uploads world-space
vertices and uses independently authored WGSL for the recovered low-quality
lighting/fog equations. Only opaque Plastic is accepted. A linear UNORM target,
back culling, LEQUAL depth/write and disabled blending follow the native default
Technique. Grid/shadow are explicit constant probe inputs. No PBR, tone mapping
or automatic sRGB conversion establishes historical behavior.

`audit-render` executes Metal and reads back aligned rows, compares repeated
frames, logs coverage/hash and saves a PPM inside analysis. Synthetic overlapping
triangles check near-over-far, fog, equal fog bounds and RGBA/BGRA equivalence.
`shader_pack` reads native RBXS headers/96-byte records and borrows payloads;
range/uniqueness checks are research restrictions, not claims about native malformed
input acceptance. All four native shader profiles are inventoried.

The winit viewport uses the same shader and fixed probe camera. Two smoke failures
stopped GUI testing. Offscreen format consistency is fixed and numerically tested;
surface presentation after that fix is **not tested**. It is not the Studio UI.
