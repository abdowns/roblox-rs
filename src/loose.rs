//! Unanchored Parts in the legacy kernel: SimBody lists, contact connector
//! lists and the throttling-body half of Kernel::step. No sleep stage.
//!
//! Recovered from:
//! - body classification 00a9be30 and list removal 00a9c142 (ab1c28 when a
//!   body leaves free fall),
//! - connector insert 00a99514 and remove 00a99ff2,
//! - contact displacement 00a9457c / 00a942ee,
//! - Kernel::step 00a96588: pre-step 00a96eb0, then stepWorld 00a9768c,
//! - connector bodies a92c26 (the first body in the kernel is A; the normal is
//!   negated when only the second one is).
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile},
    block_contact::{Block, BlockBlockContact, BlockTables, ContactEvent, ContactPoint},
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
use std::collections::BTreeMap;

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
    Joint,
    Contact,
}

/// A primitive taking part in a contact pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prim {
    Loose(usize),
    Static(usize),
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
    /// SimBody +0x110 total, +0x120 joint, +0x128 contact connector counts.
    total: u32,
    joint: u32,
    contact: u32,
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
        }
    }
}

#[derive(Clone, Debug)]
struct Pair {
    contact: BlockBlockContact<Link>,
    parameters: ContactParameters,
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
    free_fall: Vec<usize>,
    real_time: Vec<usize>,
    joint: Vec<usize>,
    contact_bodies: Vec<usize>,
    pairs: BTreeMap<(Prim, Prim), Pair>,
    contact_connectors: Vec<u64>,
    joint_connectors: Vec<u64>,
    owners: BTreeMap<u64, (Prim, Prim)>,
    next_id: u64,
    profile: StepProfile,
    mass: MassProfile,
    pub stats: StepStats,
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
            free_fall: Vec::new(),
            real_time: Vec::new(),
            joint: Vec::new(),
            contact_bodies: Vec::new(),
            pairs: BTreeMap::new(),
            contact_connectors: Vec::new(),
            joint_connectors: Vec::new(),
            owners: BTreeMap::new(),
            next_id: 0,
            profile: StepProfile::load(bundle)?,
            mass: MassProfile::load(bundle)?,
            stats: StepStats::default(),
        })
    }
    /// A static (anchored, colliding) block, in Statics order.
    pub fn add_static(&mut self, properties: PhysicalProperties, stiffness: f32) {
        self.statics.push((properties, stiffness));
    }
    /// Add an unanchored Part at rest. Its SimBody enters the kernel with no
    /// connectors, so 00a9be30 files it under freeFallBodies.
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
        let index = self.parts.len();
        self.parts.push(LoosePart {
            name: part.name.clone(),
            size: part.size,
            body: KernelBody::new(&assembly, &state, kernel)?,
            list: BodyList::FreeFall,
            can_collide: part.can_collide,
            properties,
            stiffness: materials.stiffness(part.size, Shape::Block)?,
            total: 0,
            joint: 0,
            contact: 0,
        });
        self.enter(index, BodyList::FreeFall);
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
        self.contact_connectors.clear();
        self.joint_connectors.clear();
        self.owners.clear();
        self.stats = StepStats::default();
        for i in 0..self.parts.len() {
            self.parts[i].list = BodyList::FreeFall;
            self.enter(i, BodyList::FreeFall);
        }
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
        let target = if p.joint < 1 {
            if p.contact > 0 {
                BodyList::Contact
            } else if p.total == 0 {
                BodyList::FreeFall
            } else {
                BodyList::Joint
            }
        } else {
            BodyList::Joint
        };
        if p.list == target {
            return Ok(());
        }
        self.leave(index)?;
        self.enter(index, target);
        Ok(())
    }
    fn block(&self, prim: Prim, statics: &[crate::world::StaticBlock]) -> Result<Block, String> {
        Ok(match prim {
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
        };
        let (pa, sa) = props(a);
        let (pb, sb) = props(b);
        materials.contact_params(&pa, sa, &pb, sb).parameters()
    }
    /// The connector's bodies as a92c26 picks them: (A, B, flip normal).
    fn bodies(first: Prim, second: Prim) -> Option<(usize, Option<usize>, bool)> {
        match (first, second) {
            (Prim::Loose(a), Prim::Loose(b)) => Some((a, Some(b), false)),
            (Prim::Loose(a), Prim::Static(_)) => Some((a, None, false)),
            (Prim::Static(_), Prim::Loose(b)) => Some((b, None, true)),
            (Prim::Static(_), Prim::Static(_)) => None,
        }
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
        let Some((a, b, flip)) = Self::bodies(prims.0, prims.1) else {
            return Ok(());
        };
        let g = geometry(point, flip);
        let (a, b) = two_mut(&mut self.parts, a, b)?;
        displace(&mut a.body, b.map(|b| &mut b.body), &g, sign);
        Ok(())
    }
    /// Connector insert 00a99514 for a CONTACT connector between throttling
    /// bodies: jointConnectors when either body is in jointBodies, else
    /// contactConnectors; then both bodies are reclassified.
    fn insert(&mut self, id: u64, prims: (Prim, Prim)) -> Result<Option<ConnectorList>, String> {
        let loose: Vec<usize> = [prims.0, prims.1]
            .iter()
            .filter_map(|p| match p {
                Prim::Loose(i) => Some(*i),
                Prim::Static(_) => None,
            })
            .collect();
        if loose.is_empty() {
            return Ok(None);
        }
        let list = if loose.iter().any(|i| self.parts[*i].list == BodyList::Joint) {
            ConnectorList::Joint
        } else {
            ConnectorList::Contact
        };
        match list {
            ConnectorList::Joint => self.joint_connectors.push(id),
            ConnectorList::Contact => self.contact_connectors.push(id),
        }
        for i in &loose {
            let p = &mut self.parts[*i];
            p.total += 1;
            match list {
                ConnectorList::Joint => p.joint += 1,
                ConnectorList::Contact => p.contact += 1,
            }
        }
        for i in loose {
            self.classify(i)?;
        }
        Ok(Some(list))
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
            ConnectorList::Joint => remove_item(&mut self.joint_connectors, &id)?,
            ConnectorList::Contact => remove_item(&mut self.contact_connectors, &id)?,
        }
        let loose: Vec<usize> = [prims.0, prims.1]
            .iter()
            .filter_map(|p| match p {
                Prim::Loose(i) => Some(*i),
                Prim::Static(_) => None,
            })
            .collect();
        for i in &loose {
            let p = &mut self.parts[*i];
            p.total -= 1;
            match list {
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
        for key in keys {
            let blocks = [self.block(key.0, statics)?, self.block(key.1, statics)?];
            if !self.pairs.contains_key(&key) {
                let parameters = self.pair_parameters(key.0, key.1, materials);
                self.pairs.insert(
                    key,
                    Pair {
                        contact: BlockBlockContact::default(),
                        parameters,
                    },
                );
            }
            let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
            pair.contact
                .update([&blocks[0], &blocks[1]], true, tables)?;
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
                            let list = self.insert(id, prims)?;
                            let link =
                                &mut self.pairs.get_mut(&key).unwrap().contact.connectors_mut()
                                    [index]
                                    .state;
                            link.id = id;
                            link.list = list;
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
        }
        Ok(())
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
        let Some((a, b, flip)) = Self::bodies(prims.0, prims.1) else {
            return Ok(0.0);
        };
        let g = geometry(&c.point, flip);
        let fixed = match (prims.0, prims.1) {
            (Prim::Static(s), _) | (_, Prim::Static(s)) => {
                let s = statics.get(s).ok_or("missing static block")?;
                ContactBodyState {
                    center: s.frame.position,
                    velocity: [0.0; 3],
                    angular_velocity: [0.0; 3],
                }
            }
            _ => ContactBodyState {
                center: [0.0; 3],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
            },
        };
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
    /// The substep half of stepWorld for loose bodies: jointConnectors
    /// springs, then realTimeBodies and jointBodies integrate (ab1d44).
    pub fn substep(&mut self, kernel: &KernelProfile) -> Result<(), String> {
        for at in 0..self.joint_connectors.len() {
            let id = self.joint_connectors[at];
            let key = *self.owners.get(&id).ok_or("missing connector owner")?;
            let pair = self.pairs.get_mut(&key).ok_or("missing contact pair")?;
            let parameters = pair.parameters;
            let c = pair
                .contact
                .connectors_mut()
                .iter_mut()
                .find(|c| c.state.id == id)
                .ok_or("missing kernel connector")?;
            let prims = Self::connector_prims(key, c.pair.body_a);
            let Some((a, b, flip)) = Self::bodies(prims.0, prims.1) else {
                continue;
            };
            let _ = (a, b, flip, parameters);
            return Err("loose jointConnectors springs are not ported yet".into());
        }
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
    /// One line per part for logs.
    pub fn report(&self) -> String {
        let mut out = String::new();
        for p in &self.parts {
            out.push_str(&format!(
                "[{} list={:?} position={:?} velocity={:?} angular_velocity={:?} up_y={} resting={:?} connectors={}/{}]",
                p.name,
                p.list,
                p.body.frame.position,
                p.body.velocity,
                p.body.angular_velocity,
                p.body.frame.rotation[1][1],
                p.body.resting,
                p.contact,
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
