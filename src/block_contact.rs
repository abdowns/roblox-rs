//! BlockBlockContact narrowphase (vtable 01c86860): the box/box separating
//! axis test, face clipping and the contact connectors it keeps between world
//! steps, with the native GeoPair contact points (types 3, 4 and 5).
//!
//! Vectors and matrices use the G3D source order: a dot product or matrix row
//! is ((x + y) + z) and a point is (R * p) + t, as the disassembly of every
//! function below shows. Body index 0 is the contact's first primitive
//! (Contact +0x10) and 1 the second (+0x14).
use crate::{bundle::Bundle, g3d::unitize, native_data::NativeImage, scene::CoordinateFrame};
use std::fmt::Write;

type Matrix = [[f32; 3]; 3];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}
fn mul(m: &Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (m[i][0] * v[0] + m[i][1] * v[1]) + m[i][2] * v[2])
}
fn mul_transpose(m: &Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (m[0][i] * v[0] + m[1][i] * v[1]) + m[2][i] * v[2])
}
fn to_world(f: &CoordinateFrame, p: [f32; 3]) -> [f32; 3] {
    let r = mul(&f.rotation, p);
    std::array::from_fn(|i| r[i] + f.position[i])
}
fn column(m: &Matrix, axis: usize) -> [f32; 3] {
    [m[0][axis], m[1][axis], m[2][axis]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
/// NormalId sign: (id / 3) * -2 | 1.
fn sign(id: i32) -> f32 {
    if id / 3 == 0 { 1.0 } else { -1.0 }
}
/// Direction of a NormalId on a body: sign * rotation column.
fn normal_direction(m: &Matrix, id: i32) -> [f32; 3] {
    let axis = id.rem_euclid(3) as usize;
    let s = sign(id);
    [m[0][axis] * s, m[1][axis] * s, s * m[2][axis]]
}
/// Edge NormalId used as the GeoPair id: (e % 2) * 3 + e / 4 (ae33a8/ae4e72).
fn edge_id(edge: usize) -> i32 {
    ((edge % 2) * 3 + edge / 4) as i32
}

/// Block face vertex table 017a3e68 and face edge table 017a3ec8, read from
/// the install. An edge e (axis e / 4, quadrant e % 4) starts at the vertex
/// `face_vertices` holds at flat index e (Block::getEdgeVertex 00ae582e).
#[derive(Clone, Debug)]
pub struct BlockTables {
    pub face_vertices: [[usize; 4]; 6],
    pub face_edges: [[usize; 4]; 6],
}

impl BlockTables {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let data = image.region(0x017a3e68, 192)?;
        let read = |table: usize| -> [[usize; 4]; 6] {
            std::array::from_fn(|f| {
                std::array::from_fn(|i| {
                    let at = table * 96 + (f * 4 + i) * 4;
                    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
                })
            })
        };
        let tables = Self {
            face_vertices: read(0),
            face_edges: read(1),
        };
        if tables.face_vertices.iter().flatten().any(|v| *v >= 8)
            || tables.face_edges.iter().flatten().any(|e| *e >= 12)
        {
            return Err("invalid native block face tables".into());
        }
        Ok(tables)
    }
    fn edge_vertex(&self, edge: usize) -> usize {
        self.face_vertices[edge / 4][edge % 4]
    }
}

/// A block primitive: its body frame and Part size.
#[derive(Clone, Debug)]
pub struct Block {
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
}

impl Block {
    /// Block vertex i (00ae07f2): X outer, Y middle, Z inner; + before -.
    pub fn vertex(&self, i: usize) -> [f32; 3] {
        std::array::from_fn(|a| self.size[a] * 0.5 * if i & (4 >> a) == 0 { 1.0 } else { -1.0 })
    }
    /// Primitive extents b61c9a: half extents from vertices 0..3 (adf0a0),
    /// widened by 0.01 on each side.
    pub fn extents(&self) -> ([f32; 3], [f32; 3]) {
        let r = &self.frame.rotation;
        let mut half = mul(r, self.vertex(0)).map(f32::abs);
        for i in 1..4 {
            let p = mul(r, self.vertex(i));
            for a in 0..3 {
                if half[a] <= p[a].abs() {
                    half[a] = p[a].abs();
                }
            }
        }
        let t = self.frame.position;
        (
            std::array::from_fn(|a| (t[a] - half[a]) + -0.01),
            std::array::from_fn(|a| (t[a] + half[a]) + 0.01),
        )
    }
}

/// Extents overlap aec6e4 (touching counts).
pub fn extents_overlap(a: &([f32; 3], [f32; 3]), b: &([f32; 3], [f32; 3])) -> bool {
    let (alo, ahi) = a;
    let (blo, bhi) = b;
    if alo[0] > bhi[0] || alo[1] > bhi[1] || alo[2] > bhi[2] || blo[1] > ahi[1] || blo[0] > ahi[0] {
        return false;
    }
    blo[2] <= ahi[2]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairKind {
    /// Native 3: a vertex of body A against a face plane of body B.
    VertexPlane,
    /// Native 4: an edge of A against an edge of B's face, clipped to both.
    EdgeFace,
    /// Native 5: two infinite edge lines.
    EdgeEdge,
}

/// GeoPair fields the native constructor leaves uninitialized (pool memory).
pub const UNSET: i32 = i32::MIN;

/// GeoPair (connector +0x78).
#[derive(Clone, Debug)]
pub struct GeoPair {
    pub kind: PairKind,
    pub point_a: [f32; 3],
    pub point_b: [f32; 3],
    pub body_a: usize,
    pub body_b: usize,
    pub length_a: f32,
    pub length_b: f32,
    /// +0x2c, +0x30, +0x34.
    pub id_a: i32,
    pub id_b: i32,
    pub id_c: i32,
}

/// The connector contact point (connector +0xdc): normal, signed length
/// (negative = penetrating) and world position.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactPoint {
    pub normal: [f32; 3],
    pub length: f32,
    pub position: [f32; 3],
}

impl GeoPair {
    fn new(kind: PairKind, body_a: usize, body_b: usize) -> Self {
        Self {
            kind,
            point_a: [0.0; 3],
            point_b: [0.0; 3],
            body_a,
            body_b,
            length_a: f32::NAN,
            length_b: f32::NAN,
            id_a: UNSET,
            id_b: UNSET,
            id_c: UNSET,
        }
    }
    /// Lookup comparison ae6024, by the queried kind (the stored kind is not
    /// compared).
    fn matches(&self, a: usize, b: usize, kind: PairKind, first: i32, second: i32) -> bool {
        match kind {
            PairKind::EdgeFace => {
                self.body_a == a && self.body_b == b && self.id_a == first && self.id_b == second
            }
            PairKind::VertexPlane => {
                self.body_a == a && self.body_b == b && self.id_c == first && self.id_b == second
            }
            PairKind::EdgeEdge => {
                (self.body_a == a && self.body_b == b && self.id_a == first && self.id_b == second)
                    || (self.body_b == a
                        && self.body_a == b
                        && self.id_b == first
                        && self.id_a == second)
            }
        }
    }
    /// GeoPair::computeContactPoint (ae5f1a).
    pub fn compute(&self, blocks: [&Block; 2]) -> ContactPoint {
        let a = &blocks[self.body_a].frame;
        let b = &blocks[self.body_b].frame;
        match self.kind {
            PairKind::VertexPlane => self.vertex_plane(a, b),
            PairKind::EdgeFace => self.edge_face(a, b),
            PairKind::EdgeEdge => self.edge_edge(a, b),
        }
    }
    /// aa1d78.
    fn vertex_plane(&self, a: &CoordinateFrame, b: &CoordinateFrame) -> ContactPoint {
        let position = to_world(a, self.point_a);
        let plane = to_world(b, self.point_b);
        let m = normal_direction(&b.rotation, self.id_b);
        let d = sub(plane, position);
        ContactPoint {
            normal: m.map(|v| -v),
            length: (d[0] * -m[0] - d[1] * m[1]) - d[2] * m[2],
            position,
        }
    }
    /// aa1f76: closest points of the two edge lines in A space, kept only
    /// while both lie within their edges (+0.05).
    fn edge_face(&self, a: &CoordinateFrame, b: &CoordinateFrame) -> ContactPoint {
        let ua = mul_transpose(&a.rotation, normal_direction(&a.rotation, self.id_a));
        let pb = mul_transpose(&a.rotation, sub(to_world(b, self.point_b), a.position));
        let ub = mul_transpose(&a.rotation, normal_direction(&b.rotation, self.id_b));
        let normal = normal_direction(&b.rotation, self.id_c).map(|v| -v);
        let c = dot(ua, ub);
        let fallback = ContactPoint {
            normal,
            length: 0.0,
            position: to_world(a, self.point_a),
        };
        if (f64::from(c.abs()) + -1.0).abs() <= 1e-5 {
            return fallback;
        }
        let w = sub(self.point_a, pb);
        let wa = dot(w, ua);
        let wb = dot(w, ub);
        let wac = wa * c;
        let denominator = 1.0 - c * c;
        let s = (c * wb - c * wac) / denominator - wa;
        let t = (wb - wac) / denominator;
        let qa: [f32; 3] = std::array::from_fn(|i| ua[i] * s + self.point_a[i]);
        let qb: [f32; 3] = std::array::from_fn(|i| ub[i] * t + pb[i]);
        let qb_in_b = mul_transpose(&b.rotation, sub(to_world(a, qb), b.position));
        let within = |v: f32, length: f32| {
            f64::from(v.abs()) <= f64::from(length) * 0.5 + f64::from(0.05f32)
        };
        if !within(qa[self.id_a.rem_euclid(3) as usize], self.length_a)
            || !within(qb_in_b[self.id_b.rem_euclid(3) as usize], self.length_b)
        {
            return fallback;
        }
        let position = to_world(a, qa);
        ContactPoint {
            normal,
            length: dot(sub(to_world(b, self.point_b), position), normal),
            position,
        }
    }
    /// aa29c0: closest points of two infinite lines.
    fn edge_edge(&self, a: &CoordinateFrame, b: &CoordinateFrame) -> ContactPoint {
        let da = normal_direction(&a.rotation, self.id_a);
        let db = normal_direction(&b.rotation, self.id_b);
        let c = dot(da, db);
        let denominator = 1.0 - c * c;
        let pa = to_world(a, self.point_a);
        if denominator <= 1e-6 || denominator.is_nan() {
            return ContactPoint {
                normal: da,
                length: 0.0,
                position: pa,
            };
        }
        let pb = to_world(b, self.point_b);
        let d = sub(pb, pa);
        let e = dot(d, da);
        let f = dot(d, db);
        let inverse = 1.0 / denominator;
        let s = (e - f * c) * inverse;
        let t = (e * c - f) * inverse;
        let qa: [f32; 3] = std::array::from_fn(|i| da[i] * s + pa[i]);
        let qb: [f32; 3] = std::array::from_fn(|i| db[i] * t + pb[i]);
        let mut normal = sub(qa, qb);
        let length = -unitize(&mut normal, 1e-6);
        ContactPoint {
            normal,
            length,
            position: std::array::from_fn(|i| (qa[i] + qb[i]) * 0.5),
        }
    }
}

/// A contact connector: its pair, current contact point and the kernel state
/// (spring/impulse accumulators) that persists while the connector is reused.
#[derive(Clone, Debug)]
pub struct Connector<T> {
    pub pair: GeoPair,
    pub point: ContactPoint,
    pub state: T,
}

/// A connector lifetime event of the last `update`, in native order: kernel
/// insertion happens as each new connector is generated (ae376c, ae4e72,
/// ae33a8 call 00a95f22), deletion of a found-but-rejected connector at
/// once (ae277e), and of the unmatched leftovers at the end (+0x1c).
#[derive(Clone, Debug)]
pub enum ContactEvent<T> {
    /// Kept at `index` of the live buffer. `reused` when it was found again
    /// and its geometry recomputed (virtual +0x24).
    Kept { index: usize, reused: bool },
    /// Deleted. `recomputed` when its geometry was refreshed first (edge
    /// connectors rejected after +0x24).
    Removed {
        connector: Connector<T>,
        recomputed: bool,
    },
}

/// Native connector buffer capacity (BlockBlockContact data, 8 per buffer).
const CAPACITY: usize = 8;

/// BlockBlockContact data (+0x30, 0x70 bytes, initialized by ae4916).
#[derive(Clone, Debug)]
pub struct BlockBlockContact<T> {
    buffers: [Vec<Connector<T>>; 2],
    current: usize,
    pub separating_body: usize,
    pub separating_axis: usize,
    /// Face 0..5, edge + 6, or -1.
    pub features: [i32; 2],
    pub reference_body: usize,
    pub incident_body: usize,
    pub reference_face: i32,
    pub incident_face: i32,
    /// Connector events of the last `update`.
    pub events: Vec<ContactEvent<T>>,
}

impl<T> Default for BlockBlockContact<T> {
    fn default() -> Self {
        Self {
            buffers: [Vec::new(), Vec::new()],
            current: 0,
            separating_body: 0,
            separating_axis: 0,
            features: [-1, -1],
            reference_body: 0,
            incident_body: 0,
            reference_face: 0,
            incident_face: 0,
            events: Vec::new(),
        }
    }
}

/// Edge quadrant adf6ac: the edge of `axis` nearest `direction`.
fn edge_toward(rotation: &Matrix, axis: usize, direction: [f32; 3]) -> i32 {
    let l = mul_transpose(rotation, direction);
    let (u, v) = match axis {
        0 => (l[2], l[1]),
        1 => (l[0], l[2]),
        _ => (l[1], l[0]),
    };
    let quadrant = if u > 0.0 {
        if v > 0.0 { 0 } else { 1 }
    } else if v > 0.0 {
        3
    } else {
        2
    };
    (axis * 4 + quadrant) as i32
}

/// Face of `rotation` most aligned with world `direction` (006b2077).
fn face_toward(rotation: &Matrix, direction: [f32; 3]) -> i32 {
    let l = mul_transpose(rotation, direction);
    let axis = if l[0].abs() <= l[1].abs() {
        if l[2].abs() < l[1].abs() { 1 } else { 2 }
    } else if l[2].abs() < l[0].abs() {
        0
    } else {
        2
    };
    axis as i32 + if l[axis] <= 0.0 { 3 } else { 0 }
}

/// Face plane coordinates adf63a.
fn project(p: [f32; 3], face: i32) -> [f32; 2] {
    match face {
        0 => [p[1], p[2]],
        1 => [p[2], p[0]],
        2 => [p[0], p[1]],
        3 => [p[2], p[1]],
        4 => [p[0], p[2]],
        _ => [p[1], p[0]],
    }
}

fn inverse(f: &CoordinateFrame) -> CoordinateFrame {
    let rotation: Matrix = std::array::from_fn(|r| std::array::from_fn(|c| f.rotation[c][r]));
    let negated = rotation.map(|row| row.map(|v| -v));
    CoordinateFrame {
        position: mul(&negated, f.position),
        rotation,
    }
}
fn mat_mul(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| (a[r][0] * b[0][c] + a[r][1] * b[1][c]) + a[r][2] * b[2][c])
    })
}
/// CoordinateFrame multiply 00972080.
fn compose(a: &CoordinateFrame, b: &CoordinateFrame) -> CoordinateFrame {
    CoordinateFrame {
        position: to_world(a, b.position),
        rotation: mat_mul(&a.rotation, &b.rotation),
    }
}

impl<T: Default> BlockBlockContact<T> {
    /// The live connectors (the current buffer), in generation order.
    pub fn connectors(&self) -> &[Connector<T>] {
        &self.buffers[self.current]
    }
    pub fn connectors_mut(&mut self) -> &mut [Connector<T>] {
        &mut self.buffers[self.current]
    }
    /// Connector geometry refresh (virtual +0x24) for the current frames.
    pub fn refresh(&mut self, blocks: [&Block; 2]) {
        for c in &mut self.buffers[self.current] {
            c.point = c.pair.compute(blocks);
        }
    }
    /// computeIsColliding ae3de4. Returns (colliding, plane contact).
    pub fn compute_is_colliding(&mut self, blocks: [&Block; 2], ignored: f32) -> (bool, bool) {
        let previous = self.features;
        let previous_face = (previous[0] as u32) < 6 || (previous[1] as u32) < 6;
        let mut face_min = f32::INFINITY;
        let mut edge_min = f32::INFINITY;
        let mut previous_overlap = f32::INFINITY;
        let mut plane = false;
        let mut b = self.separating_body;
        loop {
            let i = b % 2;
            let o = (b + 1) % 2;
            let (fi, fo) = (&blocks[i].frame, &blocks[o].frame);
            let (hi, ho) = (blocks[i].vertex(0), blocks[o].vertex(0));
            let local = mul_transpose(&fi.rotation, sub(fo.position, fi.position));
            let mut a = self.separating_axis;
            loop {
                let axis = a % 3;
                let p = mul_transpose(&fo.rotation, column(&fi.rotation, axis));
                let overlap = (((p[0] * ho[0]).abs() + (p[1] * ho[1]).abs())
                    + (p[2] * ho[2]).abs()
                    + hi[axis])
                    - local[axis].abs();
                if overlap <= ignored || overlap.is_nan() {
                    self.separating_body = i;
                    self.separating_axis = axis;
                    return (false, plane);
                }
                if previous_face && previous[i] % 3 == axis as i32 {
                    previous_overlap = overlap;
                }
                if face_min > overlap {
                    self.features[i] = (axis + if local[axis] > 0.0 { 0 } else { 3 }) as i32;
                    self.features[o] = -1;
                    self.separating_body = i;
                    self.separating_axis = axis;
                    plane = true;
                    face_min = overlap;
                }
                let more = a < self.separating_axis + 2;
                a += 1;
                if !more {
                    break;
                }
            }
            let more = b <= self.separating_body;
            b += 1;
            if !more {
                break;
            }
        }
        if previous_face {
            let wider = previous_overlap > face_min * 1.01;
            let keep = if self.features[0] == previous[0] {
                wider || self.features[1] == previous[1]
            } else {
                wider
            };
            if !keep {
                self.features = previous;
                face_min = previous_overlap;
            }
        }
        let (fa, fb) = (&blocks[0].frame, &blocks[1].frame);
        let (ha, hb) = (blocks[0].vertex(0), blocks[1].vertex(0));
        let d = sub(fb.position, fa.position);
        for i in 0..3 {
            for j in 0..3 {
                let x = column(&fa.rotation, i);
                let y = column(&fb.rotation, j);
                let mut n = [
                    x[1] * y[2] - x[2] * y[1],
                    x[2] * y[0] - y[2] * x[0],
                    y[1] * x[0] - x[1] * y[0],
                ];
                if 0.001 >= unitize(&mut n, 1e-6) {
                    return (plane, plane);
                }
                let distance = dot(d, n);
                let pa = mul_transpose(&fa.rotation, n);
                let pb = mul_transpose(&fb.rotation, n);
                let overlap = (((pb[0] * hb[0]).abs() + (pb[1] * hb[1]).abs())
                    + (pb[2] * hb[2]).abs()
                    + (((pa[0] * ha[0]).abs() + (pa[1] * ha[1]).abs()) + (pa[2] * ha[2]).abs()))
                    - distance.abs();
                if overlap <= ignored || overlap.is_nan() {
                    return (false, false);
                }
                if edge_min > overlap {
                    edge_min = overlap;
                    if f64::from(face_min) > f64::from(overlap) * 10.0 {
                        let minus = n.map(|v| -v);
                        let (to_b, to_a) = if distance > 0.0 {
                            (n, minus)
                        } else {
                            (minus, n)
                        };
                        self.features[0] = edge_toward(&fa.rotation, i, to_b) + 6;
                        self.features[1] = edge_toward(&fb.rotation, j, to_a) + 6;
                        plane = false;
                    }
                }
            }
        }
        (true, plane)
    }
    /// BlockBlockContact::update ae32f4 (extents test ae31f8 first).
    /// `in_kernel` is the native kernel-stage check (009e1220 stage 0xf).
    pub fn update(
        &mut self,
        blocks: [&Block; 2],
        in_kernel: bool,
        tables: &BlockTables,
    ) -> Result<bool, String> {
        self.events.clear();
        let (colliding, plane) = if extents_overlap(&blocks[0].extents(), &blocks[1].extents()) {
            self.compute_is_colliding(blocks, 0.0)
        } else {
            (false, false)
        };
        if !colliding {
            self.remove_leftovers();
            self.features = [-1, -1];
            return Ok(false);
        }
        if in_kernel {
            if plane {
                self.face_connectors(blocks, tables)?;
            } else {
                let [a, b] = self.features;
                self.edge_edge(blocks, tables, (a - 6) as usize, (b - 6) as usize)?;
            }
            self.remove_leftovers();
            self.current ^= 1;
        }
        Ok(true)
    }
    /// Lookup ae30a8: swap-remove from the current buffer, rebinding bodies.
    fn take(
        &mut self,
        a: usize,
        b: usize,
        kind: PairKind,
        first: i32,
        second: i32,
    ) -> Option<Connector<T>> {
        let buffer = &mut self.buffers[self.current];
        let at = buffer
            .iter()
            .position(|c| c.pair.matches(a, b, kind, first, second))?;
        let mut found = buffer.swap_remove(at);
        found.pair.body_a = a;
        found.pair.body_b = b;
        Some(found)
    }
    fn keep(&mut self, connector: Connector<T>, reused: bool) -> Result<(), String> {
        let buffer = &mut self.buffers[self.current ^ 1];
        if buffer.len() == CAPACITY {
            return Err("BlockBlockContact connector buffer overflow".into());
        }
        buffer.push(connector);
        let index = buffer.len() - 1;
        self.events.push(ContactEvent::Kept { index, reused });
        Ok(())
    }
    /// Delete the unmatched connectors of the current buffer (+0x1c).
    fn remove_leftovers(&mut self) {
        for connector in self.buffers[self.current].drain(..) {
            self.events.push(ContactEvent::Removed {
                connector,
                recomputed: false,
            });
        }
    }
    /// Plane-side test ae3bd4: the vertex is more than 0.01 inside the face.
    #[allow(clippy::too_many_arguments)] // Native argument list.
    fn below_plane(
        blocks: [&Block; 2],
        tables: &BlockTables,
        vertex_body: usize,
        plane_body: usize,
        corner: usize,
        vertex_face: i32,
        plane_face: i32,
    ) -> bool {
        let v = blocks[vertex_body].vertex(tables.face_vertices[vertex_face as usize][corner]);
        let p = &blocks[plane_body];
        let world = sub(to_world(&blocks[vertex_body].frame, v), p.frame.position);
        let local = mul_transpose(&p.frame.rotation, world);
        let plane = p.vertex(tables.face_vertices[plane_face as usize][0]);
        let mut n = [0.0; 3];
        n[plane_face as usize % 3] = sign(plane_face);
        dot(sub(local, plane), n) < -0.01
    }
    /// Vertex/plane connector ae376c.
    #[allow(clippy::too_many_arguments)] // Native argument list.
    fn vertex_plane(
        &mut self,
        blocks: [&Block; 2],
        tables: &BlockTables,
        vertex_body: usize,
        plane_body: usize,
        corner: usize,
        vertex_face: i32,
        plane_face: i32,
    ) -> Result<(), String> {
        let found = self.take(
            vertex_body,
            plane_body,
            PairKind::VertexPlane,
            corner as i32,
            plane_face,
        );
        let accept = Self::below_plane(
            blocks,
            tables,
            vertex_body,
            plane_body,
            corner,
            vertex_face,
            plane_face,
        );
        if !accept {
            if let Some(connector) = found {
                self.events.push(ContactEvent::Removed {
                    connector,
                    recomputed: false,
                });
            }
            return Ok(());
        }
        let reused = found.is_some();
        let mut connector = found.unwrap_or_else(|| Connector {
            pair: GeoPair::new(PairKind::VertexPlane, vertex_body, plane_body),
            point: ContactPoint {
                normal: [0.0; 3],
                length: 0.0,
                position: [0.0; 3],
            },
            state: T::default(),
        });
        let pair = &mut connector.pair;
        pair.kind = PairKind::VertexPlane;
        pair.point_a =
            blocks[vertex_body].vertex(tables.face_vertices[vertex_face as usize][corner]);
        pair.point_b = blocks[plane_body].vertex(tables.face_vertices[plane_face as usize][0]);
        pair.id_b = plane_face;
        pair.id_c = corner as i32;
        connector.point = connector.pair.compute(blocks);
        self.keep(connector, reused)
    }
    /// Clipped edge connector ae4e72 (incident edge against reference edge).
    fn edge_face(
        &mut self,
        blocks: [&Block; 2],
        tables: &BlockTables,
        incident_edge: usize,
        reference_edge: usize,
    ) -> Result<(), String> {
        let (inc, re) = (self.incident_body, self.reference_body);
        let (id_a, id_b) = (edge_id(incident_edge), edge_id(reference_edge));
        let found = self.take(inc, re, PairKind::EdgeFace, id_a, id_b);
        let reused = found.is_some();
        let mut connector = found.unwrap_or_else(|| Connector {
            pair: GeoPair::new(PairKind::EdgeFace, inc, re),
            point: ContactPoint {
                normal: [0.0; 3],
                length: 0.0,
                position: [0.0; 3],
            },
            state: T::default(),
        });
        let pair = &mut connector.pair;
        pair.kind = PairKind::EdgeFace;
        pair.point_a = blocks[inc].vertex(tables.edge_vertex(incident_edge));
        pair.point_b = blocks[re].vertex(tables.edge_vertex(reference_edge));
        pair.id_a = id_a;
        pair.id_b = id_b;
        pair.id_c = self.reference_face;
        pair.length_a = blocks[inc].size[(id_a % 3) as usize];
        pair.length_b = blocks[re].size[(id_b % 3) as usize];
        connector.point = connector.pair.compute(blocks);
        if -0.01 <= connector.point.length {
            if reused {
                self.events.push(ContactEvent::Removed {
                    connector,
                    recomputed: true,
                });
            }
            return Ok(());
        }
        self.keep(connector, reused)
    }
    /// Edge/edge connector ae33a8 (bodies 0 and 1).
    fn edge_edge(
        &mut self,
        blocks: [&Block; 2],
        tables: &BlockTables,
        edge_a: usize,
        edge_b: usize,
    ) -> Result<(), String> {
        let (id_a, id_b) = (edge_id(edge_a), edge_id(edge_b));
        let found = self.take(0, 1, PairKind::EdgeEdge, id_a, id_b);
        let reused = found.is_some();
        let mut connector = found.unwrap_or_else(|| Connector {
            pair: GeoPair::new(PairKind::EdgeEdge, 0, 1),
            point: ContactPoint {
                normal: [0.0; 3],
                length: 0.0,
                position: [0.0; 3],
            },
            state: T::default(),
        });
        let pair = &mut connector.pair;
        pair.kind = PairKind::EdgeEdge;
        pair.point_a = blocks[0].vertex(tables.edge_vertex(edge_a));
        pair.point_b = blocks[1].vertex(tables.edge_vertex(edge_b));
        pair.id_a = id_a;
        pair.id_b = id_b;
        connector.point = connector.pair.compute(blocks);
        if -0.01 <= connector.point.length {
            if reused {
                self.events.push(ContactEvent::Removed {
                    connector,
                    recomputed: true,
                });
            }
            return Ok(());
        }
        self.keep(connector, reused)
    }
    /// Face contact ae4974 and clipping ae533e.
    fn face_connectors(&mut self, blocks: [&Block; 2], tables: &BlockTables) -> Result<(), String> {
        let reference = if self.features[0] >= 0 { 0 } else { 1 };
        self.reference_face = self.features[reference];
        self.reference_body = reference;
        self.incident_body = 1 - reference;
        let (re, inc) = (self.reference_body, self.incident_body);
        let reference_frame = &blocks[re].frame;
        let relative = compose(&inverse(reference_frame), &blocks[inc].frame);
        let toward = normal_direction(&reference_frame.rotation, self.reference_face).map(|v| -v);
        self.incident_face = face_toward(&blocks[inc].frame.rotation, toward);
        let (rf, inf) = (self.reference_face, self.incident_face);
        let half = project(blocks[re].vertex(tables.face_vertices[rf as usize][0]), rf);
        let polygon: [[f32; 2]; 4] = std::array::from_fn(|k| {
            let v = blocks[inc].vertex(tables.face_vertices[inf as usize][k]);
            project(to_world(&relative, v), rf)
        });
        let [hx, hy] = half;
        // side[s][k]: incident corner k is inside reference side s
        // (0 top, 1 left, 2 bottom, 3 right).
        let mut side = [[false; 4]; 4];
        let mut inside = [true; 4];
        for k in (0..4).rev() {
            let [px, py] = polygon[k];
            side[0][k] = hy >= py;
            side[1][k] = px >= -hx;
            side[2][k] = py >= -hy;
            side[3][k] = hx >= px;
            inside[k] = (0..4).all(|s| side[s][k]);
        }
        let mut incident_count = 0;
        for k in (0..4).rev() {
            if inside[k] {
                self.vertex_plane(blocks, tables, inc, re, k, inf, rf)?;
                incident_count += 1;
            }
        }
        if incident_count == 4 {
            return Ok(());
        }
        let corners = [[hx, hy], [-hx, hy], [-hx, -hy], [hx, -hy]];
        // left[e][m]: reference corner m is left of incident edge e
        // (from corner e to corner (e + 3) % 4).
        let mut left = [[false; 4]; 4];
        let mut reference_inside = [true; 4];
        for e in (0..4).rev() {
            let p = polygon[e];
            let q = polygon[(e + 3) % 4];
            let edge = [q[0] - p[0], q[1] - p[1]];
            for m in 0..4 {
                let cross = (corners[m][1] - p[1]) * edge[0] - (corners[m][0] - p[0]) * edge[1];
                left[e][m] = cross >= 0.0;
                if !left[e][m] {
                    reference_inside[m] = false;
                }
            }
        }
        let mut reference_count = 0;
        for (m, inside) in reference_inside.into_iter().enumerate() {
            if inside {
                self.vertex_plane(blocks, tables, re, inc, m, rf, inf)?;
                reference_count += 1;
            }
        }
        if incident_count == 0 && reference_count == 4 {
            return Ok(());
        }
        for s in 0..4 {
            let next = (s + 1) % 4;
            for step in (3..=6).rev() {
                let k = step % 4;
                let e = step - 3;
                if side[s][e] != side[s][k] && left[e][next] != left[e][s] {
                    self.edge_face(
                        blocks,
                        tables,
                        tables.face_edges[inf as usize][k],
                        tables.face_edges[rf as usize][s],
                    )?;
                }
            }
        }
        Ok(())
    }
}

fn identity() -> Matrix {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn rotation_y(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rotation_x(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn report_case(
    out: &mut String,
    name: &str,
    blocks: [&Block; 2],
    tables: &BlockTables,
) -> Result<BlockBlockContact<()>, String> {
    let mut contact = BlockBlockContact::<()>::default();
    let colliding = contact.update(blocks, true, tables)?;
    writeln!(
        out,
        "block_contact case={name} colliding={colliding} features={:?} reference_body={} reference_face={} incident_face={} connectors={}",
        contact.features,
        contact.reference_body,
        contact.reference_face,
        contact.incident_face,
        contact.connectors().len()
    )
    .unwrap();
    for c in contact.connectors() {
        writeln!(
            out,
            "block_contact_connector case={name} kind={:?} bodies=({},{}) ids=({},{},{}) length={:.6} normal=({:.6},{:.6},{:.6}) position=({:.6},{:.6},{:.6})",
            c.pair.kind,
            c.pair.body_a,
            c.pair.body_b,
            c.pair.id_a,
            c.pair.id_b,
            c.pair.id_c,
            c.point.length,
            c.point.normal[0],
            c.point.normal[1],
            c.point.normal[2],
            c.point.position[0],
            c.point.position[1],
            c.point.position[2]
        )
        .unwrap();
    }
    Ok(contact)
}

/// Fixed scenes against the Studio Baseplate, logged for verification.
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let tables = BlockTables::load(bundle)?;
    let mut out = String::new();
    writeln!(
        out,
        "block_tables face_vertices={:?} face_edges={:?}",
        tables.face_vertices, tables.face_edges
    )
    .unwrap();
    let baseplate = Block {
        frame: CoordinateFrame {
            position: [0.0, -10.0, 0.0],
            rotation: identity(),
        },
        size: [512.0, 20.0, 512.0],
    };
    // Default Part (4, 1.2, 2) sunk 0.05 into the Baseplate top.
    let resting = Block {
        frame: CoordinateFrame {
            position: [0.0, 0.55, 0.0],
            rotation: identity(),
        },
        size: [4.0, 1.2, 2.0],
    };
    let contact = report_case(&mut out, "resting", [&resting, &baseplate], &tables)?;
    let resting_ok = contact.connectors().len() == 4
        && contact
            .connectors()
            .iter()
            .all(|c| c.pair.kind == PairKind::VertexPlane && (c.point.length + 0.05).abs() < 1e-4);
    let above = Block {
        frame: CoordinateFrame {
            position: [0.0, 0.7, 0.0],
            rotation: identity(),
        },
        size: [4.0, 1.2, 2.0],
    };
    let contact = report_case(&mut out, "separated", [&above, &baseplate], &tables)?;
    let separated_ok = contact.connectors().is_empty();
    // Rotated 45 degrees about Y and 30 about X, resting on a lowest edge.
    let tilted = Block {
        frame: CoordinateFrame {
            position: [0.0, 0.0, 0.0],
            rotation: mat_mul(
                &rotation_y(std::f32::consts::FRAC_PI_4),
                &rotation_x(std::f32::consts::FRAC_PI_6),
            ),
        },
        size: [4.0, 1.2, 2.0],
    };
    let mut lowest = f32::INFINITY;
    for i in 0..8 {
        lowest = lowest.min(to_world(&tilted.frame, tilted.vertex(i))[1]);
    }
    let tilted = Block {
        frame: CoordinateFrame {
            position: [0.0, -lowest - 0.05, 0.0],
            rotation: tilted.frame.rotation,
        },
        size: tilted.size,
    };
    let contact = report_case(&mut out, "tilted", [&tilted, &baseplate], &tables)?;
    let tilted_ok = !contact.connectors().is_empty();
    writeln!(
        out,
        "block_contact_audit resting_ok={resting_ok} separated_ok={separated_ok} tilted_ok={tilted_ok}"
    )
    .unwrap();
    if !(resting_ok && separated_ok && tilted_ok) {
        return Err(format!("block contact audit failed\n{out}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tables() -> Option<BlockTables> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("RobloxStudio");
        let bundle = Bundle::open(&root).ok()?;
        BlockTables::load(&bundle).ok()
    }
    fn block(position: [f32; 3], rotation: Matrix, size: [f32; 3]) -> Block {
        Block {
            frame: CoordinateFrame { position, rotation },
            size,
        }
    }
    #[test]
    fn edge_quadrants_select_the_nearest_edge() {
        let r = identity();
        assert_eq!(edge_toward(&r, 0, [0.0, 1.0, 1.0]), 0);
        assert_eq!(edge_toward(&r, 0, [0.0, -1.0, 1.0]), 1);
        assert_eq!(edge_toward(&r, 0, [0.0, -1.0, -1.0]), 2);
        assert_eq!(edge_toward(&r, 0, [0.0, 1.0, -1.0]), 3);
        assert_eq!(edge_toward(&r, 1, [1.0, 0.0, -1.0]), 5);
        assert_eq!(edge_toward(&r, 2, [-1.0, 1.0, 0.0]), 9);
        assert_eq!(face_toward(&r, [0.0, -1.0, 0.0]), 4);
        assert_eq!(face_toward(&r, [0.2, 0.1, 0.9]), 2);
    }
    #[test]
    fn extents_touching_overlaps() {
        let a = block([0.0; 3], identity(), [2.0; 3]);
        let b = block([2.02, 0.0, 0.0], identity(), [2.0; 3]);
        assert!(extents_overlap(&a.extents(), &b.extents()));
        let c = block([2.1, 0.0, 0.0], identity(), [2.0; 3]);
        assert!(!extents_overlap(&a.extents(), &c.extents()));
    }
    #[test]
    fn separating_axis_is_cached() {
        let mut contact = BlockBlockContact::<()>::default();
        let a = block([0.0; 3], identity(), [2.0; 3]);
        let b = block([0.0, 0.0, 3.0], identity(), [2.0; 3]);
        assert!(!contact.compute_is_colliding([&a, &b], 0.0).0);
        assert_eq!((contact.separating_body, contact.separating_axis), (0, 2));
    }
    #[test]
    fn resting_box_gets_four_vertex_connectors() {
        let Some(tables) = tables() else { return };
        let ground = block([0.0, -10.0, 0.0], identity(), [512.0, 20.0, 512.0]);
        let part = block([1.0, 0.55, -2.0], identity(), [4.0, 1.2, 2.0]);
        let mut contact = BlockBlockContact::<u32>::default();
        assert!(contact.update([&part, &ground], true, &tables).unwrap());
        let kept = |c: &BlockBlockContact<u32>, reused: bool| {
            c.events
                .iter()
                .enumerate()
                .all(|(i, e)| matches!(e, ContactEvent::Kept { index, reused: r } if *index == i && *r == reused))
        };
        assert_eq!(contact.events.len(), 4);
        assert!(kept(&contact, false));
        // Body 0 bottom (-Y) is the smallest overlap: it is the reference.
        assert_eq!(contact.features, [4, -1]);
        assert_eq!(contact.incident_face, 1);
        let c = contact.connectors();
        assert_eq!(c.len(), 4);
        for c in c {
            // The reference (part) corners lie on the ground's top face.
            assert_eq!(c.pair.kind, PairKind::VertexPlane);
            assert_eq!((c.pair.body_a, c.pair.body_b, c.pair.id_b), (0, 1, 1));
            assert!((c.point.length + 0.05).abs() < 1e-5);
            assert_eq!(c.point.normal, [-0.0, -1.0, -0.0]);
            assert!((c.point.position[1] + 0.05).abs() < 1e-5);
        }
        // Persistence: the same connectors are found again and keep state.
        contact.connectors_mut()[0].state = 7;
        let first = contact.connectors()[0].pair.id_c;
        assert!(contact.update([&part, &ground], true, &tables).unwrap());
        let again = contact
            .connectors()
            .iter()
            .find(|c| c.pair.id_c == first)
            .unwrap();
        assert_eq!(again.state, 7);
        assert_eq!(contact.events.len(), 4);
        assert!(kept(&contact, true));
        // Lifted clear: connectors are deleted and features reset.
        let lifted = block([1.0, 0.7, -2.0], identity(), [4.0, 1.2, 2.0]);
        assert!(!contact.update([&lifted, &ground], true, &tables).unwrap());
        assert!(contact.connectors().is_empty());
        assert_eq!(contact.events.len(), 4);
        assert!(contact.events.iter().all(|e| matches!(
            e,
            ContactEvent::Removed {
                recomputed: false,
                ..
            }
        )));
        assert_eq!(contact.features, [-1, -1]);
    }
    #[test]
    fn crossed_edges_make_an_edge_connector() {
        let Some(tables) = tables() else { return };
        // Two long bars crossing at right angles, edge to edge, 0.05 deep.
        let a = block(
            [0.0; 3],
            rotation_x(std::f32::consts::FRAC_PI_4),
            [8.0, 1.0, 1.0],
        );
        let reach = 0.5 * std::f32::consts::SQRT_2;
        let b = block(
            [0.0, 2.0 * reach - 0.05, 0.0],
            mat_mul(
                &rotation_y(std::f32::consts::FRAC_PI_2),
                &rotation_x(std::f32::consts::FRAC_PI_4),
            ),
            [8.0, 1.0, 1.0],
        );
        let mut contact = BlockBlockContact::<()>::default();
        assert!(contact.update([&a, &b], true, &tables).unwrap());
        assert!(contact.features.iter().all(|f| *f >= 6));
        let c = contact.connectors();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].pair.kind, PairKind::EdgeEdge);
        assert!((c[0].point.length + 0.05).abs() < 1e-4, "{:?}", c[0].point);
        assert!(c[0].point.normal[1].abs() > 0.999);
    }
}
