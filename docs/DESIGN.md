# Design

This is a standalone offline reimplementation. The supplied Studio 0.255.0.82159
installation supplies data at runtime. The original executable is never patched
or launched. Bevy supplies application scheduling and entity storage at this stage.
Each historical behavior needs evidence; modern engine defaults do not establish it.
The current priority is the core offline client. Studio editor and editor GUI are
deferred; world presentation and player/camera input remain core client work.

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

Twenty-six source-only tests use invented inputs: version-line quirks, raw float words,
malformed layouts/indices, matrix row order, nested referents, mip rules, DXT1
endpoint branches and surface coordinates. The actual bundle
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
scaling. Unsupported shapes and reflectance return errors. Surface UVs are supplied
by a separate stage. `camera` supplies recovered view/perspective matrices.
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

## Decal boundary

`texture` independently decodes bounded square power-of-two RGBA8 PNGs with
image 0.25.10. Native libpng 1.2.40 uses straight alpha and no gamma conversion.
Mips follow the recovered horizontal/vertical rounded-up pair averages, including
alpha; hardware mip generation would not establish the same bytes. Other image
formats/resizing remain unsupported. The original PNG is read only at runtime.

`character_decals` follows explicit XML: empty texture creates no material,
SpecialMesh Head uses the full existing mesh/UVs and white decal color. It assumes
probe LocalTransparencyModifier=0; generic block decal UVs are unsupported.
TexturedDraw is a separate Bevy component; existing Draw/RenderVertex signatures
stay stable. The low-quality decal shader follows the bundle's DefaultFS albedo
equation, SRC_ALPHA color blending, separate alpha blending, LEQUAL/no depth writes,
and bias -16/slope -0.5. Metal versus original GL bias precision is **not tested**.
Anisotropy=1 is explicit probe input, not a recovered quality default.

The original character rotates its face toward +Z. The probe camera now faces
that side. A failed back-side probe changed zero pixels; corrected probe changes
120 pixels. Synthetic textures verify row orientation/clamp, straight-alpha
blending and absence of decal depth writes. Full original pixels remain **not tested**.

## Plastic surface boundary

`surface::character` reads explicit XML face surface values and the six native
atlas rows from the verified executable. For character blocks, face U extends
half the face width and V extends one thirty-second of the face height. Smooth
surfaces and the file head use zero coordinates. Unsupported surface values and
faces needing eight-stud subdivisions return errors instead of approximations.

`dds` borrows bounded legacy DXT1 mip records and implements the recovered native
squish fallback: RGB565 bit replication, floor-weighted thirds or the transparent
fourth-color branch. All twelve original 128x2048 stud atlas mips upload as linear
RGBA8. The native compressed GPU path is **not tested**; this explicitly selects
its recovered uncompressed fallback. DDS parsing restrictions are research bounds,
not claims about the native malformed-input contract.

Existing public Draw/RenderVertex signatures stay stable. A private 64-byte GPU
adapter appends separate stud UVs. Low-quality Plastic samples with repeat and
trilinear filtering, multiplying vertex color by twice the atlas red channel;
atlas alpha is ignored. Outlines are an explicit disabled probe option, with no
edge/tangent geometry or high-quality normal perturbation implied. A synthetic
texture gives `[32,64,96,255]`; the real atlas changes 5,518 pixels with identical
repeat output. Full original lighting and visual parity remain **not tested**.

## Humanoid command boundary

`player` adds HumanoidProperties and HumanoidMotion components without changing
existing public scene/runtime signatures. Properties are read explicitly from the
bundled character. Motion stores final and raw unit directions, analog fraction
and WalkTo-active state. DataModel attachment, camera presence and the full-rotation
state branch are explicit contexts; no modern Player or Humanoid defaults apply.

The direct internal setter has a squared-length fuzzy-zero threshold; the Move
command first normalizes nonzero input and stores its magnitude separately. Zero
Move preserves the prior fraction. Missing camera occurs after raw/fraction writes;
detached Move is a no-op. Ground movement follows the projected native lookAt
basis, strips Y, then unitizes; the other branch uses the full camera rotation.
Degenerate ground frames and non-finite/overflowing vectors are rejected within
this diagnostic subset. Reflection signals, replication, state transitions,
locomotion forces, collisions and animation remain separate unrecovered behavior.
Seven Bevy command updates log these decisions in `audit-player`.

## Physics boundary

`physics::NativePhysics` reads the verified image for free-fall/kernel timesteps,
kernel subdivisions, angular damping, legacy acceleration and the three equal
tiny velocity additions. Bounded x86 instruction decoding recovers immediates;
no native instructions execute. DynamicGravity selection is an explicit input,
with its original feature-flag default unresolved.

`FreeFallBody` holds a frame, linear/angular velocity and the native-sign quaternion.
Its step implements only the unconstrained native SimBody branch: damp angular
velocity, quaternion derivative and normalization, half-gravity position update,
full velocity update, then tiny additions. Public scene, renderer and import
signatures remain stable. Matrix/quaternion conversion follows native branch
choices rather than Bevy defaults.

`audit-physics` reads original Part Velocity/RotVelocity into seven independent
bodies and runs 240 explicit Bevy updates. It logs positions, velocities and
quaternions, checks the analytic Y curve at every step, and bounds errors at 0.002.
It is neither a native differential test nor an assembled character simulation.
Contact classification, constraints, mass/inertia forces, collisions, sleeping,
anchored scheduling and Humanoid locomotion remain separate work. The new
unconstrained motion probe connects physical frames to rendered geometry; static
render diagnostics continue to render the original pose.

## Visible launch and live evidence

No-argument launch opens the existing recovered renderer in a winit window.
Before showing it, a BGRA offscreen warm-up checks that geometry covers nonzero
pixels. The window uses the same low-quality Plastic/decal passes, linear BGRA
surface and explicit probe camera/lighting. An incremental log created through
the confined runtime logger flushes startup, resize, presented-frame counts and
timing while the app runs. The frame-limited smoke command exits after 120
presentations; normal launch stays open until Escape/window close.

## Physics to render bridge

The private `motion` module adds seven FreeFallBody entities to the character
viewport, six with local geometry and one transparent root. Original source
geometry is rebuilt in Part space once at setup; original world bytes are checked
before accepting the binding. Bevy orders native free-fall updates, Part geometry
updates, parented decal updates and extraction. The original scene/render public
function signatures and data fields remain stable.

GPU vertex buffers accept pose updates; topology/count/size checks guard every
upload. Textures, samplers, indices and stud UVs remain bound. Decals follow the
same body entity as their parent. Reset restores the imported body states and
recomputes from immutable local geometry, preventing transform accumulation.
`audit-motion` compares dynamic buffers to fresh GPU uploads and verifies repeat
and reset pixels. The viewport's Space trigger uses an elapsed-time accumulator
with native dt, stopping at 60 steps; R resets. The quarter-second stop is a
research limit while contacts and rig construction are incomplete. Native remote
interpolation, assembly classification and locomotion are not implied.
