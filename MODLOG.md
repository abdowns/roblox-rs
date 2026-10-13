# MODLOG

## 2026-10-13 The character stands on unanchored Parts

**Changed:**
- `World::raycast` and `Surroundings::raycast`: the Humanoid rays now see loose Parts as well as the static blocks and fixture planes, and report which Part was hit (`FloorPart`). This follows the HumanoidState hit filter 00c02844, which skips only dragged, CanCollide-off and own-assembly Parts.
- `FloorQuery::part` (+0x60) and `find_floor_parts`. `find_floor` is kept as a wrapper.
- Humanoid:
  - floor motion from 00c02c28 (point velocity at the touch point, plus spin) feeds RunningBase's yaw, horizontal and vertical targets and RunningNoPhysics' kinematic velocity and yaw;
  - RunningBase support force uses min(character, floor) mass on a loose floor, capped at 10000 / 1e5 when the floor is lighter, and pushes the floor with -0.5 of it at the touch point (00c0abf2);
  - the Jumping push uses min(character, floor) mass and pushes the floor with -(force - existing);
  - the ceiling rays see loose Parts;
  - the steep-floor rule (MaxSlopeAngle, WalkSpeed downhill) replaces the "sloped floor projection is not implemented" errors.
- `World::step_with` / `Humanoid::apply_forces_in` give the force callback the loose Parts. `World::step` and `apply_forces` keep their signatures as wrappers.
- `LooseWorld::static_mass`, `LoosePart::can_collide`.
- New audit `audit-part-stand`. The part-push and viewport labels now say `static_and_loose`.

**Why:** The user reported the character sinking into loose Parts and walking through low ones: the floor ray ignored them.

**Tested how:**
- Ran audit-part-stand.
- Re-ran audit-walking, part-wall, part-drop, baseplate, animate, player, locomotion, runtime, walking-render, camera, kernel and baseplate-render against a build of the previous commit. All are identical apart from evidence paths and timing.
- audit-part-push changes, as expected.
- viewport-baseplate-smoke: exit 0.
- 75 tests pass; clippy and fmt are clean.

**Result:**
- audit-part-stand:
  - the character (mass 9.9) lands on a 7.56-mass Pad at step 241 with `floor=Loose(0)` and stands at Pad top + 3 (3.183);
  - it walks off at 2.997;
  - it steps up onto the 1-stud Step (`floor=Loose(1)` at walk step 148) and idles there in RunningNoPhysics at 4.001 (Step top + 3);
  - neither Part moves more than 0.02 sideways.
- audit-part-push: the character rides up on the tipping Crate (root y up to about 7.5) and then walks off. The Crate ends 13.0 studs along, not 14.7.

**Still broken / not tested:**
- The Pad sinks about 0.11 into the Baseplate while the character stands on it. The reaction moves it into realTimeBodies, its Baseplate connectors stay contactConnectors, and nothing moves it back without a sleep stage. Native behaviour is not tested.
- The jump floor reaction is not tested.
- The steep-floor rule is not exercised by an audit (only by the tipping Crate).
- The getFastHit cell order for exact ties is not recovered.
- The floor friction factor is still 1.
- The PopperCam camera ray still ignores loose Parts.
- No sleep stage.
- Native parity is not tested.

**Next:** Sleep stage (Sim::AssemblyState, SleepStage b707f4), so resting and stood-on Parts settle.

## 2026-10-13 Viewport demo shows unanchored Parts

**Changed:**
- New `Workspace::loose_parts_demo()`: the new-document Baseplate plus Crate, Rest, Stack, Drop and Tumble.
- `viewport-baseplate` (and its smoke) now runs that place.
- Loose Parts are generated part-local, then posed every frame from their kernel bodies by the new `motion::sync_loose`.
- The walking report in the live log now includes a `loose_parts` line.
- `motion::install_world` takes an optional Workspace instead of a bool (crate-internal).
- `audit-baseplate-render` keeps the pristine new document.

**Why:** The user asked for the visual demo to show the new unanchored Part features.

**Tested how:**
- `viewport-baseplate-smoke`: 120 frames, exit 0. It logged all five Parts, which settle (Crate, Rest, Stack) or are still falling (Drop, Tumble) at the last log.
- `audit-baseplate-render` is identical to the previous commit, apart from timing lines and log paths.
- Tests, clippy and fmt are clean.
- The interactive window was **not tested** by me: I can't see it.

**Still broken / not tested:** No standing on loose Parts (the floor ray ignores them). No sleep stage. The demo layout is mine, not a native place.

## 2026-10-13 The character pushes unanchored Parts

**Changed:**
- `loose.rs`:
  - `Prim::Character` colliders and realTimeConnectors, following 00a99514: a body that cannot throttle files its connector as realtime, with +0x110/+0x11c counts and 00a9be30 reclassification.
  - `real_time_springs` for character/loose connectors and `joint_springs` for loose jointConnectors (this replaces the "not ported" error).
  - `integrate` (realTimeBodies, then jointBodies).
  - `update_character_contacts`, `collider_touching` (ae26a6 through the persistent pair) and `clear_character_contacts`.
- `World::step`:
  - updates character/loose contacts after the static-block ones;
  - runs the springs in substep order (realtime, then joint), then the Humanoid force, then integration.
- `World::step_kinematic` deletes the character/loose connectors, because the character's edges are not in the kernel stage.
- New `World::collider_touching`. HumanoidState's Head touch test (c0192c) now also sees loose Parts.
- New audit `audit-part-push`.
- `LooseWorld::report` shows contact/joint/realtime/total counts.

**Why:** Unanchored Parts step 4d: character ↔ loose Part contacts.

**Tested how:**
- Ran audit-part-push.
- Re-ran audit-walking, part-wall, part-drop, baseplate, animate, player, locomotion, runtime, walking-render, camera and kernel against a build of the previous commit. All are identical apart from timing lines, evidence paths and the new report columns.
- 74 tests pass; clippy and fmt are clean.

**Result:**
- At step 101 the Head touches the Crate and RunningNoPhysics ends: 7 realtime connectors appear, and the character bounces back at +10.9 studs/s, having gone 0.1 deep kinematically first.
- The Crate moves Contact → RealTime → Joint and is pushed at walking speed.
- It then tips onto its face (bouncing once into free fall) about 14.7 studs along.
- The character walks over it because the floor ray ignores loose Parts.

**Still broken / not tested:**
- The Humanoid floor ray ignores loose Parts.
- No sleep stage.
- The realtime list order between static-block and loose connectors is assumed.
- The edge-stage rule while the character is kinematic is assumed from a string.
- The contact order in the Head test is assumed.
- Static blocks still use the SAT touch stand-in.
- Native parity is not tested.

**Next:** Humanoid floor raycast and force reaction on loose Parts, then rendering moving Parts.

## 2026-10-13 Unanchored Parts as kernel bodies (loose.rs)

**Changed:**
- New `loose.rs` holds `LooseWorld`, the KernelData subset for unanchored Parts:
  - body lists and classification (00a9be30, 00a9c142);
  - connector insert and remove (00a99514, 00a99ff2) and contact displacement (a9457c, a942ee);
  - the pre-step (00a96eb0);
  - stepWorld's free-fall, impulse-iteration and contact-body phases (00a9768c): at most 60 iterations, error goal 0.1 × (n/400 + 1);
  - the realTimeBodies and jointBodies integration in each substep.
- `World::in_workspace` now adds unanchored Parts instead of rejecting them, and records each static block's material for contact parameters.
- `World::step` and the kinematic step both run the loose Parts.
- `World` gained a public `loose` field; `AwakeContact` now derives Clone and Debug.
- New audit `audit-part-drop` puts four Parts on the Baseplate:
  - Rest: 0.05 above the Baseplate;
  - Stack: 0.05 above Rest;
  - Drop: flat from y = 8;
  - Tumble: tilted from y = 10.

**Why:** Unanchored Parts step 3 (falling and stacking).

**Tested how:**
- Ran audit-part-drop.
- Re-ran audit-walking, audit-part-wall and audit-baseplate: byte-identical, since they have no loose Parts.
- audit-legacy-contact, audit-block-contact and audit-kernel pass.
- A new displacement unit test; 74 tests pass, plus Clippy and fmt.
- A temporary per-step probe of Drop's landing (not committed).

**Result:**
- First contacts: Rest step 7, Stack 10, Drop 67, Tumble 71.
- After 6 s:
  - Rest y = 0.5822 and Stack y = 1.7609, both flat (up_y 0.999995);
  - Drop is on its side at y = 0.988;
  - Tumble is flat at y = 0.584 and nearly still (0.0002 studs/s).
- Impulse iterations peak at 9; resting steps take 2-4.
- Drop lands at -54 studs/s: four corner connectors, 2 iterations, then it leaves at v.y = +22 with spin.
  - The resting flags hold for the first corner.
  - Later corners clear them through fast tangential velocity and get elastic rebound.
  - So it bounces and tumbles.
- Rest and Stack creep about 0.03 studs/s toward -x/-z (0.15 studs over 5 s); average error stays around 0.08 with 4 iterations per step.

**Still broken / not tested:**
- The sleep stage is not ported; native sleep would stop the creep.
- Whether native bounces and creeps the same way is not tested.
- The resting-flag |angular|² float order is assumed to be z + (y + x).
- Loose jointConnectors springs return an error if reached.
- The character does not touch loose Parts.
- Loose Parts are not drawn moving.
- The broadphase pair order is assumed.

**Next:** Character ↔ loose Part contacts (realtime springs, jointBodies) and raycasts against loose Parts, then rendering.

## 2026-10-13 BlockBlockContact reports connector events in native order

**Changed:**
- `BlockBlockContact::update` now records `events` for that update, in generation order:
  - `Kept { index, reused }` for each connector placed in the live buffer;
  - `Removed { connector, recomputed }` for each deletion.
- Deletions fall into three cases:
  - a found vertex/plane connector rejected by the plane test (ae376c → ae277e, no recompute);
  - a found edge connector rejected after its geometry was recomputed (ae4e72 → +0x24 then ae277e; ae33a8 assumed to match);
  - the unmatched leftovers at the end (+0x1c), including every connector when the pair stops colliding.
- The kernel connector lists need this order. Native inserts a new connector when it is generated and deletes rejected ones at once.

**Why:** Unanchored Parts step 3 (connector kernel lists and contact displacement).

**Tested how:** The resting-box unit test now checks the events: 4 new Kept, then 4 reused Kept, then 4 Removed on separation. 73 tests pass, plus Clippy and fmt. audit-part-wall and audit-baseplate are byte-identical; audit-block-contact passes.

**Still broken / not tested:** That ae33a8 deletes found-and-rejected connectors the same way as ae4e72 is assumed, not decompiled.

**Next:** Loose Parts in the World.

## 2026-10-13 AwakeContact follows native a936d0 (resting flags, cache, separation)

**Changed in `AwakeContact::solve`:**
- Its signature is unchanged.
- **Resting flags:** it reads both bodies' resting flags (KernelBody `resting`, SimBody +0x12c/+0x12d).
  - A fast contact (|ns| > 60 or |vt|² > 1) clears a body's angular flag.
  - Resting needs both bodies' angular flags.
  - The friction cone is skipped only when both linear flags are also set.
- **Separating contacts:** a separating contact (ns ≥ 0) is still impulsed when resting and iterations ≠ 0.
- **Elastic rebound:** it now also requires !resting.
- **Rebound target and clip:** these are computed only in the call that rebuilds the +0x74 cache.
- **Separation:** zeroes iterations, clip and rebound but keeps the cache flag, as native does.
- **Error term:** the per-call `error_term` (|delta/clip|) is exposed for the stepWorld error sum.
- **Float order:** all three-term sums use the asm-verified z + (y + x) order (`dot_zyx`). This covers the correction, n·J, |t|², |vt|² and n·Md.
- **Thresholds:** the fast-contact thresholds are read from the binary at 0x01749d10 (60) and 0x015d8978 (1), found through PIC base 00a936e1.
- **Test support:** `KernelProfile::test_profile()` is added for unit tests (test builds only).

**Why:** Unanchored Parts step 2. Resting Parts depend on the resting path.

**Tested how:** Two new unit tests cover the separating-but-resting impulse, the fast flag clear and the separation-keeps-cache behavior. 73 tests pass, plus Clippy and fmt. audit-legacy-contact, audit-walking, audit-part-wall and audit-baseplate are byte-identical (none sets the resting flags).

**Still broken / not tested:** The float order of the G3D Matrix3 products in the mass matrix is not verified. Native differential not tested.

**Next:** Loose Parts in the World (kernel lists, free fall, contact bodies, impulse schedule).

## 2026-10-13 SimBody free-fall, contact pre-step and contact integration (kernel.rs)

**Changed:**
- `KernelBody` gains four new public fields:
  - `reference_impulse` (SimBody +0xf0);
  - `resting` flags (+0x12c/+0x12d);
  - angular and linear displacement (+0x130/+0x13c).
- It also gains the native routines a loose Part needs:
  - `free_fall_step` (ab2d9a): ω damped by 0.99621, quaternion, half-step legacy gravity on Y, tiny additions;
  - `contact_prestep` (ab230a): L = (T·dt + Aᵢ) + L·0.99621, ω = I⁻¹_world·L, reference impulse = F·dt + J, v += dp·invMass, accumulators reset;
  - `contact_integrate` (ab290c): quaternion from ω, center += v·dt, no gravity or damping;
  - `rebuild_momentum` (ab1c28): L = R I Rᵀ ω, run when a body leaves free fall;
  - `has_external_force_or_impulse`.
- The ω row products use the asm-verified z + (y + x) order (ab230a and ab267c loops). The damping constant is 0x017a1f74 (verified through ab230a's PIC base).
- Nothing calls these routines yet.

**Why:** Unanchored Parts step 1.

**Tested how:**
- Three new unit tests:
  - `free_fall_step` equals `FreeFallBody::step` exactly for a centered body;
  - the pre-step stores the reference impulse and keeps the position;
  - momentum is rebuilt from the world inertia.
- 71 tests pass, plus Clippy and fmt.
- audit-walking, audit-part-wall and audit-baseplate are byte-identical to the previous commit.

**Still broken / not tested:** Native differential not tested.

**Next:** Bring AwakeContact up to native a936d0 (resting flags, cache semantics).

## 2026-10-13 Native substep order: contact springs before the Humanoid connector

**Changed:**
- Each kernel substep in `World::step` now runs the character's contact springs, then the Humanoid force callback, then the integration. Before, the callback ran first.
- The walking audit's "wall release still rotating" check now measures the orientation change over the last 240 release ticks instead of the instantaneous angular velocity. It reports `walking_wall_release rotation_rate=… release_jitter=…`.

**Why:**
- stepWorld 00a9768c runs realTimeConnectors before humanoidConnectors in every substep.
- The Humanoid embeds a kernel Connector at +0xdc. Its type slot (00be5f4a) returns 2, so connector insert 00a99514 files it under humanoidConnectors (KernelData +0x60). Its computeForce (00bddb7a) calls 00bddaae.
- The Humanoid force reads the accumulated force and torque (RunningBase, balancing, Freefall), so it now sees the contact forces, as in native.

**Tested how:** Rebuilt, then ran audit-walking, audit-part-wall and audit-baseplate against the previous commit's output. A temporary per-tick probe (not committed) logged the release phase. Also ran 68 tests, Clippy and fmt.

**Result:**
- audit-baseplate is byte-identical.
- audit-part-wall: stops at z = -9.5105, max penetration 0.067, max connectors 6, upright_y 0.99999994.
- audit-walking:
  - the wall hold is x = 3.5104, z = 22.6231;
  - max penetration 0.080;
  - min animated upright_y 0.99683 (was 0.99978).
- Against the wall, ω_y alternates between +0.078 and -0.078 rad/s every substep. The probe shows the sign flips every tick (19 substeps) and the orientation does not drift: rotation_rate = 0. This is a period-2 yaw jitter of about 1.7e-5 rad, not rotation.

**Still broken / not tested:** Whether native shows the same yaw jitter is not tested. It may come from the yaw controller gain against the tangential contact spring, or from an unported detail.

**Next:** Native awake-impulse schedule for contactConnectors, then loose Parts.

## 2026-10-13 Character contacts are realtime connectors (springs only)

**Changed:**
- `World::step` no longer runs the awake impulse pass for the character's wall and block connectors. It keeps the connector bookkeeping, then runs the 19 substeps of force callback, springs and integration.
- Removed the now-unused per-connector `AwakeContact` state and the `ImpulseProfile` from World.
- `impulse_applications` stays public and is always 0.
- The walking audit's wall check is now "the wall was reached (max_penetration > 0) and penetration ≤ 0.5" instead of "impulses > 0". Its failure message prints the fixture report.

**Why:** Native evidence:
- Every HumanoidState ctor (00bfd2a2 -> 00bfd678 -> 00b633fa) sets canThrottle (Body +0x44) false on the character's Parts.
- Connector insert 00a99514 puts a connector with a non-throttling body into realTimeConnectors.
- stepWorld 00a9768c runs impulses only over contactConnectors.

**Tested how:** Rebuilt, then ran audit-walking, audit-part-wall and audit-baseplate against the previous commit's output. Also ran 68 tests, Clippy and fmt.

**Result:**
- audit-baseplate is byte-identical.
- audit-part-wall: the root still stops at z = -9.5104, upright_y 0.99999994, impulses 0. Max penetration is 0.069 (was 0.0507).
- audit-walking:
  - the wall root holds at x = 3.5104 from step 480 to step 7680, with z constant at 22.6307;
  - max wall penetration is 0.080 (was 0.067);
  - min animated upright_y is 0.99978;
  - impulses 0.

**Still broken / not tested:**
- The substep order is still force callback, then springs. Native runs realTimeConnectors before the Humanoid connector; that is the next fix.
- Native parity is not tested.

**Next:** Native substep order.

## 2026-10-13 Fixture wall uses recovered Plastic ContactParams

**Changed:**
- The synthetic fixture wall now uses Contact::computeContactParams (00a43fb4) for each collider against Plastic, in both the impulse pass and the springs.
- Block stiffness (00a92640) grows with size, so a large wall's MINSS picks the collider's own stiffness. The wall's stiffness is therefore passed as equal to the collider's. This is an assumption about a non-native fixture.
- `World::step`'s `parameters` argument is now unused; the signature is kept stable.

**Why:** This was the last contact still using the unrecovered 7500/3750 spring constants. It is needed before the character's connectors become springs-only realtime connectors: with 7500 the wall penetration reaches 0.63.

**Tested how:** Ran audit-walking, audit-part-wall and audit-baseplate, then 68 tests, Clippy and fmt.

**Result:**
- audit-part-wall and audit-baseplate are byte-identical.
- In audit-walking, the root holds against the wall at x = 3.5104. That is the same 0.0104 standoff as the Part wall. Before, it was 3.5399 and drifted along z from 22.77 to 23.34 over 7680 steps; now it stays at 22.654 to 22.664.
- Max wall penetration is 0.067 (was 0.113). Wall impulses dropped from 37932 to 17525.

**Still broken / not tested:** Native parity is not tested (the wall itself is a fixture).

**Next:** The character's connectors become realtime connectors (springs only).

## 2026-10-13 Recovered ContactParams for character/Part contacts

**Changed:**
- At construction the World computes Contact::computeContactParams for every (character collider, Workspace block) pair. The inputs are the collider's material read from character3 (Plastic, no custom properties) and the Part's material, with Primitive stiffness from the sizes.
- Block connectors use these values in the awake impulse pass and in the contact springs.
- The `parameters` argument of `World::step` now applies only to the synthetic fixture wall. Its signature is unchanged.
- `MaterialProfile` resolves every material at load and keeps no copy of the binary. The audit-material output is byte-identical.

**Why:** Parts step 6. The 7500/3750 fixture spring was not native. Torso against the Baseplate is kSpring 3,360,000, kNeg 1,915,200, friction 0.3.

**Tested how:** Compared before and after for audit-part-wall, audit-baseplate and audit-walking. Also ran 68 tests and Clippy.

**Result:**
- audit-baseplate and audit-walking are byte-identical: the character never touches the Baseplate through connectors.
- audit-part-wall changed:
  - first contact is still at step 145;
  - the root holds at z = -9.5104 (was -9.5405, creeping at -0.71 studs/s);
  - penetration is about 0.010-0.011 (maximum 0.0507, was 0.064);
  - upright_y 0.99999994;
  - impulses 757, maximum connectors 8.
- The root height shows a small vertical bob of 2.98-3.03.

**Still broken / not tested:** Native parity of the bob and of the wall standoff is not tested. The broadphase pair order is assumed.

**Next:** Unanchored Parts as kernel bodies.

## 2026-10-13 Material properties and contact parameters (material.rs)

**Changed:**
- New `material.rs` with these parts:
  - `MaterialProfile::defaults(material)` is PhysicalProperties(material), 00a4331a. It reads the density, friction, elasticity and friction-weight switches from the binary by walking their compiled compare/branch trees.
  - `stiffness(size, shape)` is the Primitive stiffness, 00a92640/00a92276. It uses the size-table constants.
  - `contact_params` is Contact::computeContactParams, 00a43fb4. It combines weighted friction and elasticity, takes the smaller kSpring, and sets kNeg = kSpring * elasticity factor (00a91fda).
- Added `audit-material`.

**Why:** These are the inputs the multi-body World needs: per-Part density and per-pair contact settings. Until now the character used fixture values: 7500/3750 and friction 0.3 or 0.

**Tested how:** `sh tools/cargo.sh run --locked -- audit-material`, 68 tests, Clippy.

**Result:**
- Plastic: density 0.7, elasticity 0.5, friction 0.3, both weights 1. These match the decompiled switches.
- Stiffness values:
  - Torso 3,360,000
  - Head 1,545,600
  - a 4x1.2x2 Part 4,598,400
  - the Baseplate 1.258e12
- Torso/Baseplate contact: kSpring 3,360,000, kNeg 1,915,200 (factor 0.57), friction 0.3.
- 31 of 36 materials resolve.

**Still broken / not tested:**
- Limestone, Pavement, LeafyGrass, Asphalt and Salt hit the density switch's assert path. They are reported as unsupported rather than assumed 1.0.
- Freefall's friction=0 write (Primitive +0xbc, 00bd3ab4) does not reach these parameters in this build. The character still uses the fixture values until the next step.
- Native numeric parity is not tested.

**Next:** Give the character the recovered contact parameters.

## 2026-10-12 Torso/Head collide with Workspace Parts (BlockBlockContact in World)

**Changed:**
- The World keeps a BlockBlockContact per (collider, static block). Its
  connectors join the awake impulse loop and the per-substep contact springs.
- Added `WalkingFixture::in_workspace`, `World::block_contact_report` and
  `World::block_contact_kinds`.
- Added `audit-part-wall`.
- The baseplate pass line now reads block_contacts=block_block_contact.

**Why:** Parts step 5: the character must be stopped by Parts, not only stand
on the Baseplate through the floor ray.

**Tested how:**
- audit-part-wall.
- Before/after comparison: audit-walking, audit-camera, audit-animate,
  audit-motion, audit-walking-render, audit-baseplate-render and
  audit-legacy-contact are byte-identical. audit-baseplate differs only in the
  pass-line marker.
- viewport-baseplate-smoke (120 frames), 68 tests, Clippy.

**Result:**
- First contact at walk step 145, on the Head.
- From then on, 6 vertex/plane connectors (Head 2, Torso 4).
- The root stops at z = -9.5405 (Torso face at -10.04).
- Maximum penetration 0.064, 725 impulse applications, upright_y 0.99976,
  state Running.
- Test: `sh tools/cargo.sh run --locked -- audit-part-wall`.

**Still broken / not tested:**
- The broadphase primitive order and the connector order across pairs are
  assumed.
- `touching` still uses the box-overlap stand-in.
- Unanchored Parts are not simulated.
- Native numeric parity is not tested.

**Next:** Unanchored Parts (multi-body world). I'll ask before that
refactor.

## 2026-10-12 Port the BlockBlockContact narrowphase (box/box connectors)

**Changed:** New `block_contact` module: `Block`, `BlockTables`,
`BlockBlockContact<T>` (separating-axis test, face clipping, edge/edge),
`GeoPair` contact types 3/4/5 and connector reuse. Added `audit-block-contact`.
**Why:** Parts step 4: Torso/Head must collide with Workspace Parts through the
native narrowphase instead of the box-overlap stand-in.
**Tested how:** Ghidra decompile and disassembly of ae32f4, ae31f8, ae3de4,
adf6ac, ae4974, ae533e, ae376c, ae3bd4, ae4e72, ae33a8, ae30a8, ae6024,
aa1d78, aa1f76, aa29c0, b61c9a, adf0a0, aec6e4, 006b2077, adf63a and ae4916.
Constants read from the image: 1.01, 0.001, 10.0, 1e-5, 0.05, 1e-6, 0.5. Five
new unit tests (68 total), Clippy, audit-block-contact.
**Result:** Resting default Part sunk 0.05: features [4,-1], four vertex/plane
connectors, length -0.050000, normal (0,-1,0). Lifted: none, features reset.
Tilted on an edge: two connectors. Crossed bars: one edge/edge connector.
Reused connectors keep their state.
Test: `sh tools/cargo.sh run --locked -- audit-block-contact`.
**Still broken / not tested:** Not wired into World yet. The broadphase
primitive order and the post-compute kernel bookkeeping (a9457c) are not
ported. Uninitialized native GeoPair ids are modeled as UNSET. The 00972080
multiply order is assumed from G3D (not disassembled). Native numeric parity
is not tested.
**Next:** Use it for Torso/Head against static blocks in World.

## 2026-10-12 Render the Baseplate (native addBlock studs/inlet strips)

**Changed:** `geometry::workspace_block`/`workspace_draws` (addBlock port for
Workspace blocks), `motion::install_world` with a Baseplate option and
`pose_walking`, `viewport-baseplate`, `viewport-baseplate-smoke` and
`audit-baseplate-render`.
**Why:** Parts step 3: the Baseplate must be drawn with its native geometry.
**Tested how:** Ghidra: addBlock 010cca80 (exported symbol), corner/row tables
017f0140/017f045c. audit-baseplate-render (GPU readback), viewport-baseplate-smoke
and viewport-walking-smoke in a real window, Clippy, tests; audit-render,
audit-walking-render, audit-motion, audit-animation-render and audit-geometry
byte-identical to before.
**Result:** Baseplate 132 quads / 528 vertices, color (99,95,98); top strips
start at z=+256 stepping -Z on the Studs row, bottom strips step +Z on the
Inlet row. Landed frame: Baseplate covers 207,145 pixels, studs change
192,135. Window smoke: falls, lands at root y 3.0 and walks (120 frames).
The fixture walking smoke, previously not tested (display asleep), now passes.
Test: `sh tools/cargo.sh run --locked -- viewport-baseplate`.
**Still broken / not tested:** Native pixel parity; outlines, material UVs and
tangents; the first camera frame is a fixture offset (Studio edit camera not
ported); manual keys not tested.
**Next:** Studio camera, then BlockBlockContact for Torso/Head and unanchored Parts.

## 2026-10-12 Play Solo on the Baseplate (Part statics and native ray test)

**Changed:** World statics can hold anchored Parts (`StaticBlock`);
`Statics::raycast` adds the native block hit test; `World::in_workspace` and
`WalkingFixture::baseplate` spawn the character in the Studio default place.
Added `audit-baseplate`.
**Why:** Parts step 2: the character must stand on real Parts, not the fixture
floor plane.
**Tested how:** Ghidra: getFastHit 00ae8274, getSlowHit 00ae7d08, Primitive
hitTest 00b631de, Block vtable 01c866e8 (+0x70 00adeffe), G3D
collisionLocationForMovingPointFixedAABox 0124e766. Two new unit tests (65
total), Clippy, fmt; audit-baseplate; audit-walking, audit-camera,
audit-animate and audit-walking-render byte-identical to before the change.
**Result:** Head spawns at (0,107,0); Freefall after 0.12 s; lands at world
step 241 (1.004 s, -196.2 studs/s) with the root never below y=3; settles at
root y 3.0 upright; animations idle -> fall -> idle; one second of W walks
15.87 studs; jump apex 8.66 studs.
Test: `sh tools/cargo.sh run --locked -- audit-baseplate`; expect `baseplate_pass`.
**Still broken / not tested:** Baseplate not rendered yet; no Torso/Head
contacts with Parts (touching is a box-overlap stand-in); unanchored Parts
rejected; grid-walk ordering of equal-distance hits not modeled; native
differential not tested.
**Next:** Render the Baseplate (addBlock studs/inlets on large faces) and add
it to the walking viewport.

## 2026-10-12 Studio default world data (Part defaults, Baseplate, spawn)

**Changed:** Added `workspace.rs` (PartInstance defaults, Workspace with the
Studio new-document Baseplate, no-SpawnLocation spawn point and the
SpawnLocation.cpp character placement transform), `audit-workspace`, and
`tools/macho_exports.py` (lists the binary's exported C++ names).
**Why:** First step of getting Parts working: the default place every new
Studio document starts with.
**Tested how:** Ghidra: Part/PartInstance constructors, Part vtable setters and
property descriptors, RobloxIDEDoc 001017a6, 00f6bdf6, 00ae8e9a, 009f4706,
009c1bc6, 009703f0, 0096e468, 005e5110 (notes in
RobloxStudio/analysis/workspace-native-20261012.md). Three unit tests and
`audit-workspace`.
**Result:** Baseplate 512x20x512 at (0,-10,0), Anchored, Locked, BrickColor
199, Top Studs/Bottom Inlet; spawn (0,100,0); the Head is placed at
(0,107,0) facing -Z (Torso at (0,105.5,0)).
Test: `sh tools/cargo.sh run --locked -- audit-workspace`.
**Still broken / not tested:** Data only: nothing renders or simulates the
Baseplate yet. The occupied-region safe-spot search is not ported (returns an
error). The Model mover's obstruction search is not ported (never needed for
an empty sky). Terrain/Camera handling in workspace extents is assumed empty.
**Next:** World raycasts and kernel statics against anchored Parts, then drop
the character onto the Baseplate.

## 2026-10-12 August 2016 keyframe animations (Animator + Animate script)

**Changed:** Added `rbxm.rs` (binary/XML model reader), `g3d.rs` (exact
toAxisAngle, fromAxisAngleFast, unitize, fuzzyEq), `animator.rs`
(KeyframeSequence cache, AnimationTrack, Animator::onStepped),
`animate_script.rs` (the shipped R6 Animate LocalScript) and
`tools/fetch_animations.py`. Motor6D now has the native animated pose and
full stepUi; its frame comes from the link rotation vector. Walking uses the
keyframe script instead of the legacy procedural one. Added `audit-animate`.
**Why:** The character used the legacy procedural animation; the 0.255 loader
selects the keyframe script for R6.
**Tested how:** Ghidra: 00f6660c, AnimationTrack.cpp/KeyframeSequence.cpp
functions, Animator::onStepped, Motor/Motor6D IAnimatableJoint vtables,
Motor6DJoint setAnimatedPose/stepUi/link, toAxisAngle, fromAxisAngleFast (see
RobloxStudio/analysis/animator-native-20261012.md). Asset dates from the
Roblox economy API. 58 tests, Clippy, fmt; audit-animate, audit-walking,
audit-walking-render, audit-camera, audit-lighting, audit-sky,
audit-scheduler, audit-rig, audit-animation, audit-motion, bundle check.
**Result:** 23 clips load (run.xml is a never-played local id). Idle loops with
End events; Running(16) plays walk at speed 1.1034483 and the Right Shoulder
link equals the sampled clip pose exactly; the 0.625 s jump ends on frame 37
and fades to rest. Walking: one second of W still 15.746414 studs; jump plays
jump -> fall -> idle with apex 8.69 studs; limb swings 1.48-2.08 rad.
Test: `python3 tools/fetch_animations.py`, then
`sh tools/cargo.sh run --locked -- audit-animate`; expect `animate_pass`.
**Still broken / not tested:** Window smoke could not run (surface reported
Occluded twice, display likely asleep): **not tested** in the window. Lua RNG
seed and asset-load latency assumed; native pixel parity not tested; climb,
sit, emotes and tools are loaded but their states are not reachable yet.
**Next:** Remaining Humanoid states (Climbing, Seated, FallingDown), then Parts.

## 2026-10-12 Split walking.rs into World and Humanoid (refactor only)

**Changed:** Moved RBX::World behavior (contacts, kernel substeps, fixture
statics and raycasts) to `world.rs` and RBX::Humanoid with its state objects
(dispatch, per-state update, force callbacks) to `humanoid.rs`. The Humanoid
now fires its Lua signals as `HumanoidEvent`s (mapping from 00bfda98:
Freefall/Flying/Landed -> FreeFalling, Running/RunningNoPhysics ->
Running(speed), etc.); the legacy procedural script consumes them.
`walking.rs` keeps `WalkingFixture` with the same public methods as the
fixture/game loop. Callers use `fixture.world.body` / `world.statics.*`
instead of `fixture.body` / `floor_y` / `wall_x`.
**Why:** walking.rs mixed three native classes; Parts and keyframe animation
need World and Humanoid as separate pieces.
**Tested how:** Captured audit-walking, audit-walking-render, audit-camera,
audit-lighting and audit-sky before the change and diffed after: all
identical. 50 tests, Clippy, fmt, walking window smoke, bundle check.
**Result:** No behavior change.
Test: `sh tools/cargo.sh run --locked -- audit-walking`; expect the same
`walking_pass ... distance_one_second=15.746414` as before.
**Still broken / not tested:** Nothing new.
**Next:** August 2016 keyframe animations (Animate script + Animator).

## 2026-10-12 Render the default sky: skybox, sun and moon

**Changed:** Added `sky.rs` and `sky.wgsl`: port of Sky::update/Sky::render for
the default (no Sky instance) path with the original sky512 DDS faces and
sun/moon JPEGs, opaque and additive pipelines drawn after the scene. viewport-walking
enables it; added `audit-sky`. Enabled the `image` crate's jpeg feature
(adds zune-jpeg/zune-core to Cargo.lock).
**Why:** The background was the clear color; the original client always draws
a skybox.
**Tested how:** Ghidra: Sky ctor (quad), Sky::update, Sky::render, loadSkyBox
slot order, billboard basis FUN_006b3ea1; shipped sky.hlsl. 50 tests, Clippy,
fmt; audit-sky, audit-lighting, audit-camera, audit-walking,
audit-walking-render, audit-scheduler, walking window smoke, bundle check.
**Result:** Spawn camera view: 297,312 clear pixels before, 0 after, repeat
identical. Looking at the 14:00 sun: center (255,255,255) vs corner sky
(97,147,198). All six axis views fully covered. Window smoke logs
`sky_enabled` and final coverage rises 1,805,286 -> 2,764,482 pixels.
Test: `sh tools/cargo.sh run --locked -- audit-sky`; expect `sky_pass`.
**Still broken / not tested:** Star field, custom Sky instances (CelestialBodies,
StarCount, SkyboxXx), mip selection; blend enum mapping inferred; native pixel
parity **not tested**.
**Next:** Light grid / OutdoorAmbient skylight, or remaining Humanoid states
(Climbing, FallingDown).

## 2026-10-12 Recover native Lighting defaults, day-cycle colors and lamp math

**Changed:** Added `lighting.rs`: store tracer for the Lighting and
LightingParameters constructors and setTime's keyframe tables; port of
LightingParameters::setTime, RenderView::presetLighting and SceneManager::setFog;
`audit-lighting` with a Metal comparison. viewport-walking renders with these
globals instead of the explicit probe lighting.
**Why:** Lighting was an explicit probe guess (ambient 0.25, lamps 0.65/0.26).
**Tested how:** Ghidra: Lighting ctor/reflection, LightingParameters ctor/setTime,
static init, spline evaluator, updateLighting, presetLighting, renderPrepareImpl,
setFog; shipped default.hlsl/sky.hlsl. 50 tests, Clippy, fmt; audit-lighting,
walking window smoke, bundle check.
**Result:** All five splines decode (including the computed 0.75/0.4 keys).
14:00 sun (-0.475, 0.823, 0.313), Lamp0 0.675, Lamp1 0.27, Ambient 0.5, fog end
100000. Character render changes 6,613 pixels vs probe lighting, repeat
identical. Window smoke passes with native lighting.
Test: `sh tools/cargo.sh run --locked -- audit-lighting`; expect `lighting_pass`
and `lighting_render_pass`.
**Still broken / not tested:** Light grid (OutdoorAmbient skylight), shadows,
fog culling, high-quality materials; native pixel parity **not tested**. First
tracer pass missed pointer loads from Color3::black(); fixed.
**Next:** Default skybox, sun and moon rendering (Sky::render).

## 2026-10-12 Port the bundled 2016 ClassicCamera and camera-relative movement

**Changed:** Added `camera_script.rs`: RootCamera/ClassicCamera, PopperCam and
TransparencyController from characterCameraScript.rbxmx, with literals parsed
from the original Lua. viewport-walking now uses it (right-drag, arrows, wheel,
I/O) and WASD goes through camera-relative Player:Move as the ControlScript does.
Added `audit-camera`.
**Why:** The walking view used a fixed translation-follow fixture camera and
world-axis input.
**Tested how:** Read the bundled CameraScript/ControlScript sources; Ghidra for
Humanoid.Torso (00bd5050), Player zoom setters and Camera.FieldOfView setter.
50 tests, Clippy, fmt; audit-camera, audit-walking, audit-walking-render,
walking window smoke (now camera-relative), bundle check.
**Result:** Spawn eye 12.5 studs behind/above the head focus, look matching the
script's 0.23 drop to 5e-8; one second of W moves 15.749 studs along the camera
look; quarter-width drag = half turn; pitch clamps at -80 deg; wheel zoom
12.5 -> 9.0809 (RK4); PopperCam keeps the near plane at the wall (eye x 4.41,
unpopped 15.70); transparency fades 0.05/frame to 0.7. Window smoke: 120
frames, 467 world steps, wall contact via camera-relative A.
Test: `sh tools/cargo.sh run --locked -- viewport-walking`; expect a camera
behind the character, right-drag to orbit, wheel to zoom, WASD relative to view.
**Still broken / not tested:** Character transparency not rendered; FOV 70 and
zoom limit defaults are explicit (constructors not located); first-person
rotation, ShiftLock, gamepad/touch/VR/Follow modes not implemented. First audit
attempts used ancestor-matched script modules and wrong test signs; fixed.
Manual mouse/keyboard interaction and native visual parity are **not tested**.
**Next:** Sky and lighting.

## 2026-10-12 Recover native Humanoid state dispatch, floor rays and jumping

**Changed:** Added `humanoid_state.rs` (native STATE_TABLE, floor rays, state
constants read at runtime). Walking now runs Humanoid::onStepped at 60 Hz:
findFloor, timers, table transitions, state update and events. Implemented
Running, Landed, Jumping, Freefall and RunningNoPhysics (kinematic). Floor
support height comes from the rays instead of a constant. Space jumps in
`viewport-walking`. Script poses come from native Humanoid events.
**Why:** The fixture held the character at a fixed height with no states, no
jump, no falling and a 240 Hz AutoRotate cadence.
**Tested how:** Live Ghidra decompilation/disassembly of Humanoid::onStepped,
HumanoidState transitions/events, findFloor, all five states' constructors and
force/update functions, ControlScript jump; 48 tests, Clippy, fmt;
audit-walking (wall, directions, RNP, jump, drop), audit-walking-render, walking
window smoke, animation regression, bundle check.
**Result:** One-second walk 15.746414 (unchanged; RNP starts after 1 s). Open
runs settle in RunningNoPhysics at exactly 16 studs/s and torso Y 18. Jump:
Running->Jumping->Freefall->Landed->Running, rise 8.66 studs, poses Jumping,
FreeFall, Running, Standing. 10-stud drop: Freefall->Landed->Running, returns to
the rest height. 30 s wall hold min up Y 0.9999202. Limb pixels 5567/3100/5764.
Window: 120 frames, 474 world steps. Original files unchanged.
Test: `sh tools/cargo.sh run --locked -- viewport-walking`; Space jumps, the
arms raise while airborne, landing returns to Running.
**Still broken / not tested:** Floor/wall world remains a fixture; climbing,
FallingDown, Seated, Swimming, slopes, moving floors and PGS branches are not
implemented. Contact "touching" is approximated by wall connector generation.
First RunningNoPhysics placement inverted frames and amplified rounding x3 per
frame (degenerate rotation after 0.5 s); fixed by rotating root and offset by
the same yaw matrix. Native jump height and manual keys are **not tested**.
**Next:** Camera system from characterCameraScript.rbxmx, then sky/lighting.

## 2026-10-12 Recover native joint, Heartbeat and Lua wait scheduling

**Changed:** Added `scheduler.rs` and `audit-scheduler`. Walking now steps
Motor6D once per recovered 60 Hz PhysicsJob, before its four world steps, and
resumes the procedural script's `wait(0.1)` on wall-clock time with 30 Hz
Heartbeat time as `move(time)` input. Public signatures are unchanged.
**Why:** The previous 60 Hz motor step and 0.1 s script grid were fixture
choices; motors stepped before the fourth world step and the script used
world-step time.
**Tested how:** Live Ghidra: Motor6D vtable +0x44, MovingAssemblyStage,
World::uiStep/step, world-step accumulator, DataModel/PhysicsJob, RunService
gameStepped/heartbeat, HeartbeatTask, WaitingScriptsJob, Lua wait/resume and
settings. 46 tests, fmt, Clippy; audit-scheduler, audit-walking,
audit-walking-render, walking window smoke, animation regressions, bundle check.
**Result:** Constants decoded at runtime: 60 Hz, dt 0.016666668, 240
steps/s, cap 12, ceiling 1.99999, Heartbeat 30 Hz, minimum wait 0.03. Exactly 4
world steps per job. Two seconds: 120 jobs, 480 world steps, 60 heartbeats, 18
script resumes (gaps 6 or 7 frames). Rest-pose walking numbers identical to
the previous log. Animated swing 1.994/1.962 rad, minimum up Y 0.9996104,
penetration 0.1043663. Limb pixels 5618/3133/5805; exact fresh/repeat/reset.
Window: 120 frames, 476 world steps, 119 joint steps. Standalone animation hash
unchanged; 2,606 original entries unchanged.
Test: `sh tools/cargo.sh run --locked -- audit-scheduler`; expect
`scheduler_pass ... script_resumes=18`. `sh tools/cargo.sh run --locked -- viewport-walking`:
WASD walking with swinging limbs, now updated every 6-7 frames.
**Still broken / not tested:** Ideal jitter-free frame clock and the
Heartbeat→WaitingScripts→Physics order inside a frame are explicit choices.
Throttled/legacy step branches, sleeping-joint skip, long-step work and keyframe
Animator stepping are recorded only. A first window-smoke attempt used `timeout`,
which is absent on macOS, so it never ran; rerun directly and passed. Manual keys
and native visual/behavior parity are **not tested**.
**Next:** Recover the Humanoid state/floor dispatch that replaces the fixture
support, or the original keyframe payloads if a verifiable source appears.

## 2026-10-12 Animate walking limbs and refresh their assembly properties

**Changed:** Walking now advances the bundled procedural script and original
Motor6D angle branch. Recovered Running speed cutoff/deadband selects
Running/Standing. Posed mass parts rebuild Cofm/inertia via native dirty SimBody
semantics. Added walking animation logs, sustained animated diagnostics and
`audit-walking-render`; public signatures remain stable.
**Why:** Walking previously translated the rigid rest pose and never stepped
limb motors. Using the rest tensor after articulation would also distort contact
and balancing response.
**Tested how:** Live Ghidra Running publisher, Motor6D, Cofm refresh and default
loader; all 44 source tests, Clippy; 32-second animated wall hold, four direction
changes, release, pivots/reset; three Metal poses compared to rest at identical
root frames; 120-frame walking window and standalone animation regression.
**Result:** Four limbs swing 1.78-1.96 rad; pivot error 0.00000191 studs.
Minimum animated up Y 0.99958944; penetration 0.10572481 studs. Limb-only pixel
changes 5871/3984/5614; fresh uploads/reset match exactly. Window 467 ticks/116
motor callbacks; all 2,606 original Contents entries unchanged.
Test: `sh tools/cargo.sh run --locked -- viewport-walking`; WASD should move and
turn an upright character with swinging arms/legs; release changes to Standing;
R resets. Automated: `sh tools/cargo.sh run --locked -- audit-walking` expects
`walking_animation_pass limbs=4`; `sh tools/cargo.sh run --locked -- audit-walking-render`
expects `walking_render_pass`.
**Still broken / not tested:** Matching 2016 default remote keyframe payloads are
unavailable; live Ghidra confirms their loader. The shipped procedural branch
is selected explicitly; native callback cadence, full state/floor dispatch and
default-keyframe parity are **not tested**. Manual keys/visual parity are
**not tested**. First Clippy flagged test-module placement; moved it to the end
and reran. Slight lateral sliding persists during sustained fixture wall contact.
**Next:** Recover original keyframe payloads and native joint/state scheduling.

## 2026-10-12 Fix walking tipping and uncontrolled rotation

**Changed:** Added the recovered legacy Balancing torque controller, native
AutoRotate desired yaw and RunningBase yaw feedback. Walking reset clears cached
balancing torque. Added sustained wall, directional, angular-release and impulse
recovery diagnostics; public signatures remain stable.
**Why:** Wall contacts spun and tipped the character because neither native
angular controller was called by the walking fixture.
**Tested how:** Live Ghidra constructors, vtable, decompilation and assembly;
30-second wall hold, four eight-second directional runs, release and off-center
impulse recovery; source tests, Clippy and original bundle fingerprint.
**Result:** Minimum up Y 0.9999184, max wall yaw speed 0.003509 rad/s, max
penetration 0.11379337 studs. Test:
`sh tools/cargo.sh run --locked -- audit-walking`; expect
`walking_balance_pass sustained_wall_seconds=30`. Interactive test:
`sh tools/cargo.sh run --locked -- viewport-walking`; hold D into the wall, change
directions, release; expect upright posture and turning toward movement.
**Still broken / not tested:** Native differential parity and manual keys are
**not tested**. Rest-pose wall still slides slightly laterally under sustained
force. Walking limbs remain motionless until the next animation change. Initial
audit final impulse count accidentally reflected a reset; corrected. A first
template lookup used the project root instead of reference templates; corrected.
**Next:** Advance original limb motors during walking and refresh animated Cofm.

## 2026-10-12 Wire native movement and wall response into a playable fixture

**Changed:** walking.rs integrates Humanoid input, original WalkSpeed, RunningBase
horizontal/support forces, assembly substeps, original colliders and native wall
connectors. Added viewport-walking, viewport-walking-smoke and audit-walking;
WASD/release/reset/follow-camera and independently authored floor/wall geometry.
Existing public signatures and default probes remain stable.
**Why:** Exercise the recovered core physics/render paths together with playable
input while keeping unrecovered floor/state/scheduler choices explicit.
**Tested how:** Live Ghidra spring/face-selection constants; 42 source tests,
Clippy, one-second movement/release and two-second wall audit; 120-frame Metal
smoke with scripted input and character-only pixel comparison; previous motion
window regression; all 2,606 original Contents entries match the fingerprint.
**Result:** WalkSpeed 16, one-second distance 15.746418, release stops; wall audit
817 impulses and maximum penetration 0.34594154. Final window: eight draws,
3,210 indices, 120 frames/466 ticks/789 impulses. Character contributes 59,432
pixels and restores identically. Previous motion hash remains identical.
Test: `sh tools/cargo.sh run --locked -- viewport-walking`; expect rendered
character/floor/wall, WASD translation and R reset. Automated verification:
`sh tools/cargo.sh run --locked -- audit-walking`; expect `walking_pass`.
**Still broken / not tested:** First Clippy flagged a needless return; corrected.
First window smoke lost its injected input to a focus event; corrected and rerun.
Fixture camera was moved to the character's side of the wall before the final
contribution check. Initial face-axis tie handling was corrected from live native
code. One glob used the wrong script path, then rg ignored install data; corrected
using the original fonts path and --no-ignore. Manual keys, native parity,
balancing, general collision, native floor/state/scheduling, default keyframes and
complete rendering are **not tested**. Limbs stay in their original rest pose.
**Next:** Recover balancing/native floor queries and original keyframe data/runtime.


## 2026-10-12 Generate legacy contained-face contacts and solve awake impulses

**Changed:** Added legacy_contact.rs, runtime collision face table, native
vertex/plane geometry, awake effective-mass/restitution/friction impulse branch,
contact impulse application, and audit-legacy-contact.
**Why:** Legacy BlockBlockContact differs from Bullet's four-point manifold and
requires angular assembly response and native friction/rebound calculations.
**Tested how:** Live Ghidra geometry/face/impulse/matrix inverse tracing; 42 tests;
generated-contact diagnostic; central restitution and two-body momentum checks;
formatting and Clippy with warnings denied.
**Result:** Four generated connectors; four impulse applications; no approaching
contact remains. Two center collision cases and equal-mass momentum pass. Spring
force integrates through SimBody. Test:
`sh tools/cargo.sh run --locked -- audit-legacy-contact`; expect
`legacy_contact_pass generated=4` and `momentum_conserved=true`.
**Still broken / not tested:** First multi-file patch missed formatted main import
context; no edits were applied, then corrected. Relative rotations, partial face
clipping/edge pairs, sleep, broadphase, native invalidation/adaptive iteration
scheduling and differential parity are **not tested**. The fixture uses explicit
material coefficients and iteration bound. Viewport walking remains incomplete.
**Next:** Couple RunningBase forces to the assembly and rendered input path.


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
