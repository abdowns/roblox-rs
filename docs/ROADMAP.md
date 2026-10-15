# Roadmap

The plan from here to running place files and Lua scripts as the original
client does. Each item is reverse engineered from the 0.255 binary and lands as
its own commit with an audit. Items move to [FEATURES.md](FEATURES.md) when
they work.

## Order

1. **Physics.** Everything the legacy solver does for Parts, joints and
   movers (below).
2. **Places.** A DataModel (the Instance tree with the reflection data from the
   binary) and the .rbxm/.rbxmx/.rbxl readers, so a file builds the World.
3. **Lua.** The 2016 Lua 5.1 VM with Roblox's changes, the object bridge,
   the value types and the script scheduler, until the shipped Animate,
   Camera and Control scripts run as scripts.

Terrain and mesh collisions (MeshPart, Unions) wait for step 2: their data
comes from place files and content, which need the loader first.

## The solver

A new place in 0.255 uses the legacy spring solver, the one already ported:
`PGSSolverDefaultOnNewPlaces` is registered off (00141e6e, the flag
registration at 001423f7 passes 0), so Workspace.PGSPhysicsSolverEnabled is
false for new documents offline. The PGS solver and the new Constraint classes
(HingeConstraint, RopeConstraint, ...) are only reached with that property on;
they come after the legacy items.

## The native pipeline

The World constructor (00ba6a0e) builds one chain of stages. Each stage
constructor creates the next one:

| Stage | Constructor | Port |
|---|---|---|
| CleanStage | 00ae11e6 | not ported |
| JointStage | 00b29a8c | not ported |
| GroundStage | 00b0ebc8 | joints.rs (ground joints) |
| EdgeStage | 00af7aa2 | not ported |
| ContactStage | 00af33f6 | not ported |
| TreeStage (SpanningTree) | 00ba2880 | joints.rs (rigid assemblies in LooseWorld) |
| MovingStage | 00b39e70 | not ported |
| SpatialFilter | 00b78fa8 | part of the broadphase wiring |
| MechToAssemblyStage | 00b2f0a2 | not ported |
| AssemblyStage | 00adbae0 | not ported |
| MovingAssemblyStage | 00b38a4a | not ported |
| StepJointsStage | 00b89fa0 | Motor6D only |
| HumanoidStage | 00b0ff5c | character only |
| SleepStage | 00b6f814 | ported (sleep.rs, loose.rs) |
| SimulateStage | 00b6eb12 | lists ported |
| Kernel | 00a95592 | ported (kernel.rs, loose.rs) |

The World also owns the ContactManager (+0x150, the broadphase), the ground
primitive (+0x5c) and the head of the chain (+0x158).

## Physics items

Done: Block geometry, BlockBlockContact and its connectors, the Kernel lists
and step, SleepStage, the broadphase, the R6 character and the Humanoid
states it uses (Running, Jumping, FreeFall, Landed, RunningNoPhysics).

Left, in order:

1. **Rigid assemblies.** Weld, Snap and ManualWeld joints, the spanning tree
   that turns them into Clumps/Assemblies/Mechanisms, and multi-Part bodies in
   the kernel. Done for joints present at load (`audit-part-weld`); joints
   made or broken while running are not.
2. **Kernel joints.** Glue/ManualGlue (breakable), Rotate, RotateP, RotateV
   (hinge and motor surfaces), Motor, DynamicRotate, VelocityMotor, and their
   connectors (JointConnector, RotateConnector, the break connectors).
3. **Shapes.** Ball (BallBall/BallBlock contacts), Cylinder, Wedge and
   CornerWedge (PolyContact), TrussPart, and the legacy ramp/prism/pyramid
   Parts.
4. **Body movers.** BodyPosition, BodyGyro, BodyVelocity,
   BodyAngularVelocity, BodyForce, BodyThrust, RocketPropulsion.
5. **Automatic joints.** JointsService and the surface pairs (studs/inlets,
   weld, glue, hinge, motor) that join touching Parts.
6. **Humanoid states.** Climbing, Seated, Swimming, PlatformStanding,
   FallingDown, Ragdoll, GettingUp, Physics, Dead.
7. **Explosions and touch events.** Explosion, Touched/TouchEnded.
8. **Throttling.** Environmental throttling and the throttled-step lists.

Open findings:

- **Fast impacts on a high center of mass.** A 4x4x2 block dropped flat at
  about 46 studs/s, with no welds, leaves its first contact with a sideways
  velocity of about 12 studs/s and a spin of about 7.7 rad/s, then falls on
  its side. A 1x4x1 block falling upright from 50 studs is thrown about 10
  studs sideways. The low 4x1.2x2 blocks of `audit-part-drop` land flat. This
  looks wrong for a symmetric drop, so the contact impulse path (a936d0 and
  the corner connectors) has to be re-checked against the binary. Not yet
  investigated.
- **Character contact velocity.** The character's springs read its SimBody's
  center-of-mass state. Natively a929f6 reads the Torso's and Head's Body PV.
  The results agree up to float rounding, but they are not bit-identical.

After step 2 (places): terrain (smooth terrain voxels, water buoyancy),
MeshPart and Union collisions (Bullet), the PGS solver and the Constraint
classes.
