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
//! The broadphase (multi-resolution spatial hash 00aecaea) is not ported:
//! every colliding (loose, static), (loose, loose) and (character, loose)
//! pair is a contact for the whole run.
use crate::sleep::{self, AssemblyState, MotionHistory, SleepProfile};
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile},
    block_contact::{
        Block, BlockBlockContact, BlockTables, ContactEvent, ContactPoint, extents_overlap,
    },
    bundle::Bundle,
    collision::ContactBodyState,
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile},
    legacy_contact::{AwakeContact, ContactGeometry, ImpulseInput, ImpulseProfile},
    material::{MaterialProfile, PhysicalProperties, Shape},
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

/// An unanchored Part: its own single-Part assembly and SimBody.
#[derive(Clone, Debug)]
pub struct LoosePart {
    pub name: String,
    pub size: [f32; 3],
    pub body: KernelBody,
    pub list: BodyList,
    can_collide: bool,
    properties: PhysicalProperties,
    stiffness: f32,
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
    /// RunningBase friction factor when the Part is a Humanoid floor.
    floor_friction: f32,
    /// Part.Transparency.
    transparency: f32,
}

impl LoosePart {
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
    pub parts: Vec<LoosePart>,
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
    /// Add an unanchored Part at rest. SleepStage::onAssemblyAdded 00b73820
    /// makes it WAKE_PENDING with a fresh history; the first wakePending
    /// pass enters it into the kernel, where 00a9be30 files it under
    /// freeFallBodies (no connectors yet).
    pub fn add_part(
        &mut self,
        part: &PartInstance,
        materials: &MaterialProfile,
        kernel: &KernelProfile,
    ) -> Result<(), String> {
        let properties = materials.defaults(part.material)?;
        let mass = (part.size[0] * part.size[1]) * part.size[2] * properties.density;
        let inertia = self.mass.block_inertia(part.size, mass)?;
        let assembly = AssemblyProperties::combine(
            &MassPart {
                name: part.name.clone(),
                frame: part.frame.clone(),
                mass,
                inertia,
            },
            &[],
        )?;
        let state = FreeFallBody::new(part.frame.clone(), [0.0; 3], [0.0; 3])?;
        // A single-Part assembly: the root primitive is the Part itself.
        let radius = sleep::primitive_radius(part.frame.position, part.frame.position, part.size);
        self.parts.push(LoosePart {
            name: part.name.clone(),
            size: part.size,
            body: KernelBody::new(&assembly, &state, kernel)?,
            list: BodyList::FreeFall,
            can_collide: part.can_collide,
            properties,
            stiffness: materials.stiffness(part.size, Shape::Block)?,
            total: 0,
            real_time: 0,
            joint: 0,
            contact: 0,
            in_kernel: false,
            sleep: AssemblyState::WakePending,
            history: MotionHistory::new(sleep::sample(&part.frame, radius)),
            radius,
            pass: 0,
            depth: -1,
            floor_friction: materials.floor_friction(&properties)?,
            transparency: part.transparency,
        });
        self.original = self.parts.clone();
        Ok(())
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
        self.joint_connectors.clear();
        self.owners.clear();
        self.stats = StepStats::default();
        self.stepping = [Vec::new(), Vec::new()];
        self.touching = [Vec::new(), Vec::new()];
        self.character = CharacterNode::default();
        self.pass = 0;
        self.transitions.clear();
        self.step_id = 0;
    }
    pub fn frames(&self) -> Vec<(String, CoordinateFrame)> {
        self.parts
            .iter()
            .map(|p| (p.name.clone(), p.body.frame.clone()))
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
            Prim::Loose(i) => {
                let p = self.parts.get(i).ok_or("missing loose part")?;
                Block {
                    frame: p.body.frame.clone(),
                    size: p.size,
                }
            }
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
            Prim::Loose(i) => (self.parts[i].properties, self.parts[i].stiffness),
            Prim::Static(i) => self.statics[i],
            Prim::Character(i) => self.colliders[i],
        };
        let (pa, sa) = props(a);
        let (pb, sb) = props(b);
        materials.contact_params(&pa, sa, &pb, sb).parameters()
    }
    /// The kernel loose bodies of a connector between throttling bodies,
    /// sleeping Parts and static blocks, as a92c26 picks them: (A, B, flip
    /// normal); a primitive outside the kernel is fixed. None for a
    /// connector touching the character or with no kernel body.
    fn bodies(&self, first: Prim, second: Prim) -> Option<(usize, Option<usize>, bool)> {
        let kernel = |p: Prim| match p {
            Prim::Loose(i) if self.parts[i].in_kernel => Some(i),
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
    /// A primitive outside the kernel as a fixed contact body.
    fn fixed_state(
        &self,
        prim: Prim,
        statics: &[crate::world::StaticBlock],
    ) -> Result<ContactBodyState, String> {
        let center = match prim {
            Prim::Static(s) => statics.get(s).ok_or("missing static block")?.frame.position,
            Prim::Loose(i) => self.parts.get(i).ok_or("missing loose part")?.body.center,
            Prim::Character(_) => return Err("the character is never a fixed body".into()),
        };
        Ok(ContactBodyState {
            center,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        })
    }
    fn loose_of(prims: (Prim, Prim)) -> Vec<usize> {
        [prims.0, prims.1]
            .iter()
            .filter_map(|p| match p {
                Prim::Loose(i) => Some(*i),
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
        let loose: Vec<usize> = Self::loose_of(prims)
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
        for i in 0..self.parts.len() {
            if !self.parts[i].can_collide {
                continue;
            }
            for j in 0..statics.len() {
                keys.push((Prim::Loose(i), Prim::Static(j)));
            }
            for k in i + 1..self.parts.len() {
                if self.parts[k].can_collide {
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
            for i in 0..self.parts.len() {
                if !self.parts[i].can_collide {
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
        for i in 0..self.parts.len() {
            if !self.parts[i].can_collide {
                continue;
            }
            let key = (Prim::Character(collider), Prim::Loose(i));
            let other = self.block(key.1, &[], &[])?;
            if !self.pairs.contains_key(&key) {
                self.create_pair(key, materials);
            }
            if !extents_overlap(&block.extents(), &other.extents()) {
                continue;
            }
            let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
            if pair.contact.compute_is_colliding([block, &other], 0.0).0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Forget the character's contact caches (the character returned to the
    /// kernel). Pairs that still hold connectors are kept.
    pub fn clear_character_contacts(&mut self) {
        self.pairs.retain(|key, pair| {
            !matches!(key.0, Prim::Character(_))
                || !pair.contact.connectors().is_empty()
                || pair.edge.is_some()
        });
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
            Prim::Loose(i) => {
                let p = self.parts.get(i).ok_or("missing loose part")?;
                Block {
                    frame: p.body.frame.clone(),
                    size: p.size,
                }
            }
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
            let (loose, character_first) = match prims {
                (Prim::Character(_), Prim::Loose(i)) => (i, true),
                (Prim::Loose(i), Prim::Character(_)) => (i, false),
                _ => return Err("unexpected realtime connector".into()),
            };
            let part = self.parts[loose].body.contact_state();
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
    /// jointConnectors springs (a93074) between loose Parts and static
    /// blocks, in list order.
    pub fn joint_springs(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        for at in 0..self.joint_connectors.len() {
            let id = self.joint_connectors[at];
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
                    Prim::Loose(i) => self.parts.get(i).ok_or("missing loose part")?.body.center,
                    Prim::Character(_) => return Err("unexpected joint connector".into()),
                };
                Ok(ContactBodyState {
                    center,
                    velocity: [0.0; 3],
                    angular_velocity: [0.0; 3],
                })
            };
            // A is a kernel body; B is the other loose body or the block.
            let state_a = self.parts[a].body.contact_state();
            let state_b = match b {
                Some(b) => self.parts[b].body.contact_state(),
                None => fixed(if flip { prims.0 } else { prims.1 })?,
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
    /// The substep half of stepWorld without the character: jointConnectors
    /// springs, then realTimeBodies and jointBodies integrate.
    pub fn substep(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        self.joint_springs(kernel)?;
        self.integrate(kernel)
    }
    /// The colliding (loose, static) and (loose, loose) pairs in the
    /// update_contacts order.
    fn loose_keys(&self, statics: usize) -> Vec<(Prim, Prim)> {
        let mut keys = Vec::new();
        for i in 0..self.parts.len() {
            if !self.parts[i].can_collide {
                continue;
            }
            for j in 0..statics {
                keys.push((Prim::Loose(i), Prim::Static(j)));
            }
            for k in i + 1..self.parts.len() {
                if self.parts[k].can_collide {
                    keys.push((Prim::Loose(i), Prim::Loose(k)));
                }
            }
        }
        keys
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
        for key in self.loose_keys(statics) {
            if !self.pairs.contains_key(&key) {
                self.create_pair(key, materials);
            }
            self.add_edge(key)?;
        }
        let keys: Vec<(Prim, Prim)> = (0..colliders)
            .flat_map(|c| (0..self.parts.len()).map(move |i| (c, i)))
            .filter(|(_, i)| self.parts[*i].can_collide)
            .map(|(c, i)| (Prim::Character(c), Prim::Loose(i)))
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
                    if !self.pairs.contains_key(&key) {
                        self.create_pair(key, materials);
                    }
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
    fn node(prim: Prim) -> Option<Node> {
        match prim {
            Prim::Loose(i) => Some(Node::Loose(i)),
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
                    && (Self::node(key.0) == Some(node) || Self::node(key.1) == Some(node))
            })
            .map(|(key, _)| *key)
            .collect()
    }
    fn other(key: (Prim, Prim), node: Node) -> Option<Node> {
        if Self::node(key.0) == Some(node) {
            Self::node(key.1)
        } else {
            Self::node(key.0)
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
            if let Some(node) = Self::node(prim) {
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
    /// 00b7361a: either primitive's assembly moves faster than 1 stud/s or
    /// spins faster than 1 stud/s at its radius.
    fn fast(&self, key: (Prim, Prim)) -> bool {
        [key.0, key.1].into_iter().any(|prim| {
            let (v, w, r) = match prim {
                Prim::Static(_) => return false,
                Prim::Loose(i) => {
                    let p = &self.parts[i];
                    (p.body.velocity, p.body.angular_velocity, p.radius)
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
                    if let Some(node) = Self::node(prim) {
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
    /// Re-bind the connectors of every contact of a Part whose SimBody
    /// enters or leaves the kernel. The native re-insert of live connectors
    /// is not recovered: they are deleted, and Contact::step regenerates
    /// them for touching contacts.
    fn drop_part_connectors(&mut self, index: usize) -> Result<(), String> {
        let keys: Vec<(Prim, Prim)> = self
            .pairs
            .iter()
            .filter(|(k, p)| {
                (k.0 == Prim::Loose(index) || k.1 == Prim::Loose(index))
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
                    // b6ee0a -> Kernel insertBody 00a957ce: classify 00a9be30.
                    self.drop_part_connectors(i)?;
                    self.parts[i].in_kernel = true;
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
            let Some(other) = Self::other(key, node) else {
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
            let Some(other) = Self::other(key, node) else {
                continue;
            };
            if depth < max_depth {
                let pass = self.pass;
                let state = self.state(Some(other));
                let (p, d) = self.pass_depth(other);
                if *p != pass && state != AssemblyState::Awake {
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
            Prim::Loose(i) => self.parts[i].in_kernel,
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
                awake(self.state(Self::node(key.0))) || awake(self.state(Self::node(key.1)));
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
        self.wake_pending(false)
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
