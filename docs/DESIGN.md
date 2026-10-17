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

## Module layout follows the native classes

The character runtime is split the way the original client splits it:
`world.rs` is RBX::World (the assembly, realtime contact springs, kernel substeps and
collision queries against the static contents), `humanoid.rs` is RBX::Humanoid
with its HumanoidState objects (onStepped dispatch, per-state update, the
per-substep kernel force callback, and the Lua signals fired on state edges by
00bfda98), and the animation script only sees those signals. `walking.rs` is
not a native class: it is the explicit test fixture (floor plane, wall
half-space, one character) and the game loop that runs the native jobs over
World and Humanoid. Before 2026-10-12 all of this lived in walking.rs; the
split changed no numbers (every audit output was identical before and after).

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
lighting/fog equations. Only opaque Parts are accepted; non-Plastic materials
need `add_materials` (see Workspace materials). A linear UNORM target,
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
probe LocalTransparencyModifier=0.
TexturedDraw is a separate Bevy component; existing Draw/RenderVertex signatures
stay stable. The low-quality decal shader follows the bundle's DefaultFS albedo
equation, SRC_ALPHA color blending, separate alpha blending, LEQUAL/no depth writes,
and bias -16/slope -0.5. Metal versus original GL bias precision is **not tested**.
Anisotropy=1 is explicit probe input, not a recovered quality default.

The original character rotates its face toward +Z. The probe camera now faces
that side. A failed back-side probe changed zero pixels; corrected probe changes
120 pixels. Synthetic textures verify row orientation/clamp, straight-alpha
blending and absence of decal depth writes. Full original pixels remain **not tested**.

### Workspace decals

A Decal or Texture on a Workspace Part is a `DecalInstance` in
`Workspace::decals` (parent Part index, Content URL, Face, Transparency and,
for a Texture, StudsPerTileU/V). They load and save as children of their
Part; the model writes them after the joints so the Part indices joints use
stay the model indices.

`texture::workspace_decals` follows FastCluster::updateGeometry: every
Decal child whose transparency is below 1 is drawn, whatever the Part's own
transparency. An empty URL has no material and draws nothing; a URL that is
not rbxasset:// cannot load offline and draws nothing. Each texture file is
decoded once.

`geometry::workspace_decal` is addBlock's decal branch: one quad on the
Face, corners from table 017f01b8, the block's face normal, white vertex
color with the decal's alpha, the Part's specular. Texture coordinates run
(U, 0), (0, 0), (U, V), (0, V); (U, V) is (1, 1) for a Decal and the face's
size over StudsPerTile (signed per face) for a Texture. The draw's
`geometry` tag is "decal" or "texture"; the decal pass picks the clamped or
the wrapped sampler from it, as createTexturedMaterial picks the address
mode from the 0x8/0x10 flag. Decals on loose Parts are generated
part-local and posed from the Part's kernel body each frame
(`motion::LooseDecal`), like the Part draws.

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

## CustomPhysicalProperties

`PartInstance::custom_physical_properties` keeps the file's value
(property type 0x19, stored order [density, friction, elasticity,
frictionWeight, elasticityWeight]; None means the material defaults), so
places round-trip byte for byte. Physics asks
`MaterialProfile::properties(material, custom)`, the port of
Primitive::getPhysicalProperties 00a43dbe: the custom record when it is
enabled, else the material switch result. Every consumer reads that one
record: mass (density), Contact params (00a43fb4/00a44104: weighted friction
and elasticity), and the RunningBase floor friction (00a43e10 for custom,
00a43f4c for material; the same weighting). `PhysicalProperties::custom`
applies the five-value ctor clamps from Lua `PhysicalProperties.new`
(0058125c), identical to the XML reader (00ab7e9e): density [0.01, 100],
friction [0, 2], elasticity [0, 1], weights [0, 100]. Whether the binary
reader clamps on load is **not verified**; the port clamps when it builds the
physics record, which gives the same physics either way. The character's XML
colliders accept custom properties too.

## Workspace materials

`material_generator` selects the native low-quality (quality level 2) Technique
per Part, as createDefaultMaterial 010c0e00 and createRenderMaterial 010bc630 do:

- Plastic and SmoothPlastic use DefaultPlasticFS (stud atlas only).
- Plastic or SmoothPlastic with Reflectance > 0.015 use
  DefaultPlasticReflectionFS / DefaultSmoothPlasticReflectionFS (the name is
  "Default" + material + "Reflection" + "FS"; the low-quality code of both is
  the same shader). The albedo is lerped toward a cube environment map
  by the vertex reflectance, quantized as (int)(min(r,1) x 0.8 x 255) / 255.
- Neon uses DefaultNeonFS: color x glow (2.2, from the SceneManager ctor at
  011c084e, used while PostFX is below level 1), opaque blending.
- Cobblestone uses LowQMaterialWangFS; the 17 other render materials use
  LowQMaterialFS. Codes outside the 20 render materials have no material and
  are an error.

LowQMaterial samples `textures/<lowercase name>/diffuse.dds` from the install's
PlatformContent (DXT1 or DXT5, decoded with the squish fallback; DXT5 alpha uses
integer palettes) with linear/wrap filtering, plus `wangIndex.dds` (128x128
L8A8, point/wrap) for Cobblestone. The far tiling factor is 0.25 for Slate,
Concrete, Granite, Aluminum, Grass and Ice, 0.5 for Rust and 1 otherwise. The
shader formulas follow the shipped default.hlsl/common.h (fade over 300 studs,
cutoff 0.6, `lerp(1, color, a) x albedo`).

Material UVs come from getTiling 010c3c40 (read from the binary by following the
switch) and the 64-float offset table 017f0218. The native origin seed is
`part + (part >> 3)` of the Part's heap address, which cannot be reproduced;
`part_seed(index)` substitutes a deterministic 16-byte-aligned pseudo address,
so texture *offsets* differ from any given native run while tiling, the offset
table and the UV layout per face (blocks 010cca80, balls addSphere, wedges) are
the native ones. Brick and WoodPlanks use the table values directly; the rest
use ((T + T) - 1) x ((x + y) + z) x 0.5.

The GPU path (`Gpu::add_materials`) re-uploads 68-byte vertices with the
reflectance attribute and builds one pipeline per Technique and one bind group
per material. The environment map (EnvMap 010a7310, renderFace 010a8030) is a
256x256 cube with mips; each face renders the sky without stars from the origin
with perspective(pi/2, 1, 0.5, 3000). It is built once: the native time-based
EnvMap::update refresh is **not ported**, and mips use CPU 2x2 averaging (the
native GPU filter is not recovered). High-quality techniques (normal, specular
and normal-detail maps, tangents) are **not ported**.

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

## Developer panel boundary

The dev panel is development tooling, not part of the rewrite. It compiles
only with `--features dev` (optional egui crates) and all of it is in
`src/dev/panel.rs`. That file is mounted as a private child module of
`render` (`#[path]`), so it reads and changes `Viewport` fields directly
instead of widening the public API. render.rs has five cfg-gated hooks:
the `dev` field and its init, `dev::event` at the top of `window_event`
(an event the panel uses never reaches the viewport), `dev::before_frame`
at the start of a redraw, and `dev::draw` after the scene pass. Pause, Step
and time scale work by moving the viewport's motion clock, so the physics
stepping code is unchanged. TimeOfDay goes through the same
`LightingProfile::sky` / `render_lighting` path as the viewport's start-up.
To remove it, delete `src/dev/`, the hooks and the feature.
The decal tool (`src/dev/decal_tool.rs`) adds Decal entities at runtime.
The GPU decal list must stay in the extract system's iteration order, and
that order differs from a fresh query's. So a change is uploaded on the
next frame from `ExtractedDecals`, and that frame takes no physics step.

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

## R6 body mesh selection (2026-10-12)

Live Ghidra: fetchResources 010d4260 selects fonts/leftarm.mesh, rightarm.mesh,
leftleg.mesh, rightleg.mesh and torso.mesh when FastCluster's material flag 0x20
is set. HumanoidIdentifier 012a02c4 recognizes Parts in the Humanoid's parent;
012a09ba accepts the supplied opaque Plastic Parts at zero reflectance. These
are not the Composit*Base meshes. Mesh scale 010cabd0 divides Part.size by
getBodyPartScale 012a0814. Its initializer 012a10d9 copies the first four vectors
from file-backed 018363c0: head unit scale, torso 2x2x1, legs/arms 1x2x1.
These vectors are read from the verified binary at runtime.

The renderer now imports the five native files, keeps normals unscaled, retains
source UVs and builds both world/local geometry through the existing signatures.
addFileMesh 010c9d90 clears specular under flag 0x40 and emits no stud coordinates.
A private GPU attribute sentinel selects this branch without changing public
native vertex fields. The bare fixture retains per-Part color and the separate
face pass; native texture-atlas composition, UV remapping, clothing and all quality
variants are not tested or fully implemented. No visual/native differential parity
is claimed. Raw traces stay ignored in RobloxStudio/analysis/body-*.json.

## Native R6 links and Motor6D

Humanoid builder 00bdb2dc creates RootJoint, shoulders, hips and Neck as Motor6D
instances via 00bdaa52, with MaxVelocity=0.1. Its ten C0/C1 globals are populated
by 00bf9a9a, not stored in BSS on disk. The new rig module decodes the traced
initializer translation writes and NormalId arguments at runtime. NormalId
006b6c54/006b714c produces the corresponding rotations. C0/C1 preserve native
pivot offsets, including the differing shoulder and hip offsets. Incomplete
R6 sets are rejected; R15, custom joints and alternate root selection are pending.

Motor6D SetDesiredAngle 00923e7a uses (abs(value)+1)*threshold < abs(delta);
threshold is read from 01767c14. Step 00b3717c limits the angle change to
abs(MaxVelocity) per callback, without multiplying by dt. Only its constructor
zero-offset/unit-weight branch is implemented; offset decay and animator blend
inputs are not implemented. The link relation is Part0*C0*Transform*inverse(C1)
(00b370ec, 00a9d39a, 00b36fde). Root integration and the kinematic pose bridge
use existing render entities and public signatures. Native assembly classification,
force distribution, child velocity propagation and contact solving remain pending.

The rest pose matches all seven imported frames numerically with zero residual.
An animated shoulder maintains its pivot exactly. Reset restores identical
frames/pixels. Initial motion setup first failed the existing byte comparison
because rest-frame recomposition changes signed-zero arithmetic; serialized
rest frames are now preserved until a motion step. The second run passes initial
bytes, fresh uploads and pixel reset. Evidence is rig-motion*.log in the ignored
analysis directory. Callback cadence and native differential parity are not tested.

## Bundled procedural animation diagnostic

The no-tool portions of humanoidAnimateLocal.rbxm are ported into Rust, with
literals read from that source at runtime. Pose events select Standing, Running,
Jumping, FreeFall, Climbing and Seated. The Lua uses double-precision absolute
time sine inputs, then the native setter receives float angles. Desired-angle
dead zones and per-callback Motor6D limits remain active. Shoulders retain their
MaxVelocity when Standing; hip speed stays at the native constructor value.
The source uses 3.14, not a substituted mathematical pi. Tools are not implemented.

The script's wait(0.1) requests are modeled as explicitly scheduled samples,
with a 60 Hz joint-callback research input. This does not claim the native wait
queue, RunService timing or native joint scheduling has been recovered. The
keyframe scripts reference remote Animation IDs; clips are not present in the
read inventory and are not fetched. Native default script-selection/flag behavior
remains unresolved, so A and viewport-animation-smoke explicitly choose this
bundled procedural profile. Root translation is disabled for this probe; walking
forces are separate work. Existing public render signatures remain stable.

Numeric animation audit covers six states, 360 callbacks, pivot invariants and
reset. Metal animation audit checks four samples, repeated pixels, fresh-upload
comparison and exact reset. Final Running sample changes 3,950 pixels at 640x480.
The visible smoke completes 60 callbacks/120 presentations and final readback.
Manual key events and native visual/sine parity are not tested. GPU audit's first
compile attempted a nonexistent InstanceDescriptor Default; using the existing
pinned constructor fixed the second compile.

## RunningBase horizontal force boundary

Ghidra RunningBase 00c09632 establishes velocity-error feedback, cancellation of
accumulated force divided by root mass, radial XZ acceleration limiting, effective
mass multiplication and floor friction. Gain accessors 00bfd990/00bfd9a2 read
1250/150 from 017af5a0/017af5a4. Ground/air limits are read from 017acbf8/017b05c4
(500/143). The new locomotion module reads these verified binary values at runtime.
Solver choice, desired/floor velocities, material friction, root mass and selected
effective mass are explicit inputs; no modern defaults or guessed floor is used.

Six numeric cases cover start, moving-floor matching, air limiting, friction,
small PGS error and cancellation of an existing force. A diagonal test checks
radial rather than independent-axis limiting. Native floor queries, body assembly
mass/inertia, vertical spring and balancing, force integration, reaction forces
and full state scheduling remain pending. This calculation is not yet connected
to the viewport and does not establish a playable walking client.

## Contact persistence and velocity boundary

The collision module ports native btPersistentManifold::refreshContactPoints
0040294a: transform all cached local points first; recompute signed separation;
increment lifetimes; reject normal separation strictly above the threshold; reject
orthogonal drift strictly above factor-squared times threshold-squared. The old
distance chooses the activation factor only on a positive-to-negative transition.
Normals stay in world space. Removal copies the last point into the removed slot
(00402b88); reverse iteration avoids reprocessing it. File-default factors are
read at runtime from 01ca8d98/01ca8d9c; native runtime overrides are not tested.
Breaking threshold is explicit, not assumed to be a universal 0.02 for all shapes.

Roblox connector 00a929f6 computes each contact-point velocity as angular velocity
cross displacement from the body's center plus linear velocity, then B minus A
and its normal/tangent projection. 00a92bd6 negates the scalar normal projection.
Tests cover boundary equality, penetration, separation, lateral drift, activation
selection, reverse swap removal, rotation of local points and angular velocity.
Callback side effects, cached impulses, contact generation, broadphase/narrowphase,
friction/restitution impulses and the native assembly solver are not implemented.
No floor clamp or guessed modern collision engine is used. Neither contact module
nor horizontal-force calculation is yet wired to the viewport's root integrator.


## R6 assembly mass and inertia

New assembly.rs recovers the explicit R6 builder branch. Legacy XML shape=1 maps
to internal Block=2 (00708d58); the factory's internal shape=1 is Ball. Block
volume is size.x*size.y*size.z (00adf1e0), density is queried from custom properties
or MaterialProperties (00a44226/00a4346a). Plastic density is read from 01782284.
RootJoint construction sets its Part0 mass to 0.1 (00bdb6a6); this immediate is
decoded from the verified executable. Another Flying branch sets mass to 10;
that branch is not selected by this explicit fixture.

Block inertia 00adf960 calls 00adee8a. Its surface-weighted face polynomial is
implemented in observed SSE order, using the native divisor at 01834cd8. It is
not uniform-solid m/12 inertia. The seven-body tree follows recovered R6 joints;
Cofm 00a90502 copies leaf properties, sums child mass/weighted world centers,
adds rotated inertias and parallel-axis matrices (006b0140), then rotates the
result into body space. Signed off-diagonal terms remain. SimBody 00ab16aa takes
the local diagonal, reciprocates it and rotates it into world space; a general
matrix inverse would substitute a different contract.

Audit logs per-Part tensors, aggregate mass/center/tensor and inverse inertia.
Independent thin-shell integrals, rotated child offsets and translation checks
pass. Child ordering is the explicit constructed R6 order; native assembly root
selection, flags/state-dependent mass overrides and runtime differential parity
remain untested. This step does not yet integrate collisions or move the viewport.
Raw evidence stays in ignored RobloxStudio/analysis/assembly-*, *mass* and
block-inertia-asm JSON files.


## Constrained kernel and contact spring

kernel.rs adds the legacy SimBody constrained substep (00ab1d44), separate from
its free-fall branch. Initialization (00ab16aa/00ab1c28) moves velocity to Cofm,
builds angular momentum from the rotated local inertia diagonal, and retains the
body-to-center offset. Each substep applies torque/impulses to momentum, damps it
with 017a1f70, rotates diagonal inverse inertia, updates/normalizes the quaternion,
then integrates center velocity and position semi-implicitly. Force resets to
gravity; torque and impulses reset to zero. Tiny additions happen after motion.
Body publication (00ab13e8) subtracts the rotated Cofm offset and corrects linear
velocity back to the body origin. Existing free-fall public signatures are stable.

ContactSpring ports 00a93074 with explicit shape/material stiffness parameters,
unit normal, signed distance, point and both body states. Tangent displacement is
integrated over kernelDt and limited with the PREVIOUS normal force. Compression
and decompression histories decay toward -0.01; the normal-speed sign chooses
stiffness. First penetration stores its depth and produces zero normal force.
Separation clears depths/normal force but retains tangent displacement. The 1e-8
comparison constant at 0179fcb8 is a double, not a float. Returned force applies
to B and its negative to A; apply_force also accumulates contact-arm torque.

The assembly diagnostic performs 19 substeps per 1/240-second interval for one
second under gravity plus a constant center force. Independent discrete constant-
acceleration equations check center position/velocity; the rig retains seven
poses. This explicitly exercises the constrained branch without claiming native
classification. Native 00a9768c first performs adaptive contact impulses, then
separate connector passes and 19 constrained steps. That scheduler and impulse
pass are not yet integrated. BlockBlockContact's native SAT (00ae3de4), face
selection (00ae4974), up-to-eight face contacts (00ae533e), vertex-plane overlap
threshold (00ae3bd4) and geometry (00aa1d78) are now traced; porting is pending.
Raw kernel/contact evidence remains in ignored RobloxStudio/analysis JSON files.


## Legacy block connectors and awake impulse response

legacy_contact.rs implements a separate boundary from Bullet's cached manifold.
Block vertex order comes from 00ae07f2; collision face indices are read at runtime
from 017a3e68. GeoPair's vertex-plane path (00aa1d78) transforms the original
vertex into the world contact point, negates the plane face's outward normal and
computes signed separation. New connectors require separation strictly below
-0.01 (00ae3bd4). contained_faces limits the native SAT face path to identical
box rotations and one fully contained face rectangle. Native 00ae533e emits
incident corners in reverse order and returns immediately if all four are inside;
otherwise the reference corners emit forward. Partial clipping/edge pairs and
relative box rotations are explicitly rejected, not silently approximated.

AwakeContact recovers 00a936d0's awake-body branch. Effective contact mass is
inverse mass times identity minus each arm-skew/inverse-inertia/arm-skew product.
Its inverse follows 01255ab2's cofactor/determinant order and strict 1e-20
threshold. Explicit reference impulses supply the native +0xf0 correction;
restitution uses the decompression/compression ratio, incoming speed and iteration
count. Penetration target uses -0.02, freeFallDt and 0.1. Friction first uses the
unconstrained impulse; when it exceeds the cone, direction uses friction*1.2 and
the native branch enforces a minimum correction magnitude. Those constants are
read from the exact binary. Body application 00ab267c also damps angular momentum,
updates linear/angular velocity and adds the native tiny values. Public signatures
of previous steps remain stable. Native invalidation/sleep/throttling/adaptive
iteration scheduling and geometry-displacement corrections are still pending.

The audit generates four contained face contacts, applies iterative awake impulses,
checks no approaching contact remains, then sends a contact spring force through
the constrained body. Separate central collisions verify zero restitution,
0.5 restitution and equal-mass momentum conservation. Forty iterations and
material coefficients are explicit synthetic fixtures, not recovered defaults.
No complete collision world or native execution parity is claimed.


## Playable walking fixture

walking.rs connects the existing native Humanoid Move input and legacy horizontal
RunningBase forces to the recovered R6 assembly and 19 SimBody substeps per
1/240-second interval. WalkSpeed is read from character3. The selected stationary,
anchored-floor branch of 00c09632 also supplies support force: original torso
height minus current torso height times 30000, minus root Y velocity times 1100;
only a positive acceleration exceeding existing force/mass is applied, after
subtracting existing force/mass and limiting force to +/-1e7. Constants are loaded
from 017b05a0/017b05a4/017b05bc. This preserves the torso/root distinction. Native
floor queries, state flags, movable-floor reaction and balancing remain pending.
The fixture explicitly keeps support active at the original torso height and
renders a floor at the original legs' lower bounds (15). It does not floor-clamp
the root or use a modern controller. Original CanCollide flags select Head/Torso.

An explicitly defined infinite plane at x=4 generates collidable-body face
vertices. Face selection 006b2077 prefers Y over X on a tie, then Z on ties with
the largest component; live decompilation corrected the initially strict-first
selection. Vertex-plane points use the existing native contract; normals point
from the dynamic character into the wall.

Character contacts are realtime connectors, so they get springs only. Every
HumanoidState constructor (00bfd2a2 -> 00bfd678 -> 00b633fa) clears Body +0x44
(canThrottle) on the character's Parts. Connector insert 00a99514 files any
connector with a non-throttling body under KernelData realTimeConnectors. stepWorld
00a9768c runs awake impulses over contactConnectors only, so the character
receives only spring forces in the 19 substeps. Within each substep the springs
(realTimeConnectors) run before the Humanoid's own connector (Humanoid +0xdc,
type 2 = humanoidConnectors, computeForce 00bddb7a). The Humanoid force therefore
reads accumulators that already hold the contact forces. Against a wall this
produces a period-2 yaw jitter (±0.078 rad/s per substep, no net rotation);
whether native does the same is not tested. Wall and block contacts use the
recovered ContactParams 00a43fb4. The fixture wall is treated as a large Plastic
Part, so kSpring is the collider's own stiffness. Full native adaptive/sleep/invalidation/displacement
scheduling is not claimed. Mass/inertia remain the rigid rest-pose assembly;
walking poses do not play guessed clips or substitute procedural animation for
the missing default keyframe data. AutoRotate, jump, stairs, slopes and complete
balance/state dispatch remain pending. Two seconds of pressing into the wall
produce up to 0.34594154 studs penetration and some lateral/angular response.

The optional viewport-walking mode uses the existing Bevy pose/extraction path,
adds independently authored floor/wall quads, bypasses their studs and follows
root translation with an explicit fixture camera. W/A/S/D feed world-space input;
opposite keys cancel and focus loss releases inputs. R resets the assembly and
input. Existing public viewport and audit signatures and default probe behavior
remain stable. The walking smoke driver restores its scripted input each redraw
because macOS startup focus events cleared its first injected input. That first
run presented correctly but did not walk; it is not counted as walking validation.

Numeric checks reach 16 studs/second, cover 15.746418 studs during acceleration
and one second of movement, stop on release, and record nonzero wall impulses.
Metal warmup hides only character geometry/face, compares against the scene, then
restores it: 59,432 changed pixels and repeat-identical restoration prove character
contribution. A 120-frame smoke test reaches 466 physics ticks and 789 impulses.
Motion regression preserves the previous 46,558-pixel frame/hash exactly. Manual
keyboard interaction and native visual/behavior equivalence are not tested.
The shipped keyframe scripts still contain remote asset IDs; no matching 2016
KeyframeSequence data has been recovered from the local bundle.


## Walking balance and AutoRotate correction

The previous walking path omitted both native angular controllers. Wall impulses
therefore accumulated tilt and yaw while translation continued. Walking now calls
Balancing (00bd2008) before RunningBase forces on each kernel substep. Constructor
00bd1f42 copies gain 2250 and damping 50 from 017ac940/44; RunningBase 00c09128
inherits them. World up comes from 005e5110. The controller transforms world-up
cross torso-up, torso angular velocity, and existing torque into root space,
applies the full assembly tensor to the orientation error, then diagonal damping.
X/Z threshold 4000, the unusual correction-minus-existing comparison, cached
torque/countdown, negative interval divisor -12000 and offset 20 are preserved
from assembly, rather than accepting the broken decompiler stack expressions.
Torque is added to SimBody through the recovered 00a6c134 contract. No orientation
clamp, frozen axis or artificially increased damping is used.

Humanoid vtable slot +e0 points to 00bdc44c. Its non-camera AutoRotate branch uses
atan2(-desiredX,-desiredZ), current torso column-2 yaw (00bdcc6c), wrapped
float-angle difference and gain 8. Walking publishes that desired rate once per
explicit 240 Hz callback. RunningBase 00c09632 applies world-Y torque with gain
7500, root local Y inertia and +/-100000 limit on every substep; anchored-floor
angular velocity is zero. This is the supplied character's default AutoRotate
branch, not camera-relative, slope, swimming or state dispatch.

Numeric regression holds against the wall another 30 seconds: minimum torso up Y
0.9999184 (about 0.74 degrees tilt), maximum wall yaw speed 0.003509 rad/s, wall
penetration 0.11379337 studs. Four directions settle facing movement; release
damps angular velocity below 0.01 rad/s; an off-center impulse initially creates
more than 1 rad/s and recovers upright. Existing freefall and animation probes
retain their separate controllers. Native differential parity is not tested. Raw
MCP decompilation and disassembly stay in RobloxStudio/analysis/*fix-20261012.json.


## Walking limb playback and animated Cofm correction

Walking no longer uses rest joint angles. A separate additive step_animated entry
point drives the existing procedural Lua port and Motor6D step, then the same
walking/contact/kernel path. Live Ghidra Running event publisher 00c02f36 clips
nonnegative speed below 0.5 to zero and suppresses events within 95%-105% of the
previous emitted speed. Both signed/unsigned branches agree for these inputs.
The resulting event determines Running/Standing exactly as the bundled
humanoidAnimateLocal script's speed>0 branch. This avoids tiny integrator nudges
keeping the player in Running after release. Script targets update every 0.1s;
motor angles advance once per four explicit 240 Hz ticks, retaining the existing
60 Hz research cadence (native cadence not verified). Frame batching never
changes callback counts. Root/Neck remain fixed by the original no-tool branch.

When a motor changes angle, original MassPart properties are posed through the
six native links and recursively recombined. KernelBody.refresh_assembly follows
dirty SimBody 00ab16aa: preserve root origin position/velocity and angular
velocity, recompute Cofm offset/center velocity, diagonal inverse tensor and
angular momentum, retain force/torque/impulse accumulators. Balancing uses the
updated full local tensor; RunningBase yaw and contact impulses use the updated
diagonal inertia. Bodies and decals share the final rig pose and GPU updates.
Reset restores the original rig through MotionControl and the rest assembly.

Live original loader 00f6660c selects humanoidAnimateLocalKeyframe2.rbxm for R6,
which references remote ID 180426354 for walk and remote idle IDs. Those matching
2016 keyframe payloads are not present locally. Procedural playback is explicitly
selected for the offline walking fixture, not claimed as the default keyframe
behavior. The raw loader/Running/Motor/Cofm MCP evidence is kept under
RobloxStudio/analysis.

Audit-walking measures four limb swings (1.958 rad shoulders, 1.777 rad hips),
all six joint pivot errors below 0.00000191 studs, Running-to-Standing release,
32s animated wall stability and four subsequent direction changes. Minimum up Y
0.99958944 (about 1.64 degrees); max penetration 0.10572481 studs. The separate
audit-walking-render uses the actual Bevy driver, extraction and Metal upload.
It compares animated and rest limbs at the identical translated/rotated root:
5,871 / 3,984 / 5,614 changed pixels at ticks 48/120/240. Fresh upload, repeated
readback, pose restoration and full reset match byte-for-byte. No visual
inspection was performed. The 120-frame window smoke produces 467 ticks and
116 joint callbacks; previous standalone animation render hash is unchanged.


## Native job, joint and Lua wait scheduling

scheduler.rs replaces walking's explicit 60 Hz / 0.1 s grid with the recovered
default Studio chain. PhysicsJob 006bff80 runs at 60 Hz when the TaskScheduler
cyclic executive is on (data default 1 at 01cd5500; rate table 01749cf0) and
always passes the constant dt 0.016666668 (0179fbf8). DataModel 0079fb6e asks
World for a step count (00ba9892): 240*dt plus a float remainder, floored,
capped at MaxMissedWorldStepsRemembered=12, remainder ceiling 1.99999. In float
arithmetic this is exactly 4 every job. If zero, the whole physics step is
skipped. RunService gameStepped 006bc910 then advances game time by
steps*(float)1/240 and fires Stepped, the keyframe "Animation" signal and the
"Script" signal. World::step 00ba99d4 calls World::uiStep once and then the
world steps. uiStep reaches MovingAssemblyStage 00b39200, which calls Motor6D
vtable +0x44 (00b3717c) once per job, before the four world steps. Walking
now steps motors there instead of before the fourth world step. The same
pass runs the motor hinges' stepUi (see Kernel joints). A Motor6D whose
link changes wakes Part0's and Part1's assembly recursively (00b37550 ->
World::wakeAssembly 00ba7fb0), so an animating character goes
RECURSIVE_WAKE_PENDING every frame while it is in SleepStage, with
SleepStage +0x16c set: the recursive pass then wakes every assembly it
touches, up to depth 8, awake or not (`LooseWorld::wake_character_recursive`).

Lua `wait` (005b0774) clamps to the Lua setting minimum 0.03 and enqueues
wall-clock now and now+seconds. WaitingScriptsJob (60 Hz) resumes threads whose
wake time has passed and pushes (now-enqueued, RunService+0x78). That second
value is accumulated only by the 30 Hz HeartbeatTask (006c05ca/006c0ad8) using
float-rounded wall dt. The bundled script uses `local _, time = wait(0.1)`, so
its sine input is Heartbeat time, not world-step time. Wake times compare
against wall-clock time; on the ideal 60 Hz model clock, double rounding makes
resumes occur every 6 or 7 frames (native jitter would also vary this).

Fixture assumptions that remain: ideal frame clock k/60 with no jitter; job
order Heartbeat, WaitingScripts, PhysicsJob in each frame; script start
(first wait) at clock zero; first Heartbeat dt zero. The throttled
world-step branch, legacy non-cyclic 8-step branch, sleeping-assembly joint skip,
long-step work and keyframe Animator stepping are recorded, not implemented.
The rest-pose walking audit is numerically unchanged. The standalone A probe and
audit-animation keep their explicit 60 Hz research cadence. Evidence summary:
RobloxStudio/analysis/scheduler-native-20261012.md.


## Native Humanoid state dispatch

humanoid_state.rs reads the native STATE_TABLE (017af5d0, 17 states x 26
events, 18 = none) and the floor-ray, timer, gain and limit constants from the
exact binary. Walking now runs Humanoid::onStepped (00bde4f2) on each 60 Hz
Stepped, before Motor6D stepUi and the world steps: findFloor (00bff784),
state timers, then the first event in table order whose target is enabled and
whose condition holds (00c0090e/00c0132c). A transition builds a fresh state
object (timers, floor, deadband, balancing gains reset; Humanoid.Jump cleared),
runs the new state's onStepped, then its update and the event slot.

Floor search casts the native ray pattern from the Torso: five Z-offset rays at
-0.4*height (first three may define the floor), else four corner rays; length
is leg height * 1.1 (no previous floor) or 1.5 (+|vy|/100 beyond 100) plus 1.
The support height is the mean hit Y + half Torso height + leg height, which
replaces the earlier constant torso-height fixture. Implemented states:
- Running / Landed (RunningBase): balancing 2250/50 or 7500/50, AutoRotate yaw
  and horizontal force from desired motion computed at Stepped, vertical spring
  only with a floor. Landed returns to Running after 0.05 s.
- Jumping: no balancing; pushes 500*(JumpPower - v.dir)*mass along +Y until the
  ceiling rays hit, the 0.5 s timer expires or the floor is lost.
- Freefall: balancing kP 5000, yaw torque 7500*I limited to 120000*I, air
  control gain 1250 limited to 143, Head/Torso friction 0. Lands when a floor is
  found and Torso relative vertical velocity is not upward.
- RunningNoPhysics: entered from Running after more than 1 s without Torso or
  Head contacts and no BodyMover. The Torso assembly leaves the
  kernel; each Stepped moves it kinematically toward the desired velocity
  (gain 1250, limits 500/143), snaps it to the support height and yaws it by
  AxisAngle(up, yaw*dt). A Torso or Head contact returns it to Running.
  - The touch test c0192c checks Humanoid +0x2a4 (bdd830), then +0x27c
    (bdbe54). The RootJoint builder 00bdb2dc shows what they are: +0x2a4 is
    RootJoint's Part1 and Part0 of the shoulders, hips and Neck, so it is
    the Torso; +0x27c is Neck's Part1, the Head. (+0x274 is the
    HumanoidRootPart, RootJoint's Part0.)
  - c02209 walks the Part's contacts, skips those whose other Part is inside
    the character, and returns at the first whose vtable +0x24 reports a
    collision.
Events follow 00bfda98: Jumping fires Jumping, Freefall/Landed fire
FreeFalling, Running/RNP fire raw Running(speed) on either edge plus the
per-state deadband. The bundled script's handlers turn them into poses.

**Floor Parts and movable floors.** The rays go through ContactManager::getHit
(00ae818c, getFastHit 00ae8274, getSlowHit 00ae7d08) with the HumanoidState hit
filter (00c02844): it ignores a primitive that is being dragged (+0xca,
DraggingV1), has CanCollide off (+0xcc) or belongs to the character's own
assembly. Anchored and unanchored Parts are treated alike, so `World::raycast`
offers the static blocks, then the loose Parts, then the fixture planes, and
the nearest hit wins. The first primary hit's Part is the floor (+0x60, kept
as `FloorQuery::part`).
- Floor motion (00c02c28): the floor primitive's velocity at the touch point
  (+0x6c) and its angular velocity, times the environmental-throttle factor
  00ba69b0 (1 - 1/100000000 = 1.0 in f32 when unthrottled). Anchored floors
  give zero. 00be32ea zeroes it when the character's SpatialFilter phase is
  above 2, which never happens in Play Solo (mode 0, phase 0).
- RunningBase uses it in the yaw target (+0x114 + desired), the horizontal
  target (floor + desired) and the vertical damping (vy - floor vy).
  RunningNoPhysics adds it to the kinematic velocity and yaw (00c07626).
- Mass: an anchored floor uses the character's mass. A loose floor uses
  min(character, floor assembly) mass, and when the floor is lighter the
  support acceleration is capped at 10000 (00c0ab74) and the force at 1e5.
  The support force's -0.5 share (016f71e8) is applied to the floor body at the
  touch point (00c0abf2). The legacy horizontal reaction is the zero vector,
  because Running's Y component is 0.
- Jumping (00c05e0e) uses min(character, floor) mass for every floor (Body
  mass is set for anchored Parts too, 00b63366). It pushes the floor with
  -(force - existing Y force) at the touch point. Order checked against the
  instructions:
  - v . d is (vy·dy + vx·dx) + vz·dz;
  - the mass is MINSS(floor, root);
  - the push runs only when force > existing (UCOMISS/JBE);
  - the torque is r × (0, -f, 0) about the floor body's +0x38.
  A sleeping floor still takes the pushes on its SimBody, outside the
  kernel; they are dropped when it wakes (00b7439e, see setAssemblyState).
  `Humanoid::jump_pushes` logs every push for audit-part-jump.
- The kernel pre-step (00a96eb0) only moves contact bodies into
  realTimeBodies (on any force, torque or impulse). A loose floor under a
  standing Humanoid therefore stays a realtime body while it is pushed.
- Steep floors (00c0a498/00c07626): when up . n < cos(MaxSlopeAngle), the
  velocity loses its normal component and its downhill component becomes
  WalkSpeed. Walkable slopes are left alone. The test is written `!(d < limit)`
  so a NaN skips the rewrite, as the native `<` does. Each rewrite is logged
  in `Humanoid::steep_floors` for audit-humanoid-slope.
- Jump event 2 (00c0177e): the slope is c0215e:
  - with a floor: up . floor normal;
  - otherwise: the stored last normal (+0x22c) when 01d409c4 is set, else 0.
  In Running (8) or RunningNoPhysics (10), the event is false when the
  slope <= cos(MaxSlopeAngle). Otherwise it is Jump && JumpPower > 0.
- Event 17/22's new-contact test (00be141e) counts HumanoidRootPart contacts.
  CanCollide-off Parts get no contacts (00ae714a), so it stays false.

Fixture boundaries: the world is still a floor plane plus wall half-space;
"touching" means the part generates a wall vertex/plane connector; climbing,
FallingDown/GettingUp, Seated, PlatformStanding, Swimming, Dead and the PGS
solver branches are not implemented (FallingDown returns an explicit error).
RunningBase floor friction (00c09632): when there is a floor, the horizontal
force is scaled by 00a43f4c(floor material): (w_floor*f_floor + w_plastic*f_plastic)
/ (w_floor + w_plastic), or 0 when the weights sum to 1e-6 or less; Parts with custom
physical properties use 00a43e10 with the same formula. Plastic on Plastic gives
0.3. The legacy MovingNoPhysicsBase path has no friction.
Jumping from rest rises 8.66 studs in this model (three push frames, then free
flight); native jump height is not tested. Evidence summary:
RobloxStudio/analysis/humanoid-state-native-20261012.md.


## PlayerScripts camera (ClassicCamera)

camera_script.rs ports the bundled characterCameraScript.rbxmx desktop path:
CameraScript selects ClassicCamera (Computer, Default -> Classic), PopperCam
(DevCameraOcclusionMode Zoom) and TransparencyController, all updated in the
"cameraRenderUpdate" render step. Literals are parsed from the original Lua at
runtime and its code contracts are checked, so a changed script fails loudly.
- Spawn: ZoomCamera(12.5), then after one wait the look is set behind the
  character: (Torso.lookVector - (0,0.23,0)).unit via RotateInput.
- Update: focus = Torso.p + Torso:vectorToWorldSpace((0,1.5,0)); eye =
  focus - zoom*look, frame = CFrame.new(eye, focus) (G3D lookAt), zoom >= 0.5.
  RotateVector uses CFrame.Angles(0,-x,0) * lookAt * CFrame.Angles(-y,0,0);
  pitch is clamped to +-80 degrees.
- Input: right-drag pan (delta/1920, delta/1200) * (4pi, 1.9pi); arrows turn
  120 deg/s (delta capped at 0.1 s); wheel ZoomCameraBy(clamp(-1,1,-z)*1.4);
  I/O ZoomCameraBy(-+5). ZoomCameraBy starts from the actual eye distance and
  runs the Lua RK4 integrator (UserBetterInertialScrolling assumed off). First
  person when zoom < 2. Zoom limits come from Player setters 00b8a9f8/00b8a99a
  (0.5..400); constructor defaults were not located.
- PopperCam: five near-plane points (FOV, viewport), rays from focus-shifted
  starts, pulls the eye forward by the largest cutoff and restores 0.3/frame.
  The near clip plane ends on the obstruction, so the eye can stay up to 0.5
  behind it. Its CastRay is workspace:FindPartOnRayWithIgnoreList (00882368):
  getHit 00ae818c with the ignore-list filter 0086dc78, which ignores only
  descendants of the listed instances (the player characters). It has no
  CanCollide test. A miss returns the ray end. PiercingCast recasts past
  Transparency > 0.95 or CanCollide-false hits. `World::camera_raycast` takes
  the nearest acceptable hit over static blocks and loose Parts; this is the
  same Part the recast loop ends on.
- TransparencyController: (7 - distance)/5, zero below 0.5, 2.8/s tween,
  rounded to 0.01. The value is computed and logged; character transparency is
  not yet rendered.
WASD now goes through the ControlScript's Player:Move(move, true): input is
camera-relative using the recovered camera projection. Camera.FieldOfView stays
the explicit 70 degrees (setter 00734682 clamps 1..120; constructor default not
located). First-person (CameraRelative) Humanoid rotation, ShiftLock, gamepad,
touch, VR, Follow/ClickToMove modes and vehicle branches are not implemented.
Camera updates use the presented frame's wall-clock time; the headless audit
uses ideal 60 Hz frames ordered camera update, then four world steps.


## Native Lighting defaults and day-cycle colors

lighting.rs recovers the Lighting service defaults and Roblox's modified
G3D::LightingParameters from the binary with a small store tracer over the
original constructors and setTime's static-init blocks (SSE constant copies,
immediates, Color3::white()/black() results and scalar multiplies). Defaults:
TimeOfDay 14:00:00, latitude 41.7333, Ambient 0.5, OutdoorAmbient 0.5,
Brightness 1, ColorShift 0, FogColor 0.75, Fog 0..100000, ShadowColor
(0.7,0.7,0.72), Outlines on, GlobalShadows off. setTime places the sun on a
daily circle, tilts it by season (23.5 deg axial tilt) and latitude about
Z x sun, and the moon by phase with a 5 deg tilt. With physicallyCorrect the
light is the moon when the sun's Y <= -0.3. Five colors (light, ambient,
diffuse ambient, sky ambient, horizon) are piecewise-linear splines over the
local day. presetLighting turns them into Lamp0 = 0.9*Brightness*lightColor
plus ColorShift_Top deviation (clamped to 1), Lamp1 = 0.4 of that with
ColorShift_Bottom, AmbientColor = Ambient, Lamp0Dir = -light direction. Fog
uses SceneManager::setFog's (0, start, end, 1/(end-start)), matching the
existing shader packing; the shipped default.hlsl uses the same ambient/lamp
equation as the WGSL adapter. At 14:00 Lamp0 is 0.675, Lamp1 0.27, Ambient 0.5.
viewport-walking now uses these values; other probes keep their explicit
lighting so their regression hashes stay comparable. Not implemented: the voxel
light grid (OutdoorAmbient skylight and local lights, LightBorder), shadows,
FrameRateManager fog culling and the PGS/high-quality material paths.

## Keyframe animation (Animator, AnimationTrack, Animate script)

The R6 character animates with the August 2016 keyframe clips, not the old
procedural motor script. The 0.255 loader picks
`content/fonts/humanoidAnimateLocalKeyframe2.rbxm` for R6; that LocalScript
names remote Animation ids (idle 180435571/180435792 weighted 9:1, walk
180426354, jump 125750702, fall 180436148, climb 180436334, sit 178130996,
tool and emote clips). `tools/fetch_animations.py` downloads them into
`RobloxStudio/assets/<id>` (untracked, outside the read-only install) and
refuses any asset updated after the build's content date (2016-08-04); all 23
were last updated in 2013-2015, so their current versions are the ones the
2016 client loaded. `rbxm.rs` reads both binary and XML model files.

`animator.rs` ports the native classes. A KeyframeSequence is cached as
keyframes sorted by time, each holding per-track poses of (translation,
axis*angle rotation vector from G3D toAxisAngle, weight clamped to [0,1]);
tracks are (parent Pose name or "__Root", Pose name) pairs and match a joint by
(Part0.Name, Part1.Name), so the root Pose drives nothing and the Torso Pose
drives RootJoint. Sampling interpolates linearly between the surrounding
keyframes that contain the track, blending rotation vectors with a short-way
wrap. An AnimationTrack advances `time += speed*dt` (looped clips wrap with
fmodf, others clamp), fades its weight linearly toward the target, fires
KeyframeReached for keyframes not named "Keyframe", and a non-looped clip at
its end fires its last keyframe, fades out over 0.3 s and stops.
Animator::onStepped runs on RunService's Animation signal right after the
Humanoid's Stepped handler: it drops faded-out tracks, then for priorities
Core(1000), Idle, Movement, Action blends each priority's tracks into a layer
and the layer over the result (later layers win by weight), and hands every
Motor6D its pose. Motor6DJoint stores the pose pre-multiplied by weight, sets
the motor-angle weight to 0 while animated (1 otherwise) and a 32-step
countdown; stepUi then builds the link rotation vector (anim.x, anim.y,
angle*motorWeight + anim.z) and translation, and the joint frame is
fromAxisAngleFast of that vector. If poses stop arriving the offsets decay
back to the plain motor angle.

`animate_script.rs` ports the Animate script as a state machine over that
Animator, reading its table and literals (transition 0.1, fall 0.3, jump
duration 0.3, walk speed/14.5, climb speed/12) from the shipped Lua source.
Humanoid signals run its handlers synchronously, its `wait(0.1)` loop runs
`move(time)` on WaitingScripts resumes, and the "End" keyframe re-rolls the
weighted choice. Assumptions: Lua math.random uses Darwin rand() seeded 1
(native seed unknown); clips are available immediately (native asset loading
is asynchronous); `run.xml` in the table is never played and is not loaded.

## Default sky (Sky::render)

sky.rs ports RBX::Graphics::Sky for the case with no Sky instance in Lighting.
Sky::update (010b5560) reads the LightingParameters from lighting.rs; Sky::render
(010b6210) draws, in order: six skybox faces, the sun billboard three times, the
moon once, all with the shipped sky.hlsl (ported to sky.wgsl). Faces are the
default `PlatformContent/pc/textures/sky/sky512_{rt,up,bk,lf,dn,ft}.tex` (DXT1
DDS) in loadSkyBox slot order, which equals NormalId 0..5. Each face's rotation is
Ry(+90 deg for face 1, -90 deg for face 4) * diag(-1,1,1) * NormalId frame, scaled
2700 and centered at camera + 2700 * (third column), so the box follows the
camera. The unit quad is (+-1, +-1, 0) as a triangle strip. The vertex shader
snaps z to w - 1/16 (behind all scene geometry, depth test on, no depth write)
and tints by lerp(Color2 = sky horizon, Color = sky ambient, worldY/1700). The
sun is a camera-facing billboard (FUN_006b3ea1 basis) at 2700 along the true sun
direction, scale 500, additive, color = emissive * 0.8 * clamp((sunY+0.1)*10).
The moon is scale 250 at the true moon direction, color min(1 - skyAmbient.g +
0.1, 1); it only sets Color, so Color2 keeps the sun's value as in the native
code. Sun/moon textures are content/sky/sun.jpg and moon.jpg (decoded with the
image crate's jpeg feature). Assumptions: the blend enum mapping (One=0, Zero=1,
SrcAlpha=3, InvSrcAlpha=4) is inferred from use, not from a symbol; faces use
mip 0 with a linear/clamp sampler; the star field (drawn when the moon is up)
and custom Sky instances are not implemented.

## Model files (rbxm.rs)

`rbxm::Model` is a plain instance tree: a list of `ModelInstance`s (class,
properties by name, children) and the top-level indices. A Ref property
holds an index into the list. It knows nothing about reflection; the
loaders that turn it into a World (and back) decide which properties a
class has.

The binary format is the native SerializerBinary (reader 00e117bb, writer
00e10866). The writer:
- puts one INST chunk per class, in class-name order. The native map is
  keyed by Name::sortIndex, which is the alphabetical rank (Name.cpp:116);
- then one PROP chunk per class and property, also in name order;
- then one PRNT chunk listing every instance in post-order;
- then END.

Referents are positions in a pre-order walk from the roots. All chunks but
END go through LZ4_compress (00e07889), and `lz4_compress` ports it exactly:
the output bytes depend on the hash size, the skip strength and the match
rules, not only on the format. So
`Model::to_binary` is byte-identical to the native writer whenever the tree
and the property sets match what native reflection would give. The native
writer emits every property whose descriptor has the XML-write bit (+0x24 bit
0x10, from functionality bit 8 in the descriptor constructor 00562476); the
loaders have to fill in all of them.

`Workspace::to_model` and `from_model` are that loader for the classes
the port simulates (Part, WedgePart and the joints). They write every
XML-writable property and read in name order. A save puts the Parts first,
then the joints, all at the top level, so a load gives back the same
creation order.

The XML format is in rbxmx.rs. The reader follows TextXmlParser (00bb92c0):
a hand-written tag scanner, not a conforming XML parser. It reads text up
to the next '<' and decodes only the entities the native decoder knows
(`&#N;` is a byte, not a Unicode character), and it keeps CDATA raw, line
endings included. The writer builds the element tree the property
descriptors build and prints it the way TextXmlWriter (00bba086) does, so
tabs, CDATA choice, escaping and number formats match. Each instance keeps
the `referent` it was read with (`ModelInstance::referent`): natively the
loader stores it on the Instance and the writer reuses it. New instances
get a counter-derived "RBX" + 32 hex id where Studio would use a random
GUID. Properties are written in name order, which is the native order
(every native file is sorted).

`Property` has Content, ProtectedString and BinaryString next to String so
an XML file keeps its tags. The binary format has one string type, so
these come back as String from a binary file. XML writes BrickColor as
`<int>`, so it comes back as Int. Native Studio gets the type from each
property's descriptor. The port has the slice of the reflection database
that this needs, in reflection.rs: the (class, property) pairs whose type
is BrickColor, Content, ProtectedString or BinaryString (69 pairs), and
the base chain of the classes that have or inherit them. It was traced in
Ghidra (RESEARCH.md). Both writers start from `Model::with_native_types`,
which retypes by that table (Int -> BrickColor, String -> Content /
ProtectedString / BinaryString) and borrows the model when nothing
changes. So a generic binary -> XML -> binary conversion gives the same
bytes, and binary -> XML writes the native tags. The readers do not
retype: a Model read from XML still holds Int for BrickColor, and one read
from binary holds String (`ModelInstance::string` and `token` accept
either). Strings that are not UTF-8 on a Content or ProtectedString
property stay `Bytes` and are written as `<string>`.

Places made for the port (the viewport demo, and later the audit places)
are binary .rbxm files in `places/`, written with `Workspace::save` and read
with `Workspace::load`. To change one: load it, edit the Workspace in code
(a one-off test or command), save it, and check the result with
`rbxm-dump`. The Studio new-document Baseplate stays in code
(`Workspace::studio_new_document`), because RobloxIDEDoc builds it in code
too.

## Studio default world (Workspace, Baseplate, Play Solo spawn)

workspace.rs holds the Workspace contents as plain `PartInstance` values with
the properties the core client reads. `PartInstance::default()` is
Instance.new("Part") from the PartInstance constructor (009828ce): size
(4, 1.2, 2), BrickColor 194, Plastic, friction 0.3, elasticity 0.5,
unanchored, Top Studs and Bottom Inlet (Primitive +0xd2+NormalId), other faces
Smooth. The size setter only raises each axis to the 0.2 minimum.
`Workspace::studio_new_document()` is RobloxIDEDoc's new place (001017a6): one
Part "Baseplate", 512x20x512 at (0, -10, 0), Anchored, Locked, BrickColor 199,
so its top face is y = 0.

Play Solo without a SpawnLocation (calculateNextSpawnLocation 00f6bdf6) spawns
at (0, 100, 0), moved over the extents only when the workspace extents miss the
origin. The safe-spot search (00ae8e9a) returns at once when the extents' max y
is below the candidate; its occupied-region search is not ported and returns an
error. `character_placement` follows SpawnLocation.cpp (009f4706): the model is
moved so its location (the PrimaryPart, which is the Head in character3) is at
spawn + (0, 7, 0), then turned by lookAt along the spawn's horizontal look
vector. In the default world the Head lands at (0, 107, 0) with identity
rotation, so the character faces -Z. Evidence:
RobloxStudio/analysis/workspace-native-20261012.md. `tools/macho_exports.py`
lists the binary's 9,471 exported names (mostly RBX::Graphics and G3D).

World statics can be the walking fixture or a Workspace. `World::in_workspace`
disables the fixture planes (infinite floor_y/wall_x), turns every anchored,
colliding Part into a `StaticBlock`, and places the character with the spawn
transform above. Ray queries follow ContactManager::getSlowHit (00ae7d08): a
bounding-sphere test, then Primitive hitTest (00b631de), which moves the unit
ray into part space and calls the Block geometry's hitTest (vtable +0x70,
00adeffe) = G3D collisionLocationForMovingPointFixedAABox (0124e766, Woo's
algorithm: a ray starting inside misses; the sign-bit test on t; the hit normal
is -1/+1 on the chosen axis). The hit with distance > 0 and strictly below
min(length, 5000) and below the best so far wins. The spatial-grid walk of
getFastHit (00ae8274) is not modeled: it only orders candidates, so ties
between equally distant hits on different Parts could resolve differently.
Unanchored Parts are rejected for now. Torso/Head contacts with Parts use the
BlockBlockContact port (next section); `touching` (the Humanoid's Head test)
still uses a box-overlap stand-in. `WalkingFixture::baseplate` runs Play Solo in
that world and `WalkingFixture::in_workspace` in any Workspace of anchored
Parts.

Workspace Parts are drawn by `geometry::workspace_block`, a port of
GeometryGenerator::addBlock (010cca80) for blocks without a decal. Corners are
world-space from the Part CFrame in the native add order; normals are the
normalized CFrame columns. Each face uses the executable's corner table
(017f0140). Smooth faces (types 0 and 10) are one quad; other faces are cut into
n = (int)(length/8 + 0.999) strips along Y (side faces) or Z (Top/Bottom),
stepping -Y on side faces, -Z on Top and +Z on Bottom. Stud coordinates for
types 1-5 run u = width/2 -> 0 and v = row -> row + 0.25 per full strip (row from
017f045c), and the last strip ends at row + clamp(rest/8, 0, 1) * 0.25, so a
single-strip face matches the character `face_uv`. The Baseplate is 132 quads:
64 Studs strips on top, 64 Inlet strips underneath, four Smooth sides. Material
UVs are produced for workspace Parts (see Workspace materials); tangents and
outline edge distances are not (the low-quality shaders do not read them;
outlines are disabled). More than
0x2000 quads is an error, as the native assert. `viewport-baseplate` and
`audit-baseplate-render` add these draws to the walking viewport path with
`motion::install_world(.., baseplate=true)`.

GeometryGenerator::addPartImpl (010d1850) picks the generator by Part type:
- 0 Ball → addSphere (010caee0);
- 1 Block → addBlock;
- 2 Cylinder → 010da0d0;
- 3 Truss → addTruss;
- 4 Wedge → 010dc290;
- 9 CornerWedge → 010e14b0;
- 0xb Operation and 0xc MeshPart have their own.

A SpecialMesh's MeshType (+0x78) reroutes to the same generators (3 Sphere,
6 Brick, 1 Torso, ...).

`geometry::workspace_sphere` ports addSphere for a Ball without a decal. It
builds a cube sphere:
- **Grid size.** side = min(x, y, z), clamped to 350 (017efdbc). A Part
  whose six surfaces are all Smooth (isPartSmooth 010cae50, raw type 0, so
  SmoothNoOutlines does not count) gets tiles = 1. Otherwise
  tiles = ceil(side / 7) (01787ef0). Each tile is rows = (tiles + 5) / tiles
  quads tall, and a face is tiles x rows quads across.
- **Face grid.** Each cube face starts at the block corner table's first
  corner (017f0140). It steps along a column direction (0224e0f0:
  +Z, +X, -X, -Z, +X, +X) and a row direction (0224e0a0: -Y, -Z, -Y, -Y,
  +Z, -Y), by 2 / (tiles x rows) each, adding the half step twice as the
  native does. Each strip has rows + 1 vertex rows; its last row repeats as
  the next strip's first.
- **Projection.** A grid point d becomes size x 0.5 x d / |d|. Its normal is
  (d / |d|) / (size x 0.5), rotated by the CFrame and normalized (an
  ellipsoid's normal; the Part size is uniform for a Ball anyway).
- **Studs.** Surface types 1-5 get stud coordinates: u runs from
  side / (2 tiles) down by side / (2 tiles rows) per vertex. v starts each
  strip at frac(strip x side / tiles) / 8 x 0.25 + row and grows by
  side x 0.25 / (8 tiles rows) per row. side here is the clamped one.
- **Indices.** Quads use the block's order (a, a+1, a+W, a+W, a+1, a+W+1).
- **Float order.** The disassembly (010cc130..010cc4f0) is the reference,
  not the decompiler:
  - sums run ((x + y) + z) in G3D order; Ghidra prints such chains reversed
    and without parentheses;
  - a position is (size x 0.5) x (d / |d|);
  - the grid steps are doubled once and then added once per vertex or row.
- **Not produced.** Tangents and outline distances are left out, as for
  blocks. Material UVs are produced (see Workspace materials). The vertex
  color is 010ca980, shared with blocks.

`audit-ball-render` checks the mesh against the native counts and the
sphere, then renders a Ball beside a cube and compares their silhouettes.

## BlockBlockContact narrowphase (box against box)

block_contact.rs ports the narrowphase the Contact class runs for two Block
primitives (BlockBlockContact, vtable 01c86860). It works on `Block` values (a
body frame and a Part size); body 0 is the contact's first primitive and body 1
the second. The face vertex (017a3e68) and face edge (017a3ec8) tables are read
from the install by `BlockTables::load`. Arithmetic keeps the G3D source order
the disassembly shows: dot products and matrix rows are ((x + y) + z) and a
point is (R * p) + t.

`update` (ae32f4) does the following:

1. It tests the primitive extents first. Each extent is the center ± the
   largest |R * vertex| over vertices 0..3, widened by 0.01 (b61c9a, adf0a0).
   The overlap test aec6e4 counts touching as overlap.
2. It runs `compute_is_colliding` (ae3de4), a 15-axis separating-axis test:
   - **Face axes.** The test starts at the cached separating body and axis and
     re-reads them as the loop bounds, so it can revisit an axis. An overlap
     ≤ 0 stores the separating (body, axis) and returns "apart". A strictly
     smaller overlap becomes the face feature: the axis, +3 if the other box
     lies on the negative side.
   - **Hysteresis.** If the previous features were face features, the previous
     face is kept unless the new minimum is 1.01 times smaller.
   - **Edge axes.** Edge pairs are tested A axis i by B axis j. A near-parallel
     cross product (length ≤ 0.001 after unitize at 1e-6) returns the current
     plane flag immediately. An edge pair wins only when its overlap times 10
     is below the face minimum. Its edges come from the quadrant of the
     separation direction (adf6ac).
3. If the boxes are not colliding, it deletes the live connectors and resets
   the features to -1.
4. Otherwise, in the kernel, it generates connectors into the other buffer,
   deletes what is left in the current one, and flips the buffers.

Face contacts (ae4974, ae533e) work as follows:

- **Reference and incident faces.** The face feature names the reference
  body. The incident face is the incident face most aligned with the reverse of
  the reference normal (006b2077).
- **Projection.** Both faces are projected to the reference face plane
  (adf63a).
- **Incident corners.** Corners that fall inside the reference rectangle are
  emitted in the order 3, 2, 1, 0 as vertex/plane connectors. If all four are
  inside, the function stops there.
- **Reference corners.** Otherwise, reference corners inside the incident
  polygon (cross ≥ 0 against each edge from corner e to corner e+3) are emitted
  in the order 0..3.
- **Edges.** Unless no incident corner and all four reference corners were
  inside, each reference side 0..3 is tested against incident edges 2, 1, 0, 3.
  Where both straddle tests change sign, a clipped edge connector is emitted.

Connectors are kept only while they penetrate by more than 0.01:

- Vertex/plane connectors are accepted by ae3bd4.
- Edge connectors are dropped when their length is ≥ -0.01.

The native GeoPair computations are ported:

- **Type 3 (aa1d78).** Vertex against plane. The normal is minus the plane's
  outward normal.
- **Type 4 (aa1f76).** Closest points of the two edge lines in body A space.
  They are kept only within each edge's length/2 + 0.05, compared in double.
  Otherwise the result is the edge start with length 0.
- **Type 5 (aa29c0).** Closest points of two infinite lines. The normal is
  unitized from B to A; parallel lines give length 0.

Connector reuse follows ae30a8/ae6024:

- The previous buffer is searched by bodies and ids, using the queried pair
  type rather than the stored one, and edge/edge matches either way round.
- A match is swap-removed from the previous buffer and keeps its kernel state
  (`Connector::state`), so spring and impulse accumulators persist across world
  steps.
- GeoPair id fields that the native constructor never initializes are `UNSET`
  here; in the native they hold whatever the pool memory held. That is a known
  difference.
- A buffer holds 8 connectors, as the native data does; overflowing it is an
  error.

Not ported:

- how the broadphase orders the two primitives. ContactManager::createContact
  (00ae714a) keeps the order the broadphase gives it for two blocks.

`audit-block-contact` logs a resting Part (four vertex connectors at -0.05), a
lifted Part (none) and a tilted Part on an edge (two). Evidence:
RobloxStudio/analysis/block-block-contact-native-20261012.md.

In the World, every character collider (Torso and Head) has one
BlockBlockContact per static block, with the character Part as body 0 and the
block as body 1. Each world step does the following:

1. It runs `update` with the step-start pose, so connectors are created, reused
   or deleted.
2. In every kernel substep, it refreshes each connector's geometry for the
   current pose and applies its contact spring. These are realTimeConnectors
   (the character cannot throttle), so the awake impulse pass skips them.

The kernel code takes the character as contact body A. When the static block
is the pair's A (a block corner on a character face), the normal is therefore
negated, which is the same as swapping the two bodies.

When the Humanoid is kinematic, `update` runs outside the kernel and no
connectors are made. `clear_contacts` (the assembly re-entering the kernel)
drops the contacts, as it does the wall states.

Two things are not native: the connector order across pairs, and the
pair-per-block lifetime (the broadphase is not ported). `audit-part-wall`
walks into an anchored 20x10x2 Part whose face is at z = -10.

## Ball geometry and its contacts (ball_contact.rs)

A Part's Shape picks its Primitive geometry (00708d58, factory 00b614f4):
Ball (Shape 0) is geometry type 1, Block (1) is 2 and Cylinder (2) is 3. The
size setter (00988cfc) makes a Ball's size (x, x, x), because
`hasThreeDimensionalSize` (0070915c) is false for a Ball.

Ball geometry (Ball.cpp, vtable 01c865e0):

- **Radius.** size.x / 2 (setSize 00adc208, stored at +0x14).
- **Volume.** ((4/3 pi r) r) r (00adc148, constant at 01834ffc). The
  Primitive mass is volume x density (00b62e8e), as for a block.
- **Moment.** ((m 0.4) r) r on the diagonal (00adcb76, 0.4 at 016f753c).
- **Extents.** center -/+ r -/+ 0.01 (00b61c9a with the Ball half extents
  00adcb5e).

`MassProfile::ball_volume` and `ball_inertia` hold the volume and moment.

ContactManager::createContact (00ae714a) picks the contact class from the
collide types (geometry +0x10: Ball 1, Block 2, Poly 3). The lower type becomes
the contact's first primitive; equal types keep the broadphase order. A
ball/ball pair is a BallBallContact and a ball/block pair a BallBlockContact.
Each holds at most one connector, which `BallContact` models. Its connectors
are `block_contact::Connector`s with four new GeoPair kinds, so the kernel code
that reads connector geometry needs no change.

- **BallBall test (00ae2824).** It does no extents test. Colliding means
  r0 + r1 - ignored > 0 and |p1 - p0|^2 < (r0 + r1 - ignored)^2.
- **BallBall connector (00a946c6).** The normal is unitize(p1 - p0). The
  length is that distance minus both radii. The point is on ball 0's surface.
- **BallBlock test (00ae2aec).** It runs in four steps:
  1. The extents must overlap.
  2. The ball center goes into block space.
  3. The Block clamp (00adf544) pulls it onto the box. It counts the clamped
     axes and records each axis's side.
  4. The pair collides when the distance from the center to the clamped point
     is below r - ignored.
- **BallBlock feature (00ae2d16).** The clamp count picks it:
  - 0 (center inside): the nearest face (00adf472);
  - 1: the clamped face (00adf30c);
  - 2: the edge along the unclamped axis, from its -axis end (00adf206);
  - 3: the clamped corner vertex.

  The connector stores the radius, the block vertex, the face or edge id and
  the kind.
- **BallBlock connector (00a947b8).**
  - **Plane (00a94800).** The normal is minus the face normal. The point is
    the center projected onto the face plane. The length is the *unsigned*
    distance to the plane minus r, so a center past the plane does not give a
    deeper penetration.
  - **Edge (00a94a18).** The point is the nearest point of the edge line. The
    normal points from the center to it.
  - **Vertex (00a94c50).** The point is the vertex.

  FFlag BallBlockNarrowphaseFixEnabled (01d72b94) is BSS, so off. Turned on,
  it would move the plane and edge points onto the ball's surface.
- **Step order.** Both steps share it:
  - A separated pair deletes its connector.
  - A colliding pair outside the kernel stage keeps its connector untouched.
    BlockBlock behaves the same way.
  - A new connector is inserted before its first geometry update (+0x24, which
    ends in 00a9457c). `update` therefore reports it as Kept, then Kept again
    as reused, so the loose world inserts it and then applies its
    displacement.

In the world (`audit-part-ball`):

- **Shape.** `PartInstance::shape` holds Part.Shape. `set_shape` and
  `set_size` keep a Ball's size uniform.
- **Loose Parts.** `LoosePrim::shape` picks the mass, inertia and stiffness
  (00a92640 with the Ball flag). It also picks the broadphase extents and the
  contact class: `ball_contact::PrimContact` holds a BlockBlockContact or a
  `BallContact`. The character's colliders are Blocks.
- **Anchored Parts.** `StaticBlock::shape` does the same for static Parts.
  `LooseWorld::add_static_shape` records it for the contact class and the
  floor mass. World's character/static contacts use `PrimContact` too.
  For a static Ball, the Torso/Head touch test runs the BallBlock test (00ae2aec)
  rather than the block overlap stand-in.
- **The character.** Its colliders meet a Ball through BallBlock contacts:
  realtime connectors for a loose Ball, character/static contacts for an
  anchored one (`audit-part-ball-push`).
- **Ray test.** The Humanoid ray query tests a loose Ball with Ball +0x70
  (00adc17c). That is G3D collisionTimeForMovingPointFixedSphere (0124e2c8),
  not solid, in part space against a sphere at the origin; a hit is a time
  that is not infinite. getSlowHit's bound for a Ball is its radius. Static
  and loose Balls are both tested this way.
- **Not ported.** Cylinders are refused when the world is built. Balls render
  as blocks.

## Wedge Parts and PolyPolyContact (poly.rs, poly_contact.rs)

A WedgePart is a Poly geometry: a convex mesh. Natively, Block is a Poly
too, so a Wedge meets a Block or another Wedge through PolyPolyContact.
Block against Block keeps BlockBlockContact.

- **Shape.** `PartInstance::shape` holds the Primitive part type
  (vtable +0xa4). It equals Part.Shape for Ball and Block, and is
  `SHAPE_WEDGE` (4) for a WedgePart. `material::Shape::Wedge` follows it
  through the world.
- **Mesh (`poly::Mesh`).** It is built from the size alone, as the native
  mesh cache does, in the native order:
  - the vertex, edge and face ids;
  - each edge's forward face first;
  - each face plane from its first three edges.

  Block and Wedge both have one. The contact rebuilds both meshes every
  time it searches.
- **Mass.**
  - Volume is the whole box, x*y*z.
  - The moment is the Block polynomial, taken about the Part's center.
  - The center of mass is offset to (0, -y/6, z/6) in the Part frame.

  The offset goes to `MassPart::center`. `assembly::combine` then puts the
  Body's center of mass there and shifts the moment by m P(offset)
  (006b0234), as Cofm 00a90502 does. Every other Part has a zero offset,
  so its numbers are unchanged.
- **Extents and ray tests.**
  - The extents are the Block's (vertices 0, 1, 3 and 4 span the box).
  - The getSlowHit bound is |size|/2.
  - The ray test (00b340c0) tries every face's plane in order and keeps the
    hit behind all neighbouring faces that is nearest by squared distance.
  - `world::part_space_hit` is Primitive hitTest 00b631de. It moves the
    ray into part space for any geometry.
- **The contact (`PolyContact`).**
  - Each step searches for a separating pair, from the cached answer first:
    - face against vertices (FaceFacePair, in both orders);
    - edge against edge (EdgeEdgePair).
  - Touching pairs generate connectors:
    - FaceVertex, FaceEdge and EdgeEdge, as `PairKind` in a `GeoPair`
      with a `PolyFeature`;
    - each takes the place of an old connector with the same ids and kind.
  - Unlike BlockBlock, a separated PolyContact keeps its connectors but
    takes them out of the kernel. A connector is put back only when it is
    deeper than -0.01.
  - `ContactEvent::Withdrawn` reports that removal. `PrimContact::is_live`
    tells which listed connectors are in the kernel:
    - loose.rs removes the kernel link and keeps the connector's impulse
      and spring state;
    - world.rs counts and pushes only live connectors.
  - `LooseWorld::apply_events` handles the events of `update` and of
    `compute_is_colliding`, because a PolyContact rebuilds its list in both.
- **Drawing.** `geometry::workspace_wedge` is the Wedge path of
  GeometryGenerator (010dc290). It is addBlock with:
  - its own face table (017f0744);
  - no Top face;
  - the slope as face 5;
  - the side triangles drawn as strips that narrow, with one edge stepping
    down the slope.
- **Ball against Wedge (BallPolyContact 00add538).** Its vtable differs
  from PolyPolyContact's only in the destructors and findPolyConnectors
  (00add542), so `PolyContact::new_ball` reuses the PolyContact step,
  collision test and rebuild. The ball is native body 0. The search:
  - Take the ball's center in the Wedge's frame, and the face it is
    furthest outside of.
  - Nothing if that is more than the radius.
  - A BallPlane connector if the center is behind the face or over it.
  - Else a BallEdge on the nearest edge of that face whose span holds the
    center.
  - Else a BallVertex on the nearer end of the face edge whose line is
    nearest.
  - BallEdge and BallVertex are dropped when further than the radius.
- **Static Wedges.** The Torso/Head touch test runs a fresh PolyContact
  test against a static Wedge. A static Block still uses the box-overlap
  stand-in.
- **Not ported.**
  - CornerWedge and Cylinder are refused.

## Unanchored Parts (loose.rs)

Each unanchored Workspace Part is its own single-Part assembly and SimBody, a
`KernelBody` with mass = (x*y)*z*density (material defaults) and the native
block inertia. `LooseWorld` keeps the KernelData lists for these throttling
bodies, all as IndexArrays (append; removal moves the last entry into the
hole):

- body lists freeFallBodies, realTimeBodies, jointBodies and contactBodies;
- connector lists contactConnectors and jointConnectors, holding connector ids.

**Classification (00a9be30).** A body with no joint connectors goes to
contactBodies if it has contact connectors, to freeFallBodies if it has no
connectors at all, and otherwise to jointBodies. Leaving free fall rebuilds the
angular momentum from the angular velocity (ab1c28). Entering free fall sets
both resting flags and clears the displacements. A new body has no connectors,
so it starts in free fall.

**Contacts.** Every world step, before the kernel, SleepStage (below) runs
`BlockBlockContact::update` (Contact::step) on the contacts it is stepping.
A contact makes connectors only while it is CONTACTING and one of its bodies is
in the kernel. Its events are applied in native order:

- A new connector is inserted (00a99514): jointConnectors if either body is in
  jointBodies, otherwise contactConnectors. Both bodies are then reclassified.
- A reused connector whose geometry changed updates the displacements (a9457c):
  the old geometry is applied with -1 and the new with +1, for contactConnectors
  only.
- A deleted connector is removed (00a99ff2). Its current geometry is applied
  with -1 if it was a contact connector, and both bodies are reclassified.

A connector's bodies follow a92c26: A is the first primitive whose body is in
the kernel; the normal is negated when only the second one is. A sleeping Part
is outside the kernel, so it is a fixed body like a static block.

**Displacement (a942ee).** d = n·len·invMass_A/(invMass_A + invMass_B) is added
to A's linear displacement and r × d to its angular displacement; B gets the
opposite with its own share.

**Kernel::step (00a96588).**

1. Pre-step (00a96eb0) over contactBodies:
   - a body with any force beyond gravity, torque or impulse moves to
     realTimeBodies;
   - otherwise the resting flags are set from the displacements (|angular|² <
     0x01700a30; |linear x| and |linear z| < 0x017a5b84) and `contact_prestep`
     (ab230a) turns the accumulated force into velocity and the reference
     impulse.
   - Every contact connector's impulse cache is then cleared.
2. stepWorld (00a9768c):
   - free-fall bodies take `free_fall_step` (ab2d9a), or move to realTimeBodies
     if something pushed them;
   - the awake impulses (a936d0) loop over contactConnectors, at most 60 times,
     while the average error term is at least 0.1 × (contactBodies/400 + 1);
   - contact bodies take `contact_integrate` (ab290c);
   - in each of the 19 substeps, the character's springs and Humanoid force run,
     then realTimeBodies and jointBodies integrate (ab1d44).

The kinematic Humanoid path still steps the loose Parts.

**SleepStage (sleep.rs, loose.rs).** Each loose Part is an assembly with a
state:

- RECURSIVE_WAKE_PENDING (1);
- WAKE_PENDING (2);
- AWAKE (3, in the kernel);
- SLEEPING_CHECKING (4) or SLEEPING_DEEPLY (5), both outside the kernel.

Each contact has an edge state:

- STEPPING (1) or CONTACTING (3), the two that are stepped;
- SLEEPING (2) or CONTACTING_SLEEPING (4), which are not stepped.

`LooseWorld::sleep_stage` runs SleepStage::step 00b707f4 in native order:

1. The wake passes (recursive to depth 8, then plain).
2. The STEPPING contacts are stepped.
3. The wake passes again.
4. Each AWAKE assembly takes a motion-history sample, and still ones are
   decided.
5. Every 4th step, the SLEEPING_CHECKING ones are re-decided.
6. A wake pass.
7. The CONTACTING contacts are stepped.
8. The wake passes again.

Character contacts form list 0 (they cannot throttle) and are stepped before
the others.

- *Contact step (b715ac).* A contact with an awake side becomes CONTACTING if
  it collides, otherwise STEPPING. With both sides asleep or anchored it becomes
  CONTACTING_SLEEPING if it was touching and still collides, otherwise
  SLEEPING.
  - Starting to touch wakes both assemblies, recursively if either moves or
    spins faster than 1 stud/s (b7361a).
  - Stopping touching wakes both.
  - Leaving CONTACTING deletes the contact's connectors.
- *Motion history (Assembly +0x3c).* It holds 15 samples, one taken every 8th
  step. A sample is the root position plus the rotation quaternion times the
  assembly radius.
  - "Still" means the countdown, restarted to 15 by every wake, has reached 0
    and the largest squared distance from the mean is below 0.01. The mean's
    quaternion sum starts at the identity, so a resting Part reads 1/225.
  - "Moving" means that distance is above 0.02.
- *Decision (b726ae).* A still assembly stays AWAKE while a touching awake
  neighbour is moving. With a still awake neighbour it becomes
  SLEEPING_CHECKING; with none it becomes SLEEPING_DEEPLY.
- *setAssemblyState (b727c0).*
  - Falling asleep zeroes the velocity and leaves the kernel.
  - Waking zeroes the velocity and impulses and restarts the countdown.
  - AWAKE enters the kernel through 00a9be30. Just before, 00b7439e resets
    the SimBody force to gravity alone and the torque to zero (only when the
    assembly is not already downstream, 00b2a8d0). Forces gathered while it
    slept, such as a Humanoid's floor pushes, are dropped.
  - The port deletes the live connectors of a Part entering or leaving the
    kernel; the next Contact::step regenerates them. The native re-insertion
    is not recovered.
- *Wake pass (b7228e).* Each pending assembly becomes AWAKE and turns its
  SLEEPING/CONTACTING_SLEEPING contacts back into STEPPING/CONTACTING. It then
  either queues its sleeping neighbours (recursive) or moves deep-sleeping
  neighbours to SLEEPING_CHECKING (plain).
- *Character.* Its assembly is in the stage only while it is in the kernel.
  - Humanoid::onStepped restarts its countdown every frame, so it never
    sleeps, but its history still says whether it is moving.
  - Leaving the stage (RunningNoPhysics) wakes the Parts it was touching.
  - The Humanoid wakes a new floor Part (00bff784), and a floor it walks over
    kinematically faster than 2 studs/s (00c07626).
- AllowSleep (`LooseWorld::allow_sleep`, default true) keeps everything
  AWAKE.

**Not ported:**

- the character-static block contact's body order: it always keeps the
  character Part as body 0 (see **Broadphase**);
- leaving realTimeBodies: a contact body moved there by an external force
  (for example the Humanoid's floor reaction) stays until a connector change
  or sleep reclassifies it;
- environmental throttling (SleepStage +0x78).

**The character against loose Parts.** Each character collider (Torso, Head)
is a `Prim::Character` in the same pair map.
- Because the character cannot throttle (Body +0x44 is cleared), 00a99514
  files its connectors under realTimeConnectors. Each one counts in the loose
  body's total (+0x110) and realtime (+0x11c) counts.
- 00a9be30 then moves a loose body:
  - from free fall to jointBodies, when it has no contact connectors;
  - into realTimeBodies at the next pre-step, when it is a contact body: the
    spring force counts as an external force.
- A connector that touches a jointBodies body goes to jointConnectors.
  Connectors never change list after insertion.
- In each substep, World runs these springs in order:
  1. the fixture-wall and static-block springs;
  2. the character/loose springs (`real_time_springs`);
  3. the jointConnectors springs (`joint_springs`).
- Each spring recomputes the connector's geometry (+0x24) and applies a93074.
  A is the connector's first primitive when both bodies are in the kernel.
  Otherwise A is the kernel body and the normal is flipped (a92c26).
- Then the Humanoid force runs, the character integrates, and the loose
  realTimeBodies and jointBodies integrate.
- While the character is kinematic (RunningNoPhysics), its assembly is in the
  Humanoid stage (SleepStage line 790, `a->inStage(IStage::HUMANOID_STAGE)`),
  so its contacts with loose Parts leave SleepStage and are not stepped.
- The Torso/Head touch test that ends RunningNoPhysics (c0192c → c02209)
  runs vtable +0x24 (ae26a6) on each of the Torso's, then the Head's,
  contacts:
  - first the extents check, then computeIsColliding with no ignored overlap;
  - it uses the persistent pair, so it updates the cached separating axis.

  Static blocks still use the SAT stand-in.

`audit-part-drop` drops four Parts onto the Baseplate and logs every Part every
120 steps. `audit-part-push` walks the character into a 4 x 6 x 2 Crate.
`audit-part-stand` drops the character onto a loose Pad lighter than itself,
then walks it onto a 1-stud loose Step (see the Humanoid section for the floor
rules).
Evidence:
RobloxStudio/analysis/kernel-lists-native-20261013.md.


## Broadphase (SpatialHashMultiRes)

`broadphase.rs` is SpatialHash<Primitive, Contact, ContactManager, 4>. It is
built by the ContactManager constructor (00ae6b92 -> 00af0ddc with 100).
It decides which primitive pairs have a Contact. `World::broadphase` keeps it
up to date and turns its events into contact pairs (`World::apply_broadphase`):
- a character-static contact goes into `static_contacts`, stepped in
  creation order;
- every other contact becomes a `LooseWorld` pair (`contact_created` /
  `contact_destroyed`), keyed in the broadphase's order (searching primitive
  first, which 00ae714a keeps as body 0). Destroying a contact removes its
  SleepStage edge (00b73f08), takes its connectors out of the kernel, and
  deletes the pair, so its cached state starts fresh if it is created again.

The fixture world has no broadphase; there every collider meets every
static block, as before.

- **Extents (00b61c9a):** the body position -/+ the geometry's world half
  extent, -/+ 0.01. For a Block (00adf0a0) the half extent is the largest
  |R v| over its corners, that is |R| times half the size.
- **Level (00af1350):** the first level i where
  (dx + 16)(dy + 16)(dz + 16) < 51200 * 8^i. The cell size is 8 (017a9c50)
  and the volume factor 100. The level is at most 3, or at most 2
  (017a9c54) for a Part that is anchored or being dragged (+0xcb/+0xca).
  The Studio Baseplate is at level 2 and a 4x1.2x2 brick at level 0.
- **Cells (00b88880):** floor(v / (8 << level)) per axis, after clamping to
  about +-1002456.7. The clamp's rand() jitter (0073c85c) is not modelled.
- **Hash (00b8885c):** (y * 0x127409f ^ x * 0x466f45d ^ z * 0x4f9ffb7) &
  0xffff, with 65536 buckets per level. A bucket chain is newest first, and
  only nodes with the same grid id count, so collisions never matter. The
  port keys cells directly.
- **Adding a Part (World::insertPrimitive 00bab52a -> 00ae97aa -> 00aeca1c ->
  00aee438):** creates no contacts. Contacts come from findAllContacts
  (00af1684), which SpatialFilter runs for every primitive of an assembly
  when the assembly changes phase (00b79514 -> 00af6e2c).
- **Contact search (00af21cc / 00af1684):** for each cell, in this order:
  - the nodes in the same cell at the primitive's level, newest first;
  - the nodes in each ancestor cell (cell >> 1 per level, up to level 3);
  - the descendant cells (00af19dc): children by offset 0..7 (x bit, y bit
    << 1, z bit << 2), depth first.

  createContact (00ae9612) gets (searching primitive, found primitive). It
  makes no contact within one assembly, between two grounded assemblies, or
  when the geometry pair makes none (CanCollide off, 00ae714a). A third
  test (stage 7 mode 3 with grouped assemblies) is treated as never true
  offline.
- **Each step (doWorldStep 00ba9096):** after Kernel::step, updateBroadphase
  (00ae9bf4 -> 00aecaea per primitive) runs over SimulateStage's realtime
  assemblies, then (on unthrottled steps) its moving dynamic ones, each in
  the order they entered (00b6ee0a). The character cannot throttle, so it
  is first. `LooseWorld::simulating` holds the loose Parts' order. A
  kinematic Humanoid (MovingNoPhysicsBase) moves the character with
  setCoordinateFrame (00c07626 -> 00b639a2 -> 00b655a0), which updates the
  hash at once.
- **Extents changed (00aecaea):** if the new level is the stored one or one
  below, the primitive moves at its stored level (00af1400).
  - If the cell box is unchanged, nothing happens.
  - If the old and new boxes are disjoint, it leaves all old cells, then
    enters all new ones.
  - Otherwise it walks their union (00aee5ec), entering new cells (with
    contacts) and leaving old ones.
  - Then 00aef1d8 destroys every contact whose cell boxes no longer overlap
    at the coarser level. The finer box is shifted right by the level
    difference (00aef442).

  Any other level change removes the primitive (00aef058: leave the cells,
  destroy its contacts in list order) and reinserts it with contacts.
- **A primitive's contact list (+0x64):** appended on creation. Each edge
  keeps its index, so removal moves the last entry into the hole. That is
  assumed from the layout; the removal routine itself was not read.

`audit-broadphase` checks two things on the part-drop scene: every
overlapping collidable pair has a contact, and every contact's cells
overlap. Not verified against the native order: the order in which
assemblies run findAllContacts at the start (assumed Workspace order, then
the character).


## Joints and the spanning tree (joints.rs)

Every Part is a primitive in the World's pipeline (the chain in
[ROADMAP.md](ROADMAP.md)). GroundStage gives each one a ground joint with no
second Part: an AnchorJoint (type 0) when the Part is anchored or dragged, or
when a rigid joint ties it directly to such a Part (00b0f7ac), otherwise a
FreeJoint (type 20). Adding a rigid joint between a fixed and a free Part
re-evaluates the free one (00b0f4e8), which can swap its FreeJoint for an
AnchorJoint.

TreeStage keeps one spanning tree over every joint of type 0 to 20
(009241f4). The tree hangs from a null "ground" node.
- **Ranking (00b216e2):** a lower joint type ranks first. Between equal types
  the higher root priority wins: the larger of the two Parts' priorities
  (00b216ad), where a Part's priority is multiplier x floor(planar x 50) + 1
  (00b63fbe), planar is the product of its two largest axes, and the
  multiplier is 10 for HumanoidRootPart, 5 for Torso, 1 otherwise
  (PartInstance::setName 00983a3e, table 017a87e0). Then the Parts' GUIDs
  (006721c8; the later Part wins), then the joints' addresses.
- **Insert (006cc3d6):** find the worst edge on the path between the joint's
  two ends (006ccc72). If the new joint ranks better, the piece between them
  is re-rooted: the edges from the new joint's end up to the worst edge are
  unlinked, the worst edge removed, the new joint linked, and the unlinked
  edges re-linked the other way round (006cc70a, 006cc9ca, 006ccb3e).
- **Remove (006cc580):** the cut-off subtree reconnects through its best
  outside edge (006cc624, 006cd18c).

So a free group of welded Parts keeps exactly one FreeJoint, on its
highest-priority Part, which becomes the assembly root. Since an AnchorJoint
ranks above a weld, a Part welded to an anchored Part gets its own ground
edge instead of hanging from the anchored Part's Body.

**Bodies.** TreeStage's link callback (00ba2e24) makes the child Part's Body
a child of its parent's Body for rigid joints (Weld 1, ManualWeld 2, Snap 3)
and motors (Motor 4, Motor6D 5). A rigid child's offset is the parent's joint
coordinate times the inverse of the child's (00b69dd6), orthonormalized when
G3D::Matrix3::isOrthonormal fails (01255004, 01255ca6). Unlinking (00ba36a6)
detaches the Body. Body children are an IndexArray: appended, with the last
child moved into a removed child's slot (IndexedTree::setParent 00675752).
Kernel joints (6 to 19) and ground joints start a new assembly. An assembly
is grounded when its root Part's tree edge is an AnchorJoint. A Glue beats a
FreeJoint in the tree, so a Part glued under an anchored one hangs from it in
the tree but roots its own, moving assembly.

**Assemblies in the physics.** `World::in_workspace` builds the tree from
`Workspace::joints` (every Part, then every joint, in Workspace order) and
places each Part where its Body puts it (`JointTree::body_frames`). Parts of a
grounded assembly become static blocks. `LooseWorld` splits what was one
record per Part:
- `prims` (`LoosePrim`): one per Part, with its block, materials and its
  Body's parent and offset. Contacts, the broadphase and the Humanoid floor
  rays work on these (`Prim::Loose` and `FloorPart::Loose` index them).
- `parts` (`LoosePart`): one per assembly, with the SimBody (`KernelBody`),
  the kernel lists and the SleepStage state (`Node::Loose` indexes them).
  `add_assembly` takes the root Part and the Body links. Mass, center and
  inertia come from Cofm 00a90502 over the Body tree, and the radius is the
  largest primitive radius (00ad853c).

A Part's pose and velocity are its Body's PV. The root's PV is published
from the SimBody (00ab13e8). A child's PV is its parent's frame times the
offset, with the parent's velocity taken at the child's origin
(Body::updatePV 00a8eabe, whose add order matches `g3d::frame_multiply`).
The contact spring reads each side's Body PV (a929f6). The impulse solver
reads the SimBody's center-of-mass state for kernel bodies and the Body PV
for a fixed one (a92dd6). Forces and impulses go to the SimBody at the
contact point. For a single block, the Body origin and the center of mass are
bit-identical, so single-Part worlds run exactly as before. The broadphase
refuses pairs inside one assembly (00ae9612) through a per-primitive group.

**Kernel joints (kernel_joint.rs, Glue and the hinges).** A kernel joint
does not join Bodies. It puts Points and connectors in the kernel:
- *Points (Kernel::newPoint 00a96432).* A Point is a Body plus a local
  offset, kept in one IndexArray (KernelData +0x50). Asking for an existing
  (Body, offset) pair returns the old Point with one more use (00a96270);
  releasing the last use removes it, the last Point moving into its slot
  (00a964ec, 00a95fa8). Each substep starts with Point::step (00aa3156): the
  world position from the Body's PV, and the force cleared. After the
  jointConnectors, Point::forceToBody (00aa3244) adds the force, and its
  moment about the center of mass, to the root SimBody, whether or not that
  SimBody is in the kernel. A Point on an anchored Part (`JointBody::Fixed`)
  has no SimBody to push.
- *Break connectors.* PointToPointBreakConnector::computeForce (00a91948)
  is a pure spring with no damping: f = -k (p1 - p0), point 0 gets -f and
  point 1 gets +f. It breaks when |f|1 exceeds the threshold.
  NormalBreakConnector (00a91a12) breaks when the pull along one of point 0's
  Body axes exceeds it. The flag is recomputed on every call. Both are
  connector type 1, which 00a99514 files under jointConnectors even when no
  body is in the kernel. The joint and total counts go up on each side's
  SimBody, sleeping or not, so a glued assembly enters the kernel as a joint
  body and its contacts become springs.
- *Glue (GlueJoint, type 9).* putInKernel (00b0d632) runs over the four
  corners F0..F3 of its face. For each corner it adds a Point on Part0 (the
  face through C0) and one on Part1 (the face through C1), then a
  NormalBreakConnector between them:
  - k is the smaller Part stiffness (00b3ed8a);
  - the threshold is 20 x the face strength (00b0d5ea, Constants 00a92056:
    sides rounded to whole studs, at least 1; a table below 7 studs, then
    long/7 x 4.681);
  - the normal is C0's z axis as a NormalId (006b79ae).
  removeFromKernel (00b3ee8e) removes each connector, then releases its two
  Points.
- *Hinges (RotateJoint, type 6).* putInKernel (00b6b640) holds the two ends
  of the hinge axis: Points at C0 x (0, 0, -1) on Part0 and C1 x (0, 0, -1)
  on Part1, then the same at (0, 0, 1). Each pair gets a
  PointToPointBreakConnector with the smaller Part stiffness and an
  infinite threshold, so the Rotate family never breaks (isBreakable
  00be5ee0 returns 0 as well). The Parts turn freely about C0's z axis.
- *Motor hinges (RotatePJoint 7, RotateVJoint 8, via DynamicRotateJoint).*
  putInKernel (00b6bb04) adds a RotateConnector (ctor 00a9124a), a type 1
  connector in jointConnectors after the two hinge springs:
  - the angle (00a913ec) is atan2(M[1][0], M[0][0]) in double, with
    M = (R0 C0)^T (R1 C1) from the Bodies' PV rotations; the axis is
    R0 C0's z column;
  - k = jointK x s x s, where s is 0.1 x the smaller of the two Parts'
    larger cross sizes at the joint face (00b6b53c);
  - computeForce (00a9178e) takes current = angle - BaseAngle, moves
    desired (00a91724: halfway to current first when the recenter flag is
    set, 006af8f5; then plus the increment), and applies
    k x wrap(desired - current) about the axis: +torque to Part1's root
    SimBody, -torque to Part0's (accumulateTorque 00a6c134). Angles wrap
    into [-pi, pi) in double (006af7f9);
  - the joint's value comes from the axle Part's (Part0's) surface input on
    the face C0's z axis names (00b6be18): Constant gives ParamB, Sin gives
    ParamA x sin((float)t x ParamB), anything else 0. t is
    Workspace.DistributedGameTime, RunService's game time after the
    frame's increment. A Part with no inputs reads NoInput, -0.5, 0.5
    (0098fffc);
  - stepUi (00b6bd86), once per frame in uiStep after breakJoints, stores
    the value and, when it is not 0, wakes both assemblies recursively
    (World::wakeAssembly 00ba7fb0 -> 00b737c4). A recursive wake sets
    SleepStage +0x16c: the next recursive pass also walks through AWAKE
    neighbours, up to depth 8 (00b7228e), and every pass clears the flag at
    its end (00b71f08). An AWAKE assembly goes to RECURSIVE_WAKE_PENDING
    and back to AWAKE without leaving the kernel (00b727c0 only removes on a
    sleeping state);
  - stepWorld (StepJointsStage 00b8a402, after SleepStage::step and before
    Kernel::step), only while the joint is in the kernel: RotateP sets the
    increment to wrap(target - desired) / 19, so the target is reached over
    one world step (00a9159a, target = value wrapped); RotateV sets it to
    30 x value x kernel dt, so value is radians per 1/30 s, and at 0 sets
    the recenter flag (00a916cc).
  removeFromKernel (00b6bc74) removes the RotateConnector first, then the
  hinge springs and Points.
- *SleepStage.* A joint is a SleepStage edge that starts AWAKE (onEdgeAdded
  00b73cfe), and AWAKE means in the kernel (00b7339e). Waking an assembly
  wakes its SLEEPING joints (00b72b82). A joint always counts as touching for
  the sleep decision and the wake pass (00b72d00). The last thing
  SleepStage::step does (00b713bc) is put to SLEEPING every AWAKE joint whose
  two assemblies are neither awake nor waking; an anchored side counts as
  asleep. When an assembly enters the kernel, its force is reset (00b7439e),
  so pushes gathered while it slept are dropped.
- *Breaking.* World::uiStep runs breakJoints (00ba9032) once per frame,
  before MovingAssemblyStage::stepUi (00b39318: the Motor6Ds and the motor
  hinges) and the world steps. Every Glue in the kernel whose first four
  connectors include a broken one is removed
  (World::onJointBroken 00baa94e). The port assumes the JointInstance leaves
  at once. SleepStage's onEdgeRemoving (00b73f08) takes an AWAKE joint out of
  the kernel and wakes both sides of a SLEEPING one.
- *Contacts.* Glued or hinged Parts still collide:
  ContactManager::createContact (00ae9612, 00ae714a) has no joint test.

Not recovered: the order of an assembly's edge list. The port wakes the
joints after the contacts, and the awake-joint set (+0x150, ordered by
address) follows creation order. Which joints MovingAssemblyStage lists for
stepUi is not recovered either: the port steps every motor hinge still in
SleepStage, asleep or not. A surface input is read from the Part's
properties as loaded (no property changes while running yet).

**Motors (motor.rs, MotorJoint and Motor6D).** A Motor (Joint type 4) or
Motor6D (5) is a Body link inside one assembly, like a weld, but its offset
follows an angle:
- *The link (00b38196 -> 00a9d39a, RevoluteLink).* It keeps the joint frame
  on the parent Body's side and the child's (with its inverse): C0 and C1
  when Part0's Body is the parent (00b3807c), C1 and C0 otherwise. The
  offset (computeJointCoord 00a9d5b0) is parent x Rz(angle) x child^-1, Rz
  from (float)sin and cos of the angle in double (006b130e). The parent-side
  frame is always the one turned, so when the Body tree makes Part1 the
  parent the angle turns the other way relative to C0/C1.
- *stepUi (00b381f6), once per frame in MovingAssemblyStage.* The angle
  moves toward DesiredAngle by at most |MaxVelocity| (per frame, not per
  second), then the animation offset and motor weight decay as in Motor6D
  (the offset is one scalar here). A new CurrentAngle (00b3831c) re-poses
  the link (00b38100) and wakes both Parts' assemblies recursively. The
  link marks the Body tree dirty (00a9d2c0 -> 00a8e1da); the SimBody
  rebuilds its center of mass and inertia from the Body tree on its next
  access (00ab16aa), keeping the root's state and the accumulators. The
  port rebuilds right after stepUi (`LooseWorld::step_motors`), which reads
  the same state.
- *Motor6D (D6Link).* Motor6DJoint builds its link the same way (00b370ec,
  parent side from 00b36f5a). D6Link::computeJointCoord (00a9d6d0) is
  (parent x T) x child^-1, T being the joint's translation and
  fromAxisAngleFast rotation vector (x, y, CurrentAngle) (00b36fde). Its
  stepUi (00b3717c) is the one the character uses; a changed link wakes
  both assemblies recursively too (00b37550). `LooseWorld` keeps both
  kinds of motor (`MotorKind`).
- *Offline gate.* MovingAssemblyStage skips motors in network buffer zones
  (FLog StepAnimatedJointsInBufferZone, 00b39200); offline that never
  applies. A motor in an anchored assembly is moved kinematically by
  MovingAssemblyStage (not ported; `World::in_workspace` refuses it).

`audit-joints` checks the tree on the shipped R6 rig: HumanoidRootPart is the
root and the Body child order matches the order `AssemblyProperties::from_rig`
uses. Not modelled: real addresses. GUIDs follow Part creation order, address
ties follow joint creation order, and the subtree walk of 006cc624 (a set of
node pointers) follows Part creation order. Joint removal is ported but has no
caller yet. Native parity is **not tested**.

