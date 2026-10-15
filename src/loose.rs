//! Unanchored Parts in the legacy kernel: SimBody lists, contact connector
//! lists, the throttling-body half of Kernel::step, and SleepStage.
//!
//! Recovered from:
//! - body classification 00a9be30 and list removal 00a9c142 (ab1c28 when a
//!   body leaves free fall),
//! - connector insert 00a99514 and remove 00a99ff2,
//! - contact displacement 00a9457c / 00a942ee,
//! - Kernel::step 00a96588: pre-step 00a96eb0, then stepWorld 00a9768c,
//! - connector bodies a92c26 (the first body in the kernel is A; the normal is
//!   negated when only the second one is),
//! - the substep half of stepWorld: realTimeConnectors springs, then
//!   jointConnectors springs (a93074 after the +0x24 geometry recompute), then
//!   realTimeBodies and jointBodies integrate (ab1d44).
//!
//! The character's colliders take part as `Prim::Character`: the character
//! SimBody cannot throttle, so 00a99514 files every connector touching it
//! under realTimeConnectors.
//!
//! SleepStage (v8world/SleepStage.cpp, step 00b707f4) owns Contact::step:
//! - wakes: recursiveWakePending 00b70c68 (depth 8) and wakePending
//!   00b70d62 (depth 0), both through 00b71f08 -> 00b7228e;
//! - stepContacts 00b70e5c -> 00b715ac over the stepping lists (+0x110),
//!   then later over the touching lists (+0x130), each split by throttle
//!   type (character contacts cannot throttle, list 0);
//! - the edge state machine 00b72df0 (STEPPING 1, SLEEPING 2, CONTACTING 3
//!   in the kernel, CONTACTING_SLEEPING 4) with wakes when a contact starts
//!   or stops touching (recursive when fast, 00b7361a);
//! - awake assemblies that stay still go to sleep (00b70ed6, decision
//!   00b726ae) and SLEEPING_CHECKING ones are re-decided every 4th step
//!   (00b71158);
//! - setAssemblyState 00b727c0: sleeping zeroes the velocity (00ab1cc2) and
//!   leaves the kernel; waking zeroes velocity and impulses and restarts
//!   the history countdown; AWAKE enters the kernel.
//!
//! Kernel joints (`kernel_joint.rs`: Glue, Rotate, RotateP, RotateV) are
//! SleepStage joint edges: onEdgeAdded 00b73cfe makes them AWAKE at once, which puts them in
//! the kernel (00b7339e -> downstream onEdgeAdded); waking an assembly wakes
//! its SLEEPING joints (00b72b82); a joint always counts as touching
//! (00b72d00); at the end of each step a joint whose two assemblies are
//! neither awake nor waking goes to SLEEPING and leaves the kernel
//! (00b713bc); removing a SLEEPING joint wakes both sides (00b73f08).
//! World::uiStep's breakJoints (00ba9032) removes an in-kernel joint whose
//! connectors broke (World::onJointBroken 00baa94e). Then
//! MovingAssemblyStage::stepUi (00b39318) runs the motor hinges' stepUi
//! (00b6bd86), which wakes both assemblies recursively (00ba7fb0 ->
//! 00b737c4) while the motor value is not 0; that wake also sets SleepStage
//! +0x16c, so the next recursive pass walks through awake assemblies too
//! (00b7228e) until it ends (00b71f08 clears it). StepJointsStage::stepWorld
//! (00b8a402), between SleepStage::step and Kernel::step, gives each motor
//! hinge in the kernel its target or velocity.
//!
//! The contact pairs follow the broadphase (`broadphase.rs`,
//! SpatialHashMultiRes, kept by `World`): `contact_created` and
//! `contact_destroyed` mirror World::insertContact 00bab0f0 / destroyContact
//! 00bab1f6, with the primitives in the broadphase's order (searching
//! primitive first; 00ae714a keeps it for two blocks).
use crate::sleep::{self, AssemblyState, MotionHistory, SleepProfile};
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile},
    block_contact::{
        Block, BlockBlockContact, BlockTables, ContactEvent, ContactPoint, extents_overlap,
    },
    bundle::Bundle,
    collision::{ContactBodyState, point_velocity},
    g3d,
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile},
    kernel_joint::{
        self, BreakConnector, BreakTest, GLUE, JointBody, KernelPoint, ROTATE, ROTATE_P, ROTATE_V,
        RotateConnector, SurfaceInput,
    },
    legacy_contact::{AwakeContact, ContactGeometry, ImpulseInput, ImpulseProfile},
    material::{MaterialProfile, PhysicalProperties, Shape},
    motor::{self, MotorJoint},
    native_data::NativeImage,
    physics::FreeFallBody,
    scene::CoordinateFrame,
    workspace::PartInstance,
};
use std::collections::{BTreeMap, VecDeque};

/// KernelData body lists a loose SimBody can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyList {
    FreeFall,
    RealTime,
    Joint,
    Contact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectorList {
    RealTime,
    Joint,
    Contact,
}

/// A primitive taking part in a contact pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prim {
    Loose(usize),
    Static(usize),
    /// A character collider (index into the registered colliders).
    Character(usize),
}

/// The constants of stepWorld and its pre-step.
#[derive(Clone, Debug)]
struct StepProfile {
    /// a91f4e (0x01713e40): average impulse error goal.
    error_goal: f32,
    /// a91f6a (0x01700a30): |angular displacement|^2 resting bound.
    angular_rest: f32,
    /// a91f7c (0x017a5b84): |linear displacement| x/z resting bound.
    linear_rest: f32,
}
/// a91f44 and a91f60 return these immediates.
const MAX_ITERATIONS: u32 = 60;
const BODIES_PER_GOAL: usize = 400;

impl StepProfile {
    fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let result = Self {
            error_goal: float(0x01713e40)?,
            angular_rest: float(0x01700a30)?,
            linear_rest: float(0x017a5b84)?,
        };
        if [result.error_goal, result.angular_rest, result.linear_rest]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("unexpected native stepWorld profile".into());
        }
        Ok(result)
    }
}

/// An unanchored assembly: Parts joined by rigid joints sharing one SimBody
/// (the root Body's). A Part with no joints is its own assembly.
#[derive(Clone, Debug)]
pub struct LoosePart {
    /// The root Part's name.
    pub name: String,
    /// The root Body's SimBody: frame is the root Part's, center the
    /// assembly's center of mass.
    pub body: KernelBody,
    pub list: BodyList,
    /// The assembly's Parts (`LooseWorld::prims`), root first, then the Body
    /// tree breadth first.
    pub prims: Vec<usize>,
    /// SimBody +0x110 total, +0x11c realtime, +0x120 joint, +0x128 contact
    /// connector counts.
    total: u32,
    real_time: u32,
    joint: u32,
    contact: u32,
    /// The SimBody is in the kernel lists (its assembly is AWAKE).
    pub in_kernel: bool,
    /// Assembly +0x40.
    pub sleep: AssemblyState,
    history: MotionHistory,
    /// Assembly +0x74: the largest primitive radius (00ad853c).
    radius: f32,
    /// Assembly +0x68 / +0x6c: recursive wake pass id and depth.
    pass: u32,
    depth: i32,
}

/// An unanchored Part (Primitive) of a loose assembly.
#[derive(Clone, Debug)]
pub struct LoosePrim {
    pub name: String,
    pub size: [f32; 3],
    /// Its assembly in `LooseWorld::parts`.
    pub assembly: usize,
    /// The parent Body's Part and the rigid offset in it (coordInParent,
    /// 00a8fdca); None for the assembly's root.
    pub parent: Option<(usize, CoordinateFrame)>,
    /// The Part's CFrame when it was added (the root's starts the SimBody).
    frame: CoordinateFrame,
    can_collide: bool,
    properties: PhysicalProperties,
    stiffness: f32,
    /// RunningBase friction factor when the Part is a Humanoid floor.
    floor_friction: f32,
    /// Part.Transparency.
    transparency: f32,
}

impl LoosePrim {
    /// Part.CanCollide (Primitive +0xcc is its negation).
    pub fn can_collide(&self) -> bool {
        self.can_collide
    }
    /// The Part's material friction weighted against Plastic (00a43f4c).
    pub fn floor_friction(&self) -> f32 {
        self.floor_friction
    }
    pub fn transparency(&self) -> f32 {
        self.transparency
    }
}

/// A Body's PV (Body +0xb4 rotation, +0xd8 position, +0xe4 velocity,
/// +0xf0 angular velocity): the root's is published from the SimBody
/// (00ab13e8), a child's follows its parent (Body::updatePV 00a8eabe).
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPv {
    pub frame: CoordinateFrame,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}

impl BodyPv {
    /// The Body as a contact body (a929f6 reads the PV of each side).
    pub fn contact_state(&self) -> ContactBodyState {
        ContactBodyState {
            center: self.frame.position,
            velocity: self.velocity,
            angular_velocity: self.angular_velocity,
        }
    }
}

/// Per-connector kernel state.
#[derive(Clone, Debug)]
pub struct Link {
    /// 0 until the connector has been seen by the kernel lists.
    id: u64,
    list: Option<ConnectorList>,
    pub impulse: AwakeContact,
    pub spring: ContactSpring,
    /// Connector +0xc0: the geometry last applied to the displacements.
    applied: ContactPoint,
    /// The kernel loose bodies counted at insert.
    counted: Vec<usize>,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            id: 0,
            list: None,
            impulse: AwakeContact::default(),
            spring: ContactSpring::default(),
            applied: ContactPoint {
                normal: [0.0; 3],
                length: 0.0,
                position: [0.0; 3],
            },
            counted: Vec::new(),
        }
    }
}

/// Contact edge states (Edge +8) a SleepStage contact can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeState {
    Stepping = 1,
    Sleeping = 2,
    Contacting = 3,
    ContactingSleeping = 4,
}

/// SleepStage joint edge states (Edge +8): AWAKE puts the joint in the
/// kernel, SLEEPING takes it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointEdge {
    Awake = 1,
    Sleeping = 2,
}

/// A kernel joint (a GlueJoint or a RotateJoint) between two Bodies.
#[derive(Clone, Debug)]
pub struct KernelJoint {
    pub name: String,
    /// Joint::JointType: `kernel_joint::GLUE`, `ROTATE`, `ROTATE_P` or
    /// `ROTATE_V`.
    pub kind: u32,
    /// Part0's and Part1's Bodies.
    pub bodies: [JointBody; 2],
    /// C0 and C1.
    coords: [CoordinateFrame; 2],
    /// F0..F3 (GlueJoint +0xd0); zero for the hinges.
    face: [[f32; 3]; 4],
    /// DynamicRotateJoint +0xd8 (BaseAngle); 0 for the others.
    base_angle: f32,
    /// The axle's surface input (Part0's face that C0's z axis names), as
    /// loaded; the motor hinges read it every stepUi (00b6be18).
    axle: SurfaceInput,
    /// DynamicRotateJoint +0xe0: the motor value of the last stepUi.
    pub value: f32,
    /// None once the joint has left SleepStage (it broke).
    pub edge: Option<JointEdge>,
    /// Its break connectors while it is in the kernel (MultiJoint +0xc0).
    connectors: Vec<u64>,
    /// DynamicRotateJoint +0xdc: its RotateConnector while in the kernel.
    rotator: Option<u64>,
}

/// A MotorJoint linking two Parts of one assembly: the child Part's Body
/// offset follows its angle (RevoluteLink).
#[derive(Clone, Debug)]
struct LooseMotor {
    joint: MotorJoint,
    /// The child Part (`LooseWorld::prims`); its Body link names the parent.
    child: usize,
    /// The joint frames on the parent's and the child's side.
    coords: [CoordinateFrame; 2],
}

/// An anchored or grounded Part a kernel joint holds (it never moves).
#[derive(Clone, Debug)]
struct FixedBody {
    frame: CoordinateFrame,
    /// Primitive +0xb8, the joint stiffness.
    stiffness: f32,
    /// The Part's size, which the motor hinges' gain reads (00b6b53c);
    /// unknown for a Body added by `add_fixed_body`.
    size: Option<[f32; 3]>,
}

/// What `reset` restores of the kernel joints.
#[derive(Clone, Debug, Default)]
struct JointSnapshot {
    joints: Vec<KernelJoint>,
    points: Vec<KernelPoint>,
    connectors: BTreeMap<u64, BreakConnector>,
    rotators: BTreeMap<u64, RotateConnector>,
    joint_connectors: Vec<u64>,
}

#[derive(Clone, Debug)]
struct Pair {
    contact: BlockBlockContact<Link>,
    parameters: ContactParameters,
    /// None while the contact is not in SleepStage.
    edge: Option<EdgeState>,
    /// Edge +0xc: 0 CAN_NOT_THROTTLE (touches the character), 1 CAN_THROTTLE.
    throttle: usize,
}

/// An assembly SleepStage tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Node {
    Character,
    Loose(usize),
}

/// The character assembly's root primitive this step.
#[derive(Clone, Debug)]
pub struct CharacterMotion {
    pub frame: CoordinateFrame,
    pub radius: f32,
    /// Root primitive point velocity and angular velocity.
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}

/// The character assembly as SleepStage sees it.
#[derive(Clone, Debug)]
struct CharacterNode {
    in_stage: bool,
    state: AssemblyState,
    history: Option<MotionHistory>,
    motion: Option<CharacterMotion>,
    pass: u32,
    depth: i32,
}

impl Default for CharacterNode {
    fn default() -> Self {
        Self {
            in_stage: false,
            state: AssemblyState::Anchored,
            history: None,
            motion: None,
            pass: 0,
            depth: -1,
        }
    }
}

/// Last world step, for logs.
#[derive(Clone, Debug, Default)]
pub struct StepStats {
    pub iterations: u32,
    pub average_error: f32,
    pub contact_connectors: usize,
    pub joint_connectors: usize,
}

pub struct LooseWorld {
    /// The assemblies.
    pub parts: Vec<LoosePart>,
    /// The Parts, in the order they were added.
    pub prims: Vec<LoosePrim>,
    original: Vec<LoosePart>,
    /// Material defaults and stiffness of each static block.
    statics: Vec<(PhysicalProperties, f32)>,
    /// The static blocks as of the last contact update (for substep springs).
    static_blocks: Vec<Block>,
    /// Material defaults and stiffness of each character collider.
    colliders: Vec<(PhysicalProperties, f32)>,
    free_fall: Vec<usize>,
    real_time: Vec<usize>,
    joint: Vec<usize>,
    contact_bodies: Vec<usize>,
    pairs: BTreeMap<(Prim, Prim), Pair>,
    real_time_connectors: Vec<u64>,
    contact_connectors: Vec<u64>,
    joint_connectors: Vec<u64>,
    owners: BTreeMap<u64, (Prim, Prim)>,
    next_id: u64,
    profile: StepProfile,
    mass: MassProfile,
    pub stats: StepStats,
    /// SleepStage +0x110 stepping and +0x130 touching contacts by throttle.
    stepping: [Vec<(Prim, Prim)>; 2],
    touching: [Vec<(Prim, Prim)>; 2],
    character: CharacterNode,
    sleep_profile: SleepProfile,
    /// The global AllowSleep flag (01d3c2b4, default 1).
    pub allow_sleep: bool,
    /// SleepStage +0x168.
    pass: u32,
    /// Assembly state changes of the run: (step, node, new state).
    pub transitions: Vec<(u32, Node, AssemblyState)>,
    step_id: u32,
    /// The broadphase contacts in creation order.
    created: Vec<(Prim, Prim)>,
    /// SimulateStage::movingDynamicAssemblies (+0x2c): the loose Parts in
    /// the kernel, in the order they entered (00b6ee0a pushes at the back;
    /// leaving unlinks).
    simulating: Vec<usize>,
    /// The anchored (or grounded) Parts kernel joints hold.
    fixed: Vec<FixedBody>,
    /// The kernel joints, in creation order (the native awake-joint set
    /// +0x150 is ordered by address; creation order stands in).
    pub joints: Vec<KernelJoint>,
    /// KernelData +0x50: the kernel Points.
    points: Vec<KernelPoint>,
    /// The break connectors in the kernel, by connector id.
    breakers: BTreeMap<u64, BreakConnector>,
    /// The RotateConnectors in the kernel, by connector id.
    rotators: BTreeMap<u64, RotateConnector>,
    /// SleepStage +0x16c: set by a recursive wake from stepUi (00b737c4);
    /// the next recursive pass then walks through awake assemblies as well.
    wake_through_awake: bool,
    /// The MotorJoints inside assemblies, in creation order, and as added.
    motors: Vec<LooseMotor>,
    original_motors: Vec<LooseMotor>,
    original_joints: JointSnapshot,
    /// Joints that broke: (step, joint).
    pub broken: Vec<(u32, usize)>,
}

/// IndexArray::remove: the last element moves into the hole.
fn remove_item<T: PartialEq>(list: &mut Vec<T>, item: &T) -> Result<(), String> {
    let at = list
        .iter()
        .position(|x| x == item)
        .ok_or("kernel list entry missing")?;
    list.swap_remove(at);
    Ok(())
}

/// Two distinct parts, mutably.
fn two_mut(
    parts: &mut [LoosePart],
    a: usize,
    b: Option<usize>,
) -> Result<(&mut LoosePart, Option<&mut LoosePart>), String> {
    match b {
        None => Ok((parts.get_mut(a).ok_or("missing loose part")?, None)),
        Some(b) if a == b => Err("contact pair with itself".into()),
        Some(b) if a < b => {
            let (low, high) = parts.split_at_mut(b);
            Ok((&mut low[a], Some(&mut high[0])))
        }
        Some(b) => {
            let (low, high) = parts.split_at_mut(a);
            Ok((&mut high[0], Some(&mut low[b])))
        }
    }
}

/// a942ee: move `sign` times the contact's displacement share into the
/// bodies' angular (+0x130) and linear (+0x13c) displacement.
fn displace(a: &mut KernelBody, b: Option<&mut KernelBody>, g: &ContactGeometry, sign: f32) {
    let ia = a.inverse_mass();
    let sum = ia + b.as_ref().map_or(0.0, |b| b.inverse_mass());
    let [nx, ny, nz] = g.normal;
    let len = g.distance;
    let p = g.point;
    let share = ia / sum;
    let [dx, dy, dz] = [nx * len * share, ny * len * share, share * len * nz];
    if 1e-12 < dz * dz + (dy * dy + dx * dx) {
        let [dx, dy, dz] = [dx * sign, dy * sign, dz * sign];
        a.linear_displacement = [
            a.linear_displacement[0] + dx,
            a.linear_displacement[1] + dy,
            a.linear_displacement[2] + dz,
        ];
        let [rx, ry, rz] = std::array::from_fn(|i| p[i] - a.center[i]);
        a.angular_displacement = [
            (dz * ry - dy * rz) + a.angular_displacement[0],
            (rz * dx - dz * rx) + a.angular_displacement[1],
            (rx * dy - ry * dx) + a.angular_displacement[2],
        ];
    }
    if let Some(b) = b {
        let share = b.inverse_mass() / sum;
        let [dx, dy, dz] = [nx * len * share, ny * len * share, len * nz * share];
        if 1e-12 < dz * dz + (dy * dy + dx * dx) {
            let [dx, dy, dz] = [dx * sign, dy * sign, dz * sign];
            b.linear_displacement = [
                b.linear_displacement[0] - dx,
                b.linear_displacement[1] - dy,
                b.linear_displacement[2] - dz,
            ];
            let [rx, ry, rz] = std::array::from_fn(|i| p[i] - b.center[i]);
            b.angular_displacement = [
                (rz * dy - ry * dz) + b.angular_displacement[0],
                (dz * rx - rz * dx) + b.angular_displacement[1],
                (ry * dx - rx * dy) + b.angular_displacement[2],
            ];
        }
    }
}

/// The geometry from the connector's first primitive into the second.
fn geometry(point: &ContactPoint, flip: bool) -> ContactGeometry {
    ContactGeometry {
        normal: point.normal.map(|v| if flip { -v } else { v }),
        distance: point.length,
        point: point.position,
    }
}

impl LooseWorld {
    pub fn new(bundle: &Bundle) -> Result<Self, String> {
        Ok(Self {
            parts: Vec::new(),
            prims: Vec::new(),
            original: Vec::new(),
            statics: Vec::new(),
            static_blocks: Vec::new(),
            colliders: Vec::new(),
            free_fall: Vec::new(),
            real_time: Vec::new(),
            joint: Vec::new(),
            contact_bodies: Vec::new(),
            pairs: BTreeMap::new(),
            real_time_connectors: Vec::new(),
            contact_connectors: Vec::new(),
            joint_connectors: Vec::new(),
            owners: BTreeMap::new(),
            next_id: 0,
            profile: StepProfile::load(bundle)?,
            mass: MassProfile::load(bundle)?,
            stats: StepStats::default(),
            stepping: [Vec::new(), Vec::new()],
            touching: [Vec::new(), Vec::new()],
            character: CharacterNode::default(),
            sleep_profile: SleepProfile::load(bundle)?,
            allow_sleep: true,
            pass: 0,
            transitions: Vec::new(),
            step_id: 0,
            simulating: Vec::new(),
            created: Vec::new(),
            fixed: Vec::new(),
            joints: Vec::new(),
            points: Vec::new(),
            breakers: BTreeMap::new(),
            rotators: BTreeMap::new(),
            wake_through_awake: false,
            motors: Vec::new(),
            original_motors: Vec::new(),
            original_joints: JointSnapshot::default(),
            broken: Vec::new(),
        })
    }
    /// A static (anchored, colliding) block, in Statics order.
    pub fn add_static(&mut self, properties: PhysicalProperties, stiffness: f32) {
        self.statics.push((properties, stiffness));
    }
    /// A static block's Body mass (Primitive::setMass 00b63366 sets Body +0x9c
    /// whether or not the Part is anchored): size product times density, in
    /// the `add_part` order.
    pub fn static_mass(&self, index: usize, size: [f32; 3]) -> Result<f32, String> {
        let (properties, _) = self.statics.get(index).ok_or("missing static block")?;
        Ok((size[0] * size[1]) * size[2] * properties.density)
    }
    /// A character collider, in the order its blocks are passed later.
    pub fn add_collider(&mut self, properties: PhysicalProperties, stiffness: f32) {
        self.colliders.push((properties, stiffness));
    }
    /// Add an unanchored Part at rest as its own assembly (`add_prim`, then
    /// `add_assembly` with no links).
    pub fn add_part(
        &mut self,
        part: &PartInstance,
        materials: &MaterialProfile,
        kernel: &KernelProfile,
    ) -> Result<(), String> {
        let prim = self.add_prim(part, materials)?;
        self.add_assembly(prim, &[], kernel)?;
        Ok(())
    }
    /// Add an unanchored Part, not yet in an assembly. Returns its index in
    /// `prims`.
    pub fn add_prim(
        &mut self,
        part: &PartInstance,
        materials: &MaterialProfile,
    ) -> Result<usize, String> {
        let properties = materials.defaults(part.material)?;
        self.prims.push(LoosePrim {
            name: part.name.clone(),
            size: part.size,
            assembly: usize::MAX,
            parent: None,
            frame: part.frame.clone(),
            can_collide: part.can_collide,
            properties,
            stiffness: materials.stiffness(part.size, Shape::Block)?,
            floor_friction: materials.floor_friction(&properties)?,
            transparency: part.transparency,
        });
        Ok(self.prims.len() - 1)
    }
    /// Join added Parts into an assembly at rest: `root` keeps its CFrame and
    /// each link (child, parent, offset) makes the child's Body a child of
    /// the parent's, parents before children and each Body's children in
    /// IndexArray order (TreeStage, `joints.rs`). The child Parts move to
    /// parent x offset (Body::updatePV 00a8eabe). The SimBody takes the
    /// root Body's branch mass, center and inertia (Cofm 00a90502 over the
    /// Body tree) and the assembly radius is the largest primitive radius
    /// about the root (00ad853c). SleepStage::onAssemblyAdded 00b73820 makes
    /// the assembly WAKE_PENDING with a fresh history; the first wakePending
    /// pass enters it into the kernel, where 00a9be30 files it under
    /// freeFallBodies (no connectors yet). Returns its index in `parts`.
    pub fn add_assembly(
        &mut self,
        root: usize,
        links: &[(usize, usize, CoordinateFrame)],
        kernel: &KernelProfile,
    ) -> Result<usize, String> {
        let index = self.parts.len();
        let mut members = vec![root];
        members.extend(links.iter().map(|l| l.0));
        for &m in &members {
            let prim = self.prims.get(m).ok_or("missing assembly Part")?;
            if prim.assembly != usize::MAX {
                return Err("Part already in an assembly".into());
            }
        }
        if (1..members.len()).any(|i| members[..i].contains(&members[i])) {
            return Err("Part linked twice".into());
        }
        for (child, parent, offset) in links {
            if !members.contains(parent)
                || members.iter().position(|m| m == parent)
                    >= members.iter().position(|m| m == child)
            {
                return Err("Body link before its parent".into());
            }
            self.prims[*child].parent = Some((*parent, offset.clone()));
        }
        for &m in &members {
            self.prims[m].assembly = index;
        }
        let root_frame = self.prims[root].frame.clone();
        let (assembly, frames) = self.tree_properties(root, links, &root_frame)?;
        let state = FreeFallBody::new(root_frame.clone(), [0.0; 3], [0.0; 3])?;
        let radius = members
            .iter()
            .map(|m| {
                sleep::primitive_radius(
                    frames[m].position,
                    root_frame.position,
                    self.prims[*m].size,
                )
            })
            .fold(0.0f32, f32::max);
        self.parts.push(LoosePart {
            name: self.prims[root].name.clone(),
            body: KernelBody::new(&assembly, &state, kernel)?,
            list: BodyList::FreeFall,
            prims: members,
            total: 0,
            real_time: 0,
            joint: 0,
            contact: 0,
            in_kernel: false,
            sleep: AssemblyState::WakePending,
            history: MotionHistory::new(sleep::sample(&root_frame, radius)),
            radius,
            pass: 0,
            depth: -1,
        });
        self.original = self.parts.clone();
        Ok(index)
    }
    /// The Body tree's mass properties (Cofm 00a90502), each child at parent
    /// x offset from `root_frame`, and those frames.
    fn tree_properties(
        &self,
        root: usize,
        links: &[(usize, usize, CoordinateFrame)],
        root_frame: &CoordinateFrame,
    ) -> Result<(AssemblyProperties, BTreeMap<usize, CoordinateFrame>), String> {
        let mut frames = BTreeMap::new();
        frames.insert(root, root_frame.clone());
        for (child, parent, offset) in links {
            let frame = g3d::frame_multiply(&frames[parent], offset);
            frames.insert(*child, frame);
        }
        fn branch(
            world: &LooseWorld,
            n: usize,
            links: &[(usize, usize, CoordinateFrame)],
            frames: &BTreeMap<usize, CoordinateFrame>,
        ) -> Result<AssemblyProperties, String> {
            let prim = &world.prims[n];
            let size = prim.size;
            let mass = (size[0] * size[1]) * size[2] * prim.properties.density;
            let children = links
                .iter()
                .filter(|l| l.1 == n)
                .map(|l| branch(world, l.0, links, frames))
                .collect::<Result<Vec<_>, _>>()?;
            AssemblyProperties::combine(
                &MassPart {
                    name: prim.name.clone(),
                    frame: frames[&n].clone(),
                    mass,
                    inertia: world.mass.block_inertia(size, mass)?,
                },
                &children,
            )
        }
        Ok((branch(self, root, links, &frames)?, frames))
    }
    /// A MotorJoint (`motor.rs`) on a Body link `add_assembly` made: `parent`
    /// and `child` are the link's Parts and `coords` the joint frames on
    /// their sides (C0 and C1 when Part0 is the parent, else C1 and C0). The
    /// link's offset must be `motor::revolute_offset` at the motor's
    /// CurrentAngle. Returns its index.
    pub fn add_motor(
        &mut self,
        joint: MotorJoint,
        parent: usize,
        child: usize,
        coords: [CoordinateFrame; 2],
    ) -> Result<usize, String> {
        let link = self.prims.get(child).and_then(|p| p.parent.as_ref());
        let offset = motor::revolute_offset(&coords[0], &coords[1], joint.current_angle);
        match link {
            Some((p, o)) if *p == parent && *o == offset => {}
            _ => return Err(format!("motor {:?}: no matching Body link", joint.name)),
        }
        self.motors.push(LooseMotor {
            joint,
            child,
            coords,
        });
        self.original_motors = self.motors.clone();
        Ok(self.motors.len() - 1)
    }
    /// MovingAssemblyStage::stepUi for the MotorJoints (00b381f6), once per
    /// frame after breakJoints: a motor whose angle changed re-poses its
    /// child Body (RevoluteLink, 00b38100) and wakes its assembly
    /// recursively (00b3831c, once for each Part). Each assembly with a
    /// moved link then rebuilds its SimBody from the Body tree at the root's
    /// frame, keeping its state and accumulators (the dirty SimBody update
    /// 00ab16aa, which natively waits for the next SimBody access).
    /// Returns how many motors moved.
    pub fn step_motors(&mut self, kernel: &KernelProfile) -> Result<usize, String> {
        let mut moved = Vec::new();
        let mut count = 0;
        for m in 0..self.motors.len() {
            let Some(angle) = self.motors[m].joint.step_ui() else {
                continue;
            };
            count += 1;
            let motor = &self.motors[m];
            let offset = motor::revolute_offset(&motor.coords[0], &motor.coords[1], angle);
            let child = motor.child;
            self.prims[child]
                .parent
                .as_mut()
                .ok_or("motor child without a parent Body")?
                .1 = offset;
            let a = self.prims[child].assembly;
            self.wake_assembly_recursive(a)?;
            self.wake_assembly_recursive(a)?;
            if !moved.contains(&a) {
                moved.push(a);
            }
        }
        for a in moved {
            let members = self.parts[a].prims.clone();
            let links = members[1..]
                .iter()
                .map(|&m| {
                    let (parent, offset) = self.prims[m]
                        .parent
                        .clone()
                        .ok_or("assembly Part without a Body link")?;
                    Ok((m, parent, offset))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let root_frame = self.parts[a].body.frame.clone();
            let (properties, _) = self.tree_properties(members[0], &links, &root_frame)?;
            self.parts[a].body.refresh_assembly(&properties, kernel)?;
        }
        Ok(count)
    }
    /// A MotorJoint's state.
    pub fn motor(&self, m: usize) -> Option<&MotorJoint> {
        self.motors.get(m).map(|l| &l.joint)
    }
    /// A Part's Body PV: the root's from its SimBody (00ab13e8), a child's
    /// from its parent's (Body::updatePV 00a8eabe): parent x offset, and the
    /// parent's velocity at the child's origin.
    pub fn prim_pv(&self, prim: usize) -> Result<BodyPv, String> {
        let p = self.prims.get(prim).ok_or("missing loose Part")?;
        let assembly = self
            .parts
            .get(p.assembly)
            .ok_or("Part outside an assembly")?;
        match &p.parent {
            None => {
                let state = assembly.body.body_state();
                Ok(BodyPv {
                    frame: state.frame,
                    velocity: state.velocity,
                    angular_velocity: state.angular_velocity,
                })
            }
            Some((parent, offset)) => {
                let parent = self.prim_pv(*parent)?;
                let frame = g3d::frame_multiply(&parent.frame, offset);
                let velocity = point_velocity(&parent.contact_state(), frame.position);
                Ok(BodyPv {
                    frame,
                    velocity,
                    angular_velocity: parent.angular_velocity,
                })
            }
        }
    }
    /// A Part's CFrame (its Body's PV position).
    pub fn prim_frame(&self, prim: usize) -> Result<CoordinateFrame, String> {
        let p = self.prims.get(prim).ok_or("missing loose Part")?;
        match &p.parent {
            None => Ok(self
                .parts
                .get(p.assembly)
                .ok_or("Part outside an assembly")?
                .body
                .frame
                .clone()),
            Some((parent, offset)) => Ok(g3d::frame_multiply(&self.prim_frame(*parent)?, offset)),
        }
    }
    /// An anchored or grounded Part a kernel joint holds (it never moves):
    /// its CFrame and joint stiffness (Primitive +0xb8, 00b63194). Returns
    /// its index for `JointBody::Fixed`.
    pub fn add_fixed_body(&mut self, frame: CoordinateFrame, stiffness: f32) -> usize {
        self.fixed.push(FixedBody {
            frame,
            stiffness,
            size: None,
        });
        self.fixed.len() - 1
    }
    /// `add_fixed_body` with the Part's size, which a motor hinge on it
    /// needs.
    pub fn add_fixed_part(
        &mut self,
        frame: CoordinateFrame,
        size: [f32; 3],
        stiffness: f32,
    ) -> usize {
        self.fixed.push(FixedBody {
            frame,
            stiffness,
            size: Some(size),
        });
        self.fixed.len() - 1
    }
    /// World::insertJoint for a Glue whose Parts are already in their
    /// assemblies (or fixed): SleepStage::onEdgeAdded 00b73cfe files the
    /// joint SLEEPING, then makes it AWAKE (00b7339e), which puts it in the
    /// kernel at once. Returns its index in `joints`.
    pub fn add_glue(
        &mut self,
        name: &str,
        bodies: [JointBody; 2],
        coords: [CoordinateFrame; 2],
        face: [[f32; 3]; 4],
    ) -> Result<usize, String> {
        self.add_joint(KernelJoint {
            name: name.into(),
            kind: GLUE,
            bodies,
            coords,
            face,
            base_angle: 0.0,
            axle: SurfaceInput::default(),
            value: 0.0,
            edge: Some(JointEdge::Sleeping),
            connectors: Vec::new(),
            rotator: None,
        })
    }
    /// `add_glue` for a hinge: `kind` is `ROTATE`, `ROTATE_P` or `ROTATE_V`,
    /// `base_angle` the joint's BaseAngle and `axle` Part0's surface input
    /// on the face C0's z axis names (the motor hinges read it).
    pub fn add_hinge(
        &mut self,
        kind: u32,
        name: &str,
        bodies: [JointBody; 2],
        coords: [CoordinateFrame; 2],
        base_angle: f32,
        axle: SurfaceInput,
    ) -> Result<usize, String> {
        if ![ROTATE, ROTATE_P, ROTATE_V].contains(&kind) {
            return Err(format!("{name:?}: joint type {kind} is not a hinge"));
        }
        self.add_joint(KernelJoint {
            name: name.into(),
            kind,
            bodies,
            coords,
            face: [[0.0; 3]; 4],
            base_angle,
            axle,
            value: 0.0,
            edge: Some(JointEdge::Sleeping),
            connectors: Vec::new(),
            rotator: None,
        })
    }
    /// SleepStage::onEdgeAdded for a new kernel joint, which is then kept as
    /// the state `reset` returns to.
    fn add_joint(&mut self, joint: KernelJoint) -> Result<usize, String> {
        let name = &joint.name;
        for body in joint.bodies {
            let ok = match body {
                JointBody::Loose(i) => self.prims.get(i).is_some_and(|p| p.assembly != usize::MAX),
                JointBody::Fixed(k) => k < self.fixed.len(),
            };
            if !ok {
                return Err(format!("joint {name:?}: Part outside the kernel bodies"));
            }
        }
        if matches!(joint.bodies, [JointBody::Fixed(_), JointBody::Fixed(_)]) {
            return Err(format!("joint {name:?}: two anchored Parts is not ported"));
        }
        let j = self.joints.len();
        self.joints.push(joint);
        self.set_joint_edge(j, JointEdge::Awake)?;
        self.original = self.parts.clone();
        self.original_joints = JointSnapshot {
            joints: self.joints.clone(),
            points: self.points.clone(),
            connectors: self.breakers.clone(),
            rotators: self.rotators.clone(),
            joint_connectors: self.joint_connectors.clone(),
        };
        Ok(j)
    }
    /// A kernel joint Body's PV frame.
    fn body_frame(&self, body: JointBody) -> Result<CoordinateFrame, String> {
        match body {
            JointBody::Loose(i) => self.prim_frame(i),
            JointBody::Fixed(k) => Ok(self.fixed.get(k).ok_or("missing fixed Body")?.frame.clone()),
        }
    }
    /// The Body's joint stiffness (Primitive::getJointK 00b63194).
    fn joint_k(&self, body: JointBody) -> f32 {
        match body {
            JointBody::Loose(i) => self.prims[i].stiffness,
            JointBody::Fixed(k) => self.fixed[k].stiffness,
        }
    }
    /// The Body's Part size.
    fn body_size(&self, body: JointBody) -> Result<[f32; 3], String> {
        match body {
            JointBody::Loose(i) => Ok(self.prims[i].size),
            JointBody::Fixed(k) => self.fixed[k]
                .size
                .ok_or_else(|| "fixed Body added without its size".into()),
        }
    }
    /// The Body's assembly in SleepStage (none for a fixed one).
    fn joint_node(&self, body: JointBody) -> Option<Node> {
        match body {
            JointBody::Loose(i) => Some(Node::Loose(self.prims[i].assembly)),
            JointBody::Fixed(_) => None,
        }
    }
    /// GlueJoint::putInKernel 00b0d632 (legacy solver): per face corner, a
    /// Point on Part0's Body (the face through C0) and one on Part1's (the
    /// face through C1), and a NormalBreakConnector between them with the
    /// smaller joint stiffness (00b3ed8a), 20 x the face strength
    /// (00b0d5ea) and C0's z axis as the normal (006b79ae).
    fn put_in_kernel(&mut self, j: usize) -> Result<(), String> {
        if self.joints[j].kind != GLUE {
            return self.put_hinge_in_kernel(j);
        }
        let joint = &self.joints[j];
        let bodies = joint.bodies;
        let [c0, c1] = &joint.coords;
        let normal =
            kernel_joint::normal_id([c0.rotation[0][2], c0.rotation[1][2], c0.rotation[2][2]]);
        let face0 = kernel_joint::face_through(&joint.face, c0);
        let face1 = kernel_joint::face_through(&joint.face, c1);
        let threshold = kernel_joint::glue_threshold(&joint.face);
        let (k0, k1) = (self.joint_k(bodies[0]), self.joint_k(bodies[1]));
        let k = if k0 <= k1 { k0 } else { k1 };
        let mut ids = Vec::with_capacity(4);
        for i in 0..4 {
            let p0 = kernel_joint::new_point(&mut self.points, bodies[0], face0[i]);
            let p1 = kernel_joint::new_point(&mut self.points, bodies[1], face1[i]);
            self.next_id += 1;
            let id = self.next_id;
            self.breakers.insert(
                id,
                BreakConnector {
                    points: [p0, p1],
                    k,
                    threshold,
                    test: BreakTest::Normal(normal),
                    broken: false,
                },
            );
            self.insert_joint_connector(id, bodies)?;
            ids.push(id);
        }
        self.joints[j].connectors = ids;
        Ok(())
    }
    /// RotateJoint::putInKernel 00b6b640 (legacy solver): per hinge end, a
    /// Point on each Body (`kernel_joint::hinge_points` through C0 and C1)
    /// and a PointToPointBreakConnector between them with the smaller joint
    /// stiffness that never breaks. RotateP and RotateV then add their
    /// RotateConnector (DynamicRotateJoint::putInKernel 00b6bb04).
    fn put_hinge_in_kernel(&mut self, j: usize) -> Result<(), String> {
        let joint = &self.joints[j];
        let (kind, bodies, base) = (joint.kind, joint.bodies, joint.base_angle);
        let coords = joint.coords.clone();
        let ends0 = kernel_joint::hinge_points(&coords[0]);
        let ends1 = kernel_joint::hinge_points(&coords[1]);
        let (k0, k1) = (self.joint_k(bodies[0]), self.joint_k(bodies[1]));
        let k = if k0 <= k1 { k0 } else { k1 };
        let mut ids = Vec::with_capacity(2);
        for end in 0..2 {
            let p0 = kernel_joint::new_point(&mut self.points, bodies[0], ends0[end]);
            let p1 = kernel_joint::new_point(&mut self.points, bodies[1], ends1[end]);
            self.next_id += 1;
            let id = self.next_id;
            self.breakers.insert(
                id,
                BreakConnector {
                    points: [p0, p1],
                    k,
                    threshold: f32::INFINITY,
                    test: BreakTest::PointToPoint,
                    broken: false,
                },
            );
            self.insert_joint_connector(id, bodies)?;
            ids.push(id);
        }
        self.joints[j].connectors = ids;
        if kind == ROTATE {
            return Ok(());
        }
        // 00b6b53c: each Part's larger size across its joint face (Part1's
        // face is the opposite of C1's z axis, 006b6bbc), the smaller of the
        // two, times 0.1.
        let axis = |c: &CoordinateFrame| {
            kernel_joint::normal_id([c.rotation[0][2], c.rotation[1][2], c.rotation[2][2]])
        };
        let n0 = axis(&coords[0]);
        let n1 = (axis(&coords[1]) + 3) % 6;
        let s0 = kernel_joint::cross_size(self.body_size(bodies[0])?, n0);
        let s1 = kernel_joint::cross_size(self.body_size(bodies[1])?, n1);
        let size = if s0 <= s1 { s0 } else { s1 };
        let r0 = self.body_frame(bodies[0])?.rotation;
        let r1 = self.body_frame(bodies[1])?.rotation;
        let rotator = RotateConnector::new(bodies, coords, base, k, size * 0.1, [&r0, &r1]);
        self.next_id += 1;
        let id = self.next_id;
        self.rotators.insert(id, rotator);
        self.insert_joint_connector(id, bodies)?;
        self.joints[j].rotator = Some(id);
        Ok(())
    }
    /// MultiJoint::removeFromKernel 00b3ee8e: each connector leaves the
    /// kernel, then its two Points are released. A motor hinge's
    /// RotateConnector goes first (00b6bc74).
    fn remove_from_kernel(&mut self, j: usize) -> Result<(), String> {
        let bodies = self.joints[j].bodies;
        if let Some(id) = self.joints[j].rotator.take() {
            self.remove_joint_connector(id, bodies)?;
            self.rotators
                .remove(&id)
                .ok_or("missing rotate connector")?;
        }
        let ids = std::mem::take(&mut self.joints[j].connectors);
        for id in ids {
            self.remove_joint_connector(id, bodies)?;
            let c = self.breakers.remove(&id).ok_or("missing break connector")?;
            let [p0, mut p1] = c.points;
            if let Some((from, to)) = self.release_point(p0)
                && p1 == from
            {
                p1 = to;
            }
            self.release_point(p1);
        }
        Ok(())
    }
    /// releasePoint, keeping the connectors on the Point that moved.
    fn release_point(&mut self, index: usize) -> Option<(usize, usize)> {
        let moved = kernel_joint::release_point(&mut self.points, index);
        if let Some((from, to)) = moved {
            for c in self.breakers.values_mut() {
                for p in c.points.iter_mut().filter(|p| **p == from) {
                    *p = to;
                }
            }
        }
        moved
    }
    /// Connector insert 00a99514 for a break connector (type 1, Kernel +0x28
    /// off): jointConnectors whether or not a body is in the kernel; the
    /// joint and total counts go up on each side's SimBody (a sleeping one
    /// too), and the bodies in the kernel are reclassified.
    fn insert_joint_connector(&mut self, id: u64, bodies: [JointBody; 2]) -> Result<(), String> {
        self.joint_connectors.push(id);
        let loose: Vec<usize> = bodies
            .iter()
            .filter_map(|b| match b {
                JointBody::Loose(i) => Some(self.prims[*i].assembly),
                JointBody::Fixed(_) => None,
            })
            .collect();
        for &a in &loose {
            let p = &mut self.parts[a];
            p.joint += 1;
            p.total += 1;
        }
        for a in loose {
            self.classify(a)?;
        }
        Ok(())
    }
    /// Connector removal 00a99ff2 from jointConnectors.
    fn remove_joint_connector(&mut self, id: u64, bodies: [JointBody; 2]) -> Result<(), String> {
        remove_item(&mut self.joint_connectors, &id)?;
        let loose: Vec<usize> = bodies
            .iter()
            .filter_map(|b| match b {
                JointBody::Loose(i) => Some(self.prims[*i].assembly),
                JointBody::Fixed(_) => None,
            })
            .collect();
        for &a in &loose {
            let p = &mut self.parts[a];
            p.joint -= 1;
            p.total -= 1;
        }
        for a in loose {
            self.classify(a)?;
        }
        Ok(())
    }
    /// SleepStage joint edge change 00b7339e: becoming AWAKE puts the joint
    /// in the kernel, leaving AWAKE takes it out.
    fn set_joint_edge(&mut self, j: usize, new: JointEdge) -> Result<(), String> {
        let current = self.joints[j].edge.ok_or("joint outside SleepStage")?;
        if current == new {
            return Err("joint edge state unchanged".into());
        }
        match new {
            JointEdge::Awake => self.put_in_kernel(j)?,
            JointEdge::Sleeping => self.remove_from_kernel(j)?,
        }
        self.joints[j].edge = Some(new);
        Ok(())
    }
    /// World::uiStep's breakJoints 00ba9032: every joint in the kernel whose
    /// connectors broke (GlueJoint::isBroken 00b0d9ae -> MultiJoint 00b3f0a4:
    /// any of its 4 breaking connectors) is removed (World::onJointBroken
    /// 00baa94e; the JointInstance leaves the DataModel, assumed at once).
    /// Returns how many broke.
    pub fn break_joints(&mut self) -> Result<usize, String> {
        let mut count = 0;
        for j in 0..self.joints.len() {
            // Only Glue is breakable (the Rotate family's isBreakable
            // 00be5ee0 returns 0).
            if self.joints[j].edge != Some(JointEdge::Awake) || self.joints[j].kind != GLUE {
                continue;
            }
            let broken = self.joints[j]
                .connectors
                .iter()
                .take(4)
                .any(|id| self.breakers.get(id).is_some_and(|c| c.broken));
            if broken {
                self.remove_joint(j)?;
                self.broken.push((self.step_id, j));
                count += 1;
            }
        }
        Ok(count)
    }
    /// World::removeJoint as SleepStage::onEdgeRemoving 00b73f08 sees it: an
    /// AWAKE joint leaves the kernel, a SLEEPING one wakes both assemblies.
    fn remove_joint(&mut self, j: usize) -> Result<(), String> {
        match self.joints[j].edge {
            Some(JointEdge::Awake) => self.set_joint_edge(j, JointEdge::Sleeping)?,
            Some(JointEdge::Sleeping) => {
                for body in self.joints[j].bodies {
                    if let Some(node) = self.joint_node(body) {
                        self.wake(node)?;
                    }
                }
            }
            None => return Err("joint already removed".into()),
        }
        self.joints[j].edge = None;
        Ok(())
    }
    /// MovingAssemblyStage::stepUi 00b39318 for the motor hinges
    /// (DynamicRotateJoint::stepUi 00b6bd86): the motor value is the axle's
    /// surface input at `time` (Workspace.DistributedGameTime); while it is
    /// not 0, both assemblies wake recursively (World::wakeAssembly
    /// 00ba7fb0). Which joints the stage lists is not recovered: every motor
    /// hinge still in SleepStage steps, asleep or not.
    pub fn step_ui(&mut self, time: f64) -> Result<(), String> {
        for j in 0..self.joints.len() {
            let joint = &mut self.joints[j];
            if joint.edge.is_none() || !matches!(joint.kind, ROTATE_P | ROTATE_V) {
                continue;
            }
            joint.value = joint.axle.value(time);
            // NaN wakes too.
            if joint.value != 0.0 {
                for body in joint.bodies {
                    if let Some(Node::Loose(a)) = self.joint_node(body) {
                        self.wake_assembly_recursive(a)?;
                    }
                }
            }
        }
        Ok(())
    }
    /// StepJointsStage::stepWorld 00b8a402 for the motor hinges in the
    /// kernel, between SleepStage::step and Kernel::step: RotateP sets its
    /// RotateConnector's target (00b6bf20 -> 00a9159a), RotateV its velocity
    /// (00b6c11e -> 00a916cc), from the value of the last stepUi.
    pub fn step_joints(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        for joint in &self.joints {
            let Some(id) = joint.rotator else {
                continue;
            };
            let r = self
                .rotators
                .get_mut(&id)
                .ok_or("missing rotate connector")?;
            match joint.kind {
                ROTATE_P => r.set_target(joint.value, kernel.physics.kernel_substeps),
                ROTATE_V => r.set_velocity(joint.value, kernel.physics.kernel_dt),
                _ => return Err("rotate connector on a joint that is not a motor".into()),
            }
        }
        Ok(())
    }
    /// Point::step 00aa3156 for every kernel Point (the first thing in each
    /// stepWorld substep).
    pub fn step_points(&mut self) -> Result<(), String> {
        for at in 0..self.points.len() {
            let frame = self.body_frame(self.points[at].body)?;
            self.points[at].step(&frame);
        }
        Ok(())
    }
    /// Point::forceToBody 00aa3244 for every kernel Point, after the
    /// jointConnectors: the force and its moment about the center of mass
    /// go to the assembly's SimBody, in the kernel or not.
    pub fn points_to_bodies(&mut self) -> Result<(), String> {
        for at in 0..self.points.len() {
            let JointBody::Loose(prim) = self.points[at].body else {
                continue;
            };
            let (force, world) = (self.points[at].force, self.points[at].world);
            let assembly = self.prims[prim].assembly;
            self.parts[assembly].body.apply_force(force, world)?;
        }
        Ok(())
    }
    /// A motor hinge's RotateConnector angles, (current, desired) less the
    /// BaseAngle, while it is in the kernel.
    pub fn hinge_angle(&self, j: usize) -> Option<(f32, f32)> {
        let r = self.rotators.get(&self.joints.get(j)?.rotator?)?;
        Some((r.current, r.desired))
    }
    /// The kernel Points (KernelData +0x50).
    pub fn point_count(&self) -> usize {
        self.points.len()
    }
    /// One line per kernel joint for logs: SleepStage edge, broken flags of
    /// its connectors and the largest Point gap.
    pub fn joint_report(&self) -> String {
        let mut out = String::new();
        for joint in &self.joints {
            let connectors: Vec<&BreakConnector> = joint
                .connectors
                .iter()
                .filter_map(|id| self.breakers.get(id))
                .collect();
            let gap = connectors
                .iter()
                .map(|c| {
                    let [a, b] = c.points.map(|p| self.points[p].world);
                    let d: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .fold(0.0f32, f32::max);
            out.push_str(&format!(
                "[{} edge={:?} connectors={} broken={:?} gap={}",
                joint.name,
                joint.edge,
                connectors.len(),
                connectors.iter().map(|c| c.broken).collect::<Vec<_>>(),
                gap
            ));
            if let Some(r) = joint.rotator.and_then(|id| self.rotators.get(&id)) {
                out.push_str(&format!(
                    " value={} current={} desired={} increment={}",
                    joint.value, r.current, r.desired, r.increment
                ));
            }
            out.push(']');
        }
        out
    }
    /// Back to the parts as added, with no contacts.
    pub fn reset(&mut self) {
        self.parts = self.original.clone();
        for list in [
            &mut self.free_fall,
            &mut self.real_time,
            &mut self.joint,
            &mut self.contact_bodies,
        ] {
            list.clear();
        }
        self.pairs.clear();
        self.real_time_connectors.clear();
        self.contact_connectors.clear();
        // The kernel joints as built (their connectors are in the kernel).
        let joints = self.original_joints.clone();
        self.joints = joints.joints;
        self.points = joints.points;
        self.breakers = joints.connectors;
        self.rotators = joints.rotators;
        self.joint_connectors = joints.joint_connectors;
        self.wake_through_awake = false;
        // The motors at their loaded angles, their links with them.
        self.motors = self.original_motors.clone();
        for m in &self.motors {
            let offset = motor::revolute_offset(&m.coords[0], &m.coords[1], m.joint.current_angle);
            if let Some(link) = self.prims[m.child].parent.as_mut() {
                link.1 = offset;
            }
        }
        self.broken.clear();
        self.owners.clear();
        self.stats = StepStats::default();
        self.stepping = [Vec::new(), Vec::new()];
        self.touching = [Vec::new(), Vec::new()];
        self.character = CharacterNode::default();
        self.pass = 0;
        self.transitions.clear();
        self.step_id = 0;
        self.simulating.clear();
        self.created.clear();
    }
    /// The loose Parts in SimulateStage, in entry order.
    pub fn simulating(&self) -> &[usize] {
        &self.simulating
    }
    /// Every Part's name and CFrame (Parts already in an assembly).
    pub fn frames(&self) -> Vec<(String, CoordinateFrame)> {
        (0..self.prims.len())
            .filter_map(|i| Some((self.prims[i].name.clone(), self.prim_frame(i).ok()?)))
            .collect()
    }
    pub fn connector_count(&self) -> usize {
        self.pairs
            .values()
            .map(|p| p.contact.connectors().len())
            .sum()
    }
    fn list_mut(&mut self, list: BodyList) -> &mut Vec<usize> {
        match list {
            BodyList::FreeFall => &mut self.free_fall,
            BodyList::RealTime => &mut self.real_time,
            BodyList::Joint => &mut self.joint,
            BodyList::Contact => &mut self.contact_bodies,
        }
    }
    /// Join a body list (the tail of 00a9be30).
    fn enter(&mut self, index: usize, list: BodyList) {
        self.list_mut(list).push(index);
        let part = &mut self.parts[index];
        part.list = list;
        if list == BodyList::FreeFall {
            part.body.resting = [true, true];
            part.body.angular_displacement = [0.0; 3];
            part.body.linear_displacement = [0.0; 3];
        }
    }
    /// 00a9c142: leave the current list; leaving free fall rebuilds L.
    fn leave(&mut self, index: usize) -> Result<(), String> {
        let list = self.parts[index].list;
        remove_item(self.list_mut(list), &index)?;
        if list == BodyList::FreeFall {
            self.parts[index].body.rebuild_momentum();
        }
        Ok(())
    }
    /// SimBody classification 00a9be30 for a throttling body.
    fn classify(&mut self, index: usize) -> Result<(), String> {
        let p = &self.parts[index];
        if !p.in_kernel {
            return Ok(());
        }
        let target = Self::target(p);
        if p.list == target {
            return Ok(());
        }
        self.leave(index)?;
        self.enter(index, target);
        Ok(())
    }
    fn target(p: &LoosePart) -> BodyList {
        if p.joint < 1 {
            if p.contact > 0 {
                BodyList::Contact
            } else if p.total == 0 {
                BodyList::FreeFall
            } else {
                BodyList::Joint
            }
        } else {
            BodyList::Joint
        }
    }
    fn block(
        &self,
        prim: Prim,
        statics: &[crate::world::StaticBlock],
        characters: &[Block],
    ) -> Result<Block, String> {
        Ok(match prim {
            Prim::Character(i) => characters
                .get(i)
                .ok_or("missing character collider")?
                .clone(),
            Prim::Loose(i) => Block {
                frame: self.prim_frame(i)?,
                size: self.prims[i].size,
            },
            Prim::Static(i) => {
                let s = statics.get(i).ok_or("missing static block")?;
                Block {
                    frame: s.frame.clone(),
                    size: s.size,
                }
            }
        })
    }
    fn pair_parameters(&self, a: Prim, b: Prim, materials: &MaterialProfile) -> ContactParameters {
        let props = |p: Prim| match p {
            Prim::Loose(i) => (self.prims[i].properties, self.prims[i].stiffness),
            Prim::Static(i) => self.statics[i],
            Prim::Character(i) => self.colliders[i],
        };
        let (pa, sa) = props(a);
        let (pb, sb) = props(b);
        materials.contact_params(&pa, sa, &pb, sb).parameters()
    }
    /// The kernel loose bodies (assemblies) of a connector between
    /// throttling bodies, sleeping Parts and static blocks, as a92c26 picks
    /// them: (A, B, flip normal); a primitive outside the kernel is fixed.
    /// None for a connector touching the character or with no kernel body.
    fn bodies(&self, first: Prim, second: Prim) -> Option<(usize, Option<usize>, bool)> {
        let kernel = |p: Prim| match p {
            Prim::Loose(i) if self.parts[self.prims[i].assembly].in_kernel => {
                Some(self.prims[i].assembly)
            }
            _ => None,
        };
        if matches!(first, Prim::Character(_)) || matches!(second, Prim::Character(_)) {
            return None;
        }
        match (kernel(first), kernel(second)) {
            (Some(a), Some(b)) => Some((a, Some(b), false)),
            (Some(a), None) => Some((a, None, false)),
            (None, Some(b)) => Some((b, None, true)),
            (None, None) => None,
        }
    }
    /// A primitive outside the kernel as a fixed contact body (a sleeping
    /// assembly's velocity is zero).
    fn fixed_state(
        &self,
        prim: Prim,
        statics: &[crate::world::StaticBlock],
    ) -> Result<ContactBodyState, String> {
        let center = match prim {
            Prim::Static(s) => statics.get(s).ok_or("missing static block")?.frame.position,
            Prim::Loose(i) => self.prim_frame(i)?.position,
            Prim::Character(_) => return Err("the character is never a fixed body".into()),
        };
        Ok(ContactBodyState {
            center,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        })
    }
    /// The assemblies of a pair's loose Parts.
    fn loose_of(&self, prims: (Prim, Prim)) -> Vec<usize> {
        [prims.0, prims.1]
            .iter()
            .filter_map(|p| match p {
                Prim::Loose(i) => Some(self.prims[*i].assembly),
                _ => None,
            })
            .collect()
    }
    /// The connector's primitives in its own order (+0x94, +0x98).
    fn connector_prims(key: (Prim, Prim), body_a: usize) -> (Prim, Prim) {
        if body_a == 0 { key } else { (key.1, key.0) }
    }
    fn apply_displacement(
        &mut self,
        prims: (Prim, Prim),
        point: &ContactPoint,
        sign: f32,
    ) -> Result<(), String> {
        let Some((a, b, flip)) = self.bodies(prims.0, prims.1) else {
            return Ok(());
        };
        let g = geometry(point, flip);
        let (a, b) = two_mut(&mut self.parts, a, b)?;
        displace(&mut a.body, b.map(|b| &mut b.body), &g, sign);
        Ok(())
    }
    /// Connector insert 00a99514 for a CONTACT connector (Kernel +0x28 off):
    /// realTimeConnectors when either body cannot throttle (the character),
    /// else jointConnectors when either body is in jointBodies, else
    /// contactConnectors; then both bodies are reclassified. A contact with
    /// no body in the kernel is not inserted.
    fn insert(
        &mut self,
        id: u64,
        prims: (Prim, Prim),
    ) -> Result<(Option<ConnectorList>, Vec<usize>), String> {
        let loose: Vec<usize> = self
            .loose_of(prims)
            .into_iter()
            .filter(|i| self.parts[*i].in_kernel)
            .collect();
        let character =
            matches!(prims.0, Prim::Character(_)) || matches!(prims.1, Prim::Character(_));
        if loose.is_empty() && !character {
            return Ok((None, loose));
        }
        let list = if character {
            ConnectorList::RealTime
        } else if loose.iter().any(|i| self.parts[*i].list == BodyList::Joint) {
            ConnectorList::Joint
        } else {
            ConnectorList::Contact
        };
        match list {
            ConnectorList::RealTime => self.real_time_connectors.push(id),
            ConnectorList::Joint => self.joint_connectors.push(id),
            ConnectorList::Contact => self.contact_connectors.push(id),
        }
        for i in &loose {
            let p = &mut self.parts[*i];
            p.total += 1;
            match list {
                ConnectorList::RealTime => p.real_time += 1,
                ConnectorList::Joint => p.joint += 1,
                ConnectorList::Contact => p.contact += 1,
            }
        }
        for i in &loose {
            self.classify(*i)?;
        }
        Ok((Some(list), loose))
    }
    /// Connector removal 00a99ff2.
    fn remove(
        &mut self,
        id: u64,
        link: &Link,
        prims: (Prim, Prim),
        point: &ContactPoint,
    ) -> Result<(), String> {
        let Some(list) = link.list else {
            return Ok(());
        };
        match list {
            ConnectorList::RealTime => remove_item(&mut self.real_time_connectors, &id)?,
            ConnectorList::Joint => remove_item(&mut self.joint_connectors, &id)?,
            ConnectorList::Contact => remove_item(&mut self.contact_connectors, &id)?,
        }
        let loose = link.counted.clone();
        for i in &loose {
            let p = &mut self.parts[*i];
            p.total -= 1;
            match list {
                ConnectorList::RealTime => p.real_time -= 1,
                ConnectorList::Joint => p.joint -= 1,
                ConnectorList::Contact => p.contact -= 1,
            }
        }
        if list == ConnectorList::Contact {
            self.apply_displacement(prims, point, -1.0)?;
        }
        for i in loose {
            self.classify(i)?;
        }
        Ok(())
    }
    /// Contact::step for every pair (SleepStage::stepContacts b715ac runs
    /// before StepJoints and Kernel::step in doWorldStep). Pairs are every
    /// colliding (loose, static) and (loose, loose) combination in a fixed
    /// order; the native broadphase order is not recovered.
    pub fn update_contacts(
        &mut self,
        statics: &[crate::world::StaticBlock],
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        if statics.len() != self.statics.len() {
            return Err("static block materials out of step".into());
        }
        let mut keys = Vec::new();
        for i in 0..self.prims.len() {
            if !self.prims[i].can_collide {
                continue;
            }
            for j in 0..statics.len() {
                keys.push((Prim::Loose(i), Prim::Static(j)));
            }
            for k in i + 1..self.prims.len() {
                if self.prims[k].can_collide && self.prims[k].assembly != self.prims[i].assembly {
                    keys.push((Prim::Loose(i), Prim::Loose(k)));
                }
            }
        }
        self.static_blocks = statics
            .iter()
            .map(|b| Block {
                frame: b.frame.clone(),
                size: b.size,
            })
            .collect();
        for key in keys {
            let blocks = [
                self.block(key.0, statics, &[])?,
                self.block(key.1, statics, &[])?,
            ];
            self.update_pair(key, blocks, true, tables, materials)?;
        }
        Ok(())
    }
    /// Contact::step for every (character collider, colliding loose Part)
    /// pair, with the colliders at their current pose in registration order.
    /// `in_kernel` is false while the character is placed kinematically:
    /// the contacts then delete their connectors (ae32f4 checks stage 0xf).
    pub fn update_character_contacts(
        &mut self,
        colliders: &[Block],
        in_kernel: bool,
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        if colliders.len() != self.colliders.len() {
            return Err("character collider materials out of step".into());
        }
        for (c, collider) in colliders.iter().enumerate() {
            for i in 0..self.prims.len() {
                if !self.prims[i].can_collide {
                    continue;
                }
                let key = (Prim::Character(c), Prim::Loose(i));
                let blocks = [collider.clone(), self.block(key.1, &[], &[])?];
                self.update_pair(key, blocks, in_kernel, tables, materials)?;
            }
        }
        Ok(())
    }
    /// A contact for a primitive pair, outside SleepStage.
    fn create_pair(&mut self, key: (Prim, Prim), materials: &MaterialProfile) {
        let parameters = self.pair_parameters(key.0, key.1, materials);
        let character = matches!(key.0, Prim::Character(_)) || matches!(key.1, Prim::Character(_));
        self.pairs.insert(
            key,
            Pair {
                contact: BlockBlockContact::default(),
                parameters,
                edge: None,
                throttle: usize::from(!character),
            },
        );
    }
    /// BlockBlockContact::update for one pair, then its events in order.
    /// Returns whether the blocks collide.
    fn update_pair(
        &mut self,
        key: (Prim, Prim),
        blocks: [Block; 2],
        in_kernel: bool,
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<bool, String> {
        if !self.pairs.contains_key(&key) {
            self.create_pair(key, materials);
        }
        let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
        let colliding = pair
            .contact
            .update([&blocks[0], &blocks[1]], in_kernel, tables)?;
        let events = std::mem::take(&mut pair.contact.events);
        for event in events {
            match event {
                ContactEvent::Kept { index, reused } => {
                    let c = &self.pairs[&key].contact.connectors()[index];
                    let prims = Self::connector_prims(key, c.pair.body_a);
                    let point = c.point.clone();
                    if !reused {
                        self.next_id += 1;
                        let id = self.next_id;
                        let (list, counted) = self.insert(id, prims)?;
                        let link = &mut self.pairs.get_mut(&key).unwrap().contact.connectors_mut()
                            [index]
                            .state;
                        link.id = id;
                        link.list = list;
                        link.counted = counted;
                        self.owners.insert(id, key);
                    } else {
                        self.redisplace(key, index, prims, point)?;
                    }
                }
                ContactEvent::Removed {
                    connector,
                    recomputed,
                } => {
                    let prims = Self::connector_prims(key, connector.pair.body_a);
                    let mut link = connector.state;
                    if recomputed && link.list == Some(ConnectorList::Contact) {
                        self.redisplace_link(&mut link, prims, &connector.point)?;
                    }
                    self.remove(link.id, &link, prims, &connector.point)?;
                    self.owners.remove(&link.id);
                }
            }
        }
        Ok(colliding)
    }
    /// The Head touch test of HumanoidState c0192c -> c02209: each contact of
    /// the collider with a loose Part runs vtable +0x24 (ae26a6: extents,
    /// then computeIsColliding with no ignored overlap, which updates the
    /// contact's cached separating axis), stopping at the first colliding
    /// one. Contacts are visited in loose Part order (the primitive's contact
    /// list order is not recovered).
    pub fn collider_touching(
        &mut self,
        collider: usize,
        block: &Block,
        materials: &MaterialProfile,
    ) -> Result<bool, String> {
        if collider >= self.colliders.len() {
            return Err("missing character collider".into());
        }
        let _ = materials;
        for i in 0..self.prims.len() {
            let forward = (Prim::Character(collider), Prim::Loose(i));
            let backward = (Prim::Loose(i), Prim::Character(collider));
            let (key, flipped) = if self.pairs.contains_key(&forward) {
                (forward, false)
            } else if self.pairs.contains_key(&backward) {
                (backward, true)
            } else {
                // No broadphase contact: nothing to test.
                continue;
            };
            let other = self.block(Prim::Loose(i), &[], &[])?;
            if !extents_overlap(&block.extents(), &other.extents()) {
                continue;
            }
            let blocks = if flipped {
                [&other, block]
            } else {
                [block, &other]
            };
            let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
            if pair.contact.compute_is_colliding(blocks, 0.0).0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Forget the character's contact caches (the character returned to the
    /// kernel). Pairs that still hold connectors are kept.
    pub fn clear_character_contacts(&mut self) {
        for (key, pair) in self.pairs.iter_mut() {
            let character =
                matches!(key.0, Prim::Character(_)) || matches!(key.1, Prim::Character(_));
            if character && pair.contact.connectors().is_empty() && pair.edge.is_none() {
                pair.contact = BlockBlockContact::default();
            }
        }
    }
    /// Live connectors touching the character.
    pub fn character_connector_count(&self) -> usize {
        self.real_time_connectors.len()
    }
    /// a9457c after a geometry recompute of a live connector.
    fn redisplace(
        &mut self,
        key: (Prim, Prim),
        index: usize,
        prims: (Prim, Prim),
        point: ContactPoint,
    ) -> Result<(), String> {
        let mut link = self.pairs[&key].contact.connectors()[index].state.clone();
        if link.list == Some(ConnectorList::Contact) {
            self.redisplace_link(&mut link, prims, &point)?;
            self.pairs.get_mut(&key).unwrap().contact.connectors_mut()[index]
                .state
                .applied = link.applied;
        }
        Ok(())
    }
    fn redisplace_link(
        &mut self,
        link: &mut Link,
        prims: (Prim, Prim),
        point: &ContactPoint,
    ) -> Result<(), String> {
        if link.applied == *point {
            return Ok(());
        }
        let old = link.applied.clone();
        self.apply_displacement(prims, &old, -1.0)?;
        self.apply_displacement(prims, point, 1.0)?;
        link.applied = point.clone();
        Ok(())
    }
    /// Kernel::step pre-step 00a96eb0 for the throttling bodies.
    pub fn prestep(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        let mut at = 0;
        while at < self.contact_bodies.len() {
            let index = self.contact_bodies[at];
            let body = &mut self.parts[index].body;
            if body.has_external_force_or_impulse() {
                self.contact_bodies.swap_remove(at);
                self.real_time.push(index);
                self.parts[index].list = BodyList::RealTime;
                continue;
            }
            let [ax, ay, az] = body.angular_displacement;
            body.resting[0] = az * az + (ay * ay + ax * ax) < self.profile.angular_rest;
            let [lx, _, lz] = body.linear_displacement;
            body.resting[1] =
                lx.abs() < self.profile.linear_rest && lz.abs() < self.profile.linear_rest;
            body.contact_prestep(kernel)?;
            at += 1;
        }
        for pair in self.pairs.values_mut() {
            for c in pair.contact.connectors_mut() {
                if c.state.list == Some(ConnectorList::Contact) {
                    c.state.impulse.invalidate();
                }
            }
        }
        Ok(())
    }
    /// stepWorld 00a9768c up to the substeps: free-fall bodies, the awake
    /// impulses over contactConnectors, then the contact bodies.
    pub fn step_world(
        &mut self,
        kernel: &KernelProfile,
        impulses: &ImpulseProfile,
        statics: &[crate::world::StaticBlock],
    ) -> Result<(), String> {
        let mut at = 0;
        while at < self.free_fall.len() {
            let index = self.free_fall[at];
            if self.parts[index].body.has_external_force_or_impulse() {
                self.parts[index].body.rebuild_momentum();
                self.free_fall.swap_remove(at);
                self.real_time.push(index);
                self.parts[index].list = BodyList::RealTime;
                continue;
            }
            self.parts[index].body.free_fall_step(kernel)?;
            at += 1;
        }
        self.stats = StepStats {
            contact_connectors: self.contact_connectors.len(),
            joint_connectors: self.joint_connectors.len(),
            ..StepStats::default()
        };
        if !self.contact_connectors.is_empty() {
            let goal =
                self.profile.error_goal * (self.contact_bodies.len() / BODIES_PER_GOAL + 1) as f32;
            let mut iterations = 0;
            let mut average;
            loop {
                if iterations >= MAX_ITERATIONS {
                    break;
                }
                let mut sum = 0.0f32;
                for at in 0..self.contact_connectors.len() {
                    let id = self.contact_connectors[at];
                    sum += self.impulse(id, kernel, impulses, statics)?;
                }
                average = sum / self.contact_connectors.len() as f32;
                iterations += 1;
                self.stats.average_error = average;
                // do { ... } while (goal <= average): NaN also stops.
                let again = goal <= average;
                if !again {
                    break;
                }
            }
            self.stats.iterations = iterations;
        }
        for at in 0..self.contact_bodies.len() {
            let index = self.contact_bodies[at];
            self.parts[index].body.contact_integrate(kernel)?;
        }
        Ok(())
    }
    /// One contact connector's impulse (vtable +0x14 = a936d0). Returns its
    /// error term.
    fn impulse(
        &mut self,
        id: u64,
        kernel: &KernelProfile,
        profile: &ImpulseProfile,
        statics: &[crate::world::StaticBlock],
    ) -> Result<f32, String> {
        let key = *self.owners.get(&id).ok_or("missing connector owner")?;
        let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
        let parameters = pair.parameters;
        let at = pair
            .contact
            .connectors()
            .iter()
            .position(|c| c.state.id == id)
            .ok_or("missing kernel connector")?;
        let c = &mut pair.contact.connectors_mut()[at];
        let prims = Self::connector_prims(key, c.pair.body_a);
        let point = c.point.clone();
        let Some((a, b, flip)) = self.bodies(prims.0, prims.1) else {
            return Ok(0.0);
        };
        let g = geometry(&point, flip);
        let fixed = match b {
            Some(_) => ContactBodyState {
                center: [0.0; 3],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
            },
            None => self.fixed_state(if flip { prims.0 } else { prims.1 }, statics)?,
        };
        let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
        let c = &mut pair.contact.connectors_mut()[at];
        let mut link = std::mem::take(&mut c.state);
        let (pa, pb) = two_mut(&mut self.parts, a, b)?;
        let input = ImpulseInput {
            geometry: g.clone(),
            parameters,
            reference_impulse_a: pa.body.reference_impulse,
            reference_impulse_b: pb.as_ref().map_or([0.0; 3], |b| b.body.reference_impulse),
            friction_enabled: true,
        };
        let result = link.impulse.solve(
            &input,
            &mut pa.body,
            pb.map(|b| &mut b.body),
            &fixed,
            kernel,
            profile,
        );
        if g.distance >= 0.0 {
            // a936d0 separation also clears the spring depths (+0xf8..+0x100).
            link.spring.compression_depth = 0.0;
            link.spring.decompression_depth = 0.0;
            link.spring.normal_force = 0.0;
        }
        let term = link.impulse.error_term;
        self.pairs
            .get_mut(&key)
            .ok_or("missing contact pair")?
            .contact
            .connectors_mut()[at]
            .state = link;
        result?;
        Ok(term)
    }
    /// The block of a primitive for a substep spring.
    fn substep_block(&self, prim: Prim, characters: &[Block]) -> Result<Block, String> {
        Ok(match prim {
            Prim::Loose(i) => Block {
                frame: self.prim_frame(i)?,
                size: self.prims[i].size,
            },
            Prim::Static(i) => self
                .static_blocks
                .get(i)
                .ok_or("missing static block")?
                .clone(),
            Prim::Character(i) => characters
                .get(i)
                .ok_or("missing character collider")?
                .clone(),
        })
    }
    /// Recompute a connector's geometry (vtable +0x24 without the
    /// contactConnectors-only displacement) and return its spring force on
    /// body B with the geometry from A into B (`flip` when a92c26 swapped
    /// the connector's primitives).
    #[allow(clippy::too_many_arguments)] // Both bodies, their order and the pose.
    fn spring(
        &mut self,
        id: u64,
        characters: &[Block],
        flip: bool,
        a: &ContactBodyState,
        b: &ContactBodyState,
        kernel: &KernelProfile,
    ) -> Result<([f32; 3], ContactGeometry), String> {
        let key = *self.owners.get(&id).ok_or("missing connector owner")?;
        let blocks = [
            self.substep_block(key.0, characters)?,
            self.substep_block(key.1, characters)?,
        ];
        let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
        let parameters = pair.parameters;
        let c = pair
            .contact
            .connectors_mut()
            .iter_mut()
            .find(|c| c.state.id == id)
            .ok_or("missing kernel connector")?;
        c.point = c.pair.compute([&blocks[0], &blocks[1]]);
        let g = geometry(&c.point, flip);
        let force = c
            .state
            .spring
            .force(kernel, parameters, g.distance, g.normal, g.point, a, b)?;
        Ok((force, g))
    }
    /// The connector's primitives in its own order.
    fn prims_of(&self, id: u64) -> Result<(Prim, Prim), String> {
        let key = *self.owners.get(&id).ok_or("missing connector owner")?;
        let c = self.pairs[&key]
            .contact
            .connectors()
            .iter()
            .find(|c| c.state.id == id)
            .ok_or("missing kernel connector")?;
        Ok(Self::connector_prims(key, c.pair.body_a))
    }
    /// realTimeConnectors springs (vtable +0x10, a93074) between the
    /// character and loose Parts, in list order. The character body is A when
    /// its collider is the connector's first primitive (a92c26: both bodies
    /// are in the kernel). Returns the deepest penetration.
    pub fn real_time_springs(
        &mut self,
        colliders: &[Block],
        character: &mut KernelBody,
        kernel: &KernelProfile,
    ) -> Result<f32, String> {
        let mut deepest = 0.0f32;
        for at in 0..self.real_time_connectors.len() {
            let id = self.real_time_connectors[at];
            let prims = self.prims_of(id)?;
            let (prim, character_first) = match prims {
                (Prim::Character(_), Prim::Loose(i)) => (i, true),
                (Prim::Loose(i), Prim::Character(_)) => (i, false),
                _ => return Err("unexpected realtime connector".into()),
            };
            // a929f6 reads the Part's Body PV.
            let part = self.prim_pv(prim)?.contact_state();
            let loose = self.prims[prim].assembly;
            let own = character.contact_state();
            let (a, b) = if character_first {
                (&own, &part)
            } else {
                (&part, &own)
            };
            let (force, g) = self.spring(id, colliders, false, a, b, kernel)?;
            deepest = deepest.max(-g.distance);
            // a92c26: a sleeping Part is outside the kernel and stays fixed.
            let in_kernel = self.parts[loose].in_kernel;
            let body = &mut self.parts[loose].body;
            if character_first {
                character.apply_force(force.map(|v| -v), g.point)?;
                if in_kernel {
                    body.apply_force(force, g.point)?;
                }
            } else {
                if in_kernel {
                    body.apply_force(force.map(|v| -v), g.point)?;
                }
                character.apply_force(force, g.point)?;
            }
        }
        Ok(deepest)
    }
    /// jointConnectors in list order: contact springs (a93074) between
    /// loose Parts and static blocks, the kernel joints' break connectors
    /// (computeForce 00a91948 / 00a91a12 on the Points) and the motor
    /// hinges' RotateConnectors (00a9178e, torques straight to the bodies).
    pub fn joint_springs(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        for at in 0..self.joint_connectors.len() {
            let id = self.joint_connectors[at];
            if let Some(r) = self.rotators.get(&id) {
                // 00a914d6 reads both Bodies' PV rotations (+0xb4); the
                // torque goes to each side's root SimBody (00a6c134), body
                // 0 first and negated. A fixed Body never moves.
                let bodies = r.bodies;
                let r0 = self.body_frame(bodies[0])?.rotation;
                let r1 = self.body_frame(bodies[1])?.rotation;
                let torque = self
                    .rotators
                    .get_mut(&id)
                    .ok_or("missing rotate connector")?
                    .compute_torque([&r0, &r1]);
                for (body, t) in [(bodies[0], torque.map(|v| -v)), (bodies[1], torque)] {
                    if let JointBody::Loose(prim) = body {
                        let a = self.prims[prim].assembly;
                        self.parts[a].body.apply_torque(t)?;
                    }
                }
                continue;
            }
            if let Some(c) = self.breakers.get(&id) {
                // The normal test reads point 0's Body rotation (+0xb4).
                let axes = self.body_frame(self.points[c.points[0]].body)?.rotation;
                let c = self
                    .breakers
                    .get_mut(&id)
                    .ok_or("missing break connector")?;
                c.compute_force(&mut self.points, &axes);
                continue;
            }
            let prims = self.prims_of(id)?;
            let Some((a, b, flip)) = self.bodies(prims.0, prims.1) else {
                return Err("unexpected joint connector".into());
            };
            let fixed = |p: Prim| -> Result<ContactBodyState, String> {
                let center = match p {
                    Prim::Static(s) => {
                        self.static_blocks
                            .get(s)
                            .ok_or("missing static block")?
                            .frame
                            .position
                    }
                    Prim::Loose(i) => self.prim_frame(i)?.position,
                    Prim::Character(_) => return Err("unexpected joint connector".into()),
                };
                Ok(ContactBodyState {
                    center,
                    velocity: [0.0; 3],
                    angular_velocity: [0.0; 3],
                })
            };
            // A is a kernel body; B is the other loose body or the block.
            // a929f6 reads each side's Body PV.
            let (prim_a, prim_b) = if flip {
                (prims.1, prims.0)
            } else {
                (prims.0, prims.1)
            };
            let pv = |p: Prim| match p {
                Prim::Loose(i) => Ok(self.prim_pv(i)?.contact_state()),
                _ => Err("joint connector body is not a loose Part".to_string()),
            };
            let state_a = pv(prim_a)?;
            let state_b = match b {
                Some(_) => pv(prim_b)?,
                None => fixed(prim_b)?,
            };
            let (force, g) = self.spring(id, &[], flip, &state_a, &state_b, kernel)?;
            let (pa, pb) = two_mut(&mut self.parts, a, b)?;
            pa.body.apply_force(force.map(|v| -v), g.point)?;
            if let Some(pb) = pb {
                pb.body.apply_force(force, g.point)?;
            }
        }
        Ok(())
    }
    /// realTimeBodies then jointBodies integrate (ab1d44).
    pub fn integrate(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        for at in 0..self.real_time.len() {
            let index = self.real_time[at];
            self.parts[index].body.step(kernel)?;
        }
        for at in 0..self.joint.len() {
            let index = self.joint[at];
            self.parts[index].body.step(kernel)?;
        }
        Ok(())
    }
    /// The substep half of stepWorld without the character: the Points,
    /// jointConnectors, the Points' forces into their bodies, then
    /// realTimeBodies and jointBodies integrate.
    pub fn substep(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        self.step_points()?;
        self.joint_springs(kernel)?;
        self.points_to_bodies()?;
        self.integrate(kernel)
    }
    /// World::insertContact 00bab0f0 for a broadphase contact: the pair, and
    /// its SleepStage edge (00b73cfe, STEPPING) unless one side is the
    /// character outside the stage.
    pub fn contact_created(
        &mut self,
        key: (Prim, Prim),
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        if self.pairs.contains_key(&key) || self.pairs.contains_key(&(key.1, key.0)) {
            return Err("contact created twice".into());
        }
        self.create_pair(key, materials);
        self.created.push(key);
        let character = matches!(key.0, Prim::Character(_)) || matches!(key.1, Prim::Character(_));
        if !character || self.character.in_stage {
            self.add_edge(key)?;
        }
        Ok(())
    }
    /// World::destroyContact 00bab1f6: SleepStage onEdgeRemoving (00b73f08),
    /// the connectors leave the kernel, the pair is deleted.
    pub fn contact_destroyed(&mut self, key: (Prim, Prim)) -> Result<(), String> {
        let key = if self.pairs.contains_key(&key) {
            key
        } else {
            (key.1, key.0)
        };
        if !self.pairs.contains_key(&key) {
            return Err("destroyed contact is missing".into());
        }
        if self.pairs[&key].edge.is_some() {
            self.remove_edge(key)?;
        }
        self.drop_connectors(key)?;
        self.pairs.remove(&key);
        self.created.retain(|k| *k != key);
        for list in self.stepping.iter_mut().chain(self.touching.iter_mut()) {
            list.retain(|k| *k != key);
        }
        Ok(())
    }
    /// The live contact pairs, in creation order.
    pub fn contact_keys(&self) -> &[(Prim, Prim)] {
        &self.created
    }
    /// SleepStage::onEdgeAdded 00b73cfe: a contact enters STEPPING.
    fn add_edge(&mut self, key: (Prim, Prim)) -> Result<(), String> {
        let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
        if pair.edge.is_some() {
            return Ok(());
        }
        pair.edge = Some(EdgeState::Stepping);
        let throttle = pair.throttle;
        self.stepping[throttle].push(key);
        Ok(())
    }
    /// SleepStage::onEdgeRemoving 00b73f08 for a contact.
    fn remove_edge(&mut self, key: (Prim, Prim)) -> Result<(), String> {
        let Some(edge) = self.pairs.get(&key).ok_or("missing contact pair")?.edge else {
            return Ok(());
        };
        if matches!(edge, EdgeState::Contacting | EdgeState::ContactingSleeping) {
            self.wake_both(key)?;
        }
        if edge != EdgeState::Sleeping {
            self.set_edge(key, EdgeState::Sleeping)?;
        }
        self.pairs.get_mut(&key).ok_or("missing contact pair")?.edge = None;
        Ok(())
    }
    /// Assemblies entering or leaving the stage, and their contacts.
    fn update_membership(
        &mut self,
        character: Option<&CharacterMotion>,
        colliders: usize,
        statics: usize,
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        let _ = (colliders, statics, materials);
        // The character's existing contacts, in creation order.
        let keys: Vec<(Prim, Prim)> = self
            .created
            .iter()
            .copied()
            .filter(|k| matches!(k.0, Prim::Character(_)) || matches!(k.1, Prim::Character(_)))
            .collect();
        match (character, self.character.in_stage) {
            (Some(motion), false) => {
                // 00b73820: WAKE_PENDING with a fresh history (00ad88c4).
                self.character.in_stage = true;
                self.character.history = Some(MotionHistory::new(sleep::sample(
                    &motion.frame,
                    motion.radius,
                )));
                self.set_state(Node::Character, AssemblyState::WakePending)?;
                for key in keys {
                    self.add_edge(key)?;
                }
            }
            (None, true) => {
                for key in keys {
                    if self.pairs.contains_key(&key) {
                        self.remove_edge(key)?;
                    }
                }
                self.character = CharacterNode::default();
            }
            _ => {}
        }
        if let Some(motion) = character {
            self.character.motion = Some(motion.clone());
        }
        Ok(())
    }
    /// The primitive's assembly.
    fn node(&self, prim: Prim) -> Option<Node> {
        match prim {
            Prim::Loose(i) => Some(Node::Loose(self.prims[i].assembly)),
            Prim::Character(_) => Some(Node::Character),
            Prim::Static(_) => None,
        }
    }
    /// Assembly +0x40 (ANCHORED for a static block).
    fn state(&self, node: Option<Node>) -> AssemblyState {
        match node {
            Some(Node::Loose(i)) => self.parts[i].sleep,
            Some(Node::Character) => self.character.state,
            None => AssemblyState::Anchored,
        }
    }
    fn in_stage(&self, node: Node) -> bool {
        match node {
            Node::Loose(_) => true,
            Node::Character => self.character.in_stage,
        }
    }
    fn nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        if self.character.in_stage {
            nodes.push(Node::Character);
        }
        nodes.extend((0..self.parts.len()).map(Node::Loose));
        nodes
    }
    /// The assembly's contacts in the stage (Assembly edge list 00ad9bf6;
    /// its native order is not recovered).
    fn edges_of(&self, node: Node) -> Vec<(Prim, Prim)> {
        self.pairs
            .iter()
            .filter(|(key, pair)| {
                pair.edge.is_some()
                    && (self.node(key.0) == Some(node) || self.node(key.1) == Some(node))
            })
            .map(|(key, _)| *key)
            .collect()
    }
    fn other(&self, key: (Prim, Prim), node: Node) -> Option<Node> {
        if self.node(key.0) == Some(node) {
            self.node(key.1)
        } else {
            self.node(key.0)
        }
    }
    /// 00b72d00: the contact touches (CONTACTING or CONTACTING_SLEEPING).
    fn touching_edge(&self, key: (Prim, Prim)) -> bool {
        matches!(
            self.pairs.get(&key).and_then(|p| p.edge),
            Some(EdgeState::Contacting | EdgeState::ContactingSleeping)
        )
    }
    fn moving(&self, node: Node) -> bool {
        let history = match node {
            Node::Loose(i) => Some(&self.parts[i].history),
            Node::Character => self.character.history.as_ref(),
        };
        history.is_some_and(|h| h.moving(&self.sleep_profile))
    }
    /// 00b7357c.
    fn wake(&mut self, node: Node) -> Result<(), String> {
        if self.state(Some(node)) > AssemblyState::Awake {
            self.set_state(node, AssemblyState::WakePending)?;
        }
        Ok(())
    }
    /// 00b735e6.
    fn wake_recursive(&mut self, node: Node) -> Result<(), String> {
        if self.state(Some(node)) > AssemblyState::RecursiveWakePending {
            self.set_state(node, AssemblyState::RecursiveWakePending)?;
        }
        Ok(())
    }
    /// 00b73544 / 00b735ae over both primitives' assemblies.
    fn wake_both(&mut self, key: (Prim, Prim)) -> Result<(), String> {
        for prim in [key.0, key.1] {
            if let Some(node) = self.node(prim) {
                self.wake(node)?;
            }
        }
        Ok(())
    }
    /// World::wakeAssembly 00ba7fb0 -> 00b737c4 (not recursive): the
    /// history countdown restarts and a sleeping assembly becomes
    /// WAKE_PENDING.
    pub fn wake_assembly(&mut self, index: usize) -> Result<(), String> {
        let part = self.parts.get_mut(index).ok_or("missing loose part")?;
        part.history.reset_countdown();
        self.wake(Node::Loose(index))
    }
    /// World::wakeAssembly 00ba7fb0 -> 00b737c4, recursive: the history
    /// countdown restarts, SleepStage +0x16c is set, and a sleeping or
    /// WAKE_PENDING assembly becomes RECURSIVE_WAKE_PENDING (00b735e6).
    pub fn wake_assembly_recursive(&mut self, index: usize) -> Result<(), String> {
        let part = self.parts.get_mut(index).ok_or("missing loose part")?;
        part.history.reset_countdown();
        self.wake_through_awake = true;
        self.wake_recursive(Node::Loose(index))
    }
    /// 00b7361a: either primitive's assembly moves faster than 1 stud/s or
    /// spins faster than 1 stud/s at its radius (the root primitive's PV,
    /// 00ad8aee -> 00b63daa).
    fn fast(&self, key: (Prim, Prim)) -> bool {
        [key.0, key.1].into_iter().any(|prim| {
            let (v, w, r) = match prim {
                Prim::Static(_) => return false,
                Prim::Loose(i) => {
                    let p = &self.parts[self.prims[i].assembly];
                    let pv = p.body.body_state();
                    (pv.velocity, pv.angular_velocity, p.radius)
                }
                Prim::Character(_) => match &self.character.motion {
                    Some(m) => (m.velocity, m.angular_velocity, m.radius),
                    None => return false,
                },
            };
            v.iter().any(|x| 1.0 < x.abs()) || w.iter().any(|x| 1.0 < (x * r).abs())
        })
    }
    /// Contact edge state change 00b72df0.
    fn set_edge(&mut self, key: (Prim, Prim), new: EdgeState) -> Result<(), String> {
        let pair = self.pairs.get(&key).ok_or("missing contact pair")?;
        let current = pair.edge.ok_or("contact outside SleepStage")?;
        if current == new {
            return Err("contact edge state unchanged".into());
        }
        let t = pair.throttle;
        if current == EdgeState::Stepping {
            remove_item(&mut self.stepping[t], &key)?;
        }
        if current == EdgeState::Contacting {
            remove_item(&mut self.touching[t], &key)?;
            // The contact leaves the kernel stage with its connectors.
            self.drop_connectors(key)?;
        }
        if new == EdgeState::Stepping {
            self.stepping[t].push(key);
        }
        if new == EdgeState::Contacting {
            // Kernel insert: connectors follow at its next Contact::step.
            self.touching[t].push(key);
        }
        if new == EdgeState::Contacting && current == EdgeState::Stepping {
            if self.fast(key) {
                for prim in [key.0, key.1] {
                    if let Some(node) = self.node(prim) {
                        self.wake_recursive(node)?;
                    }
                }
            } else {
                self.wake_both(key)?;
            }
        } else if new == EdgeState::Stepping && current == EdgeState::Contacting {
            self.wake_both(key)?;
        }
        self.pairs.get_mut(&key).ok_or("missing contact pair")?.edge = Some(new);
        Ok(())
    }
    /// Delete a contact's connectors from the kernel (a99ff2 for each).
    fn drop_connectors(&mut self, key: (Prim, Prim)) -> Result<(), String> {
        let connectors = self
            .pairs
            .get_mut(&key)
            .ok_or("missing contact pair")?
            .contact
            .take_connectors();
        for connector in connectors {
            let prims = Self::connector_prims(key, connector.pair.body_a);
            let link = connector.state;
            self.remove(link.id, &link, prims, &connector.point)?;
            self.owners.remove(&link.id);
        }
        Ok(())
    }
    /// Re-bind the connectors of every contact of an assembly whose SimBody
    /// enters or leaves the kernel. The native re-insert of live connectors
    /// is not recovered: they are deleted, and Contact::step regenerates
    /// them for touching contacts.
    fn drop_part_connectors(&mut self, index: usize) -> Result<(), String> {
        let node = Some(Node::Loose(index));
        let keys: Vec<(Prim, Prim)> = self
            .pairs
            .iter()
            .filter(|(k, p)| {
                (self.node(k.0) == node || self.node(k.1) == node)
                    && !p.contact.connectors().is_empty()
            })
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            self.drop_connectors(key)?;
        }
        Ok(())
    }
    /// setAssemblyState 00b727c0.
    fn set_state(&mut self, node: Node, new: AssemblyState) -> Result<(), String> {
        let current = self.state(Some(node));
        if current == new {
            return Err("assembly state unchanged".into());
        }
        if let Node::Loose(i) = node {
            if current.sleeping() && !new.sleeping() {
                // 00b63bf2 velocity 0, 00ad8aa0 history, SimBody J and Ja 0.
                let body = &mut self.parts[i].body;
                body.velocity = [0.0; 3];
                body.angular_velocity = [0.0; 3];
                body.linear_impulse = [0.0; 3];
                body.angular_impulse = [0.0; 3];
                self.parts[i].history.reset_countdown();
            }
            if new == AssemblyState::Awake {
                if !self.parts[i].in_kernel {
                    // 00b7439e: forces gathered outside the kernel (a
                    // Humanoid's floor pushes, joints) are dropped.
                    self.parts[i].body.reset_force();
                    // b6ee0a -> Kernel insertBody 00a957ce: classify 00a9be30.
                    self.drop_part_connectors(i)?;
                    self.parts[i].in_kernel = true;
                    self.simulating.push(i);
                    let target = Self::target(&self.parts[i]);
                    self.enter(i, target);
                }
            } else if new.sleeping() && self.parts[i].in_kernel {
                // 00ab1cc2 then removeBody 00a95a32.
                let body = &mut self.parts[i].body;
                body.velocity = [0.0; 3];
                body.angular_velocity = [0.0; 3];
                self.drop_part_connectors(i)?;
                self.leave(i)?;
                self.simulating.retain(|p| *p != i);
                let part = &mut self.parts[i];
                part.in_kernel = false;
                part.body.resting = [true, true];
                part.body.angular_displacement = [0.0; 3];
                part.body.linear_displacement = [0.0; 3];
            }
            self.parts[i].sleep = new;
        } else {
            self.character.state = new;
        }
        self.transitions.push((self.step_id, node, new));
        Ok(())
    }
    /// 00b726ae: AWAKE (3) while a touching awake assembly moves; else
    /// SLEEPING_CHECKING (4) with a still awake neighbour or SLEEPING_DEEPLY
    /// (5) with none.
    fn decide(&self, node: Node) -> AssemblyState {
        if !self.allow_sleep {
            return AssemblyState::Awake;
        }
        let mut neighbour = false;
        for key in self.edges_of(node) {
            if !self.touching_edge(key) {
                continue;
            }
            let Some(other) = self.other(key, node) else {
                continue;
            };
            if self.state(Some(other)) == AssemblyState::Awake && self.in_stage(other) {
                if self.moving(other) {
                    return AssemblyState::Awake;
                }
                neighbour = true;
            }
        }
        // 00b72d00: a joint always touches.
        for j in self.joint_edges_of(node) {
            let Some(other) = self.joint_other(j, node) else {
                continue;
            };
            if self.state(Some(other)) == AssemblyState::Awake && self.in_stage(other) {
                if self.moving(other) {
                    return AssemblyState::Awake;
                }
                neighbour = true;
            }
        }
        if neighbour {
            AssemblyState::SleepingChecking
        } else {
            AssemblyState::SleepingDeeply
        }
    }
    /// 00b70ed6: sample every awake assembly; still ones go to sleep.
    fn sleep_awake(&mut self) -> Result<(), String> {
        let mut deep = Vec::new();
        let mut checking = Vec::new();
        for node in self.nodes() {
            if self.state(Some(node)) != AssemblyState::Awake {
                continue;
            }
            let still = match node {
                Node::Loose(i) => {
                    let part = &mut self.parts[i];
                    let (frame, radius) = (part.body.frame.clone(), part.radius);
                    part.history
                        .step(&self.sleep_profile, || sleep::sample(&frame, radius))
                }
                Node::Character => {
                    let motion = self
                        .character
                        .motion
                        .clone()
                        .ok_or("missing character motion")?;
                    let history = self
                        .character
                        .history
                        .as_mut()
                        .ok_or("missing character history")?;
                    history.step(&self.sleep_profile, || {
                        sleep::sample(&motion.frame, motion.radius)
                    });
                    // Humanoid::onStepped 00c0072c restarts the countdown
                    // every frame (00ad8aa0), so it never reaches 0.
                    false
                }
            };
            if still {
                match self.decide(node) {
                    AssemblyState::SleepingDeeply => deep.push(node),
                    AssemblyState::SleepingChecking => checking.push(node),
                    _ => {}
                }
            }
        }
        for node in deep {
            self.set_state(node, AssemblyState::SleepingDeeply)?;
        }
        for node in checking {
            self.set_state(node, AssemblyState::SleepingChecking)?;
        }
        Ok(())
    }
    /// 00b71158: re-decide SLEEPING_CHECKING assemblies.
    fn recheck(&mut self) -> Result<(), String> {
        let mut wake = Vec::new();
        let mut deep = Vec::new();
        for node in self.nodes() {
            if self.state(Some(node)) != AssemblyState::SleepingChecking {
                continue;
            }
            match self.decide(node) {
                AssemblyState::SleepingDeeply => deep.push(node),
                AssemblyState::Awake => wake.push(node),
                _ => {}
            }
        }
        for node in wake {
            self.set_state(node, AssemblyState::WakePending)?;
        }
        for node in deep {
            self.set_state(node, AssemblyState::SleepingDeeply)?;
        }
        Ok(())
    }
    fn pass_depth(&mut self, node: Node) -> (&mut u32, &mut i32) {
        match node {
            Node::Loose(i) => {
                let p = &mut self.parts[i];
                (&mut p.pass, &mut p.depth)
            }
            Node::Character => (&mut self.character.pass, &mut self.character.depth),
        }
    }
    /// 00b71f08: wake the pending assemblies, breadth first through
    /// touching contacts up to `max_depth` (8 recursive, 0 otherwise).
    fn wake_pending(&mut self, recursive: bool) -> Result<(), String> {
        let (check, max_depth) = if recursive {
            (AssemblyState::RecursiveWakePending, 8)
        } else {
            (AssemblyState::WakePending, 0)
        };
        self.pass += 1;
        let pass = self.pass;
        let mut queue = VecDeque::new();
        for node in self.nodes() {
            if self.state(Some(node)) == check {
                let (p, d) = self.pass_depth(node);
                *p = pass;
                *d = 0;
                queue.push_back(node);
            }
        }
        while let Some(node) = queue.pop_front() {
            self.wake_node(node, max_depth, &mut queue)?;
            *self.pass_depth(node).1 = -1;
        }
        self.wake_through_awake = false;
        Ok(())
    }
    /// 00b7228e.
    fn wake_node(
        &mut self,
        node: Node,
        max_depth: i32,
        queue: &mut VecDeque<Node>,
    ) -> Result<(), String> {
        let depth = *self.pass_depth(node).1;
        if self.state(Some(node)) != AssemblyState::Awake {
            self.set_state(node, AssemblyState::Awake)?;
        }
        for key in self.edges_of(node) {
            // 00b72b82: SLEEPING -> STEPPING, CONTACTING_SLEEPING -> CONTACTING.
            match self.pairs[&key].edge {
                Some(EdgeState::Sleeping) => self.set_edge(key, EdgeState::Stepping)?,
                Some(EdgeState::ContactingSleeping) => self.set_edge(key, EdgeState::Contacting)?,
                _ => {}
            }
            if !self.touching_edge(key) {
                continue;
            }
            let Some(other) = self.other(key, node) else {
                continue;
            };
            if depth < max_depth {
                let pass = self.pass;
                let through =
                    self.wake_through_awake || self.state(Some(other)) != AssemblyState::Awake;
                let (p, d) = self.pass_depth(other);
                if *p != pass && through {
                    *p = pass;
                    *d = depth + 1;
                    queue.push_back(other);
                }
            } else if self.state(Some(other)) == AssemblyState::SleepingDeeply {
                self.set_state(other, AssemblyState::SleepingChecking)?;
            }
        }
        // The joints after the contacts (the native edge list order is not
        // recovered). 00b72b82: a SLEEPING joint becomes AWAKE (back in the
        // kernel); a joint always touches.
        for j in self.joint_edges_of(node) {
            if self.joints[j].edge == Some(JointEdge::Sleeping) {
                self.set_joint_edge(j, JointEdge::Awake)?;
            }
            let Some(other) = self.joint_other(j, node) else {
                continue;
            };
            if depth < max_depth {
                let pass = self.pass;
                let through =
                    self.wake_through_awake || self.state(Some(other)) != AssemblyState::Awake;
                let (p, d) = self.pass_depth(other);
                if *p != pass && through {
                    *p = pass;
                    *d = depth + 1;
                    queue.push_back(other);
                }
            } else if self.state(Some(other)) == AssemblyState::SleepingDeeply {
                self.set_state(other, AssemblyState::SleepingChecking)?;
            }
        }
        Ok(())
    }
    /// The assembly's kernel joints in SleepStage, in creation order.
    fn joint_edges_of(&self, node: Node) -> Vec<usize> {
        (0..self.joints.len())
            .filter(|&j| {
                self.joints[j].edge.is_some()
                    && self.joints[j]
                        .bodies
                        .iter()
                        .any(|b| self.joint_node(*b) == Some(node))
            })
            .collect()
    }
    /// The assembly on the joint's other side (none for a fixed Body).
    fn joint_other(&self, j: usize, node: Node) -> Option<Node> {
        let [a, b] = self.joints[j].bodies.map(|b| self.joint_node(b));
        if a == Some(node) { b } else { a }
    }
    /// The joint pass at the end of SleepStage::step (00b713bc): an AWAKE
    /// joint whose two assemblies are neither waking nor awake (a fixed side
    /// is ANCHORED) goes to SLEEPING and leaves the kernel.
    fn sleep_joints(&mut self) -> Result<(), String> {
        let waking = |s: AssemblyState| {
            matches!(
                s,
                AssemblyState::RecursiveWakePending
                    | AssemblyState::WakePending
                    | AssemblyState::Awake
            )
        };
        let asleep: Vec<usize> = (0..self.joints.len())
            .filter(|&j| {
                self.joints[j].edge == Some(JointEdge::Awake)
                    && self.joints[j]
                        .bodies
                        .iter()
                        .all(|b| !waking(self.state(self.joint_node(*b))))
            })
            .collect();
        for j in asleep {
            self.set_joint_edge(j, JointEdge::Sleeping)?;
        }
        Ok(())
    }
    /// The block of a contact primitive for Contact::step.
    fn step_block(
        &self,
        prim: Prim,
        colliders: &[Block],
        statics: &[crate::world::StaticBlock],
    ) -> Result<Block, String> {
        self.block(prim, statics, colliders)
    }
    /// Contact::step ae2510 -> BlockBlockContact::update: connectors only
    /// while the contact is CONTACTING and touches a kernel body.
    fn step_contact(
        &mut self,
        key: (Prim, Prim),
        colliders: &[Block],
        statics: &[crate::world::StaticBlock],
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<bool, String> {
        let edge = self.pairs.get(&key).and_then(|p| p.edge);
        let kernel_body = |p: Prim| match p {
            Prim::Loose(i) => self.parts[self.prims[i].assembly].in_kernel,
            Prim::Character(_) => self.character.in_stage,
            Prim::Static(_) => false,
        };
        let in_kernel =
            edge == Some(EdgeState::Contacting) && (kernel_body(key.0) || kernel_body(key.1));
        let blocks = [
            self.step_block(key.0, colliders, statics)?,
            self.step_block(key.1, colliders, statics)?,
        ];
        self.update_pair(key, blocks, in_kernel, tables, materials)
    }
    /// stepContacts 00b715ac over one list, then its edge changes in the
    /// native order (to SLEEPING, STEPPING, CONTACTING, CONTACTING_SLEEPING).
    fn step_contacts(
        &mut self,
        touching: bool,
        throttle: usize,
        colliders: &[Block],
        statics: &[crate::world::StaticBlock],
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        let list = if touching {
            self.touching[throttle].clone()
        } else {
            self.stepping[throttle].clone()
        };
        let mut changes: [Vec<(Prim, Prim)>; 4] = Default::default();
        for key in list {
            let awake = |s: AssemblyState| {
                matches!(
                    s,
                    AssemblyState::RecursiveWakePending
                        | AssemblyState::WakePending
                        | AssemblyState::Awake
                )
            };
            let awake_side =
                awake(self.state(self.node(key.0))) || awake(self.state(self.node(key.1)));
            let colliding = self.step_contact(key, colliders, statics, tables, materials)?;
            let current = self.pairs[&key].edge.ok_or("contact outside SleepStage")?;
            // Both primitives collide (pairs exist only for CanCollide
            // Parts, and nothing is dragged).
            let new = if awake_side {
                if colliding {
                    EdgeState::Contacting
                } else {
                    EdgeState::Stepping
                }
            } else if colliding && current == EdgeState::Contacting {
                EdgeState::ContactingSleeping
            } else {
                EdgeState::Sleeping
            };
            if new != current {
                let slot = match new {
                    EdgeState::Sleeping => 0,
                    EdgeState::Stepping => 1,
                    EdgeState::Contacting => 2,
                    EdgeState::ContactingSleeping => 3,
                };
                changes[slot].push(key);
            }
        }
        for (slot, state) in [
            EdgeState::Sleeping,
            EdgeState::Stepping,
            EdgeState::Contacting,
            EdgeState::ContactingSleeping,
        ]
        .into_iter()
        .enumerate()
        {
            for key in std::mem::take(&mut changes[slot]) {
                self.set_edge(key, state)?;
            }
        }
        Ok(())
    }
    /// SleepStage::step 00b707f4 (throttling off, so both throttle lists
    /// step), with the character assembly in the stage when `character` is
    /// given (it is in the Humanoid stage while placed kinematically).
    #[allow(clippy::too_many_arguments)] // The stage's world inputs.
    pub fn sleep_stage(
        &mut self,
        step: u32,
        character: Option<&CharacterMotion>,
        colliders: &[Block],
        statics: &[crate::world::StaticBlock],
        tables: &BlockTables,
        materials: &MaterialProfile,
    ) -> Result<(), String> {
        if statics.len() != self.statics.len() || colliders.len() != self.colliders.len() {
            return Err("SleepStage inputs out of step".into());
        }
        self.step_id = step;
        self.static_blocks = statics
            .iter()
            .map(|b| Block {
                frame: b.frame.clone(),
                size: b.size,
            })
            .collect();
        self.update_membership(character, colliders.len(), statics.len(), materials)?;
        self.wake_pending(true)?;
        self.wake_pending(false)?;
        for t in 0..2 {
            self.step_contacts(false, t, colliders, statics, tables, materials)?;
        }
        self.wake_pending(true)?;
        self.wake_pending(false)?;
        self.sleep_awake()?;
        if step & 3 == 0 {
            self.recheck()?;
        }
        self.wake_pending(false)?;
        for t in 0..2 {
            self.step_contacts(true, t, colliders, statics, tables, materials)?;
        }
        self.wake_pending(true)?;
        self.wake_pending(false)?;
        self.sleep_joints()
    }
    pub fn sleep_profile(&self) -> &SleepProfile {
        &self.sleep_profile
    }
    /// Contacts per edge state, for logs.
    pub fn edge_counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for pair in self.pairs.values() {
            if let Some(edge) = pair.edge {
                counts[edge as usize - 1] += 1;
            }
        }
        counts
    }
    /// One line per part for logs.
    pub fn report(&self) -> String {
        let mut out = String::new();
        for p in &self.parts {
            out.push_str(&format!(
                "[{} sleep={:?} in_kernel={} spread={} list={:?} position={:?} velocity={:?} angular_velocity={:?} up_y={} resting={:?} connectors={}/{}/{}/{}]",
                p.name,
                p.sleep,
                p.in_kernel,
                p.history.spread(),
                p.list,
                p.body.frame.position,
                p.body.velocity,
                p.body.angular_velocity,
                p.body.frame.rotation[1][1],
                p.body.resting,
                p.contact,
                p.joint,
                p.real_time,
                p.total
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::Tensor;
    fn body(position: [f32; 3], mass: f32) -> KernelBody {
        let frame = CoordinateFrame {
            position,
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let inertia: Tensor = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let a = AssemblyProperties {
            mass,
            center_world: position,
            center_in_body: [0.0; 3],
            inertia_in_body: inertia,
            body_frame: frame.clone(),
            parts: 1,
        };
        KernelBody::new(
            &a,
            &FreeFallBody::new(frame, [0.0; 3], [0.0; 3]).unwrap(),
            &KernelProfile::test_profile(),
        )
        .unwrap()
    }
    #[test]
    fn displacement_moves_the_penetration_share_and_its_moment() {
        // Corner (1, -0.5, 0) 0.1 into a static floor: normal from the body
        // into the floor.
        let g = ContactGeometry {
            normal: [0.0, -1.0, 0.0],
            distance: -0.1,
            point: [1.0, -0.5, 0.0],
        };
        let mut a = body([0.0; 3], 2.0);
        displace(&mut a, None, &g, 1.0);
        assert_eq!(a.linear_displacement, [0.0, 0.1, 0.0]);
        assert!((a.angular_displacement[2] - 0.1).abs() < 1e-7);
        displace(&mut a, None, &g, -1.0);
        assert_eq!(a.linear_displacement, [0.0, 0.0, 0.0]);
        // Two bodies split by inverse mass; B moves the other way.
        let mut a = body([0.0; 3], 1.0);
        let mut b = body([0.0, -1.0, 0.0], 3.0);
        displace(&mut a, Some(&mut b), &g, 1.0);
        assert!((a.linear_displacement[1] - 0.075).abs() < 1e-7);
        assert!((b.linear_displacement[1] + 0.025).abs() < 1e-7);
    }
}
