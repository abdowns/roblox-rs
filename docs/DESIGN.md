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
from the dynamic character into the wall. Awake impulses precede 19 force/contact
spring passes. Material coefficients, zero reference impulses and up-to-60 awake
iterations are fixture inputs. Full native adaptive/sleep/invalidation/displacement
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
now steps motors there instead of before the fourth world step.

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
- RunningNoPhysics: entered from Running after more than 1 s without Head or
  HumanoidRootPart contacts and no BodyMover. The Torso assembly leaves the
  kernel; each Stepped moves it kinematically toward the desired velocity
  (gain 1250, limits 500/143), snaps it to the support height and yaws it by
  AxisAngle(up, yaw*dt). Head contact returns it to Running.
Events follow 00bfda98: Jumping fires Jumping, Freefall/Landed fire
FreeFalling, Running/RNP fire raw Running(speed) on either edge plus the
per-state deadband. The bundled script's handlers turn them into poses.

Fixture boundaries: the world is still a floor plane plus wall half-space;
"touching" means the part generates a wall vertex/plane connector; climbing,
FallingDown/GettingUp, Seated, PlatformStanding, Swimming, Dead, slopes,
movable floors and the PGS solver branches are not implemented (FallingDown and
slopes return explicit errors). Floor/part friction combination is not recovered.
Jumping from rest rises 8.66 studs in this model (three push frames, then free
flight); native jump height is not tested. Evidence summary:
RobloxStudio/analysis/humanoid-state-native-20261012.md.
