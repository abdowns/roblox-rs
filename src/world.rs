//! RBX::World subset: one character assembly, the static contents, realtime
//! contact springs, and kernel substeps with a per-substep force callback
//! (the Humanoid state's kernel connector, vtable +0xc). The static contents
//! are either the walking fixture (floor plane, wall half-space) or the
//! anchored Parts of a Workspace (`World::in_workspace`).
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile, Tensor},
    ball_contact::{self, PrimContact},
    block_contact::{Block, BlockTables, Connector},
    broadphase::{Event, Extents, SpatialHash, ball_extents, block_extents},
    bundle::Bundle,
    collision::ContactBodyState,
    humanoid_state::{FloorPart, Hit},
    joints::{GLUE, Link, MOTOR, MOTOR_6D, ROTATE, ROTATE_P, ROTATE_V},
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile, cross},
    kernel_joint::{self, JointBody},
    legacy_contact::{BlockContactProfile, ContactGeometry, ImpulseProfile},
    loose::{CharacterMotion, LoosePrim, LooseWorld, Prim},
    material::{MaterialProfile, Shape},
    motor::{self, MotorJoint},
    physics,
    rig::{Motor6D, R6Rig, compose},
    scene::{CoordinateFrame, Scene},
    workspace::{SHAPE_BALL, SHAPE_BLOCK, Workspace, character_placement},
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
    /// Each primitive's assembly (the static blocks share one, as do the
    /// character's Parts).
    groups: Vec<usize>,
    loose_ids: Vec<usize>,
    character_ids: Vec<usize>,
    /// Every contact change: (world step, event).
    pub log: Vec<(u32, Event)>,
    /// How many log entries the contacts have taken in.
    applied: usize,
    /// Character/static contacts the broadphase created static first (the
    /// block contact keeps the character Part as body 0 regardless).
    pub static_first: usize,
}

impl Broadphase {
    fn add(&mut self, prim: BroadPrim, collides: bool, group: usize) -> usize {
        let id = self.prims.len();
        self.prims.push(prim);
        self.collides.push(collides);
        self.groups.push(group);
        match prim {
            BroadPrim::Loose(_) => self.loose_ids.push(id),
            BroadPrim::Character(_) => self.character_ids.push(id),
            BroadPrim::Static(_) => {}
        }
        id
    }
    /// 00ae9612: not in the same assembly, not both grounded, and both
    /// collide.
    fn accept(groups: &[usize], collides: &[bool], a: usize, b: usize) -> bool {
        groups[a] != groups[b] && collides[a] && collides[b]
    }
    /// Both primitives are in one assembly, or both are grounded.
    pub fn same_assembly(&self, a: usize, b: usize) -> bool {
        self.groups[a] == self.groups[b]
    }
    fn drain(&mut self, step: u32) {
        for event in self.hash.events.drain(..) {
            self.log.push((step, event));
        }
    }
    fn insert(&mut self, id: usize, extents: Extents, fixed: bool) {
        let (groups, collides) = (&self.groups, &self.collides);
        self.hash.insert(id, extents, fixed, false, |a, b| {
            Self::accept(groups, collides, a, b)
        });
    }
    fn find_all(&mut self, id: usize, extents: Extents, step: u32) {
        let (groups, collides) = (&self.groups, &self.collides);
        self.hash
            .find_all_contacts(id, extents, |a, b| Self::accept(groups, collides, a, b));
        self.drain(step);
    }
    fn update(&mut self, id: usize, extents: Extents, step: u32) {
        let (groups, collides) = (&self.groups, &self.collides);
        self.hash
            .update(id, extents, |a, b| Self::accept(groups, collides, a, b));
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
/// Broadphase groups of the grounded blocks and the character's Parts (a
/// loose Part's group is its assembly root's Workspace index).
const STATIC_GROUP: usize = usize::MAX;
const CHARACTER_GROUP: usize = usize::MAX - 1;

/// An anchored, colliding Part in the world: a Block, or a Ball of radius
/// size.x / 2.
#[derive(Clone, Debug)]
pub struct StaticBlock {
    pub name: String,
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
    pub shape: Shape,
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

/// Primitive hitTest (00b631de) with Ball geometry (vtable +0x70, 00adc17c):
/// G3D::CollisionDetection::collisionTimeForMovingPointFixedSphere
/// (0124e2c8, not solid) in part space against the ball at the origin. The
/// hit counts when the time is not infinite.
fn ball_hit_test(
    frame: &CoordinateFrame,
    radius: f32,
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<Hit> {
    let r = &frame.rotation;
    let t = frame.position;
    let p: [f32; 3] = std::array::from_fn(|i| origin[i] - t[i]);
    let point: [f32; 3] = std::array::from_fn(|i| r[0][i] * p[0] + r[1][i] * p[1] + r[2][i] * p[2]);
    let v: [f32; 3] = std::array::from_fn(|i| {
        r[0][i] * direction[0] + r[1][i] * direction[1] + r[2][i] * direction[2]
    });
    let speed = ((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]).sqrt();
    let inverse = 1.0 / speed;
    let u = [v[0] * inverse, v[1] * inverse, inverse * v[2]];
    // The sphere center is Vector3::zero (0025a930).
    let l = point.map(|x| 0.0 - x);
    let d = (u[0] * l[0] + u[1] * l[1]) + u[2] * l[2];
    let l2 = (l[0] * l[0] + l[1] * l[1]) + l[2] * l[2];
    let r2 = radius * radius;
    if (d < 0.0 && l2 > r2) || l2 - d * d > r2 {
        return None;
    }
    let q = (r2 - (l2 - d * d)).sqrt();
    let time = (d + if r2 < l2 { -q } else { q }) / speed;
    if time == f32::INFINITY {
        return None;
    }
    let hit: [f32; 3] = std::array::from_fn(|i| point[i] + v[i] * time);
    let inverse = 1.0 / ((hit[0] * hit[0] + hit[1] * hit[1]) + hit[2] * hit[2]).sqrt();
    let normal = [hit[0] * inverse, hit[1] * inverse, inverse * hit[2]];
    Some((
        std::array::from_fn(|i| r[i][2] * hit[2] + r[i][1] * hit[1] + r[i][0] * hit[0] + t[i]),
        std::array::from_fn(|i| r[i][2] * normal[2] + r[i][1] * normal[1] + r[i][0] * normal[0]),
    ))
}

/// One getSlowHit (00ae7d08) candidate: the bounding-sphere test (vtable
/// +0x10 radius: half the size's length for a Block, the radius for a Ball),
/// the hit test, then a positive distance strictly below the best so far.
fn slow_hit_candidate(
    frame: &CoordinateFrame,
    size: [f32; 3],
    shape: Shape,
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
    let radius = match shape {
        Shape::Block => (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt(),
        Shape::Ball => crate::ball_contact::radius(size),
    };
    if (off[2] * off[2] + off[1] * off[1] + off[0] * off[0]).sqrt() > radius {
        return None;
    }
    let (point, normal) = match shape {
        Shape::Block => box_hit_test(frame, size, origin, direction)?,
        Shape::Ball => ball_hit_test(frame, radius, origin, direction)?,
    };
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
            if let Some((distance, hit)) = slow_hit_candidate(
                &block.frame,
                block.size,
                block.shape,
                origin,
                direction,
                best,
            ) {
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
        let collider = Block {
            frame: frame.clone(),
            size,
        };
        // A static Ball runs its contact's own test (BallBlock 00ae2aec).
        let touches = |b: &StaticBlock| match b.shape {
            Shape::Block => b.overlaps(frame, size),
            Shape::Ball => {
                let ball = Block {
                    frame: b.frame.clone(),
                    size: b.size,
                };
                ball_contact::ball_block_colliding(&ball, &collider, 0.0).0
            }
        };
        if self.blocks.iter().any(touches) {
            return Ok(true);
        }
        Ok(self.wall_enabled()
            && !self
                .geometry
                .plane_vertices(frame, size, &self.wall(), 3)?
                .is_empty())
    }
}

/// Primitive extents 00b61c9a of a loose Part.
fn loose_extents(frame: &CoordinateFrame, part: &LoosePrim) -> Extents {
    match part.shape {
        Shape::Block => block_extents(frame, part.size),
        Shape::Ball => ball_extents(frame, ball_contact::radius(part.size)),
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
        LoosePrim::can_collide,
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
    accept_loose: impl Fn(&LoosePrim) -> bool,
) -> Option<(Hit, FloorPart)> {
    let mut best = statics.block_hit_where(origin, direction, length, accept_static);
    if let Some(loose) = loose {
        let mut limit = best.as_ref().map_or(length.min(5000.0), |b| b.0);
        for (index, part) in loose.prims.iter().enumerate() {
            if !accept_loose(part) {
                continue;
            }
            let Ok(frame) = loose.prim_frame(index) else {
                continue;
            };
            if let Some((distance, hit)) =
                slow_hit_candidate(&frame, part.size, part.shape, origin, direction, limit)
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
                let prim = loose.prims.get(index).ok_or("missing loose floor")?;
                let part = loose
                    .parts
                    .get(prim.assembly)
                    .ok_or("missing loose floor")?;
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
                    .prims
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
            let assembly = loose
                .prims
                .get(index)
                .ok_or("missing loose floor")?
                .assembly;
            let part = loose.parts.get_mut(assembly).ok_or("missing loose floor")?;
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
    block_contacts: BTreeMap<(String, usize), PrimContact<ContactState>>,
    mass_parts: BTreeMap<String, MassPart>,
    /// Every character Part's size, for the assembly radius (00ad853c).
    part_sizes: Vec<(String, [f32; 3])>,
    /// SpatialHashMultiRes over the Workspace Parts and the character. Its
    /// contacts are the contact pairs.
    pub broadphase: Broadphase,
    original_broadphase: Broadphase,
    /// Character/static-block contacts from the broadphase, in creation
    /// order: (collider name, block index).
    static_contacts: Vec<(String, usize)>,
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
            static_contacts: Vec::new(),
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
        // The joints group the Parts into assemblies (`joints.rs`); a Part
        // takes the CFrame its Body gives it. Grounded assemblies (anchored,
        // or rigidly joined to an anchored Part) never move: their Parts
        // become static blocks.
        let (tree, ids) = workspace.joint_tree_indexed()?;
        // A Motor or Motor6D link (`motor.rs`): the joint frames on the
        // parent's and the child's side (00b38196, 00b370ec), and the offset
        // at CurrentAngle with no animation.
        let threshold = rig.joints.first().ok_or("rig without motors")?.threshold();
        let motor_6d = |w: usize| {
            let joint = &workspace.joints[w];
            let name = |n: usize| workspace.parts[n].name.as_str();
            Motor6D::new(
                &joint.name,
                name(joint.part0),
                name(joint.part1),
                joint.c0.clone(),
                joint.c1.clone(),
                [joint.current_angle, joint.desired_angle],
                joint.max_velocity,
                threshold,
            )
        };
        let motor_link = |j: usize, parent: usize| {
            let w = ids.iter().position(|&id| id == j)?;
            let joint = &workspace.joints[w];
            if joint.kind != MOTOR && joint.kind != MOTOR_6D {
                return None;
            }
            let coords = if joint.part0 == parent {
                [joint.c0.clone(), joint.c1.clone()]
            } else {
                [joint.c1.clone(), joint.c0.clone()]
            };
            Some((w, coords))
        };
        let motor_offset = |j: usize, parent: usize, _child: usize| {
            let (w, [p, c]) = motor_link(j, parent)?;
            Some(if workspace.joints[w].kind == MOTOR {
                motor::revolute_offset(&p, &c, workspace.joints[w].current_angle)
            } else {
                motor::d6_offset(&p, &c, &motor_6d(w).transform())
            })
        };
        let frames = tree.body_frames_with(
            &workspace
                .parts
                .iter()
                .map(|p| p.frame.clone())
                .collect::<Vec<_>>(),
            motor_offset,
        )?;
        // World::insertPrimitive 00bab52a -> onPrimitiveAdded 00ae97aa: each
        // Part enters the hash without contacts, in Workspace order.
        // Grounded CanCollide-off Parts are left out: they never get a
        // contact (00ae714a), so they cannot change anyone's contact order.
        let mut entered = Vec::new();
        let mut prim_of = vec![None; workspace.parts.len()];
        for (n, part) in workspace.parts.iter().enumerate() {
            let grounded = part.anchored || tree.grounded(n);
            let shape = match part.shape {
                SHAPE_BALL => Shape::Ball,
                SHAPE_BLOCK => Shape::Block,
                _ => return Err(format!("{:?}: a Cylinder Part is not ported", part.name)),
            };
            let extents = if part.shape == SHAPE_BALL {
                ball_extents(&frames[n], ball_contact::radius(part.size))
            } else {
                block_extents(&frames[n], part.size)
            };
            if grounded && matches!(tree.link(n), Some(Link::Motor(_))) {
                // MovingAssemblyStage moves an anchored assembly's motor
                // links kinematically (not ported).
                return Err(format!(
                    "{:?}: a Motor in an anchored assembly is not ported",
                    part.name
                ));
            }
            if !grounded {
                let mut placed = part.clone();
                placed.frame = frames[n].clone();
                let index = world.loose.add_prim(&placed, &world.materials)?;
                prim_of[n] = Some(index);
                let id = world.broadphase.add(
                    BroadPrim::Loose(index),
                    part.can_collide,
                    tree.assembly_root(n),
                );
                world.broadphase.insert(id, extents, false);
                entered.push((id, extents));
                continue;
            }
            if part.can_collide {
                let id = world.broadphase.add(
                    BroadPrim::Static(world.statics.blocks.len()),
                    true,
                    STATIC_GROUP,
                );
                world.broadphase.insert(id, extents, true);
                entered.push((id, extents));
                let properties = world.materials.defaults(part.material)?;
                let stiffness = world.materials.stiffness(part.size, shape)?;
                world.loose.add_static_shape(properties, stiffness, shape);
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
                    frame: frames[n].clone(),
                    size: part.size,
                    shape,
                });
                world
                    .statics
                    .floor_friction
                    .push(world.materials.floor_friction(&properties)?);
                world.statics.transparency.push(part.transparency);
            }
        }
        // The loose assemblies, in the order of their first Part (assumed:
        // the native kernel order of a loaded place is not recovered). Each
        // Body links to its parent with the joint's offset.
        let mut built = Vec::new();
        for n in 0..workspace.parts.len() {
            if prim_of[n].is_none() || built.contains(&tree.assembly_root(n)) {
                continue;
            }
            let root = tree.assembly_root(n);
            built.push(root);
            let prim = |m: usize| prim_of[m].ok_or("assembly Part is not loose");
            let mut links = Vec::new();
            let mut motors = Vec::new();
            for m in tree.assembly_parts(root).into_iter().skip(1) {
                let parent = tree.body_parent(m).ok_or("Body child without a parent")?;
                let offset = match tree.link(m) {
                    Some(Link::Rigid(offset)) => offset.clone(),
                    Some(Link::Motor(j)) => {
                        let (w, coords) = motor_link(*j, parent)
                            .ok_or("only Motor and Motor6D are ported as motor links")?;
                        let offset = motor_offset(*j, parent, m)
                            .ok_or("only Motor and Motor6D are ported as motor links")?;
                        motors.push((w, prim(parent)?, prim(m)?, coords));
                        offset
                    }
                    None => return Err("Body child without a link".into()),
                };
                links.push((prim(m)?, prim(parent)?, offset));
            }
            world
                .loose
                .add_assembly(prim(root)?, &links, &world.kernel)?;
            for (w, parent, child, coords) in motors {
                let joint = &workspace.joints[w];
                if joint.kind == MOTOR {
                    let motor = MotorJoint::new(
                        &joint.name,
                        joint.current_angle,
                        joint.desired_angle,
                        joint.max_velocity,
                    );
                    world.loose.add_motor(motor, parent, child, coords)?;
                } else {
                    world
                        .loose
                        .add_motor_6d(motor_6d(w), parent, child, coords)?;
                }
            }
        }
        // The kernel joints (Glue and the hinges), in Workspace order. A Part
        // that does not move (anchored or grounded) holds its Points at its
        // CFrame.
        let mut fixed = BTreeMap::new();
        let kernel_kinds = [GLUE, ROTATE, ROTATE_P, ROTATE_V];
        for joint in workspace
            .joints
            .iter()
            .filter(|j| kernel_kinds.contains(&j.kind))
        {
            let mut bodies = [JointBody::Fixed(0); 2];
            for (side, n) in [joint.part0, joint.part1].into_iter().enumerate() {
                let part = workspace.parts.get(n).ok_or("joint Part missing")?;
                bodies[side] = match prim_of[n] {
                    Some(prim) => JointBody::Loose(prim),
                    None => {
                        let shape = if part.shape == SHAPE_BALL {
                            Shape::Ball
                        } else {
                            Shape::Block
                        };
                        let stiffness = world.materials.stiffness(part.size, shape)?;
                        let k = *fixed.entry(n).or_insert_with(|| {
                            world
                                .loose
                                .add_fixed_part(frames[n].clone(), part.size, stiffness)
                        });
                        JointBody::Fixed(k)
                    }
                };
            }
            let coords = [joint.c0.clone(), joint.c1.clone()];
            if joint.kind == GLUE {
                world
                    .loose
                    .add_glue(&joint.name, bodies, coords, joint.face)?;
                continue;
            }
            // The axle is Part0's face that C0's z axis names (00b6be18).
            let r = &joint.c0.rotation;
            let face = kernel_joint::normal_id([r[0][2], r[1][2], r[2][2]]) as usize;
            let axle = match workspace.parts[joint.part0].surface_inputs.get(face) {
                Some(input) => *input,
                None if joint.kind == ROTATE => Default::default(),
                None => {
                    return Err(format!(
                        "motor hinge {:?}: C0's z axis is not a Part axis (not ported)",
                        joint.name
                    ));
                }
            };
            world.loose.add_hinge(
                joint.kind,
                &joint.name,
                bodies,
                coords,
                joint.base_angle,
                axle,
            )?;
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
            let id = world
                .broadphase
                .add(BroadPrim::Character(k), collides, CHARACTER_GROUP);
            world.broadphase.insert(id, extents, false);
            entered.push((id, extents));
        }
        for (id, extents) in entered {
            world.broadphase.find_all(id, extents, 0);
        }
        world.original_broadphase = world.broadphase.clone();
        world.apply_broadphase()?;
        Ok(world)
    }
    /// World::uiStep's breakJoints (00ba9032), once per frame before the
    /// world steps: broken kernel joints are removed. Returns how many broke.
    pub fn break_joints(&mut self) -> Result<usize, String> {
        self.loose.break_joints()
    }
    /// World::uiStep's MovingAssemblyStage::stepUi (00b39318) for the motor
    /// hinges and the Motors, after breakJoints: `time` is
    /// Workspace.DistributedGameTime (RunService's game time, copied in by
    /// physicsStep 00a7c4e2).
    pub fn step_ui(&mut self, time: f64) -> Result<(), String> {
        self.loose.step_ui(time)?;
        self.loose.step_motors(&self.kernel)?;
        Ok(())
    }
    /// The broadphase's new contact events become contact pairs: character
    /// against a static block in `static_contacts`, everything else in
    /// `LooseWorld`.
    fn apply_broadphase(&mut self) -> Result<(), String> {
        while self.broadphase.applied < self.broadphase.log.len() {
            let (_, event) = self.broadphase.log[self.broadphase.applied];
            self.broadphase.applied += 1;
            let (created, a, b) = match event {
                Event::Create(a, b) => (true, a, b),
                Event::Destroy(a, b) => (false, a, b),
            };
            let (pa, pb) = (self.broadphase.prims[a], self.broadphase.prims[b]);
            let static_pair = match (pa, pb) {
                (BroadPrim::Character(k), BroadPrim::Static(j)) => Some((k, j, false)),
                (BroadPrim::Static(j), BroadPrim::Character(k)) => Some((k, j, true)),
                _ => None,
            };
            if let Some((k, j, static_first)) = static_pair {
                let name = self.part_sizes[k].0.clone();
                if created {
                    self.broadphase.static_first += usize::from(static_first);
                    self.static_contacts.push((name, j));
                } else {
                    self.static_contacts.retain(|c| *c != (name.clone(), j));
                    self.block_contacts.remove(&(name, j));
                }
                continue;
            }
            let key = (self.prim_of(pa)?, self.prim_of(pb)?);
            if created {
                self.loose.contact_created(key, &self.materials)?;
            } else {
                self.loose.contact_destroyed(key)?;
            }
        }
        Ok(())
    }
    fn prim_of(&self, prim: BroadPrim) -> Result<Prim, String> {
        Ok(match prim {
            BroadPrim::Static(j) => Prim::Static(j),
            BroadPrim::Loose(i) => Prim::Loose(i),
            BroadPrim::Character(k) => {
                let name = &self.part_sizes[k].0;
                Prim::Character(
                    self.colliders
                        .iter()
                        .position(|(n, _)| n == name)
                        .ok_or("contact on a non-colliding character Part")?,
                )
            }
        })
    }
    /// The Part name of a broadphase primitive.
    pub fn broadphase_name(&self, prim: BroadPrim) -> String {
        match prim {
            BroadPrim::Static(i) => self.statics.blocks[i].name.clone(),
            BroadPrim::Loose(i) => self.loose.prims[i].name.clone(),
            BroadPrim::Character(k) => self.part_sizes[k].0.clone(),
        }
    }
    /// The primitive's current extents.
    pub fn broadphase_extents(&self, rig: &R6Rig, prim: BroadPrim) -> Result<Extents, String> {
        Ok(match prim {
            BroadPrim::Static(i) => {
                let b = &self.statics.blocks[i];
                match b.shape {
                    Shape::Block => block_extents(&b.frame, b.size),
                    Shape::Ball => ball_extents(&b.frame, ball_contact::radius(b.size)),
                }
            }
            BroadPrim::Loose(i) => loose_extents(&self.loose.prim_frame(i)?, &self.loose.prims[i]),
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
        self.apply_broadphase()
    }
    /// doWorldStep's updateBroadphase (00ba9096) for the loose assemblies in
    /// SimulateStage, in entry order, each Part in the assembly's order
    /// (root first; the native primitive order is not recovered).
    fn broadphase_loose(&mut self) -> Result<(), String> {
        for index in self.loose.simulating().to_vec() {
            for prim in self.loose.parts[index].prims.clone() {
                let Some(&id) = self.broadphase.loose_ids.get(prim) else {
                    continue;
                };
                let frame = self.loose.prim_frame(prim)?;
                let extents = loose_extents(&frame, &self.loose.prims[prim]);
                self.broadphase.update(id, extents, self.steps);
                self.apply_broadphase()?;
            }
        }
        Ok(())
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
        self.static_contacts.clear();
        // The start contacts again (the reset world is the one just built).
        if let Err(e) = self.apply_broadphase() {
            panic!("broadphase reset: {e}");
        }
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
    /// BlockBlockContact::update for each broadphase contact between a
    /// character collider and a static block, in creation order (body 0 =
    /// character Part, body 1 = static block). In the fixture world, which
    /// has no broadphase, every collider meets every block.
    fn update_block_contacts(
        &mut self,
        frames: &BTreeMap<String, CoordinateFrame>,
        in_kernel: bool,
    ) -> Result<(), String> {
        let pairs: Vec<(String, usize)> = if self.broadphase.prims.is_empty() {
            self.colliders
                .iter()
                .flat_map(|(n, _)| (0..self.statics.blocks.len()).map(move |j| (n.clone(), j)))
                .collect()
        } else {
            self.static_contacts.clone()
        };
        for (name, index) in pairs {
            let size = self
                .colliders
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or("missing block collider")?
                .1;
            let part = Block {
                frame: frames
                    .get(&name)
                    .ok_or("missing block collider frame")?
                    .clone(),
                size,
            };
            let block = &self.statics.blocks[index];
            let other = Block {
                frame: block.frame.clone(),
                size: block.size,
            };
            let shape = block.shape;
            self.block_contacts
                .entry((name, index))
                .or_insert_with(|| PrimContact::new([Shape::Block, shape]))
                .update([&part, &other], in_kernel, &self.statics.tables)?;
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
    /// HumanoidState Torso/Head test c0192c): the static blocks' stand-in first,
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
            FloorPart::Loose(index) => {
                let assembly = self
                    .loose
                    .prims
                    .get(index)
                    .ok_or("missing loose floor")?
                    .assembly;
                self.loose.wake_assembly(assembly)
            }
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
        // doWorldStep: StepJointsStage::stepWorld (00b8a402), then Kernel::step.
        self.loose.step_joints(&self.kernel)?;
        self.step_loose()?;
        for _ in 0..self.kernel.physics.kernel_substeps {
            self.loose.substep(&self.kernel)?;
        }
        self.broadphase_loose()?;
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
        self.loose.step_joints(&self.kernel)?;
        self.step_loose()?;
        self.contacts = active.len()
            + self
                .block_contacts
                .values()
                .map(|c| c.connectors().len())
                .sum::<usize>()
            + self.loose.character_connector_count();
        for _ in 0..self.kernel.physics.kernel_substeps {
            // 00a9768c: the kernel Points step first.
            self.loose.step_points()?;
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
            self.loose.points_to_bodies()?;
            // 00a9768c substep order: Points, realTimeConnectors (the
            // character's contact springs), jointConnectors, the Points'
            // forces into their bodies, then humanoidConnectors (Humanoid
            // +0xdc, type 2, computeForce 00bddb7a -> 00bddaae), then
            // realTimeBodies and jointBodies integrate.
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
        self.broadphase_loose()?;
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
    fn ball_hit_is_the_first_sphere_crossing() {
        let ball = CoordinateFrame {
            position: [0.0, 5.0, 0.0],
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let (point, normal) =
            ball_hit_test(&ball, 1.0, [0.0, 10.0, 0.0], [0.0, -1.0, 0.0]).unwrap();
        assert_eq!(point, [0.0, 6.0, 0.0]);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        // Passing beside it, or pointing away, misses.
        assert!(ball_hit_test(&ball, 1.0, [1.5, 10.0, 0.0], [0.0, -1.0, 0.0]).is_none());
        assert!(ball_hit_test(&ball, 1.0, [0.0, 10.0, 0.0], [0.0, 1.0, 0.0]).is_none());
        // From the center (not solid): the far side.
        let (point, _) = ball_hit_test(&ball, 1.0, [0.0, 5.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
        assert!((point[0] - 1.0).abs() < 1e-6 && point[1] == 5.0);
        // getSlowHit takes the radius, not the half diagonal, as its bound.
        let near = [0.0, 10.0, 1.2];
        assert!(
            slow_hit_candidate(&ball, [2.0; 3], Shape::Ball, near, [0.0, -1.0, 0.0], 100.0)
                .is_none()
        );
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
            shape: Shape::Block,
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
