//! Poly geometry (v8world/Poly.cpp, Mesh.cpp): the convex mesh the legacy
//! narrowphase runs PolyPolyContact on (`poly_contact`), and the Wedge's mass
//! properties.
//!
//! Recovered from:
//! - Primitive part types (vtable +0xa4, addPartImpl 010d1850): 1 Block,
//!   4 Wedge. Block and Wedge geometries both derive from Poly (Block vtable
//!   01c866e8, WedgePoly 01c88878);
//! - Poly::setSize 00b4fcae: the size at +4, the radius |size| / 2 at +0x14,
//!   then buildMesh (vtable +0x80);
//! - the shared mesh caches: Block 00adec1a -> 00ae0138 -> 00b2f962, Wedge
//!   00ba4950 -> 00ba50b2 -> 00b31f14. A mesh is built from the size alone,
//!   so building it again is the same mesh;
//! - Mesh.cpp: add vertex 00b3540e, quad face 00b2fd00, triangle 00b307ec,
//!   getOrCreateEdge 00b33b08 (00b33d70 searches the first vertex's edges,
//!   00b33e3e creates the edge and adds it to both vertices), Edge::setFace
//!   00b35542 (forward first, then backward), Face plane 00b33cee;
//! - Mesh.h: Edge::getVertex 00d1baf8, Edge::otherFace 00b357cc,
//!   Vertex::getFace 00b3427c -> 00b3574e, the edge direction 00d200a0, the
//!   side plane 00d1fe1e, Face::inside 00b553e0 -> 00b33fd6, the clip
//!   tests 00b344fa / 00b346f6 / 00b348d6 and their edge searches 00b344aa,
//!   00b347aa, 00b345e8, Vertex::findEdge 00b34fea, the support vertex
//!   00b34046 and the plane projection 00b55032;
//! - G3D: Plane(p0, p1, p2) 01257274, Plane(normal, point) 0125737e,
//!   halfSpaceContains 0129c468 and its mirror 00d1fc5a (both clamp the
//!   point to +-FLT_MAX), Plane toWorldSpace 01250578 and toObjectSpace
//!   01250474, Line::fromTwoPoints 00b5554c, Line toWorldSpace 01250682,
//!   Line::intersection 01254c28 (with Plane::getEquation 012574b2);
//! - Wedge: volume 00b0ab80 (x y z, the whole box), center of mass 00ba4c20
//!   (0, -y/6, z/6), moment 00ba4aac (the Block polynomial of
//!   `MassProfile::block_inertia`, about the Part's center), extents 00ba4de6
//!   (vertices 0, 1, 3, 4: the box's extents).
//!
//! Every float expression follows the disassembly: a dot product is
//! ((x + y) + z) and a point is (R * p) + t.
// The negated compares keep the native branches' NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::{
    block_contact::{dot, mul, mul_transpose, sub, to_world},
    scene::CoordinateFrame,
};

/// A Poly geometry's mesh kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolyShape {
    Block,
    Wedge,
}

/// G3D::Plane: the points p with normal . p = distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub distance: f32,
}

/// The clamp 0129c468 and 00d1fc5a apply first: each coordinate held to
/// [-FLT_MAX, FLT_MAX] (NaN passes through).
fn clamp(p: [f32; 3]) -> [f32; 3] {
    p.map(|x| {
        let r = if f32::MAX <= x { f32::MAX } else { x };
        if x <= f32::MIN { f32::MIN } else { r }
    })
}

fn length_squared(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]
}

/// v * (1 / |v|), as G3D::Vector3::direction compiles.
fn direction(v: [f32; 3]) -> [f32; 3] {
    let inverse = 1.0 / length_squared(v).sqrt();
    v.map(|x| x * inverse)
}

/// The cross product as the binary computes it.
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - b[2] * a[0],
        b[1] * a[0] - a[1] * b[0],
    ]
}

impl Plane {
    /// G3D::Plane(p0, p1, p2) 01257274.
    pub fn from_points(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]) -> Self {
        let normal = direction(cross(sub(p1, p0), sub(p2, p0)));
        Self {
            normal,
            distance: dot(normal, p0),
        }
    }
    /// G3D::Plane(normal, point) 0125737e.
    pub fn from_normal_point(normal: [f32; 3], point: [f32; 3]) -> Self {
        let normal = direction(normal);
        Self {
            normal,
            distance: dot(normal, point),
        }
    }
    /// halfSpaceContains 0129c468: on the normal's side or on the plane.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        dot(self.normal, clamp(p)) >= self.distance
    }
    /// 00d1fc5a: behind the plane or on it.
    pub fn behind(&self, p: [f32; 3]) -> bool {
        self.distance >= dot(self.normal, clamp(p))
    }
    /// CoordinateFrame::toWorldSpace(Plane) 01250578.
    pub fn to_world(&self, f: &CoordinateFrame) -> Self {
        let n = mul(&f.rotation, self.normal);
        let point = to_world(f, self.normal.map(|v| v * self.distance));
        Self::from_normal_point(n, point)
    }
    /// CoordinateFrame::toObjectSpace(Plane) 01250474.
    pub fn to_object(&self, f: &CoordinateFrame) -> Self {
        let n = mul_transpose(&f.rotation, self.normal);
        let point: [f32; 3] =
            std::array::from_fn(|a| self.normal[a] * self.distance - f.position[a]);
        Self::from_normal_point(n, mul_transpose(&f.rotation, point))
    }
}

/// G3D::Line: a point and a unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub point: [f32; 3],
    pub direction: [f32; 3],
}

impl Line {
    /// Line::fromTwoPoints 00b5554c: the direction is normalized twice.
    pub fn from_two_points(p0: [f32; 3], p1: [f32; 3]) -> Self {
        Self {
            point: p0,
            direction: direction(direction(sub(p1, p0))),
        }
    }
    /// CoordinateFrame::toWorldSpace(Line) 01250682.
    pub fn to_world(&self, f: &CoordinateFrame) -> Self {
        Self {
            point: to_world(f, self.point),
            direction: direction(mul(&f.rotation, self.direction)),
        }
    }
    /// Line::intersection(Plane) 01254c28: Vector3::inf when parallel.
    pub fn intersection(&self, plane: &Plane) -> [f32; 3] {
        let rate = dot(self.direction, plane.normal);
        if rate == 0.0 {
            return [f32::INFINITY; 3];
        }
        let e = -plane.distance;
        let t = -(dot(plane.normal, self.point) + e) / rate;
        std::array::from_fn(|a| self.direction[a] * t + self.point[a])
    }
}

/// RBX::RbxRay::intersectionPlane 0129b702: `None` (Vector3::inf) when
/// |rate| < 1e-6 or the plane is behind the origin.
pub fn ray_plane(origin: [f32; 3], direction: [f32; 3], plane: &Plane) -> Option<[f32; 3]> {
    let rate = dot(plane.normal, direction);
    if 1e-6 > rate.abs() {
        return None;
    }
    let t = (plane.distance - dot(plane.normal, origin)) / rate;
    // JNC: NaN is not a hit.
    if !(t >= 0.0) {
        return None;
    }
    Some(std::array::from_fn(|a| origin[a] + direction[a] * t))
}

/// The closest points of two lines (00aaa994, and inline in the EdgeEdge
/// connector 00aaa48a): `None` when 1 - (da . db)^2 <= 1e-6.
pub fn closest_points(a: &Line, b: &Line) -> Option<([f32; 3], [f32; 3])> {
    let c = dot(a.direction, b.direction);
    let denominator = 1.0 - c * c;
    if !(denominator > 1e-6) {
        return None;
    }
    let w = sub(b.point, a.point);
    let e = dot(w, a.direction);
    let f = dot(w, b.direction);
    let inverse = 1.0 / denominator;
    let s = (e - f * c) * inverse;
    let t = (e * c - f) * inverse;
    Some((
        std::array::from_fn(|i| a.direction[i] * s + a.point[i]),
        std::array::from_fn(|i| b.direction[i] * t + b.point[i]),
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    /// Edges in creation order (00b356a8).
    pub edges: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub vertices: [usize; 2],
    pub forward: Option<usize>,
    pub backward: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub edges: Vec<usize>,
    pub plane: Plane,
}

/// A convex mesh in the Part's frame; vertex, edge and face ids are their
/// indices.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    pub faces: Vec<Face>,
}

impl Mesh {
    /// The mesh of a Poly geometry of `size`.
    pub fn new(shape: PolyShape, size: [f32; 3]) -> Self {
        match shape {
            PolyShape::Block => Self::block(size),
            PolyShape::Wedge => Self::wedge(size),
        }
    }
    /// Block mesh 00b2f962.
    pub fn block(size: [f32; 3]) -> Self {
        let [x, y, z] = size.map(|v| v * 0.5);
        let mut mesh = Self::empty();
        for p in [
            [x, y, z],
            [x, y, -z],
            [x, -y, z],
            [x, -y, -z],
            [-x, y, z],
            [-x, y, -z],
            [-x, -y, z],
            [-x, -y, -z],
        ] {
            mesh.add_vertex(p);
        }
        for face in [
            [1, 0, 2, 3],
            [1, 5, 4, 0],
            [0, 4, 6, 2],
            [4, 5, 7, 6],
            [7, 3, 2, 6],
            [5, 1, 3, 7],
        ] {
            mesh.add_face(&face);
        }
        mesh
    }
    /// Wedge mesh 00b31f14: the slope runs from the top back edge (+z) to
    /// the bottom front edge (-z).
    pub fn wedge(size: [f32; 3]) -> Self {
        let [x, y, z] = size.map(|v| v * 0.5);
        let mut mesh = Self::empty();
        for p in [
            [x, y, z],
            [x, -y, z],
            [x, -y, -z],
            [-x, y, z],
            [-x, -y, z],
            [-x, -y, -z],
        ] {
            mesh.add_vertex(p);
        }
        mesh.add_face(&[0, 3, 4, 1]);
        mesh.add_face(&[3, 0, 2, 5]);
        mesh.add_face(&[0, 1, 2]);
        mesh.add_face(&[3, 5, 4]);
        mesh.add_face(&[5, 2, 1, 4]);
        mesh
    }
    fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
            faces: Vec::new(),
        }
    }
    fn add_vertex(&mut self, position: [f32; 3]) {
        self.vertices.push(Vertex {
            position,
            edges: Vec::new(),
        });
    }
    /// getOrCreateEdge 00b33b08: the first edge of `a` that has `b`, else a
    /// new edge a -> b.
    fn edge_between(&mut self, a: usize, b: usize) -> usize {
        if let Some(e) = self.find_edge(a, b) {
            return e;
        }
        let id = self.edges.len();
        self.edges.push(Edge {
            vertices: [a, b],
            forward: None,
            backward: None,
        });
        self.vertices[a].edges.push(id);
        self.vertices[b].edges.push(id);
        id
    }
    /// Quad 00b2fd00 / triangle 00b307ec: the edges around the face, the
    /// face, each edge's side (00b35542), then the plane through the first
    /// vertex of edges 0, 1, 2 (00b33cee).
    fn add_face(&mut self, vertices: &[usize]) {
        let n = vertices.len();
        let edges: Vec<usize> = (0..n)
            .map(|i| self.edge_between(vertices[i], vertices[(i + 1) % n]))
            .collect();
        let id = self.faces.len();
        for &e in &edges {
            let edge = &mut self.edges[e];
            if edge.forward.is_none() {
                edge.forward = Some(id);
            } else {
                edge.backward = Some(id);
            }
        }
        self.faces.push(Face {
            edges,
            plane: Plane {
                normal: [0.0; 3],
                distance: 0.0,
            },
        });
        let p: [[f32; 3]; 3] = std::array::from_fn(|k| {
            let e = self.faces[id].edges[k];
            self.vertices[self.edge_vertex(e, Some(id), 0)].position
        });
        self.faces[id].plane = Plane::from_points(p[0], p[1], p[2]);
    }
    /// Vertex::findEdge 00b34fea (and 00b33d70): the first edge of `a`
    /// touching `b`.
    pub fn find_edge(&self, a: usize, b: usize) -> Option<usize> {
        self.vertices[a]
            .edges
            .iter()
            .copied()
            .find(|&e| self.edges[e].vertices[0] == b || self.edges[e].vertices[1] == b)
    }
    /// Edge::getVertex 00d1baf8: vertex `i` going round `face`.
    pub fn edge_vertex(&self, e: usize, face: Option<usize>, i: usize) -> usize {
        let edge = &self.edges[e];
        if face.is_none() || face == edge.forward {
            edge.vertices[i]
        } else {
            edge.vertices[(i + 1) % 2]
        }
    }
    /// Edge::otherFace 00b357cc.
    pub fn other_face(&self, e: usize, face: usize) -> Option<usize> {
        let edge = &self.edges[e];
        if edge.forward == Some(face) {
            edge.backward
        } else {
            edge.forward
        }
    }
    /// Vertex::getFace 00b3427c: the face on the vertex's side of its edge i.
    pub fn vertex_face(&self, v: usize, i: usize) -> Option<usize> {
        let edge = &self.edges[self.vertices[v].edges[i]];
        if edge.vertices[0] == v {
            edge.forward
        } else {
            edge.backward
        }
    }
    /// The unit direction of edge `e` going round `face` (00d200a0).
    pub fn edge_direction(&self, e: usize, face: usize) -> [f32; 3] {
        let end = self.vertices[self.edge_vertex(e, Some(face), 1)].position;
        let start = self.vertices[self.edge_vertex(e, Some(face), 0)].position;
        direction(sub(end, start))
    }
    /// The side plane of the face's edge k (00d1fe1e): through the edge,
    /// facing out of the face.
    pub fn side_plane(&self, face: usize, k: usize) -> Plane {
        let e = self.faces[face].edges[k];
        let normal = cross(self.edge_direction(e, face), self.faces[face].plane.normal);
        let start = self.vertices[self.edge_vertex(e, Some(face), 0)].position;
        Plane::from_normal_point(normal, start)
    }
    /// Face::inside 00b553e0: the point dropped onto the face's plane is
    /// behind every neighbouring face (00b33fd6).
    pub fn inside(&self, face: usize, p: [f32; 3]) -> bool {
        let plane = &self.faces[face].plane;
        let d = dot(plane.normal, p) - plane.distance;
        let q: [f32; 3] = std::array::from_fn(|a| p[a] - plane.normal[a] * d);
        self.behind_neighbours(face, q)
    }
    /// 00b33fd6: the point is behind (or on) the plane of every face
    /// sharing an edge with `face`.
    pub fn behind_neighbours(&self, face: usize, p: [f32; 3]) -> bool {
        self.faces[face].edges.iter().all(|&e| {
            self.other_face(e, face)
                .is_some_and(|other| self.faces[other].plane.behind(p))
        })
    }
    /// The Poly geometry's hitTest (vtable +0x70, WedgePoly 00b4fbd6 ->
    /// Mesh 00b340c0) on a ray in part space: each face in order is hit by
    /// the ray (00b33f22: RbxRay::intersectionPlane, then
    /// `behind_neighbours`), and the hit nearest the origin by squared
    /// distance wins, the first on ties. Returns the point and the face's
    /// normal.
    pub fn hit_test(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
        let mut best = f32::MAX;
        let mut found = None;
        for (i, face) in self.faces.iter().enumerate() {
            let Some(point) = ray_plane(origin, direction, &face.plane) else {
                continue;
            };
            if !self.behind_neighbours(i, point) {
                continue;
            }
            let d = length_squared(sub(point, origin));
            if best > d {
                best = d;
                found = Some((point, face.plane.normal));
            }
        }
        found
    }
    /// 00b348d6: `x` lies between the ends of the face's edge k.
    fn between(&self, face: usize, x: [f32; 3], side: &Plane, k: usize) -> bool {
        let c = cross(self.faces[face].plane.normal, side.normal);
        let e = self.faces[face].edges[k];
        let start = self.vertices[self.edge_vertex(e, Some(face), 0)].position;
        if !Plane::from_normal_point(c, start).contains(x) {
            return false;
        }
        let end = self.vertices[self.edge_vertex(e, Some(face), 1)].position;
        Plane::from_normal_point(c.map(|v| -v), end).contains(x)
    }
    /// 00b344fa (with the face-plane test) / 00b346f6 (without): the segment
    /// p0 p1 crosses the side plane of edge k inside the edge's span.
    fn crosses(&self, face: usize, p0: [f32; 3], p1: [f32; 3], k: usize, below: bool) -> bool {
        let side = self.side_plane(face, k);
        if side.contains(p0) == side.contains(p1) {
            return false;
        }
        let x = Line::from_two_points(p0, p1).intersection(&side);
        if below && !self.faces[face].plane.behind(x) {
            return false;
        }
        self.between(face, x, &side, k)
    }
    /// 00b344aa: the first edge of the face the segment crosses below it.
    pub fn crossing_edge(&self, face: usize, p0: [f32; 3], p1: [f32; 3]) -> Option<usize> {
        (0..self.faces[face].edges.len()).find(|&k| self.crosses(face, p0, p1, k, true))
    }
    /// 00b347aa: the first crossing edge, and the last of the others.
    pub fn crossing_edges(
        &self,
        face: usize,
        p0: [f32; 3],
        p1: [f32; 3],
    ) -> (Option<usize>, Option<usize>) {
        let mut found = (None, None);
        for k in 0..self.faces[face].edges.len() {
            if self.crosses(face, p0, p1, k, true) {
                if found.0.is_none() {
                    found.0 = Some(k);
                } else {
                    found.1 = Some(k);
                }
            }
        }
        found
    }
    /// 00b345e8: the first edge the segment crosses, at any height.
    pub fn crossing_edge_any(&self, face: usize, p0: [f32; 3], p1: [f32; 3]) -> Option<usize> {
        (0..self.faces[face].edges.len()).find(|&k| self.crosses(face, p0, p1, k, false))
    }
    /// 00b34046: the vertex furthest along `d` (the first on ties).
    pub fn support(&self, d: [f32; 3]) -> Option<usize> {
        let mut best = f32::MIN;
        let mut found = None;
        for (i, v) in self.vertices.iter().enumerate() {
            let x = dot(v.position, d);
            if x > best {
                found = Some(i);
            }
            best = if x > best { x } else { best };
        }
        found
    }
    /// 00b55032: the extent of the vertices along the plane, (min, max).
    pub fn project(&self, plane: &Plane) -> (f32, f32) {
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for v in &self.vertices {
            let x = dot(plane.normal, v.position) - plane.distance;
            if low > x {
                low = x;
            }
            if x > high {
                high = x;
            }
        }
        (low, high)
    }
}

/// The Wedge's center of mass 00ba4c20 in its frame.
pub fn wedge_center(size: [f32; 3]) -> [f32; 3] {
    [0.0, -size[1] / 6.0, size[2] / 6.0]
}

/// Body moment shift to a point (006b0234): I + m P(p), P the parallel-axis
/// matrix of `p`. Cofm 00a90502 applies it to a lone Body with a center of
/// mass offset.
pub fn moment_about(inertia: [[f32; 3]; 3], mass: f32, p: [f32; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = p;
    let m = [
        [y * y + z * z, -(x * y), -(x * z)],
        [-(x * y), z * z + x * x, -(y * z)],
        [-(x * z), -(y * z), y * y + x * x],
    ];
    std::array::from_fn(|r| std::array::from_fn(|c| inertia[r][c] + m[r][c] * mass))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(mesh: &Mesh) {
        for e in &mesh.edges {
            assert!(
                e.forward.is_some() && e.backward.is_some(),
                "open edge {e:?}"
            );
        }
        // Every face's vertices lie on its plane and the other vertices
        // behind it (convex, outward normals).
        for (f, face) in mesh.faces.iter().enumerate() {
            for (v, vertex) in mesh.vertices.iter().enumerate() {
                let d = dot(face.plane.normal, vertex.position) - face.plane.distance;
                let on = face
                    .edges
                    .iter()
                    .any(|&e| mesh.edge_vertex(e, Some(f), 0) == v);
                if on {
                    assert!(d.abs() < 1e-5, "face {f} vertex {v} off plane {d}");
                } else {
                    assert!(d < 0.0, "face {f} vertex {v} in front {d}");
                }
            }
        }
    }

    #[test]
    fn block_mesh_topology() {
        let mesh = Mesh::block([4.0, 1.2, 2.0]);
        assert_eq!(
            (mesh.vertices.len(), mesh.edges.len(), mesh.faces.len()),
            (8, 12, 6)
        );
        closed(&mesh);
        // Face 0 is +X, face 1 +Y, face 2 +Z, face 3 -X, face 4 -Y, face 5 -Z.
        let expected = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, -1.0],
        ];
        for (face, n) in mesh.faces.iter().zip(expected) {
            let d = sub(face.plane.normal, n);
            assert!(
                length_squared(d) < 1e-12,
                "{:?} != {n:?}",
                face.plane.normal
            );
        }
    }

    #[test]
    fn wedge_mesh_topology() {
        let mesh = Mesh::wedge([4.0, 1.0, 2.0]);
        assert_eq!(
            (mesh.vertices.len(), mesh.edges.len(), mesh.faces.len()),
            (6, 9, 5)
        );
        closed(&mesh);
        // The slope (face 1) faces up and forward (-z).
        let n = mesh.faces[1].plane.normal;
        assert!(n[0] == 0.0 && n[1] > 0.0 && n[2] < 0.0, "{n:?}");
        assert!(length_squared(sub(mesh.faces[0].plane.normal, [0.0, 0.0, 1.0])) < 1e-12);
        assert!(length_squared(sub(mesh.faces[4].plane.normal, [0.0, -1.0, 0.0])) < 1e-12);
    }

    #[test]
    fn wedge_hit_test_finds_the_slope() {
        // Size (4, 2, 4): the slope rises from (y -1, z -2) to (y 1, z 2).
        let mesh = Mesh::wedge([4.0, 2.0, 4.0]);
        let down = [0.0, -1.0, 0.0];
        let (point, normal) = mesh.hit_test([0.0, 10.0, 1.0], down).unwrap();
        assert!((point[1] - 0.5).abs() < 1e-6 && (point[2] - 1.0).abs() < 1e-6);
        let s = 5.0f32.sqrt();
        assert!((normal[1] - 2.0 / s).abs() < 1e-6 && (normal[2] + 1.0 / s).abs() < 1e-6);
        // From below: the bottom face, normal -y.
        let (point, normal) = mesh.hit_test([0.5, -10.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert_eq!((point[1], normal), (-1.0, [0.0, -1.0, 0.0]));
        // Beside it, and pointing away.
        assert!(mesh.hit_test([3.0, 10.0, 0.0], down).is_none());
        assert!(mesh.hit_test([0.0, 10.0, 0.0], [0.0, 1.0, 0.0]).is_none());
    }
    #[test]
    fn inside_and_crossing() {
        let mesh = Mesh::block([2.0, 2.0, 2.0]);
        // Top face (1): inside its square, outside past an edge.
        assert!(mesh.inside(1, [0.5, 5.0, -0.5]));
        assert!(!mesh.inside(1, [1.5, 5.0, 0.0]));
        // A segment from inside the top face's prism out through +x, below
        // the face, crosses exactly one edge.
        let k = mesh
            .crossing_edge(1, [0.0, 0.5, 0.0], [3.0, 0.5, 0.0])
            .unwrap();
        let e = mesh.faces[1].edges[k];
        let ends = mesh.edges[e].vertices.map(|v| mesh.vertices[v].position);
        assert!(ends.iter().all(|p| p[0] == 1.0 && p[1] == 1.0));
    }

    #[test]
    fn plane_round_trip() {
        let f = CoordinateFrame {
            position: [1.0, 2.0, 3.0],
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let p = Plane {
            normal: [1.0, 0.0, 0.0],
            distance: 2.0,
        };
        let w = p.to_world(&f);
        assert_eq!(w.normal, [0.0, 1.0, 0.0]);
        assert_eq!(w.distance, 4.0);
        assert_eq!(w.to_object(&f), p);
    }
}
