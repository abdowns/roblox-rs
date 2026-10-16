//! Ball geometry (v8world/Ball.cpp, vtable 01c865e0) and its narrowphase:
//! BallBallContact (vtable 01c868a8) and BallBlockContact (vtable 01c86928),
//! each holding at most one connector (BallBallConnector 01c85c34,
//! BallBlockConnector 01c85c88).
//!
//! Recovered from:
//! - the Primitive geometry factory 00b614f4 (type 1 Ball, 2 Block, 3
//!   Cylinder) and the Part shape setter 00708d58 (Ball 0 -> 1, Block 1 ->
//!   2, Cylinder 2 -> 3);
//! - Ball::setSize 00adc208: the radius (+0x14) is size.x / 2; getRadius
//!   00adca24; the world half extents 00adcb5e are the radius on every axis,
//!   so the Primitive extents 00b61c9a are center -/+ r -/+ 0.01. Volume and
//!   moment are `assembly::MassProfile::ball_volume` / `ball_inertia`;
//! - ContactManager::createContact 00ae714a: collide types (geometry +0x10)
//!   Ball 1, Block 2, Poly 3; the primitive of the lower type is the
//!   contact's first (equal types keep the broadphase order);
//! - BallBallContact: computeIsColliding 00ae2824 (no extents test), step
//!   00ae28ea; connector geometry 00a946c6;
//! - BallBlockContact: computeIsColliding 00ae2a9e -> 00ae2aec (extents
//!   overlap, the ball center in block space clamped to the box by
//!   Block 00adf544, the distance to the clamped point), step 00ae2d16 with
//!   the block feature lookups 00adf472 (center inside: the nearest face),
//!   00adf3e2 -> 00adf206 (edge) / 00adf30c (face) / the vertex index;
//!   connector geometry 00a947b8 -> 00a94800 (plane), 00a94a18 (edge),
//!   00a94c50 (vertex);
//! - FFlag BallBlockNarrowphaseFixEnabled (01d72b94, registered by
//!   00a9536c) is BSS, so off: the plane and edge contact points lie on the
//!   block and the vertex point is the vertex itself.
//!
//! Both steps share the native order around the stage test (009e1220 stage
//! 0xf): a colliding contact outside the kernel stage keeps its connector
//! as it is; a separated contact deletes it (+0x1c). A new connector is
//! inserted into the kernel (00a95f22) before its first geometry update
//! (+0x24, which ends in 00a9457c), so `update` reports it as kept, then
//! kept again as reused.
use crate::{
    block_contact::{
        Block, BlockBlockContact, BlockTables, Connector, ContactEvent, ContactPoint, GeoPair,
        PairKind, UNSET, dot, extents_overlap, mul_transpose, normal_direction, sub, to_world,
    },
    g3d::unitize,
    material::Shape,
    poly_contact::{PolyContact, poly_shape},
    scene::CoordinateFrame,
};

/// Ball::setSize 00adc208: the radius of a ball of `size`.
pub fn radius(size: [f32; 3]) -> f32 {
    size[0] * 0.5
}

/// Primitive extents 00b61c9a for a ball (Ball +0x60 00adcb5e): (min, max).
pub fn ball_extents(frame: &CoordinateFrame, radius: f32) -> ([f32; 3], [f32; 3]) {
    let t = frame.position;
    (
        std::array::from_fn(|a| (t[a] - radius) + -0.01),
        std::array::from_fn(|a| t[a] + radius + 0.01),
    )
}

/// The geometry pair of a ball contact, in contact order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallPair {
    /// BallBallContact: body 0 is the contact's first primitive.
    BallBall,
    /// BallBlockContact: `ball` (0 or 1) is the ball, the native first
    /// primitive (`connector_prims` order); the other body is the block.
    BallBlock { ball: usize },
}

/// Block clamp 00adf544: the point pulled onto the box, the side of each
/// clamped axis (+1 or -1, 0 when inside) and how many axes were clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub point: [f32; 3],
    pub signs: [i16; 3],
    pub count: u32,
}

/// 00adf544 against the block half sizes `half`.
pub fn clip(half: [f32; 3], p: [f32; 3]) -> Clip {
    let mut c = Clip {
        point: p,
        signs: [0; 3],
        count: 0,
    };
    for ((p, sign), h) in c.point.iter_mut().zip(&mut c.signs).zip(half) {
        if h < *p {
            *p = h;
            c.count += 1;
            *sign = 1;
        }
        if *p < -h {
            *p = -h;
            c.count += 1;
            *sign = -1;
        }
    }
    c
}

/// The block feature a BallBlockConnector follows: its kind, the feature
/// id (+0x120: face NormalId or edge axis; unset for a vertex) and the
/// block vertex it starts from (+0x114).
pub fn feature(block: &Block, clip: &Clip) -> Result<(PairKind, i32, [f32; 3]), String> {
    let neg = |s: i16| usize::from(s < 1);
    let (kind, id, vertex) = match clip.count {
        0 => {
            // 00adf472: the nearest face to the center inside the box.
            let half = block.vertex(0);
            let mut min = f32::MAX;
            let mut face = None;
            for (i, (h, p)) in half.into_iter().zip(clip.point).enumerate() {
                let d = h - p;
                if d < min {
                    face = Some(i);
                    min = d;
                }
                let d = p + h;
                if d < min {
                    face = Some(i + 3);
                    min = d;
                }
            }
            // The native asserts min != FLT_MAX and reads an unset face.
            let face = face.ok_or("ball center has no nearest block face")?;
            (
                PairKind::BallPlane,
                face as i32,
                if face < 3 { 0 } else { 7 },
            )
        }
        1 => {
            // 00adf30c.
            let [x, y, z] = clip.signs;
            if x != 0 {
                (PairKind::BallPlane, (neg(x) * 3) as i32, neg(x) * 4)
            } else if y != 0 {
                (PairKind::BallPlane, (neg(y) * 3 + 1) as i32, neg(y) * 2)
            } else {
                (PairKind::BallPlane, (neg(z) * 3 + 2) as i32, neg(z))
            }
        }
        2 => {
            // 00adf206: the edge's -axis end.
            let [x, y, z] = clip.signs;
            if x == 0 {
                (PairKind::BallEdge, 0, neg(z) + 4 + neg(y) * 2)
            } else if y == 0 {
                (PairKind::BallEdge, 1, neg(z) + 2 + neg(x) * 4)
            } else {
                (PairKind::BallEdge, 2, neg(y) * 2 + 1 + neg(x) * 4)
            }
        }
        _ => {
            let [x, y, z] = clip.signs;
            (
                PairKind::BallVertex,
                UNSET,
                neg(x) * 4 + neg(y) * 2 + neg(z),
            )
        }
    };
    Ok((kind, id, block.vertex(vertex)))
}

/// BallBallContact::computeIsColliding 00ae2824.
pub fn balls_colliding(a: &Block, b: &Block, ignored: f32) -> bool {
    let reach = (radius(a.size) + radius(b.size)) - ignored;
    if reach <= 0.0 || reach.is_nan() {
        return false;
    }
    let d = sub(b.frame.position, a.frame.position);
    reach * reach > (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]
}

/// BallBlockContact::computeIsColliding 00ae2aec. Returns whether they
/// collide and the clamp of the ball center (None when the extents miss).
pub fn ball_block_colliding(ball: &Block, block: &Block, ignored: f32) -> (bool, Option<Clip>) {
    let r = radius(ball.size);
    if !extents_overlap(&ball_extents(&ball.frame, r), &block.extents()) {
        return (false, None);
    }
    let local = mul_transpose(
        &block.frame.rotation,
        sub(ball.frame.position, block.frame.position),
    );
    let c = clip(block.vertex(0), local);
    let d = sub(to_world(&block.frame, c.point), ball.frame.position);
    let distance = ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]).sqrt();
    (r - ignored > distance, Some(c))
}

/// BallBallConnector geometry 00a946c6: from A's center toward B's, the
/// point on A's surface.
pub(crate) fn ball_ball_point(
    a: &CoordinateFrame,
    b: &CoordinateFrame,
    radius_a: f32,
    radii: f32,
) -> ContactPoint {
    let mut normal = sub(b.position, a.position);
    let length = unitize(&mut normal, 1e-6) - radii;
    ContactPoint {
        normal,
        length,
        position: std::array::from_fn(|i| normal[i] * radius_a + a.position[i]),
    }
}

/// BallBlockConnector geometry 00a947b8, A the ball and B the block.
pub(crate) fn ball_block_point(
    pair: &GeoPair,
    ball: &CoordinateFrame,
    block: &CoordinateFrame,
) -> ContactPoint {
    let c = ball.position;
    let vertex = to_world(block, pair.point_b);
    let d = sub(vertex, c);
    match pair.kind {
        PairKind::BallPlane => {
            // 00a94800: the center projected onto the face plane. The
            // length is the unsigned distance less the radius.
            let m = normal_direction(&block.rotation, pair.id_b);
            let normal = m.map(|v| -v);
            let along = ((d[0] * normal[0]) - d[1] * m[1]) - d[2] * m[2];
            let v = [normal[0] * along, normal[1] * along, along * normal[2]];
            ContactPoint {
                normal,
                length: ((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]).sqrt() - pair.length_a,
                position: std::array::from_fn(|i| c[i] + v[i]),
            }
        }
        PairKind::BallEdge => {
            // 00a94a18: the nearest point of the edge line.
            let u = normal_direction(&block.rotation, pair.id_b);
            let t = dot(u, d);
            let mut normal = [d[0] - u[0] * t, d[1] - u[1] * t, d[2] - t * u[2]];
            let position = std::array::from_fn(|i| normal[i] + c[i]);
            let length = unitize(&mut normal, 1e-6) - pair.length_a;
            ContactPoint {
                normal,
                length,
                position,
            }
        }
        _ => {
            // 00a94c50.
            let mut normal = d;
            let length = unitize(&mut normal, 1e-6) - pair.length_a;
            ContactPoint {
                normal,
                length,
                position: vertex,
            }
        }
    }
}

/// BallBallContact or BallBlockContact (contact +0x30: the one connector).
#[derive(Clone, Debug)]
pub struct BallContact<T> {
    pub pair: BallPair,
    connector: Option<Connector<T>>,
    /// Connector events of the last `update`.
    pub events: Vec<ContactEvent<T>>,
}

impl<T: Default> BallContact<T> {
    pub fn new(pair: BallPair) -> Self {
        Self {
            pair,
            connector: None,
            events: Vec::new(),
        }
    }
    /// The live connector, if any.
    pub fn connectors(&self) -> &[Connector<T>] {
        self.connector.as_slice()
    }
    pub fn connectors_mut(&mut self) -> &mut [Connector<T>] {
        self.connector.as_mut_slice()
    }
    /// Take the live connector out of the contact.
    pub fn take_connectors(&mut self) -> Vec<Connector<T>> {
        self.connector.take().into_iter().collect()
    }
    /// Connector geometry refresh (virtual +0x24) for the current frames.
    pub fn refresh(&mut self, blocks: [&Block; 2]) {
        if let Some(c) = &mut self.connector {
            c.point = c.pair.compute(blocks);
        }
    }
    /// computeIsColliding (vtable +0x28) with `ignored` overlap.
    pub fn compute_is_colliding(&self, blocks: [&Block; 2], ignored: f32) -> bool {
        match self.pair {
            BallPair::BallBall => balls_colliding(blocks[0], blocks[1], ignored),
            BallPair::BallBlock { ball } => {
                ball_block_colliding(blocks[ball], blocks[1 - ball], ignored).0
            }
        }
    }
    /// The contact step (vtable +0x20): 00ae28ea for two balls, 00ae2d16 for
    /// a ball and a block. `in_kernel` is the stage 0xf test. Returns
    /// whether they collide.
    pub fn update(&mut self, blocks: [&Block; 2], in_kernel: bool) -> Result<bool, String> {
        self.events.clear();
        let (colliding, clip) = match self.pair {
            BallPair::BallBall => (balls_colliding(blocks[0], blocks[1], 0.0), None),
            BallPair::BallBlock { ball } => {
                ball_block_colliding(blocks[ball], blocks[1 - ball], 0.0)
            }
        };
        if !colliding {
            if let Some(connector) = self.connector.take() {
                self.events.push(ContactEvent::Removed {
                    connector,
                    recomputed: false,
                });
            }
            return Ok(false);
        }
        if !in_kernel {
            return Ok(true);
        }
        let (body_a, body_b) = match self.pair {
            BallPair::BallBall => (0, 1),
            BallPair::BallBlock { ball } => (ball, 1 - ball),
        };
        let created = self.connector.is_none();
        let connector = self.connector.get_or_insert_with(|| Connector {
            pair: GeoPair {
                kind: PairKind::BallBall,
                point_a: [0.0; 3],
                point_b: [0.0; 3],
                body_a,
                body_b,
                length_a: f32::NAN,
                length_b: f32::NAN,
                id_a: UNSET,
                id_b: UNSET,
                id_c: UNSET,
                poly: None,
            },
            point: ContactPoint {
                normal: [0.0; 3],
                length: 0.0,
                position: [0.0; 3],
            },
            state: T::default(),
        });
        let ra = radius(blocks[body_a].size);
        match clip {
            None => {
                connector.pair.length_a = ra;
                connector.pair.length_b = ra + radius(blocks[body_b].size);
            }
            Some(clip) => {
                let (kind, id, vertex) = feature(blocks[body_b], &clip)?;
                connector.pair.kind = kind;
                connector.pair.point_b = vertex;
                connector.pair.id_b = id;
                connector.pair.length_a = ra;
            }
        }
        connector.point = connector.pair.compute(blocks);
        if created {
            self.events.push(ContactEvent::Kept {
                index: 0,
                reused: false,
            });
        }
        self.events.push(ContactEvent::Kept {
            index: 0,
            reused: true,
        });
        Ok(true)
    }
}

/// A contact pair's narrowphase, by the primitives' collide types
/// (ContactManager::createContact 00ae714a).
#[derive(Clone, Debug)]
pub enum PrimContact<T> {
    Block(BlockBlockContact<T>),
    Ball(BallContact<T>),
    /// PolyPolyContact (`poly_contact`): a Wedge against a Block or Wedge.
    Poly(PolyContact<T>),
    /// BallPolyContact (00add538), not ported: its step is an error.
    BallPoly,
}

impl<T: Default> PrimContact<T> {
    /// BallBallContact, BallBlockContact (the ball is the native first
    /// primitive), BlockBlockContact or PolyPolyContact.
    pub fn new(shapes: [Shape; 2]) -> Self {
        match shapes {
            [Shape::Ball, Shape::Ball] => Self::Ball(BallContact::new(BallPair::BallBall)),
            [Shape::Ball, Shape::Block] => {
                Self::Ball(BallContact::new(BallPair::BallBlock { ball: 0 }))
            }
            [Shape::Block, Shape::Ball] => {
                Self::Ball(BallContact::new(BallPair::BallBlock { ball: 1 }))
            }
            [Shape::Block, Shape::Block] => Self::Block(BlockBlockContact::default()),
            [Shape::Ball, Shape::Wedge] | [Shape::Wedge, Shape::Ball] => Self::BallPoly,
            [a, b] => Self::Poly(PolyContact::new([
                poly_shape(a).expect("poly shape"),
                poly_shape(b).expect("poly shape"),
            ])),
        }
    }
    pub fn connectors(&self) -> &[Connector<T>] {
        match self {
            Self::Block(c) => c.connectors(),
            Self::Ball(c) => c.connectors(),
            Self::Poly(c) => c.connectors(),
            Self::BallPoly => &[],
        }
    }
    pub fn connectors_mut(&mut self) -> &mut [Connector<T>] {
        match self {
            Self::Block(c) => c.connectors_mut(),
            Self::Ball(c) => c.connectors_mut(),
            Self::Poly(c) => c.connectors_mut(),
            Self::BallPoly => &mut [],
        }
    }
    /// Whether connector `index` is in the kernel. Only a PolyContact keeps
    /// connectors outside it.
    pub fn is_live(&self, index: usize) -> bool {
        match self {
            Self::Poly(c) => c.is_live(index),
            _ => index < self.connectors().len(),
        }
    }
    /// The kernel refused connector `index` (neither body in a kernel
    /// list): a PolyContact offers it again next step.
    pub fn set_outside(&mut self, index: usize) {
        if let Self::Poly(c) = self {
            c.set_outside(index);
        }
    }
    pub fn take_connectors(&mut self) -> Vec<Connector<T>> {
        match self {
            Self::Block(c) => c.take_connectors(),
            Self::Ball(c) => c.take_connectors(),
            Self::Poly(c) => c.take_connectors(),
            Self::BallPoly => Vec::new(),
        }
    }
    pub fn take_events(&mut self) -> Vec<ContactEvent<T>> {
        match self {
            Self::Block(c) => std::mem::take(&mut c.events),
            Self::Ball(c) => std::mem::take(&mut c.events),
            Self::Poly(c) => c.take_events(),
            Self::BallPoly => Vec::new(),
        }
    }
    /// computeIsColliding (vtable +0x28) with no ignored overlap. A
    /// PolyContact rebuilds its connectors here: take its events after.
    pub fn compute_is_colliding(&mut self, blocks: [&Block; 2]) -> bool {
        match self {
            Self::Block(c) => c.compute_is_colliding(blocks, 0.0).0,
            Self::Ball(c) => c.compute_is_colliding(blocks, 0.0),
            Self::Poly(c) => c.compute_is_colliding(blocks, 0.0),
            Self::BallPoly => false,
        }
    }
    /// The contact step (vtable +0x20).
    pub fn update(
        &mut self,
        blocks: [&Block; 2],
        in_kernel: bool,
        tables: &BlockTables,
    ) -> Result<bool, String> {
        match self {
            Self::Block(c) => c.update(blocks, in_kernel, tables),
            Self::Ball(c) => c.update(blocks, in_kernel),
            Self::Poly(c) => Ok(c.update(blocks, in_kernel)),
            Self::BallPoly => Err("a Ball against a Wedge (BallPolyContact) is not ported".into()),
        }
    }
    /// Connector geometry refresh (virtual +0x24) for the current frames.
    pub fn refresh(&mut self, blocks: [&Block; 2]) {
        match self {
            Self::Block(c) => c.refresh(blocks),
            Self::Ball(c) => c.refresh(blocks),
            Self::Poly(c) => c.refresh(blocks),
            Self::BallPoly => {}
        }
    }
    /// Forget the cached features (a fresh contact of the same kind).
    pub fn reset(&mut self) {
        match self {
            Self::Block(c) => *c = BlockBlockContact::default(),
            Self::Ball(c) => *c = BallContact::new(c.pair),
            Self::Poly(c) => c.reset(),
            Self::BallPoly => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    fn part(position: [f32; 3], size: [f32; 3]) -> Block {
        Block {
            frame: CoordinateFrame {
                position,
                rotation: IDENTITY,
            },
            size,
        }
    }
    #[test]
    fn two_balls_touch_along_the_center_line() {
        let a = part([0.0, 0.0, 0.0], [2.0; 3]);
        let b = part([0.0, 2.9, 0.0], [4.0; 3]);
        assert!(balls_colliding(&a, &b, 0.0));
        assert!(!balls_colliding(&a, &b, 0.2));
        let mut contact = BallContact::<()>::new(BallPair::BallBall);
        assert!(contact.update([&a, &b], true).unwrap());
        assert_eq!(contact.events.len(), 2);
        let c = &contact.connectors()[0];
        assert!((c.point.normal[1] - 1.0).abs() < 1e-6 && c.point.normal[0] == 0.0);
        assert!((c.point.length + 0.1).abs() < 1e-6, "{}", c.point.length);
        assert!((c.point.position[1] - 1.0).abs() < 1e-6);
        // Kept again while touching, deleted once apart.
        assert!(contact.update([&a, &b], true).unwrap());
        assert_eq!(contact.events.len(), 1);
        let far = part([0.0, 3.1, 0.0], [4.0; 3]);
        assert!(!contact.update([&a, &far], true).unwrap());
        assert!(matches!(contact.events[0], ContactEvent::Removed { .. }));
        assert!(contact.connectors().is_empty());
    }
    #[test]
    fn a_ball_on_a_face_projects_onto_the_plane() {
        // Ball of radius 1 sunk 0.05 into the top of a 4 x 1.2 x 2 block.
        let block = part([0.0, 0.0, 0.0], [4.0, 1.2, 2.0]);
        let ball = part([0.5, 1.55, 0.0], [2.0; 3]);
        let mut contact = BallContact::<()>::new(BallPair::BallBlock { ball: 1 });
        assert!(contact.update([&block, &ball], true).unwrap());
        let c = &contact.connectors()[0];
        assert_eq!((c.pair.kind, c.pair.id_b), (PairKind::BallPlane, 1));
        assert_eq!((c.pair.body_a, c.pair.body_b), (1, 0));
        assert_eq!(c.pair.point_b, [2.0, 0.6, 1.0]);
        assert_eq!(c.point.normal, [-0.0, -1.0, -0.0]);
        assert!((c.point.length + 0.05).abs() < 1e-6, "{}", c.point.length);
        assert!((c.point.position[1] - 0.6).abs() < 1e-6);
        assert_eq!(c.point.position[0], 0.5);
    }
    #[test]
    fn a_ball_off_an_edge_and_a_corner() {
        let block = part([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        // Beyond +x and +y, inside z: the +z/-z edge along axis 2.
        let edge = part([1.5, 1.5, 0.25], [2.0; 3]);
        let (hit, clip) = ball_block_colliding(&edge, &block, 0.0);
        let clip = clip.unwrap();
        assert!(hit);
        assert_eq!((clip.count, clip.signs), (2, [1, 1, 0]));
        let (kind, id, vertex) = feature(&block, &clip).unwrap();
        assert_eq!(
            (kind, id, vertex),
            (PairKind::BallEdge, 2, [1.0, 1.0, -1.0])
        );
        let mut contact = BallContact::<()>::new(BallPair::BallBlock { ball: 0 });
        contact.update([&edge, &block], true).unwrap();
        let p = &contact.connectors()[0].point;
        assert!((p.position[2] - 0.25).abs() < 1e-6);
        assert!((p.length - (0.5f32.sqrt() - 1.0)).abs() < 1e-6);
        // Beyond every axis: vertex 0 (+, +, +).
        let corner = part([1.4, 1.4, 1.4], [2.0; 3]);
        let (hit, clip) = ball_block_colliding(&corner, &block, 0.0);
        assert!(hit);
        let (kind, _, vertex) = feature(&block, &clip.unwrap()).unwrap();
        assert_eq!((kind, vertex), (PairKind::BallVertex, [1.0, 1.0, 1.0]));
        let far = part([1.7, 1.7, 1.7], [2.0; 3]);
        assert!(!ball_block_colliding(&far, &block, 0.0).0);
    }
    #[test]
    fn a_center_inside_the_block_takes_the_nearest_face() {
        let block = part([0.0, 0.0, 0.0], [4.0, 2.0, 4.0]);
        let ball = part([0.0, -0.8, 1.0], [1.0; 3]);
        let (hit, clip) = ball_block_colliding(&ball, &block, 0.0);
        assert!(hit);
        let (kind, id, vertex) = feature(&block, &clip.unwrap()).unwrap();
        assert_eq!(
            (kind, id, vertex),
            (PairKind::BallPlane, 4, [-2.0, -1.0, -2.0])
        );
    }
    #[test]
    fn outside_the_kernel_the_connector_is_left_alone() {
        let a = part([0.0, 0.0, 0.0], [2.0; 3]);
        let b = part([0.0, 1.9, 0.0], [2.0; 3]);
        let mut contact = BallContact::<()>::new(BallPair::BallBall);
        assert!(contact.update([&a, &b], false).unwrap());
        assert!(contact.events.is_empty() && contact.connectors().is_empty());
    }
}
