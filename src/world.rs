//! RBX::World subset: one character assembly, the static contents, realtime
//! contact springs, and kernel substeps with a per-substep force callback
//! (the Humanoid state's kernel connector, vtable +0xc). The static contents
//! are either the walking fixture (floor plane, wall half-space) or the
//! anchored Parts of a Workspace (`World::in_workspace`).
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile, Tensor},
    block_contact::{Block, BlockBlockContact, BlockTables, Connector},
    broadphase::{Event, Extents, SpatialHash, block_extents},
    bundle::Bundle,
    collision::ContactBodyState,
    humanoid_state::{FloorPart, Hit},
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile, cross},
    legacy_contact::{BlockContactProfile, ContactGeometry, ImpulseProfile},
    loose::{CharacterMotion, LoosePart, LooseWorld},
    material::{MaterialProfile, Shape},
    physics,
    rig::{R6Rig, compose},
    scene::{CoordinateFrame, Scene},
    workspace::{Workspace, character_placement},
};
use std::collections::BTreeMap;

#[derive(Default)]
struct ContactState {
    spring: ContactSpring,
}

/// A primitive in the broadphase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BroadPrim {
    Static(usize),
    Loose(usize),
    /// A character Part (index into the character's Part list).
    Character(usize),
}

/// The World's SpatialHash and what ContactManager::createContact
/// (00ae9612) needs about each primitive.
#[derive(Clone, Default)]
pub struct Broadphase {
    pub hash: SpatialHash,
    pub prims: Vec<BroadPrim>,
    /// CanCollide (00ae714a makes no contact otherwise).
    collides: Vec<bool>,
    loose_ids: Vec<usize>,
    character_ids: Vec<usize>,
    /// Every contact change: (world step, event).
    pub log: Vec<(u32, Event)>,
}

impl Broadphase {
    fn add(&mut self, prim: BroadPrim, collides: bool) -> usize {
        let id = self.prims.len();
        self.prims.push(prim);
        self.collides.push(collides);
        match prim {
            BroadPrim::Loose(_) => self.loose_ids.push(id),
            BroadPrim::Character(_) => self.character_ids.push(id),
            BroadPrim::Static(_) => {}
        }
        id
    }
    /// 00ae9612: not in the same assembly, not both grounded, and both
    /// collide.
    fn accept(prims: &[BroadPrim], collides: &[bool], a: usize, b: usize) -> bool {
        let same = match (prims[a], prims[b]) {
            (BroadPrim::Static(_), BroadPrim::Static(_)) => true,
            (BroadPrim::Character(_), BroadPrim::Character(_)) => true,
            (BroadPrim::Loose(i), BroadPrim::Loose(j)) => i == j,
            _ => false,
        };
        !same && collides[a] && collides[b]
    }
    fn drain(&mut self, step: u32) {
        for event in self.hash.events.drain(..) {
            self.log.push((step, event));
        }
    }
    fn insert(&mut self, id: usize, extents: Extents, fixed: bool) {
        let (prims, collides) = (&self.prims, &self.collides);
        self.hash.insert(id, extents, fixed, false, |a, b| {
            Self::accept(prims, collides, a, b)
        });
    }
    fn find_all(&mut self, id: usize, extents: Extents, step: u32) {
        let (prims, collides) = (&self.prims, &self.collides);
        self.hash
            .find_all_contacts(id, extents, |a, b| Self::accept(prims, collides, a, b));
        self.drain(step);
    }
    fn update(&mut self, id: usize, extents: Extents, step: u32) {
        let (prims, collides) = (&self.prims, &self.collides);
        self.hash
            .update(id, extents, |a, b| Self::accept(prims, collides, a, b));
        self.drain(step);
    }
    /// The primitive pairs with a contact now.
    pub fn contact_pairs(&self) -> Vec<(BroadPrim, BroadPrim)> {
        let mut out = Vec::new();
        for (a, prim) in self.prims.iter().enumerate() {
            for &b in self.hash.contacts_of(a) {
                if a < b {
                    out.push((*prim, self.prims[b]));
                }
            }
        }
        out
    }
}

/// PopperCam.lua PiercingCast: `hitPart.Transparency > 0.95` is pierced.
const POPPER_PIERCE_TRANSPARENCY: f32 = 0.95;

/// An anchored, colliding block Part in the world.
#[derive(Clone, Debug)]
pub struct StaticBlock {
    pub name: String,
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
}

/// G3D::CollisionDetection::collisionLocationForMovingPointFixedAABox
/// (0124e766, Woo's ray/box test). A ray starting inside the box misses.
/// The normal is -1/+1 on the hit axis (table 0177ed50) by direction sign.
fn moving_point_fixed_aabox(
    origin: [f32; 3],
    direction: [f32; 3],
    low: [f32; 3],
    high: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let mut inside = true;
    let mut max_t = [-1.0f32; 3];
    let mut location = [0.0f32; 3];
    for i in 0..3 {
        let plane = if low[i] <= origin[i] {
            if high[i] < origin[i] {
                high[i]
            } else {
                continue;
            }
        } else {
            low[i]
        };
        location[i] = plane;
        inside = false;
        if direction[i] != 0.0 {
            max_t[i] = (plane - origin[i]) / direction[i];
        }
    }
    if inside {
        return None;
    }
    let first = usize::from(max_t[0] < max_t[1]);
    let which = if max_t[2] <= max_t[first] { first } else { 2 };
    let t = max_t[which];
    // Sign-bit test on the float bits, as G3D's IR(MaxT) & 0x80000000.
    if t.to_bits() & 0x8000_0000 != 0 {
        return None;
    }
    for i in 0..3 {
        if i != which {
            location[i] = direction[i] * t + origin[i];
            if location[i] < low[i] || (high[i] <= location[i] && location[i] != high[i]) {
                return None;
            }
        }
    }
    let mut normal = [0.0; 3];
    normal[which] = if 0.0 < direction[which] { -1.0 } else { 1.0 };
    Some((location, normal))
}

/// Primitive hitTest (00b631de) with Block geometry (vtable +0x70,
/// 00adeffe): the unit ray in part space against the centered box, the hit
/// point and normal back in world space.
fn box_hit_test(
    frame: &CoordinateFrame,
    size: [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<Hit> {
    let r = &frame.rotation;
    let t = frame.position;
    let p: [f32; 3] = std::array::from_fn(|i| origin[i] - t[i]);
    let local_origin: [f32; 3] =
        std::array::from_fn(|i| r[0][i] * p[0] + r[1][i] * p[1] + r[2][i] * p[2]);
    let local_direction: [f32; 3] = std::array::from_fn(|i| {
        r[0][i] * direction[0] + r[1][i] * direction[1] + r[2][i] * direction[2]
    });
    let half = size.map(|s| s * 0.5);
    let (hit, normal) =
        moving_point_fixed_aabox(local_origin, local_direction, half.map(|h| -h), half)?;
    Some((
        std::array::from_fn(|i| r[i][2] * hit[2] + r[i][1] * hit[1] + r[i][0] * hit[0] + t[i]),
        std::array::from_fn(|i| r[i][2] * normal[2] + r[i][1] * normal[1] + r[i][0] * normal[0]),
    ))
}

/// One getSlowHit (00ae7d08) candidate: the bounding-sphere test (vtable
/// +0x10 radius), the hit test, then a positive distance strictly below the
/// best so far.
fn slow_hit_candidate(
    frame: &CoordinateFrame,
    size: [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
    best: f32,
) -> Option<(f32, Hit)> {
    let c = frame.position;
    let along = (c[2] - origin[2]) * direction[2]
        + (c[1] - origin[1]) * direction[1]
        + (c[0] - origin[0]) * direction[0];
    let off: [f32; 3] = std::array::from_fn(|i| c[i] - (direction[i] * along + origin[i]));
    let h = size.map(|s| s * 0.5);
    let radius = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
    if (off[2] * off[2] + off[1] * off[1] + off[0] * off[0]).sqrt() > radius {
        return None;
    }
    let (point, normal) = box_hit_test(frame, size, origin, direction)?;
    let distance = (point[2] - origin[2]) * direction[2]
        + (point[1] - origin[1]) * direction[1]
        + (point[0] - origin[0]) * direction[0];
    (0.0 < distance && distance < best).then_some((distance, (point, normal)))
}

impl StaticBlock {
    #[cfg(test)]
    fn hit_test(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<Hit> {
        box_hit_test(&self.frame, self.size, origin, direction)
    }
    fn corners(frame: &CoordinateFrame, size: [f32; 3]) -> [[f32; 3]; 8] {
        std::array::from_fn(|i| {
            frame.transform_point(std::array::from_fn(|a| {
                (if i & (1 << a) != 0 { 0.5 } else { -0.5 }) * size[a]
            }))
        })
    }
    /// Separating-axis overlap of two oriented boxes.
    fn overlaps(&self, frame: &CoordinateFrame, size: [f32; 3]) -> bool {
        let column =
            |f: &CoordinateFrame, c: usize| [f.rotation[0][c], f.rotation[1][c], f.rotation[2][c]];
        let mut axes = Vec::with_capacity(15);
        for c in 0..3 {
            axes.push(column(&self.frame, c));
            axes.push(column(frame, c));
        }
        for a in 0..3 {
            for b in 0..3 {
                axes.push(cross(column(&self.frame, a), column(frame, b)));
            }
        }
        let mine = Self::corners(&self.frame, self.size);
        let theirs = Self::corners(frame, size);
        let project = |points: &[[f32; 3]; 8], axis: [f32; 3]| {
            points
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                    let d = p[0] * axis[0] + p[1] * axis[1] + p[2] * axis[2];
                    (lo.min(d), hi.max(d))
                })
        };
        axes.into_iter()
            .filter(|a| a[0] * a[0] + a[1] * a[1] + a[2] * a[2] > 1e-12)
            .all(|axis| {
                let (a0, a1) = project(&mine, axis);
                let (b0, b1) = project(&theirs, axis);
                a0 <= b1 && b0 <= a1
            })
    }
}

/// Static contents and the collision queries the Humanoid makes against
/// them (floor/ceiling rays, PopperCam, touch tests). The fixture planes are
/// disabled (infinite) in a Workspace world.
pub struct Statics {
    pub floor_y: f32,
    pub wall_x: f32,
    /// Anchored, colliding Workspace blocks (empty in the walking fixture).
    pub blocks: Vec<StaticBlock>,
    geometry: BlockContactProfile,
    tables: BlockTables,
    /// RunningBase floor friction of each block, and of the fixture planes
    /// (a Plastic stand-in).
    floor_friction: Vec<f32>,
    fixture_friction: f32,
    /// Part.Transparency of each block (PopperCam pierces above 0.95).
    transparency: Vec<f32>,
}

impl Statics {
    pub fn wall_enabled(&self) -> bool {
        self.wall_x.is_finite()
    }
    /// ContactManager::getSlowHit (00ae7d08) over the static blocks: bounding
    /// sphere test, block hit test, then the nearest hit with a positive
    /// distance along the ray strictly below min(length, 5000). The spatial
    /// grid walk of getFastHit (00ae8274) only orders candidates.
    fn block_hit(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
    ) -> Option<(f32, Hit, FloorPart)> {
        self.block_hit_where(origin, direction, length, |_| true)
    }
    /// `block_hit` over the blocks the hit filter accepts.
    fn block_hit_where(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
        accept: impl Fn(usize) -> bool,
    ) -> Option<(f32, Hit, FloorPart)> {
        let mut best = length.min(5000.0);
        let mut result = None;
        for (index, block) in self.blocks.iter().enumerate() {
            if !accept(index) {
                continue;
            }
            if let Some((distance, hit)) =
                slow_hit_candidate(&block.frame, block.size, origin, direction, best)
            {
                best = distance;
                result = Some((distance, hit, FloorPart::Static(index)));
            }
        }
        result
    }
    /// Floor plane (solid below), wall half-space and the static blocks.
    pub fn raycast(&self, origin: [f32; 3], direction: [f32; 3], length: f32) -> Option<Hit> {
        self.raycast_part(origin, direction, length)
            .map(|(_, hit, _)| hit)
    }
    /// `raycast` with the distance and the Part hit.
    fn raycast_part(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
    ) -> Option<(f32, Hit, FloorPart)> {
        let best = self.block_hit(origin, direction, length);
        self.fixture_hit(origin, direction, length, best)
    }
    /// The fixture floor plane and wall against the best hit so far.
    fn fixture_hit(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
        mut best: Option<(f32, Hit, FloorPart)>,
    ) -> Option<(f32, Hit, FloorPart)> {
        if direction[1] < 0.0 && origin[1] > self.floor_y {
            let t = (origin[1] - self.floor_y) / -direction[1];
            if t <= length && best.as_ref().is_none_or(|(b, _, _)| t < *b) {
                best = Some((
                    t,
                    (
                        [
                            origin[0] + direction[0] * t,
                            self.floor_y,
                            origin[2] + direction[2] * t,
                        ],
                        [0.0, 1.0, 0.0],
                    ),
                    FloorPart::Fixture,
                ));
            }
        }
        if direction[0] > 0.0 && origin[0] < self.wall_x {
            let t = (self.wall_x - origin[0]) / direction[0];
            if t <= length && best.as_ref().is_none_or(|(b, _, _)| t < *b) {
                best = Some((
                    t,
                    (
                        [
                            self.wall_x,
                            origin[1] + direction[1] * t,
                            origin[2] + direction[2] * t,
                        ],
                        [-1.0, 0.0, 0.0],
                    ),
                    FloorPart::Fixture,
                ));
            }
        }
        best
    }
    pub fn wall(&self) -> CoordinateFrame {
        CoordinateFrame {
            position: [self.wall_x, self.floor_y, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
    /// Stand-in for a touching kernel contact: the part generates a
    /// vertex/plane connector against the fixture wall, or overlaps a static
    /// block (until the BlockBlockContact narrowphase is ported).
    pub fn touching(&self, frame: &CoordinateFrame, size: [f32; 3]) -> Result<bool, String> {
        if self.blocks.iter().any(|b| b.overlaps(frame, size)) {
            return Ok(true);
        }
        Ok(self.wall_enabled()
            && !self
                .geometry
                .plane_vertices(frame, size, &self.wall(), 3)?
                .is_empty())
    }
}

/// The Humanoid ray query (getHit 00ae818c through the HumanoidState filter
/// 00c02844): every primitive in the ContactManager, anchored or not, except
/// CanCollide-false Parts (+0xcc), dragged Parts (+0xca) and the character's
/// own assembly. Static blocks are offered before the loose Parts and the
/// nearest hit wins; getFastHit's cell order (which only matters for exact
/// ties) is not recovered. The fixture planes come last, as before.
fn raycast_all(
    statics: &Statics,
    loose: Option<&LooseWorld>,
    origin: [f32; 3],
    direction: [f32; 3],
    length: f32,
) -> Option<(Hit, FloorPart)> {
    raycast_where(
        statics,
        loose,
        origin,
        direction,
        length,
        |_| true,
        LoosePart::can_collide,
    )
}

/// The getHit walk with a hit filter for static blocks and loose Parts.
fn raycast_where(
    statics: &Statics,
    loose: Option<&LooseWorld>,
    origin: [f32; 3],
    direction: [f32; 3],
    length: f32,
    accept_static: impl Fn(usize) -> bool,
    accept_loose: impl Fn(&LoosePart) -> bool,
) -> Option<(Hit, FloorPart)> {
    let mut best = statics.block_hit_where(origin, direction, length, accept_static);
    if let Some(loose) = loose {
        let mut limit = best.as_ref().map_or(length.min(5000.0), |b| b.0);
        for (index, part) in loose.parts.iter().enumerate() {
            if !accept_loose(part) {
                continue;
            }
            if let Some((distance, hit)) =
                slow_hit_candidate(&part.body.frame, part.size, origin, direction, limit)
            {
                limit = distance;
                best = Some((distance, hit, FloorPart::Loose(index)));
            }
        }
    }
    statics
        .fixture_hit(origin, direction, length, best)
        .map(|(_, hit, part)| (hit, part))
}

/// What the Humanoid's kernel force callback reaches besides its own body:
/// the static contents and, in a World, the loose Parts (floor mass and the
/// floor reaction).
pub struct Surroundings<'a> {
    pub statics: &'a Statics,
    pub loose: Option<&'a mut LooseWorld>,
}

impl Surroundings<'_> {
    /// The Humanoid ray query (see `World::raycast`).
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
    ) -> Option<(Hit, FloorPart)> {
        raycast_all(
            self.statics,
            self.loose.as_deref(),
            origin,
            direction,
            length,
        )
    }
    /// The floor's assembly root Body mass (Body +0x34 -> +0x38 branch mass or
    /// +0x9c). None for the fixture planes, which stand in for a floor at
    /// least as heavy as the character.
    pub fn floor_mass(&self, part: FloorPart) -> Result<Option<f32>, String> {
        Ok(match part {
            FloorPart::Fixture => None,
            FloorPart::Static(index) => {
                let block = self
                    .statics
                    .blocks
                    .get(index)
                    .ok_or("missing floor block")?;
                let loose = self.loose.as_deref().ok_or("missing static block mass")?;
                Some(loose.static_mass(index, block.size)?)
            }
            FloorPart::Loose(index) => {
                let loose = self.loose.as_deref().ok_or("missing loose floor")?;
                let part = loose.parts.get(index).ok_or("missing loose floor")?;
                Some(1.0 / part.body.inverse_mass())
            }
        })
    }
    /// The RunningBase friction factor for a floor Part (00a43f4c against
    /// Plastic; no Part here has custom physical properties).
    pub fn floor_friction(&self, part: FloorPart) -> Result<f32, String> {
        Ok(match part {
            FloorPart::Fixture => self.statics.fixture_friction,
            FloorPart::Static(index) => *self
                .statics
                .floor_friction
                .get(index)
                .ok_or("missing floor block friction")?,
            FloorPart::Loose(index) => {
                let loose = self.loose.as_deref().ok_or("missing loose floor")?;
                loose
                    .parts
                    .get(index)
                    .ok_or("missing loose floor")?
                    .floor_friction()
            }
        })
    }
    /// 00c0abf2: add a force at a world point to the floor's assembly SimBody
    /// (Body +0x34 -> +0x3c). Anchored floors have no SimBody.
    pub fn push_floor(
        &mut self,
        part: FloorPart,
        force: [f32; 3],
        point: [f32; 3],
    ) -> Result<(), String> {
        if let FloorPart::Loose(index) = part {
            let loose = self.loose.as_deref_mut().ok_or("missing loose floor")?;
            let part = loose.parts.get_mut(index).ok_or("missing loose floor")?;
            part.body.apply_force(force, point)?;
        }
        Ok(())
    }
}

/// A static block as the kernel's fixed contact body.
fn block_body(block: &StaticBlock) -> ContactBodyState {
    ContactBodyState {
        center: block.frame.position,
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
    }
}

/// The recovered ContactParams of a (collider, static block) contact.
fn pair_parameters(
    colliders: &[(String, [f32; 3])],
    parameters: &BTreeMap<(usize, usize), ContactParameters>,
    name: &str,
    block: usize,
) -> Result<ContactParameters, String> {
    let collider = colliders
        .iter()
        .position(|(n, _)| n == name)
        .ok_or("missing block collider")?;
    parameters
        .get(&(collider, block))
        .copied()
        .ok_or_else(|| "missing block contact parameters".into())
}

/// Connector geometry with the character Part as contact body A: the normal
/// points from A into B, so it is negated when the static block is A.
fn character_geometry(c: &Connector<ContactState>) -> ContactGeometry {
    let flip = c.pair.body_a == 1;
    ContactGeometry {
        normal: c.point.normal.map(|v| if flip { -v } else { v }),
        distance: c.point.length,
        point: c.point.position,
    }
}

pub struct World {
    pub body: KernelBody,
    pub statics: Statics,
    pub kernel: KernelProfile,
    pub steps: u32,
    pub contacts: usize,
    pub impulse_applications: u64,
    pub max_penetration: f32,
    /// Current assembly inertia in body space (animated Cofm refresh).
    pub inertia: Tensor,
    /// Unanchored Workspace Parts (their own SimBodies in the kernel).
    pub loose: LooseWorld,
    impulses: ImpulseProfile,
    original: KernelBody,
    original_inertia: Tensor,
    colliders: Vec<(String, [f32; 3])>,
    materials: MaterialProfile,
    /// Material defaults and Primitive stiffness per collider (Plastic).
    collider_materials: Vec<(crate::material::PhysicalProperties, f32)>,
    /// Contact::computeContactParams per (collider, static block).
    block_parameters: BTreeMap<(usize, usize), ContactParameters>,
    /// Recovered ContactParams of each collider against the fixture wall.
    wall_parameters: BTreeMap<String, ContactParameters>,
    states: BTreeMap<(String, usize), ContactState>,
    /// BlockBlockContact per (character collider, static block index).
    block_contacts: BTreeMap<(String, usize), BlockBlockContact<ContactState>>,
    mass_parts: BTreeMap<String, MassPart>,
    /// Every character Part's size, for the assembly radius (00ad853c).
    part_sizes: Vec<(String, [f32; 3])>,
    /// SpatialHashMultiRes over the Workspace Parts and the character. It
    /// logs contacts but does not drive the contact pairs yet.
    pub broadphase: Broadphase,
    original_broadphase: Broadphase,
}

impl World {
    pub fn new(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        let kernel = KernelProfile::load(bundle)?;
        let parts = MassProfile::load(bundle)?.character_parts(bundle, scene)?;
        let assembly = AssemblyProperties::from_rig(&parts, rig)?;
        let root = physics::character_bodies(bundle)?
            .into_iter()
            .find(|(n, _)| n == "HumanoidRootPart")
            .ok_or("missing walking root")?
            .1;
        let body = KernelBody::new(&assembly, &root, &kernel)?;
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let materials = MaterialProfile::load(bundle)?;
        let mut colliders = Vec::new();
        let mut collider_materials = Vec::new();
        let mut part_sizes = Vec::new();
        let mut floor_y = f32::INFINITY;
        for instance in &scene.instances {
            let Some(part) = &instance.part else { continue };
            part_sizes.push((instance.name.clone(), part.size));
            if instance.name.ends_with(" Leg") {
                floor_y = floor_y.min(part.frame.position[1] - part.size[1] * 0.5);
            }
            let item = doc
                .descendants()
                .find(|n| {
                    n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent)
                })
                .ok_or("missing collider properties")?;
            let properties = item
                .children()
                .find(|n| n.has_tag_name("Properties"))
                .ok_or("missing collider properties")?;
            let property = |name| {
                properties
                    .children()
                    .find(|n| n.attribute("name") == Some(name))
            };
            match property("CanCollide").and_then(|n| n.text()) {
                Some("true") if part.shape == 1 => {
                    // Contact params read CustomPhysicalProperties (+0xc4) or
                    // the material (+0xc8); only the material branch occurs.
                    let custom = property("CustomPhysicalProperties")
                        .and_then(|n| n.children().find(|c| c.has_tag_name("CustomPhysics")))
                        .and_then(|n| n.text());
                    let material: u32 = property("Material")
                        .and_then(|n| n.text())
                        .and_then(|t| t.parse().ok())
                        .ok_or("missing collider material")?;
                    if custom != Some("false") {
                        return Err("unsupported custom collider physical properties".into());
                    }
                    collider_materials.push((
                        materials.defaults(material)?,
                        materials.stiffness(part.size, Shape::Block)?,
                    ));
                    colliders.push((instance.name.clone(), part.size))
                }
                Some("false") => {}
                _ => return Err("unsupported original walking collider".into()),
            }
        }
        if colliders.len() != 2 || !floor_y.is_finite() {
            return Err("unexpected walking fixture geometry".into());
        }
        // The fixture wall stands in for a large Plastic Part. Block stiffness
        // (00a92640) grows with size, so kSpring = MINSS(...) is the
        // collider's own stiffness; the wall's is passed as equal to it.
        let plastic = materials.defaults(crate::material::PLASTIC)?;
        let mut wall_parameters = BTreeMap::new();
        for ((name, _), (own, stiffness)) in colliders.iter().zip(&collider_materials) {
            let params = materials.contact_params(own, *stiffness, &plastic, *stiffness);
            wall_parameters.insert(name.clone(), params.parameters());
        }
        let mut loose = LooseWorld::new(bundle)?;
        for (properties, stiffness) in &collider_materials {
            loose.add_collider(*properties, *stiffness);
        }
        Ok(Self {
            original: body.clone(),
            body,
            statics: Statics {
                floor_y,
                wall_x: root.frame.position[0] + 4.0,
                blocks: Vec::new(),
                geometry: BlockContactProfile::load(bundle)?,
                tables: BlockTables::load(bundle)?,
                floor_friction: Vec::new(),
                fixture_friction: materials.floor_friction(&plastic)?,
                transparency: Vec::new(),
            },
            kernel,
            steps: 0,
            contacts: 0,
            impulse_applications: 0,
            max_penetration: 0.0,
            inertia: assembly.inertia_in_body,
            original_inertia: assembly.inertia_in_body,
            loose,
            impulses: ImpulseProfile::load(bundle)?,
            colliders,
            materials,
            collider_materials,
            block_parameters: BTreeMap::new(),
            wall_parameters,
            states: BTreeMap::new(),
            block_contacts: BTreeMap::new(),
            mass_parts: parts,
            part_sizes,
            broadphase: Broadphase::default(),
            original_broadphase: Broadphase::default(),
        })
    }
    /// The character in a Workspace instead of the fixture: the fixture
    /// planes are disabled, the anchored colliding Parts become static
    /// blocks, and the character is placed the way Play Solo spawns it
    /// (`Workspace::next_spawn_location`, `character_placement`).
    pub fn in_workspace(
        bundle: &Bundle,
        scene: &Scene,
        rig: &R6Rig,
        workspace: &Workspace,
    ) -> Result<Self, String> {
        let mut world = Self::new(bundle, scene, rig)?;
        world.statics.floor_y = f32::NEG_INFINITY;
        world.statics.wall_x = f32::INFINITY;
        // World::insertPrimitive 00bab52a -> onPrimitiveAdded 00ae97aa: each
        // Part enters the hash without contacts, in Workspace order.
        // Anchored CanCollide-off Parts are left out: they never get a
        // contact (00ae714a), so they cannot change anyone's contact order.
        let mut entered = Vec::new();
        for part in &workspace.parts {
            let extents = block_extents(&part.frame, part.size);
            if !part.anchored {
                let index = world.loose.parts.len();
                world
                    .loose
                    .add_part(part, &world.materials, &world.kernel)?;
                let id = world
                    .broadphase
                    .add(BroadPrim::Loose(index), part.can_collide);
                world.broadphase.insert(id, extents, false);
                entered.push((id, extents));
                continue;
            }
            if part.can_collide {
                let id = world
                    .broadphase
                    .add(BroadPrim::Static(world.statics.blocks.len()), true);
                world.broadphase.insert(id, extents, true);
                entered.push((id, extents));
                let properties = world.materials.defaults(part.material)?;
                let stiffness = world.materials.stiffness(part.size, Shape::Block)?;
                world.loose.add_static(properties, stiffness);
                let index = world.statics.blocks.len();
                for (collider, (own, own_stiffness)) in world.collider_materials.iter().enumerate()
                {
                    let params =
                        world
                            .materials
                            .contact_params(own, *own_stiffness, &properties, stiffness);
                    world
                        .block_parameters
                        .insert((collider, index), params.parameters());
                }
                world.statics.blocks.push(StaticBlock {
                    name: part.name.clone(),
                    frame: part.frame.clone(),
                    size: part.size,
                });
                world
                    .statics
                    .floor_friction
                    .push(world.materials.floor_friction(&properties)?);
                world.statics.transparency.push(part.transparency);
            }
        }
        let spawn = CoordinateFrame {
            position: workspace.next_spawn_location()?,
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let head = scene
            .instances
            .iter()
            .find(|i| i.name == "Head")
            .and_then(|i| i.part.as_ref())
            .ok_or("missing character Head")?;
        let placement = character_placement(&spawn, &head.frame);
        let mut state = world.body.body_state();
        state.frame = compose(&placement, &state.frame);
        state.velocity = [0.0; 3];
        state.angular_velocity = [0.0; 3];
        world.body.set_body_state(&state)?;
        world.original = world.body.clone();
        // The character's Parts follow, then each assembly's primitives run
        // findAllContacts (00af1684) as SpatialFilter moves it into the
        // simulation phase (00b79514 -> 00af6e2c). Assumed order: the
        // Workspace assemblies first, in Workspace order, then the character.
        let frames = rig.pose(&world.body.frame)?;
        for (k, (name, size)) in world.part_sizes.clone().iter().enumerate() {
            let frame = frames.get(name).ok_or("missing character Part frame")?;
            let extents = block_extents(frame, *size);
            let collides = world.colliders.iter().any(|(n, _)| n == name);
            let id = world.broadphase.add(BroadPrim::Character(k), collides);
            world.broadphase.insert(id, extents, false);
            entered.push((id, extents));
        }
        for (id, extents) in entered {
            world.broadphase.find_all(id, extents, 0);
        }
        world.original_broadphase = world.broadphase.clone();
        Ok(world)
    }
    /// The Part name of a broadphase primitive.
    pub fn broadphase_name(&self, prim: BroadPrim) -> String {
        match prim {
            BroadPrim::Static(i) => self.statics.blocks[i].name.clone(),
            BroadPrim::Loose(i) => self.loose.parts[i].name.clone(),
            BroadPrim::Character(k) => self.part_sizes[k].0.clone(),
        }
    }
    /// The primitive's current extents.
    pub fn broadphase_extents(&self, rig: &R6Rig, prim: BroadPrim) -> Result<Extents, String> {
        Ok(match prim {
            BroadPrim::Static(i) => {
                let b = &self.statics.blocks[i];
                block_extents(&b.frame, b.size)
            }
            BroadPrim::Loose(i) => {
                let p = &self.loose.parts[i];
                block_extents(&p.body.frame, p.size)
            }
            BroadPrim::Character(k) => {
                let (name, size) = &self.part_sizes[k];
                let frames = rig.pose(&self.body.frame)?;
                block_extents(
                    frames.get(name).ok_or("missing character Part frame")?,
                    *size,
                )
            }
        })
    }
    /// updateBroadphase for the character assembly: every Part's extents
    /// (00aecaea per primitive, 00ae9bf4).
    fn broadphase_character(&mut self, rig: &R6Rig) -> Result<(), String> {
        if self.broadphase.character_ids.is_empty() {
            return Ok(());
        }
        let frames = rig.pose(&self.body.frame)?;
        for (k, (name, size)) in self.part_sizes.iter().enumerate() {
            let frame = frames.get(name).ok_or("missing character Part frame")?;
            let id = self.broadphase.character_ids[k];
            let extents = block_extents(frame, *size);
            self.broadphase.update(id, extents, self.steps);
        }
        Ok(())
    }
    /// doWorldStep's updateBroadphase (00ba9096) for the loose Parts in
    /// SimulateStage, in entry order.
    fn broadphase_loose(&mut self) {
        for &index in self.loose.simulating() {
            let Some(&id) = self.broadphase.loose_ids.get(index) else {
                continue;
            };
            let part = &self.loose.parts[index];
            let extents = block_extents(&part.body.frame, part.size);
            self.broadphase.update(id, extents, self.steps);
        }
    }
    pub fn reset(&mut self) {
        self.body = self.original.clone();
        self.steps = 0;
        self.contacts = 0;
        self.impulse_applications = 0;
        self.max_penetration = 0.0;
        self.states.clear();
        self.block_contacts.clear();
        self.inertia = self.original_inertia;
        self.loose.reset();
        self.broadphase = self.original_broadphase.clone();
    }
    pub fn is_original(&self) -> bool {
        self.body.frame == self.original.frame && self.inertia == self.original_inertia
    }
    pub fn inertia_changed(&self) -> bool {
        self.inertia != self.original_inertia
    }
    /// The Humanoid ray query over the static contents and the loose Parts.
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
    ) -> Option<(Hit, FloorPart)> {
        raycast_all(&self.statics, Some(&self.loose), origin, direction, length)
    }
    /// PopperCam's PiercingCast: workspace:FindPartOnRayWithIgnoreList
    /// (00882368 -> getHit 00ae818c with the ignore-list filter 0086dc78,
    /// which ignores only descendants of the listed instances: the player
    /// characters, which are not in the loose or static lists). The Lua
    /// recasts past any hit with Transparency > 0.95 or CanCollide false;
    /// recasting until the nearest hit is acceptable is the nearest
    /// acceptable hit. Anchored CanCollide-false Parts are not static blocks,
    /// which matches them being pierced.
    pub fn camera_raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        length: f32,
    ) -> Option<Hit> {
        raycast_where(
            &self.statics,
            Some(&self.loose),
            origin,
            direction,
            length,
            |index| self.statics.transparency[index] <= POPPER_PIERCE_TRANSPARENCY,
            |part| part.can_collide() && part.transparency() <= POPPER_PIERCE_TRANSPARENCY,
        )
        .map(|(hit, _)| hit)
    }
    pub fn point_velocity(&self, point: [f32; 3]) -> [f32; 3] {
        let arm = std::array::from_fn(|a| point[a] - self.body.center[a]);
        let spin = cross(self.body.angular_velocity, arm);
        std::array::from_fn(|a| self.body.velocity[a] + spin[a])
    }
    /// The assembly returned to the kernel; contact accumulators restart.
    pub fn clear_contacts(&mut self) {
        self.states.clear();
        self.block_contacts.clear();
        self.loose.clear_character_contacts();
    }
    /// Motor angles changed: recompute the assembly mass properties (Cofm).
    pub fn refresh_assembly(&mut self, rig: &R6Rig) -> Result<(), String> {
        let frames = rig.pose(&self.body.frame)?;
        let mut parts = self.mass_parts.clone();
        for (name, part) in &mut parts {
            part.frame = frames
                .get(name)
                .ok_or("missing animated mass part")?
                .clone();
        }
        let assembly = AssemblyProperties::from_rig(&parts, rig)?;
        self.body.refresh_assembly(&assembly, &self.kernel)?;
        self.inertia = assembly.inertia_in_body;
        Ok(())
    }
    /// BlockBlockContact::update for every character collider against every
    /// static block (body 0 = character Part, body 1 = static block; the
    /// native broadphase order is not recovered).
    fn update_block_contacts(
        &mut self,
        frames: &BTreeMap<String, CoordinateFrame>,
        in_kernel: bool,
    ) -> Result<(), String> {
        for (name, size) in &self.colliders {
            let part = Block {
                frame: frames
                    .get(name)
                    .ok_or("missing block collider frame")?
                    .clone(),
                size: *size,
            };
            for (index, block) in self.statics.blocks.iter().enumerate() {
                let other = Block {
                    frame: block.frame.clone(),
                    size: block.size,
                };
                self.block_contacts
                    .entry((name.clone(), index))
                    .or_default()
                    .update([&part, &other], in_kernel, &self.statics.tables)?;
            }
        }
        Ok(())
    }
    /// The colliders as blocks at a pose, in registration order.
    fn collider_blocks(
        &self,
        frames: &BTreeMap<String, CoordinateFrame>,
    ) -> Result<Vec<Block>, String> {
        self.colliders
            .iter()
            .map(|(name, size)| {
                Ok(Block {
                    frame: frames
                        .get(name)
                        .ok_or("missing block collider frame")?
                        .clone(),
                    size: *size,
                })
            })
            .collect()
    }
    /// A character collider touching anything outside the character (the
    /// HumanoidState Head test c0192c): the static blocks' stand-in first,
    /// then the loose Parts through their contacts.
    pub fn collider_touching(
        &mut self,
        name: &str,
        frame: &CoordinateFrame,
    ) -> Result<bool, String> {
        let (index, size) = self
            .colliders
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == name)
            .map(|(i, (_, s))| (i, *s))
            .ok_or("missing touch collider")?;
        if self.statics.touching(frame, size)? {
            return Ok(true);
        }
        let block = Block {
            frame: frame.clone(),
            size,
        };
        self.loose.collider_touching(index, &block, &self.materials)
    }
    /// The character assembly for SleepStage: its root primitive
    /// (HumanoidRootPart) frame and point velocity, and the assembly radius
    /// 00ad853c over every character Part at the current pose.
    fn character_motion(&self, rig: &R6Rig) -> Result<CharacterMotion, String> {
        let frames = rig.pose(&self.body.frame)?;
        let root = self.body.frame.position;
        let mut radius = 0.0f32;
        for (name, size) in &self.part_sizes {
            let frame = frames.get(name).ok_or("missing character Part frame")?;
            let r = crate::sleep::primitive_radius(frame.position, root, *size);
            // 00ad9d72 keeps the old value unless the new one is larger.
            if r > radius {
                radius = r;
            }
        }
        Ok(CharacterMotion {
            frame: self.body.frame.clone(),
            radius,
            velocity: self.point_velocity(root),
            angular_velocity: self.body.angular_velocity,
        })
    }
    /// SleepStage::step for the loose Parts, the character in the stage
    /// unless it is placed kinematically.
    fn sleep_stage(&mut self, rig: &R6Rig, in_stage: bool) -> Result<(), String> {
        let frames = rig.pose(&self.body.frame)?;
        let blocks = self.collider_blocks(&frames)?;
        let motion = if in_stage {
            Some(self.character_motion(rig)?)
        } else {
            None
        };
        self.loose.sleep_stage(
            self.steps,
            motion.as_ref(),
            &blocks,
            &self.statics.blocks,
            &self.statics.tables,
            &self.materials,
        )
    }
    /// World::wakeAssembly 00ba7fb0 (not recursive) for a Humanoid floor.
    pub fn wake_floor(&mut self, part: FloorPart) -> Result<(), String> {
        match part {
            FloorPart::Loose(index) => self.loose.wake_assembly(index),
            // Anchored assemblies are not in SleepStage.
            FloorPart::Static(_) | FloorPart::Fixture => Ok(()),
        }
    }
    /// Contact::step for the character against the loose Parts (after its
    /// static-block contacts; the native broadphase order is not recovered).
    #[allow(dead_code)] // Replaced by SleepStage; kept for the old order.
    fn update_loose_contacts(
        &mut self,
        frames: &BTreeMap<String, CoordinateFrame>,
        in_kernel: bool,
    ) -> Result<(), String> {
        let blocks = self.collider_blocks(frames)?;
        self.loose.update_character_contacts(
            &blocks,
            in_kernel,
            &self.statics.tables,
            &self.materials,
        )
    }
    /// Refresh the live block connectors' geometry for the current pose.
    fn refresh_block_contacts(
        &mut self,
        frames: &BTreeMap<String, CoordinateFrame>,
    ) -> Result<(), String> {
        for ((name, index), contact) in &mut self.block_contacts {
            let size = self
                .colliders
                .iter()
                .find(|(n, _)| n == name)
                .ok_or("missing block collider")?
                .1;
            let part = Block {
                frame: frames
                    .get(name)
                    .ok_or("missing block collider frame")?
                    .clone(),
                size,
            };
            let block = &self.statics.blocks[*index];
            let other = Block {
                frame: block.frame.clone(),
                size: block.size,
            };
            contact.refresh([&part, &other]);
        }
        Ok(())
    }
    /// Live block connectors per (collider, block): kind, length, normal.
    pub fn block_contact_report(&self) -> String {
        let mut out = String::new();
        for ((name, index), contact) in &self.block_contacts {
            for c in contact.connectors() {
                out.push_str(&format!(
                    "[{name}/{} {:?} bodies=({},{}) length={} normal={:?} point={:?}]",
                    self.statics.blocks[*index].name,
                    c.pair.kind,
                    c.pair.body_a,
                    c.pair.body_b,
                    c.point.length,
                    c.point.normal,
                    c.point.position
                ));
            }
        }
        out
    }
    pub fn block_contact_kinds(&self) -> Vec<crate::block_contact::PairKind> {
        let mut kinds = Vec::new();
        for contact in self.block_contacts.values() {
            for c in contact.connectors() {
                if !kinds.contains(&c.pair.kind) {
                    kinds.push(c.pair.kind);
                }
            }
        }
        kinds
    }
    /// The assembly is placed kinematically (outside the kernel); only touch
    /// detection runs.
    pub fn step_kinematic(&mut self, rig: &R6Rig) -> Result<(), String> {
        let wall = self.statics.wall();
        let frames = rig.pose(&self.body.frame)?;
        for (name, size) in self
            .colliders
            .iter()
            .filter(|_| self.statics.wall_enabled())
        {
            for (_, g) in self.statics.geometry.plane_vertices(
                frames.get(name).ok_or("missing wall collider frame")?,
                *size,
                &wall,
                3,
            )? {
                self.max_penetration = self.max_penetration.max(-g.distance);
            }
        }
        self.states.clear();
        // Contacts still update outside the kernel stage; no connectors are
        // generated (ae32f4 checks stage 0xf).
        self.update_block_contacts(&frames, false)?;
        self.contacts = 0;
        // MovingNoPhysicsBase places the assembly with setCoordinateFrame
        // (00c07626 -> 00b639a2 -> 00b655a0), which updates the hash at once.
        self.broadphase_character(rig)?;
        // The character is in the Humanoid stage, outside SleepStage; the
        // loose Parts still step.
        self.sleep_stage(rig, false)?;
        self.step_loose()?;
        for _ in 0..self.kernel.physics.kernel_substeps {
            self.loose.substep(&self.kernel)?;
        }
        self.broadphase_loose();
        self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
        Ok(())
    }
    /// Kernel::step pre-step and stepWorld up to the substeps for the loose
    /// Parts (SleepStage already stepped their contacts).
    fn step_loose(&mut self) -> Result<(), String> {
        self.loose.prestep(&self.kernel)?;
        self.loose
            .step_world(&self.kernel, &self.impulses, &self.statics.blocks)
    }
    /// doWorldStep. The character's connectors are realTimeConnectors, so the
    /// awake impulse pass never touches them; each kernel substep runs the
    /// contact springs, the force callback (torso frame, current inertia),
    /// and the integration.
    /// Every contact uses recovered per-pair ContactParams (00a43fb4);
    /// `_parameters` is unused and kept for signature stability.
    pub fn step(
        &mut self,
        rig: &R6Rig,
        parameters: ContactParameters,
        mut force: impl FnMut(&mut KernelBody, &Statics, &CoordinateFrame, Tensor) -> Result<(), String>,
    ) -> Result<(), String> {
        self.step_with(rig, parameters, |body, surroundings, torso, inertia| {
            force(body, surroundings.statics, torso, inertia)
        })
    }
    /// `step` with the force callback reaching the loose Parts as well.
    pub fn step_with(
        &mut self,
        rig: &R6Rig,
        _parameters: ContactParameters,
        mut force: impl FnMut(
            &mut KernelBody,
            &mut Surroundings,
            &CoordinateFrame,
            Tensor,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        let wall = self.statics.wall();
        let fixed = ContactBodyState {
            center: wall.position,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        let frames = rig.pose(&self.body.frame)?;
        let mut active = Vec::new();
        // Character Parts cannot throttle (every HumanoidState ctor 00bfd2a2
        // clears Body +0x44), so 00a99514 files their contacts under
        // realTimeConnectors: spring forces every substep, never the awake
        // impulse pass, which 00a9768c runs over contactConnectors only.
        for (name, size) in self
            .colliders
            .iter()
            .filter(|_| self.statics.wall_enabled())
        {
            for (vertex, _) in self.statics.geometry.plane_vertices(
                frames.get(name).ok_or("missing wall collider frame")?,
                *size,
                &wall,
                3,
            )? {
                let key = (name.clone(), vertex);
                self.states.entry(key.clone()).or_default();
                active.push(key);
            }
        }
        self.states.retain(|key, _| active.contains(key));
        self.update_block_contacts(&frames, true)?;
        self.sleep_stage(rig, true)?;
        self.step_loose()?;
        self.contacts = active.len()
            + self
                .block_contacts
                .values()
                .map(|c| c.connectors().len())
                .sum::<usize>()
            + self.loose.character_connector_count();
        for _ in 0..self.kernel.physics.kernel_substeps {
            let frames = rig.pose(&self.body.frame)?;
            for (name, size) in self
                .colliders
                .iter()
                .filter(|_| self.statics.wall_enabled())
            {
                for (vertex, g) in self.statics.geometry.plane_vertices(
                    frames.get(name).ok_or("missing spring collider")?,
                    *size,
                    &wall,
                    3,
                )? {
                    self.max_penetration = self.max_penetration.max(-g.distance);
                    if let Some(state) = self.states.get_mut(&(name.clone(), vertex)) {
                        let force = state.spring.force(
                            &self.kernel,
                            *self
                                .wall_parameters
                                .get(name)
                                .ok_or("missing wall contact parameters")?,
                            g.distance,
                            g.normal,
                            g.point,
                            &self.body.contact_state(),
                            &fixed,
                        )?;
                        self.body.apply_force(force.map(|v| -v), g.point)?;
                    }
                }
            }
            self.refresh_block_contacts(&frames)?;
            for (key, contact) in &mut self.block_contacts {
                let fixed = block_body(&self.statics.blocks[key.1]);
                let pair = pair_parameters(&self.colliders, &self.block_parameters, &key.0, key.1)?;
                for c in contact.connectors_mut() {
                    let g = character_geometry(c);
                    self.max_penetration = self.max_penetration.max(-g.distance);
                    let force = c.state.spring.force(
                        &self.kernel,
                        pair,
                        g.distance,
                        g.normal,
                        g.point,
                        &self.body.contact_state(),
                        &fixed,
                    )?;
                    self.body.apply_force(force.map(|v| -v), g.point)?;
                }
            }
            // The character's springs against loose Parts close the
            // realTimeConnectors pass; jointConnectors follow.
            let blocks = self.collider_blocks(&frames)?;
            let deepest = self
                .loose
                .real_time_springs(&blocks, &mut self.body, &self.kernel)?;
            self.max_penetration = self.max_penetration.max(deepest);
            self.loose.joint_springs(&self.kernel)?;
            // 00a9768c substep order: realTimeConnectors (the character's
            // contact springs), jointConnectors, then humanoidConnectors
            // (Humanoid +0xdc, type 2, computeForce 00bddb7a -> 00bddaae),
            // then realTimeBodies and jointBodies integrate.
            let torso = frames.get("Torso").ok_or("missing state torso")?.clone();
            let mut surroundings = Surroundings {
                statics: &self.statics,
                loose: Some(&mut self.loose),
            };
            force(&mut self.body, &mut surroundings, &torso, self.inertia)?;
            self.body.step(&self.kernel)?;
            self.loose.integrate(&self.kernel)?;
        }
        // updateBroadphase after Kernel::step: SimulateStage's realtime list
        // (the character cannot throttle) first, then the others.
        self.broadphase_character(rig)?;
        self.broadphase_loose();
        self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn woo_ray_box_matches_g3d_rules() {
        let (low, high) = ([-1.0; 3], [1.0; 3]);
        let hit = moving_point_fixed_aabox([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high);
        assert_eq!(hit, Some(([0.0, 1.0, 0.0], [0.0, 1.0, 0.0])));
        // Inside the box: no hit. Pointing away: negative t, no hit.
        assert_eq!(
            moving_point_fixed_aabox([0.0; 3], [0.0, -1.0, 0.0], low, high),
            None
        );
        assert_eq!(
            moving_point_fixed_aabox([0.0, 5.0, 0.0], [0.0, 1.0, 0.0], low, high),
            None
        );
        // Missing to the side.
        assert_eq!(
            moving_point_fixed_aabox([3.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high),
            None
        );
        // Grazing the far edge counts (location == high).
        assert!(moving_point_fixed_aabox([1.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high).is_some());
        let side = moving_point_fixed_aabox([-5.0, 0.5, 0.0], [1.0, 0.0, 0.0], low, high);
        assert_eq!(side, Some(([-1.0, 0.5, 0.0], [-1.0, 0.0, 0.0])));
    }
    #[test]
    fn rotated_block_hit_is_in_world_space() {
        let block = StaticBlock {
            name: "Part".into(),
            frame: CoordinateFrame {
                position: [10.0, 0.0, 0.0],
                rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            },
            size: [4.0, 2.0, 2.0],
        };
        // Rotated 90 degrees about Z: the 4-stud axis points along world Y.
        let (point, normal) = block.hit_test([10.0, 10.0, 0.0], [0.0, -1.0, 0.0]).unwrap();
        assert_eq!(point, [10.0, 2.0, 0.0]);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        let other = CoordinateFrame {
            position: [10.0, 2.4, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        assert!(block.overlaps(&other, [1.0; 3]));
        let apart = CoordinateFrame {
            position: [10.0, 2.6, 0.0],
            ..other
        };
        assert!(!block.overlaps(&apart, [1.0; 3]));
    }
}
