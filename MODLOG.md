# MODLOG

## 2026-10-15 Anchored Ball Parts

**Changed:**
- `ball_contact::PrimContact<T>`: the contact-class enum moved out of
  loose.rs and made generic. LooseWorld (`PrimContact<Link>`) and World's
  character/static contacts (`PrimContact<ContactState>`) both use it.
- `StaticBlock::shape`. A static Ball gets the Ball stiffness, extents and
  contact class, the sphere ray test, and the Ball volume as floor mass
  (`LooseWorld::add_static_shape`; `add_static` still adds a Block).
- The Head touch test against a static Ball runs the BallBlock test
  (00ae2aec) instead of the box overlap stand-in.
- Kernel joints to an anchored Ball use the Ball stiffness.
- Only Cylinders are refused now.
- `audit-part-ball` adds "Pebble", dropped off-center on the anchored
  "Dome".

**Why:** Finishes the Ball shape, the first part of roadmap item 3.

**Tested how:** `sh tools/cargo.sh test --locked` (107 pass), clippy clean,
all 23 audits against the previous commit's outputs.

**Result:**
- `audit-part-ball` passes. The Pebble meets the Dome (a static BallBall
  connector at step 32), rolls off and lands on the Baseplate.
- The existing balls' results are unchanged. The other 22 audits are
  unchanged (audit-runtime differs only in timing).
- The character against a Ball is **not tested**. Native parity is **not
  tested**.

## 2026-10-15 Ball Parts in the world

**Changed:**
- `PartInstance::shape` (Part.Shape) and `set_shape` (00708de2). For a Ball,
  `set_size` stores (x, x, x) (00988cfc, `hasThreeDimensionalSize` 0070915c).
- `LoosePrim::shape`. A loose Ball gets the Ball mass, inertia and
  stiffness, and its pairs get a `BallContact`: `PairContact` holds either
  contact class (createContact 00ae714a).
- Broadphase extents for a loose Ball (`broadphase::ball_extents`).
- The Humanoid ray query tests a loose Ball as a sphere (`ball_hit_test`:
  Ball +0x70 00adc17c -> G3D collisionTimeForMovingPointFixedSphere
  0124e2c8), bounded by its radius.
- Anchored Balls and any Cylinder are refused with an error: statics are
  Block-only.
- `LooseWorld::contact_report` logs each live connector's kind and length.
- New `audit-part-ball`, and unit tests for the Ball size and the sphere ray
  test.

**Why:** Roadmap item 3. This puts the Ball narrowphase from the previous
commit to use.

**Tested how:** `sh tools/cargo.sh test --locked` (107 pass), clippy clean,
`audit-part-ball`, and the 22 other audits against the previous commit's
outputs.

**Result:**
- `audit-part-ball` passes:
  - the dropped ball rests 0.02 studs into the Baseplate and sleeps;
  - a ball rides a loose block and both sleep;
  - a ball rolls down a 10 degree ramp at 0.958 of 5/7 g sin 10 with
    w r / v = 1.01;
  - two balls that meet push apart.
- **First attempt failed, and the fault was in the audit.** It measured the
  roll per 1/60 s, but `step_animated` is one 240 Hz world step. A
  per-step trace showed free fall gaining g/240 per call.
- **Open:** a ball rolling on flat ground slows down (about 0.29/s). Angular
  damping explains only part of that. Not traced; native **not tested**.
  Recorded in ROADMAP.
- Every other audit is unchanged (audit-runtime differs only in timing).
  Native parity is **not tested**.

## 2026-10-15 Ball geometry and the ball contacts (narrowphase only)

**Changed:**
- New `ball_contact.rs`. It holds the Ball radius (size.x / 2), the extents
  and the BallBall and BallBlock collision tests. It also has the Block clamp
  and feature lookup, the connector geometry and `BallContact`, the
  one-connector contact with the same `update` and events as
  BlockBlockContact.
- `block_contact::PairKind` gains BallBall, BallVertex, BallEdge and
  BallPlane. `GeoPair::compute` sends them to `ball_contact`. A few vector
  helpers became `pub(crate)`.
- `MassProfile::ball_volume` and `ball_inertia`, with their constants
  (01834ffc, 016f753c) read from the install.

**Why:** Roadmap item 3 (shapes) starts with the Ball. This step is the
narrowphase only, so it can be checked on its own before the world uses
it.

**Tested how:** `sh tools/cargo.sh test --locked` (105 pass, 6 new), clippy
clean, audit-assembly still passes (MassProfile loads the two new constants).

**Result:** The unit tests pass on hand-computed cases. Not yet wired into
the world: Parts are still all blocks. Native parity is **not tested**.

## 2026-10-15 An animating character wakes what it touches

**Changed:**
- `LooseWorld::wake_character_recursive`: 00b737c4 for the character's
  assembly. While the character is in SleepStage, the history countdown
  restarts, SleepStage +0x16c is set, and the character becomes
  RECURSIVE_WAKE_PENDING.
- `WalkingFixture::step_animated` calls it for each Motor6D whose link
  changed in stepUi (00b37550 wakes Part0's and Part1's assembly, recursive),
  as native does.

**Why:** Found while decoding the Motor6D setter for the previous change.
The port never woke the character from its motors, so a Part touching an
animating character could fall asleep under it.

**Tested how:** `sh tools/cargo.sh test --locked` (99 pass), clippy clean,
all 22 audits against the outputs of the previous commit.

**Result:**
- audit-part-push changes and still passes. The Crate stays awake while
  the Torso touches it, which is to the end of the run (before, it slept
  with the character pressed against it). The character pushes it 1.4267
  studs instead of 1.3100, and the peak character connector count is 7
  instead of 8. FEATURES updated.
- Every other audit is unchanged (audit-runtime differs only in timing).
  In audit-part-stand the character stands on its Parts in
  RunningNoPhysics, which keeps it outside SleepStage, so its motors wake
  nothing. In audit-part-jump the Pad still sleeps under the standing
  character, which fits the same reason (not checked step by step).
- Whether native keeps the Crate awake this way is **not tested**.

## 2026-10-15 Motor6D links outside the character

**Changed:**
- `motor::d6_offset` (D6Link::computeJointCoord 00a9d6d0): (parent x T) x
  child^-1.
- `rig::Motor6D::new` and `threshold`, so a Workspace Motor6D can be built
  with the rig's DesiredAngle dead zone.
- `LooseWorld`: motors are `MotorKind::Revolute` (MotorJoint) or
  `MotorKind::D6` (Motor6D, stepped by the ported 00b3717c; a changed link
  state counts as moved, 00b37550). `add_motor_6d`, `motor_6d`.
- `World::in_workspace` builds Motor6D links the same way as Motor links
  (parent side from 00b36f5a, the same test as 00b3807c).
  `Workspace::joint_tree` accepts Motor6D.
- `audit-part-motor` adds "Tower3" on a Motor6D.

**Why:** Rigs in places (dummies, NPCs) hold their limbs with Motor6Ds.
Without this, a Workspace with one could not load.

**Tested how:** `sh tools/cargo.sh test --locked` (99 pass), clippy clean,
`audit-part-motor`, and the other 21 audits against the outputs of the
previous commit.

**Result:**
- audit-part-motor passes. The Motor6D tower reaches pi/2 at step 125 like
  the other two, its arm ends at (-3.8e-6, 0.6999999, -2.0) in its tower's
  frame, and it sleeps at step 239. Its center of mass differs from the
  Motor towers' by 2e-5 studs (the D6Link rotation comes from
  fromAxisAngleFast).
- Every other audit is unchanged (audit-runtime differs only in timing).
- Found while decoding 00b37550: a Motor6D whose link changes wakes its
  assembly recursively. The character's Motor6Ds do this every frame they
  animate, and the port does not do it yet (next change).

## 2026-10-15 Motor joints inside assemblies

**Changed:**
- New `motor.rs`: `MotorJoint` (CurrentAngle, DesiredAngle, MaxVelocity;
  stepUi 00b381f6 with the setter 00b3831c) and `revolute_offset`
  (RevoluteLink::computeJointCoord 00a9d5b0 with 006b130e).
- `LooseWorld`: `add_motor`, `step_motors` (the angle step, the child's new
  link offset, a recursive wake, then the SimBody rebuilt from the Body
  tree), `motor`. `reset` restores the motors and their links.
  `add_assembly`'s mass sum moved into `tree_properties` (same arithmetic).
- `JointTree::body_frames_with` (motor links), `Workspace::joint_tree_indexed`
  (ground joints share the tree's numbering), and `Workspace::joint_tree`
  accepts Motor. `JointInstance` has `current_angle`, `desired_angle`,
  `max_velocity`.
- `World::in_workspace` builds Motor links (parent-side and child-side
  frames from 00b38196) and refuses a Motor in an anchored assembly.
  `World::step_ui` steps the motors after the hinges.
- ROADMAP: ManualGlue moves to item 5. Its face is the overlap of the two
  Parts' surfaces (00b0da3a), the same face geometry JointsService needs.
- New `audit-part-motor`.

**Why:** Roadmap physics item 2. Motors join Parts in many 2016 models
(older rigs, vehicles, scripted doors).

**Tested how:** `sh tools/cargo.sh test --locked` (99 pass; new motor tests
for the angle step and the link offset), clippy clean, `audit-part-motor`
twice (identical apart from the evidence log path), and the other 21 audits
against the outputs of the previous commit.

**Result:**
- audit-part-motor passes:
  - both motors reach pi/2 at world step 125 (frame 32 at 0.05 a frame);
  - both arms end at (0, 0.7, -2) in their tower's frame (a quarter turn
    from +X to -Z), including the Motor whose Part0 is the arm: the link
    turns the parent-side frame, so the turn is the same in the world;
  - the towers stay upright (they sink 0.016 into the Baseplate as resting
    blocks do), the center of mass moves with the arm, and both towers
    sleep at step 239.
- Every other audit is unchanged (audit-runtime differs only in timing).
- Not ported: Motor6D outside the character (D6Link), VelocityMotor, motor
  animation, and motors in anchored assemblies. Native parity is **not
  tested**.

## 2026-10-15 Hinges and motor hinges in the kernel

**Changed:**
- `kernel_joint.rs`:
  - joint types `ROTATE`, `ROTATE_P`, `ROTATE_V`;
  - `SurfaceInput` (InputType, ParamA, ParamB; default NoInput, -0.5, 0.5)
    and its motor value (Constant, Sin);
  - the angle helpers `wrap_angle` (006af7f9) and `halfway` (006af8f5);
  - `hinge_points` (C x (0, 0, -1), then (0, 0, 1)) and `cross_size`
    (00b6b53c);
  - `RotateConnector`: the angle (00a913ec), `compute_torque` (00a9178e
    with 00a91724), `set_target` (RotateP, 00a9159a) and `set_velocity`
    (RotateV, 00a916cc).
- `LooseWorld`:
  - `add_hinge`, plus `add_fixed_part`, so a fixed Body knows its size;
  - `step_ui` (the motor hinges' stepUi), `step_joints` (their stepWorld);
  - `wake_assembly_recursive` and the SleepStage +0x16c flag: after a
    recursive wake from stepUi, the next recursive pass also walks through
    awake neighbours;
  - `hinge_angle`. `joint_report` adds the motor angles.
  RotateConnectors sit in jointConnectors after their hinge springs and
  push torques straight into the root SimBodies. Only Glue is breakable.
- `KernelJoint` has `kind` and `value`. `JointInstance::base_angle`,
  `PartInstance::surface_inputs`. `Workspace::joint_tree` accepts the
  hinges.
- `World::in_workspace` loads the hinges (with the axle Part's input on the
  face C0's z axis names). `World::step_ui` runs after breakJoints in
  uiStep, and both world steps call `step_joints` between SleepStage and
  Kernel::step.
- New `audit-part-hinge`.

**Why:** Roadmap physics item 2. The hinge surfaces and the old motors
(RotateP, RotateV) are how 2016 places build doors, wheels and moving
parts, and the JointsService surface joints (item 5) create them.

**Tested how:** `sh tools/cargo.sh test --locked` (97 pass; new kernel_joint
tests for the angle wrap, hinge points, surface inputs and the
RotateConnector), clippy clean, `audit-part-hinge` twice (identical apart
from the evidence log path), and the other 20 audits against the outputs of
the previous commit.

**Result:**
- audit-part-hinge passes:
  - the level arm on a Rotate swings down to -3.003 rad, nearly up the
    other side, and stays aligned with its axis (z alignment 0.9999999);
  - every hinge point stays within 0.001 studs of its axle;
  - the hanging flap sleeps at step 119 and its hinge goes SLEEPING;
  - the RotateV wheel (Constant 0.1) turns at 2.9989 rad/s over the last 2
    seconds, against 30 x 0.1 = 3 expected;
  - the RotateP paddle (Sin 0.5, 2) swings between -0.507 and 0.503 rad,
    at most 0.016 rad off 0.5 sin(2t).
- Every other audit is unchanged (audit-runtime differs only in timing).
- Checked against setAssemblyState 00b727c0: a recursive wake sends an
  AWAKE assembly to RECURSIVE_WAKE_PENDING and back each frame without
  leaving the kernel, so the wheel's forces are not reset.
- Not recovered: which joints MovingAssemblyStage lists for stepUi; the port
  steps every motor hinge still in SleepStage. Whether the native arm,
  wheel and paddle move the same way is **not tested**.

## 2026-10-15 Glue joints in the kernel

**Changed:**
- New `kernel_joint.rs`:
  - kernel Points: `KernelPoint`, `new_point` with the (Body, offset)
    dedupe, `release_point`;
  - the two break connectors: `BreakConnector`, `BreakTest`;
  - Glue's geometry: `normal_id`, `joint_strength` (Constants 00a92056,
    table 0179fc80 bit for bit), `face_size`, `glue_threshold`,
    `face_through`.
- `LooseWorld`:
  - joints: `add_fixed_body`, `add_glue`, `joints`, `break_joints`,
    `broken`;
  - the substep: `step_points`, `points_to_bodies`;
  - logs: `joint_report`, `point_count`.
  Glue connectors share jointConnectors with the contact springs, and
  joints are SleepStage edges (AWAKE = in the kernel).
- `World::in_workspace` loads Glue joints, with anchored or grounded Parts
  as fixed Bodies. The substep now runs, in 00a9768c's order: Points, the
  realtime springs, jointConnectors, the Points' forces into the bodies,
  the Humanoid, integrate. `World::break_joints` runs once per frame in
  uiStep, before the Motor6D stepUi.
- `JointInstance::face` (F0..F3). `Workspace::joint_tree` accepts Glue.
- `JointTree::grounded` now asks whether the assembly root's tree edge is an
  AnchorJoint. Before, it walked up through kernel joints, so a Part glued
  under an anchored Part counted as grounded.
- New `audit-part-glue`.

**Why:** Roadmap physics item 2 (kernel joints), starting with Glue. The
Points, the break connectors, the joint edges and breakJoints are shared by
the hinge and motor joints that come next.

**Tested how:** `sh tools/cargo.sh test --locked` (93 pass; new
`glued_under_anchored_still_moves`, and the kernel_joint tests), clippy
clean, `audit-part-glue`, and the other 19 audits against the outputs of the
previous commit.

**Result:**
- audit-part-glue passes:
  - the Part glued under an anchored Part holds within 0.00013 studs and
    sleeps at step 119, then its Glue goes SLEEPING;
  - the glued pair falls with no stretch and rests 2 studs apart;
  - the 14-stud cube's 1 x 1 Glue breaks at step 9 and the cube lands.
- Every other audit is unchanged (audit-runtime differs only in timing).
- The pair bounces on its Baseplate contacts for about 180 steps (repeated
  recursive wakes) before it rests 0.0104 into the Baseplate. Its contacts
  are springs, because a glued body is a kernel joint body. Whether native
  bounces the same way is **not tested**.
- README: the status lists the joints, and the broadphase is no longer
  under "not there yet" (it landed earlier).

## 2026-10-15 Waking drops the force gathered while asleep

**Changed:**
- `KernelBody::reset_force` (00b7439e): the force goes back to gravity alone
  and the torque to zero.
- SleepStage calls it when an assembly becomes AWAKE and enters the kernel
  (setAssemblyState 00b727c0, guarded by downstreamOfStage 00b2a8d0).
- `audit-part-jump` drops its "the Pad takes more pushes" check. It now
  checks that the Pad is asleep when the jump starts, wakes, and sinks.

**Why:** Kernel joints push on SimBodies that are outside the kernel. The
reset decides whether those forces survive a wake. The old docs said the
force was kept, which was wrong.

**Tested how:** `sh tools/cargo.sh test --locked` (89 pass), clippy clean,
the 17 baseline audits plus audit-part-weld and audit-joints.

**Result:**
- The baseline audits other than audit-part-jump are unchanged (timings
  aside). audit-part-weld and audit-joints pass; there was no saved output
  to compare them with.
- audit-part-jump: the Jumping pushes made while the Pad slept (steps
  481-483) used to land all at once when it woke at step 484. That sent the
  Pad to vy -118 and gave the character 76 extra pushes (304 against the
  Baseplate's 228).
- Now the Pad reaches vy -65.6. Both jumps take 228 pushes; the rise is
  8.59 from the Pad and 8.66 from the Baseplate.
- Native parity is **not tested**.

## 2026-10-15 Fast-impact tumble traced

**Changed:** `docs/ROADMAP.md`, open findings: the trace of the fast-impact
tumble.

**Why:** The previous step found that a tall block dropped flat at high speed
gets a sideways kick. This entry checks whether that is a porting bug.

**Tested how:** A temporary trace of every connector impulse on a 4x4x2
block's first contact step (not committed). The ported `AwakeContact::solve`
was compared with a936d0 and a92dd6, line by line.

**Result:** The four corner impulses run once each, in order; the block leaves
the floor before a second pass. The port matches the decompiled solver, so no
code changed. Native behaviour is **not tested**.

## 2026-10-15 Welded Parts in the physics

**Changed:**
- `LooseWorld` separates Parts from assemblies:
  - `prims` (`LoosePrim`): one per Part, with its block, materials and Body
    link;
  - `parts` (`LoosePart`): one per assembly, with the SimBody, kernel lists
    and sleep state;
  - new `add_prim`, `add_assembly`, `prim_frame` and `prim_pv`.
  `add_part` keeps its signature and makes a one-Part assembly.
- A Part's pose and velocity follow its Body (updatePV 00a8eabe). Contact
  springs read the Body PV (a929f6), as does SleepStage's fast-contact test
  (00b7361a).
- `Workspace::joints` (`JointInstance`) and `Workspace::joint_tree`;
  `JointTree::body_frames`. `World::in_workspace` builds assemblies from the
  tree, and grounded ones become static blocks.
- Broadphase pairs are refused inside one assembly (per-primitive group).
- Humanoid floor rays, floor mass, floor push and floor wake use the Part's
  assembly.
- New `audit-part-weld`.

**Why:** Roadmap physics item 1, part B: the joint tree now drives the
simulation.

**Tested how:**
- `audit-part-weld` passes. Its three scenes:
  - an L-shape whose misplaced Arm snaps to the weld, falls, lands flat and
    sleeps;
  - a Part welded under an anchored block, plus one welded under it, both of
    which stay put;
  - a plank with a weight that tips and rests.
- The 17 audits used as a baseline give byte-identical output before and
  after, apart from timings and log paths.
- 89 unit tests pass; clippy and fmt are clean.

**Result:** Works for joints present at load. **Failure recorded:** dropped
from 6 studs, the L-shape bounces sideways and lands on its side. A single
4x4x2 block (no welds) does the same, so the cause is in the existing
single-Part contact path, not the welds. It is logged as an open finding in
ROADMAP.md, and the audit drops the L-shape from 2 studs. Native parity is
**not tested**.

## 2026-10-15 Joint spanning tree (GroundStage and TreeStage)

**Changed:**
- New `joints.rs`, ported from the binary:
  - joint types (Anchor 0, Weld 1, ManualWeld 2, Snap 3, Motor 4, Motor6D 5,
    Rotate 6, Glue 9, Free 20) from each joint class's getJointType;
  - Part root priority 00b63fbe and the name multipliers of 00983a3e;
  - the joint ranking 00b216e2 with the GUID compare 006721c8;
  - GroundStage's ground joints (00b0f1a8, 00b0f208, 00b0f7ac, 00b0f4e8);
  - SpanningTree insert/remove with re-rooting (006cc3d6, 006ccc72, 006cc70a,
    006cc9ca, 006ccb3e, 006cc580, 006cc624);
  - TreeStage's Body links and the rigid offset (00ba2e24, 00ba36a6, 00b69dd6).
- `g3d.rs`: Matrix3 multiply (0125581a), CoordinateFrame multiply (00972080),
  the rigid inverse of 00b69dd6, isOrthonormal (01255004) and orthonormalize
  (01255ca6).
- New `audit-joints`.

**Why:** First step of rigid assemblies (roadmap physics item 1). The tree
decides each assembly's root Part and Body layout.

**Tested how:** 6 unit tests in joints.rs and one in g3d.rs; audit-joints on
the shipped R6 rig and three weld scenes. 88 tests pass; clippy and fmt are
clean. No existing audit is affected (nothing calls the module yet).

**Result:** On the R6 rig the tree roots at HumanoidRootPart and the Torso's
Body children come out as Right Arm, Left Arm, Right Leg, Left Leg, Head,
the order the character mass code already uses. Assumed: GUID order is Part
creation order, address ties follow joint creation order. Native parity is
**not tested**.

## 2026-10-15 Roadmap: physics, places, Lua

**Changed:** New `docs/ROADMAP.md`: the order of the remaining work, the
native World stage chain and the list of physics items left.

**Why:** The user asked for the rest of the physics, then place loading, then
a Lua runtime that runs scripts as the original does.

**Tested how:** Ghidra only, no code change.
- The World constructor (00ba6a0e) and each stage constructor were followed
  from CleanStage to the Kernel. Each stage was named from its vtable's
  typeinfo.
- `PGSSolverDefaultOnNewPlaces` is registered with default 0 at 001423f7, so
  new places use the legacy solver offline.

**Result:** The ported solver is the one new places use. Terrain and mesh
collisions are moved after the place loader.

## 2026-10-13 The broadphase decides the contacts

**Changed:**
- `World::apply_broadphase` turns broadphase events into contacts:
  - character/static-block contacts go into `static_contacts` (stepped in creation order);
  - every other contact becomes a `LooseWorld` pair through the new `contact_created` / `contact_destroyed` (World::insertContact 00bab0f0 / destroyContact 00bab1f6).
- Loose pairs are keyed in the broadphase's order, searching primitive first. 00ae714a keeps that as body 0, so the Baseplate is now body 0 against the bricks it finds at the start.
- Removed the all-pairs rule: `update_membership` no longer creates every (loose, static) and (loose, loose) pair. The character's edges come from its existing contacts.
- The Head touch test (`collider_touching`) only tests existing contacts.
- `clear_character_contacts` resets those caches instead of deleting the pairs.
- `World::reset` replays the start contacts.
- The character-static block contact keeps the character Part as body 0. `Broadphase::static_first` counts the contacts native would create the other way round: 0 in every audit.
- Labels: `broadphase_order=assumed` became `broadphase=spatial_hash`. audit-part-push prints `part_push_broadphase`.

**Why:** The user asked to port the broadphase 1:1. This is the second step: contacts now follow it.

**Tested how:**
- Ran all 17 audits.
  - Walking, baseplate, camera, kernel, player, locomotion, runtime, animate, the render audits and audit-humanoid-slope: byte-identical.
  - part-wall: label only.
  - part-drop, part-push, part-stand, part-jump: changed slightly (below).
- viewport-baseplate-smoke exits 0.
- 81 tests pass; clippy and fmt are clean.

**Result:**
- part-drop: the heights move in the 4th digit and Drop ends on its side (up_y -9.4e-5).
- part-push: the Crate moves 1.31 studs (was 1.08), with up to 8 character connectors (was 7). It enters contactBodies at step 780, when it wakes with only its ground contacts.
- part-stand: pad_top 0.18826233 (was 0.18830347).
- part-jump: the rise is unchanged.
- No contact is destroyed in these scenes; destruction is covered by the unit test.
- Native parity is **not tested**. Still assumed: the start order of findAllContacts, and the IndexArray swap.

## 2026-10-13 Broadphase (SpatialHashMultiRes), running alongside the physics

**Changed:**
- New `broadphase.rs`, reverse engineered from the native functions:
  - extents (00b61c9a / 00adf0a0);
  - level (00af1350; constants 8, 100, max level 3, 2 for fixed Parts);
  - cells (00b88880) and the bucket hash (00b8885c);
  - insert 00aee438, remove 00aef058, extents changed 00aecaea, move 00af1400, cell walk 00aee5ec;
  - add node / contact search 00af21cc, descend 00af19dc, remove node 00af2714;
  - separation 00aef1d8 / 00aef442, findAllContacts 00af1684.
- `World::broadphase` (`Broadphase`, `BroadPrim`):
  - Workspace Parts enter in Workspace order, then the character's Parts;
  - each then runs findAllContacts;
  - after every world step, the character, then the loose Parts in SimulateStage order (`LooseWorld::simulating`), update their extents.
- It only logs contacts (`Broadphase::log`); the physics still uses every pair. New `World::broadphase_name` and `broadphase_extents`.
- New `audit-broadphase`.
- The README change you made ("open source") is committed separately as-is. AGENTS.md already describes the new docs layout, so it is unchanged.

**Why:** The user asked to port the broadphase 1:1. This step adds it without changing the physics, so it can be checked first.

**Tested how:**
- Ran audit-broadphase, then all 16 other audits. The others are byte-identical to the previous build apart from evidence paths and timing.
- 81 tests pass (4 new broadphase tests); clippy and fmt are clean.

**Result:**
- The Baseplate is at level 2 and the bricks at level 0.
- 4 contacts at the start, Stack before Rest (the newest node in a cell comes first).
- The falling character's Torso and Head get Baseplate contacts at steps 206 and 209.
- No overlapping pair is missing a contact, and no contact is inconsistent.
- Assumed, not verified: the order in which assemblies run findAllContacts at the start; the IndexArray swap on contact removal.
- A RE finding that affects the next step: Drop, 7.4 studs above the Baseplate, already shares a level-2 cell with it, so native gives it a contact (with no connectors) from the start.

## 2026-10-13 Public-facing README; detail moved to docs/

**Changed:**
- `README.md` rewritten, modeled on the IW4L README the user supplied. It has a short description, a minimal status, install/build/run steps (clone, the Roblonium archive link for 0.255.0.82159, placing the app at `RobloxStudio/`, the optional checksum and animation fetch, then run), the documentation links, and acknowledgements/license.
- The old README content moved verbatim into three files (relative links fixed):
  - `docs/RUNNING.md`: the viewports and the first diagnostic;
  - `docs/FEATURES.md`: what works / doesn't work;
  - `docs/DATA.md`: data and rollback.
- `AGENTS.md`: the "keep updated" rules now point detailed status at `docs/FEATURES.md`.

**Why:** The user asked for a shorter, public-facing README.

**Tested how:**
- Every relative link in the four files resolves.
- The install steps were **not tested** from a fresh clone. Which of the two archive zips matches the local copy, and the zip's internal layout, are **not tested**.

**Result:** Documentation only; no code changes.

## 2026-10-13 audit-humanoid-slope: the steep-floor rules

**Changed:**
- `Humanoid::steep_floors` (`SteepFloor`) logs every steep-floor rewrite: normal, d, limit, velocity before and after.
- `steep_floor` now branches like the native `d < limit`, written `!(d < limit)`, so a NaN skips the rewrite. It takes `&mut self` (private).
- New `audit-humanoid-slope`: a 35-degree anchored slope, run with MaxSlopeAngle 89 and again with 30.
- Re-checked against Ghidra: 00c0a498 (operation order, unitize 1e-6, WalkSpeed +0x194), 00c0177e (event 2) and 00c0215e (its slope).

**Why:** The user listed "the steep-slope rule have no audit of their own."

**Tested how:**
- Ran audit-humanoid-slope and all other audits: unchanged.
- 77 tests pass; clippy and fmt are clean.

**Result:**
- 89: no rewrites; the jump fires.
- 30: 13 rewrites to (0, -9.177, 13.106), which has a normal part of 1e-6 and WalkSpeed downhill to within 1e-6. The jump is refused and the character slides 2.31 studs downhill in 52 steps.
- Native parity is **not tested**.

## 2026-10-13 audit-part-jump: the Jumping floor push

**Changed:**
- `Humanoid::jump_pushes` (`JumpPush`) logs every Jumping push (00c05e0e). For each push it records the floor, acceleration, root mass, the mass used, force, the existing y force and the reaction.
- New `audit-part-jump`: the character lands on the light Pad, idles 240 steps and jumps; then the same jump from the Baseplate. Every push is checked bit for bit against force = acceleration × min(root, floor), force > existing, and reaction = force − existing.
- Re-checked 00c05e0e against its instructions: the dot product order, MINSS, the UCOMISS/JBE compare and the floor torque all match the port.
- README: corrected the part-stand numbers. They changed with the floor friction commit, which I missed because my audit runner did not include part-stand.

**Why:** The user listed "The jump push on the floor ... no audit of their own."

**Tested how:**
- Ran audit-part-jump, then all other audits: unchanged.
- 77 tests pass; clippy and fmt are clean.

**Result:**
- Pad: 304 pushes, mass 7.56, rise 9.415.
- Baseplate: 228 pushes, mass 9.9, rise 8.658.
- Wrong assumption, recorded: I expected the lighter floor to give a lower jump. The push drives the speed to JumpPower, so it only takes longer. The audit checks the push count instead.
- Pad sink: the pre-step 00a96eb0 has no realtime → contact path, so the Pad stays a realtime body under the Humanoid's push. Native sink depth is **not tested**.

## 2026-10-13 PopperCam ray sees unanchored Parts

**Changed:**
- New `World::camera_raycast`: FindPartOnRayWithIgnoreList (00882368, getHit 00ae818c, ignore-list filter 0086dc78) over static blocks and loose Parts. It applies PiercingCast's skip of Transparency > 0.95 and CanCollide false.
- `WalkingFixture::camera_raycast` (the PopperCam query in the audits and the viewport) now uses it, not the static-only `Statics::raycast`.
- Static blocks and loose Parts now keep their Transparency. `LoosePart::transparency`.
- The ray walk is shared: `raycast_where` and `Statics::block_hit_where` take hit filters. The Humanoid query is unchanged.
- audit-part-push prints `part_push_camera_ray`.

**Why:** The user listed "The camera's ray still ignores loose Parts."

**Tested how:**
- Ran all 13 audits. Only part-push changed, by the new line.
- 77 tests pass; clippy and fmt are clean.

**Result:**
- The camera ray from 10 studs behind the Crate stops on its face (z -8.080032; the face is at -8.080033). The static-only query misses.
- Ties at equal distance and native parity are **not tested**.
- A transparent or non-colliding loose Part being pierced is **not tested**; no audit scene has one.

## 2026-10-13 RunningBase floor friction

**Changed:**
- `MaterialTable::floor_friction` (00a43f4c / 00a43e7e): the weighted mean of the floor's friction and Plastic's, (w1*f1 + w2*f2)/(w1+w2), or 0 when the weights sum to 1e-6 or less.
- The Humanoid's RunningBase horizontal force (00c09632) is scaled by it when there is a floor. Static blocks and loose Parts report their own value through `Surroundings::floor_friction`. With no floor the factor is 1.
- The legacy MovingNoPhysicsBase path is unchanged: it has no friction.

**Why:** The user listed "Floor friction is still held at 1."

**Tested how:**
- Ran all 13 audits and compared them with the previous build.
- 77 tests pass; clippy and fmt are clean.

**Result:**
- Plastic on Plastic gives 0.3.
- Walking still reaches -16 studs/s. Positions shift slightly, and the wall-release jitter drops from 0.0787 to about 0.
- audit-part-push: the Crate moves 1.08 studs instead of 37.15.
- audit-part-wall stops at z -9.51014.
- part-stand, part-drop, camera, baseplate and the render audits still pass.
- Native parity is **not tested**.

## 2026-10-13 SleepStage: still unanchored Parts fall asleep

**Changed:**
- New `sleep.rs`: Sim::AssemblyState, the assembly motion history (00adafbc, 00adb06a, 00adb118, 00adb2d8), the sample (root position and quaternion times the assembly radius, 00adaf4a / 00ad9d72), and the native thresholds (0.01 still, 0.02 moving, 2.0 kinematic wake).
- `LooseWorld::sleep_stage` runs SleepStage::step 00b707f4 in native order:
  - wake passes;
  - stepContacts over the STEPPING contacts, then later the CONTACTING ones;
  - the sleep check of awake assemblies;
  - the 4-step recheck.
  It replaces the per-step `update_contacts` / `update_character_contacts` calls in `World` (both functions are kept).
- Contacts carry the native edge states. A contact makes connectors only while CONTACTING with a kernel body.
- Sleeping Parts leave the kernel and act as fixed bodies for connectors; waking re-enters them through 00a9be30.
- The character assembly is in the stage only while it is in the kernel. Its history marks it moving or still; it never sleeps (Humanoid::onStepped restarts its countdown).
- The Humanoid wakes a new floor Part (00bff784) and a floor it walks over kinematically faster than 2 studs/s (00c07626). New `World::wake_floor` and `LooseWorld::wake_assembly`.
- `BlockBlockContact::take_connectors`; `LooseWorld::{transitions, allow_sleep, edge_counts, sleep_profile}`.
- The loose audits print `part_*_sleep` lines. part-drop and part-stand now require the Parts to end asleep.

**Why:** The user asked for the sleep stage, so resting Parts and Parts being stood on settle.

**Tested how:**
- Ran the three loose audits.
- Re-ran audit-walking, part-wall, baseplate, animate, player, locomotion, runtime, walking-render, camera, kernel and baseplate-render: identical to the previous build apart from evidence paths and timing.
- viewport-baseplate-smoke: exit 0, and the five demo Parts end asleep.
- 77 tests pass; clippy and fmt are clean.

**Result:**
- audit-part-drop: all four Parts sleep (Rest and Stack at world step 120, Tumble 271, Drop 303) with zero velocity. The slow creep of resting stacks (about 0.15 studs over the run) is gone.
- audit-part-stand:
  - the Pad sleeps before the character arrives, the landing wakes it (step 239), and it sleeps again at 367;
  - the Step does the same;
  - the Pad still sinks about 0.085 into the Baseplate. At wake it carries the landing step's 19 accumulated substep reactions, then it rests in realTimeBodies against contactConnectors until it sleeps.
- audit-part-push: the Crate is asleep when the character reaches it. The touch wakes it with the character's realtime connector first, so 00a9be30 (re-checked in Ghidra) files it under jointBodies. Its ground contact uses joint springs and it slides about 37 studs ahead of the character before tipping (it was 13 studs, tipping early).

**Still broken / not tested:**
- Native parity is not tested.
- The broadphase is not ported: every colliding pair is a contact for the whole run, in a fixed order, so edge-add and edge-remove wakes from grid changes do not happen.
- The native re-binding of live connectors when a Part enters or leaves the kernel is not recovered. The port deletes them and regenerates them on the next Contact::step.
- The awake-set iteration order (native: by pointer) is assumed: character first, then Parts by index.
- Environmental throttling is not ported.

**Next:** Floor friction in RunningBase (a43f4c / a43e10), the PopperCam ray against loose Parts, and audits for the jump floor reaction and the steep-floor rule.

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
