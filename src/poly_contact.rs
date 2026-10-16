//! PolyPolyContact (v8world/PolyPolyContact.cpp, vtable 01c87858): the
//! narrowphase between two Poly geometries (a Wedge against a Block or a
//! Wedge), with its separating pairs and the PolyConnectors.
//!
//! Recovered from:
//! - ContactManager::createContact 00ae714a: collide types 2-3 and 3-3 make
//!   a PolyPolyContact (constructor 00b52b82), the lower type first;
//! - PolyContact: computeIsColliding 00b5203a (extents 00aec6e4, then the
//!   rebuild 00b520a4 and the deepest overlap 00b521c6), the step 00b51fc4
//!   (00b51ec4 takes every connector out of the kernel when separated;
//!   00b51f34 puts in each one deeper than 0.01), the rebuild's match
//!   00b522b2 -> 00b52304 / 00b52366 (same ids +0x110, +0x114 and connector
//!   type: the old connector replaces the new one) and the leftovers'
//!   delete 00b51d88;
//! - findPolyConnectors 00b52c2a and the separating pair search 00b52d2e
//!   (the cached pair first; when it overlaps, FaceFacePair(A, B),
//!   FaceFacePair(B, A) and EdgeEdgePair(A, B) from their first features,
//!   skipping the cached pair's own kind and first primitive: the first one
//!   that separates wins, else the best that beats the cached separation by
//!   more than 0.01);
//! - FaceFacePair (vtable 01c878b4): separation 00b530b2 with 00b53392,
//!   00b534ee and the incident face 00b535a0 (+0x24 keeps the runner-up
//!   when two faces are within 0.4 above 0.3); generate 00b53872 ->
//!   00b53c40, 00b543f6, 00b544de, then the edge clipping 00b53eae,
//!   00b5405c, 00b54248 over the incident face;
//! - EdgeEdgePair (vtable 01c878d4): separation 00b54a60 (edges at the
//!   support vertices, from the cached edges round), generate 00b550ec;
//! - the connectors (PolyConnectors.cpp): FaceVertex 00b545c8 (type 6,
//!   geometry 00aa9fe2), FaceEdge 00b54788 (type 8, 00aaa11e), EdgeEdge
//!   00b5517a (type 7, 00aaa48a). Each geometry update ends in the contact
//!   displacement 00a9457c, which only acts in contactConnectors.
//!
//! Body 0 is the contact's first primitive and body 1 the second, as in
//! `block_contact`. A connector's normal points from its body A to body B
//! and its length is negative while they overlap.
// The negated compares keep the native branches' NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::{
    block_contact::{
        Block, Connector, ContactEvent, ContactPoint, GeoPair, PairKind, UNSET, dot,
        extents_overlap, mul, mul_transpose, sub, to_world,
    },
    g3d::{frame_inverse, frame_multiply, unitize},
    poly::{Line, Mesh, Plane, PolyShape, closest_points, cross},
    scene::CoordinateFrame,
};

/// A Poly connector's local geometry (GeoPair::poly).
#[derive(Clone, Debug, PartialEq)]
pub enum PolyFeature {
    /// FaceVertexConnector: a face plane of body A (+0x118) and a vertex of
    /// body B (+0x12c).
    FaceVertex { plane: Plane, vertex: [f32; 3] },
    /// FaceEdgeConnector: the face normal of body A (+0x11c), an edge of
    /// that face (+0x140) and an edge of body B (+0x15c).
    FaceEdge {
        normal: [f32; 3],
        edge_a: Line,
        edge_b: Line,
    },
    /// EdgeEdgeConnector: an edge of each body (+0x118, +0x134).
    EdgeEdge { edge_a: Line, edge_b: Line },
    /// BallPlaneConnector: the radius (+0x118), a point of the Poly's face
    /// (+0x11c) and its normal (+0x128), in the Poly's frame.
    BallPlane {
        radius: f32,
        point: [f32; 3],
        normal: [f32; 3],
    },
    /// BallEdgeConnector: the radius, the edge's first vertex and its
    /// direction.
    BallEdge {
        radius: f32,
        point: [f32; 3],
        direction: [f32; 3],
    },
    /// BallVertexConnector: the radius and the vertex.
    BallVertex { radius: f32, vertex: [f32; 3] },
}

/// The connector geometry update (vtable +0x24) of a Poly connector.
pub fn compute(pair: &GeoPair, a: &CoordinateFrame, b: &CoordinateFrame) -> ContactPoint {
    match pair.poly.as_deref() {
        // 00aa9fe2.
        Some(PolyFeature::FaceVertex { plane, vertex }) => {
            let plane = plane.to_world(a);
            let p = to_world(b, *vertex);
            ContactPoint {
                normal: plane.normal,
                length: dot(p, plane.normal) - plane.distance,
                position: p,
            }
        }
        // 00aaa11e.
        Some(PolyFeature::FaceEdge {
            normal,
            edge_a,
            edge_b,
        }) => {
            let la = edge_a.to_world(a);
            let lb = edge_b.to_world(b);
            let face = mul(&a.rotation, *normal);
            match closest_points(&la, &lb) {
                Some((qa, qb)) => {
                    let mut n = sub(qa, qb);
                    let length = unitize(&mut n, 1e-20);
                    if dot(face, n) > 0.0 {
                        ContactPoint {
                            normal: n,
                            length: -length,
                            position: qa,
                        }
                    } else {
                        ContactPoint {
                            normal: n.map(|v| -v),
                            length,
                            position: qa,
                        }
                    }
                }
                None => ContactPoint {
                    normal: face,
                    length: 0.0,
                    position: la.point,
                },
            }
        }
        // 00aaa48a.
        Some(PolyFeature::EdgeEdge { edge_a, edge_b }) => {
            let la = edge_a.to_world(a);
            let lb = edge_b.to_world(b);
            match closest_points(&la, &lb) {
                Some((qa, qb)) => {
                    let mut n = sub(qa, qb);
                    let length = unitize(&mut n, 1e-6);
                    if dot(sub(b.position, a.position), n) > 0.0 {
                        ContactPoint {
                            normal: n,
                            length: -length,
                            position: qa,
                        }
                    } else {
                        ContactPoint {
                            normal: n.map(|v| -v),
                            length,
                            position: qa,
                        }
                    }
                }
                None => ContactPoint {
                    normal: la.direction,
                    length: 0.0,
                    position: la.point,
                },
            }
        }
        // 00aa99b2. A is the ball, B the Poly.
        Some(PolyFeature::BallPlane {
            radius,
            point,
            normal,
        }) => {
            let n = mul(&b.rotation, *normal).map(|v| -v);
            let p = to_world(b, *point);
            let c = a.position;
            let s = dot(sub(p, c), n);
            let offset = n.map(|v| v * s);
            ContactPoint {
                normal: n,
                length: dot(offset, offset).sqrt() - radius,
                position: std::array::from_fn(|i| c[i] + offset[i]),
            }
        }
        // 00aa9c2a.
        Some(PolyFeature::BallEdge {
            radius,
            point,
            direction,
        }) => {
            let axis = mul(&b.rotation, *direction);
            let c = a.position;
            let d = sub(to_world(b, *point), c);
            let along = dot(d, axis);
            let mut n: [f32; 3] = std::array::from_fn(|i| d[i] - axis[i] * along);
            let position = std::array::from_fn(|i| n[i] + c[i]);
            let length = unitize(&mut n, 1e-6);
            ContactPoint {
                normal: n,
                length: length - radius,
                position,
            }
        }
        // 00aa9ea4.
        Some(PolyFeature::BallVertex { radius, vertex }) => {
            let p = to_world(b, *vertex);
            let mut n = sub(p, a.position);
            let length = unitize(&mut n, 1e-6);
            ContactPoint {
                normal: n,
                length: length - radius,
                position: p,
            }
        }
        None => ContactPoint {
            normal: [0.0; 3],
            length: 0.0,
            position: [0.0; 3],
        },
    }
}

/// BallPolyContact::findPolyConnectors 00add542 (ball native body 0, the
/// Poly body 1): the Poly face the ball's center is furthest outside
/// (00add796), then a BallPlane connector when the center is behind it or
/// over the face (Face::inside), else the nearest edge of that face whose
/// span holds the center (00adda14, 00b3507c) for a BallEdge, else the
/// nearer end of the face edge whose line is nearest (00addc50, 00addd4e)
/// for a BallVertex. Nothing when the feature is further than the radius.
fn ball_connectors(
    ball: &Block,
    poly: &Block,
    shape: PolyShape,
) -> Option<(PairKind, usize, PolyFeature)> {
    let mesh = Mesh::new(shape, poly.size);
    let radius = crate::ball_contact::radius(ball.size);
    let p = mul_transpose(
        &poly.frame.rotation,
        sub(ball.frame.position, poly.frame.position),
    );
    let mut distance = f32::MIN;
    let mut face = None;
    for (i, f) in mesh.faces.iter().enumerate() {
        let d = dot(f.plane.normal, p) - f.plane.distance;
        if distance <= d && d != distance {
            distance = d;
            face = Some(i);
        }
    }
    let face = face?;
    if radius < distance {
        return None;
    }
    let plane = |face: usize| {
        let f = &mesh.faces[face];
        let v = mesh.edge_vertex(f.edges[0], Some(face), 0);
        (
            PairKind::PolyBallPlane,
            face,
            PolyFeature::BallPlane {
                radius,
                point: mesh.vertices[v].position,
                normal: f.plane.normal,
            },
        )
    };
    if distance <= 0.0 || mesh.inside(face, p) {
        return Some(plane(face));
    }
    let ends = |e: usize| {
        let edge = &mesh.edges[e];
        (
            mesh.vertices[edge.vertices[0]].position,
            mesh.vertices[edge.vertices[1]].position,
        )
    };
    let line_distance = |e: usize| {
        let (v0, v1) = ends(e);
        let q = Line::from_two_points(v0, v1).closest_point(p);
        let d = sub(q, p);
        dot(d, d).sqrt()
    };
    // 00adda14.
    let mut best = f32::MAX;
    let mut found = None;
    for &e in &mesh.faces[face].edges {
        let d = line_distance(e);
        if d < best {
            // 00b3507c: the center is between the edge's end planes.
            let (v0, v1) = ends(e);
            let axis = sub(v1, v0);
            if 0.0 <= dot(sub(p, v0), axis) && dot(axis, sub(p, v1)) <= 0.0 {
                best = d;
                found = Some(e);
            }
        }
    }
    if let Some(e) = found {
        if radius < best {
            return None;
        }
        let (v0, _) = ends(e);
        return Some((
            PairKind::PolyBallEdge,
            e,
            PolyFeature::BallEdge {
                radius,
                point: v0,
                direction: mesh.edge_axis(e),
            },
        ));
    }
    // 00addc50, then 00addd4e.
    let mut best = f32::MAX;
    let mut nearest = None;
    for &e in &mesh.faces[face].edges {
        let d = line_distance(e);
        if d < best {
            best = d;
            nearest = Some(e);
        }
    }
    let edge = &mesh.edges[nearest?];
    let mut best = f32::MAX;
    let mut vertex = None;
    for &v in &edge.vertices {
        let d = sub(p, mesh.vertices[v].position);
        let d = dot(d, d).sqrt();
        if d < best {
            best = d;
            vertex = Some(v);
        }
    }
    let vertex = vertex?;
    if radius < best {
        return None;
    }
    Some((
        PairKind::PolyBallVertex,
        vertex,
        PolyFeature::BallVertex {
            radius,
            vertex: mesh.vertices[vertex].position,
        },
    ))
}

/// The cached separating pair (+0xd4). `a` and `b` are the pair's first
/// and second primitives (+4, +8) as contact body indices.
#[derive(Clone, Debug, PartialEq)]
enum Separation {
    /// FaceFacePair: the separating face of A (+0x1c), the incident face of
    /// B (+0x20) and the runner-up incident face (+0x24).
    Faces {
        a: usize,
        b: usize,
        face: usize,
        incident: Option<usize>,
        alternate: Option<usize>,
    },
    /// EdgeEdgePair: the best edge of each (+0x1c, +0x20).
    Edges {
        a: usize,
        b: usize,
        edges: [usize; 2],
    },
}

struct Bodies<'a> {
    frames: [&'a CoordinateFrame; 2],
    meshes: [&'a Mesh; 2],
}

impl Separation {
    fn same_kind_and_first(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Faces { a, .. }, Self::Faces { a: other, .. }) => a == other,
            (Self::Edges { a, .. }, Self::Edges { a: other, .. }) => a == other,
            _ => false,
        }
    }
    /// The pair's separation (vtable +0x10), updating its features.
    fn separation(&mut self, bodies: &Bodies) -> f32 {
        match self {
            Self::Faces {
                a,
                b,
                face,
                incident,
                alternate,
            } => {
                let (a, b) = (*a, *b);
                let mesh_a = bodies.meshes[a];
                let mesh_b = bodies.meshes[b];
                // 00b53392: B's vertices in A's frame.
                let relative = frame_multiply(&frame_inverse(bodies.frames[a]), bodies.frames[b]);
                let vertices: Vec<[f32; 3]> = mesh_b
                    .vertices
                    .iter()
                    .map(|v| to_world(&relative, v.position))
                    .collect();
                let (mut s, mut vertex) = closest_vertex(&mesh_a.faces[*face].plane, &vertices);
                if !(s <= 0.0) {
                    *incident = None;
                    return s;
                }
                let mut f = 0;
                while f < mesh_a.faces.len() {
                    if f != *face {
                        let (d, v) = closest_vertex(&mesh_a.faces[f].plane, &vertices);
                        if s < d {
                            *face = f;
                            vertex = v;
                            s = d;
                            if 0.0 < d {
                                *incident = None;
                                return d;
                            }
                        }
                    }
                    f += 1;
                }
                *incident = vertex.and_then(|v| incident_face(bodies, a, b, *face, v, alternate));
                s
            }
            Self::Edges { a, b, edges } => edge_separation(bodies, *a, *b, edges),
        }
    }
    /// The pair's connectors (vtable +0x14), appended to `out`.
    fn generate(&self, bodies: &Bodies, out: &mut Vec<GeoPair>) {
        match self {
            Self::Faces {
                a,
                b,
                face,
                incident: Some(incident),
                ..
            } => generate_faces(bodies, *a, *b, *face, *incident, out),
            Self::Faces { incident: None, .. } => {}
            Self::Edges { a, b, edges } => {
                let mesh_a = bodies.meshes[*a];
                let mesh_b = bodies.meshes[*b];
                let line = |mesh: &Mesh, e: usize| {
                    let [v0, v1] = mesh.edges[e].vertices;
                    Line::from_two_points(mesh.vertices[v0].position, mesh.vertices[v1].position)
                };
                out.push(poly_pair(
                    PairKind::PolyEdgeEdge,
                    *a,
                    *b,
                    edges[0],
                    edges[1],
                    PolyFeature::EdgeEdge {
                        edge_a: line(mesh_a, edges[0]),
                        edge_b: line(mesh_b, edges[1]),
                    },
                ));
            }
        }
    }
}

/// 00b534ee: the vertex deepest behind the plane and its distance
/// (FLT_MAX with no vertices).
fn closest_vertex(plane: &Plane, vertices: &[[f32; 3]]) -> (f32, Option<usize>) {
    let mut best = f32::MAX;
    let mut found = None;
    for (i, v) in vertices.iter().enumerate() {
        let d = dot(*v, plane.normal) - plane.distance;
        if d < best {
            found = Some(i);
            best = d;
        }
    }
    (best, found)
}

/// 00b535a0: the face of B at vertex `v` facing A's face most directly,
/// keeping the runner-up in `alternate` when they are close.
fn incident_face(
    bodies: &Bodies,
    a: usize,
    b: usize,
    face: usize,
    v: usize,
    alternate: &mut Option<usize>,
) -> Option<usize> {
    *alternate = None;
    let mesh_b = bodies.meshes[b];
    let relative = frame_multiply(&frame_inverse(bodies.frames[b]), bodies.frames[a]);
    let p = bodies.meshes[a].faces[face]
        .plane
        .to_world(&relative)
        .normal;
    let mut best = -2.0f32;
    let mut chosen = None;
    for i in 0..mesh_b.vertices[v].edges.len() {
        let Some(f) = mesh_b.vertex_face(v, i) else {
            continue;
        };
        let n = mesh_b.faces[f].plane.normal;
        let facing = (-(n[1] * p[1]) - n[0] * p[0]) - n[2] * p[2];
        if facing > best {
            if best > 0.3 && (facing - best).abs() < 0.4 {
                *alternate = chosen;
            }
            chosen = Some(f);
            best = facing;
        }
    }
    chosen
}

/// EdgeEdgePair separation 00b54a60.
fn edge_separation(bodies: &Bodies, a: usize, b: usize, edges: &mut [usize; 2]) -> f32 {
    let fa = bodies.frames[a];
    let fb = bodies.frames[b];
    let mesh_a = bodies.meshes[a];
    let mesh_b = bodies.meshes[b];
    let support_a = mesh_a.support(mul_transpose(&fa.rotation, sub(fb.position, fa.position)));
    let support_b = mesh_b.support(mul_transpose(&fb.rotation, sub(fa.position, fb.position)));
    let (count_a, count_b) = (mesh_a.edges.len(), mesh_b.edges.len());
    let (start_a, start_b) = (edges[0], edges[1]);
    let touches = |mesh: &Mesh, e: usize, v: Option<usize>| {
        v.is_some_and(|v| mesh.edges[e].vertices.contains(&v))
    };
    let line = |mesh: &Mesh, e: usize, f: &CoordinateFrame| {
        let [v0, v1] = mesh.edges[e].vertices;
        Line::from_two_points(mesh.vertices[v0].position, mesh.vertices[v1].position).to_world(f)
    };
    let mut best = f32::MIN;
    for i in start_a..start_a + count_a {
        let ea = i % count_a;
        if !touches(mesh_a, ea, support_a) {
            continue;
        }
        let la = line(mesh_a, ea, fa);
        for j in start_b..start_b + count_b {
            let eb = j % count_b;
            if !touches(mesh_b, eb, support_b) {
                continue;
            }
            let lb = line(mesh_b, eb, fb);
            let mut axis = cross(la.direction, lb.direction);
            if !(unitize(&mut axis, 1e-6) > 0.001) {
                continue;
            }
            let plane = Plane {
                normal: axis,
                distance: 0.0,
            };
            let (min_a, max_a) = mesh_a.project(&plane.to_object(fa));
            let (min_b, max_b) = mesh_b.project(&plane.to_object(fb));
            let low = if min_b > min_a { min_b } else { min_a };
            let high = if max_b < max_a { max_b } else { max_a };
            let s = low - high;
            if s > best {
                *edges = [ea, eb];
                if s > 0.0 {
                    return s;
                }
                best = s;
            }
        }
    }
    best
}

fn poly_pair(
    kind: PairKind,
    body_a: usize,
    body_b: usize,
    id_a: usize,
    id_b: usize,
    feature: PolyFeature,
) -> GeoPair {
    GeoPair {
        kind,
        point_a: [0.0; 3],
        point_b: [0.0; 3],
        body_a,
        body_b,
        length_a: f32::NAN,
        length_b: f32::NAN,
        id_a: id_a as i32,
        id_b: id_b as i32,
        id_c: UNSET,
        poly: Some(Box::new(feature)),
    }
}

/// FaceFacePair::generate 00b53872.
fn generate_faces(
    bodies: &Bodies,
    a: usize,
    b: usize,
    face: usize,
    incident: usize,
    out: &mut Vec<GeoPair>,
) {
    let prims = [a, b];
    let mesh_a = bodies.meshes[a];
    let mesh_b = bodies.meshes[b];
    // 00b53c40: frames[i] maps prim i^1 into prim i.
    let frames: [CoordinateFrame; 2] = std::array::from_fn(|i| {
        frame_multiply(
            &frame_inverse(bodies.frames[prims[i]]),
            bodies.frames[prims[i ^ 1]],
        )
    });
    let mut codes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for i in 0..2 {
        // 00b543f6: the test face's vertices against the reference face.
        let (reference, test) = if i == 0 {
            ((mesh_a, face), (mesh_b, incident))
        } else {
            ((mesh_b, incident), (mesh_a, face))
        };
        let plane = reference.0.faces[reference.1].plane;
        let mut all_inside = true;
        for &e in &test.0.faces[test.1].edges {
            let v = test.0.edge_vertex(e, Some(test.1), 0);
            let vertex = test.0.vertices[v].position;
            // 00b544de.
            let p = to_world(&frames[i], vertex);
            let below = plane.behind(p);
            let inside = reference.0.inside(reference.1, p);
            let code = match (below, inside) {
                (false, true) => 0,
                (false, false) => 1,
                (true, true) => 2,
                (true, false) => 3,
            };
            codes[i].push(code);
            if code == 2 {
                // 00b545c8.
                out.push(poly_pair(
                    PairKind::FaceVertex,
                    prims[i],
                    prims[i ^ 1],
                    reference.1,
                    v,
                    PolyFeature::FaceVertex { plane, vertex },
                ));
            } else {
                all_inside = false;
            }
        }
        if all_inside {
            return;
        }
    }
    // The incident face's edges against the separating face's edges.
    let edges = &mesh_b.faces[incident].edges;
    let n = edges.len();
    let point = |v: usize| to_world(&frames[0], mesh_b.vertices[v].position);
    let mut face_edge = |k: usize, x: usize, y: usize| {
        // 00b54788.
        let Some(eb) = mesh_b.find_edge(x, y) else {
            return;
        };
        let ea = mesh_a.faces[face].edges[k];
        let line = |mesh: &Mesh, e: usize| {
            let [v0, v1] = mesh.edges[e].vertices;
            Line::from_two_points(mesh.vertices[v0].position, mesh.vertices[v1].position)
        };
        out.push(poly_pair(
            PairKind::FaceEdge,
            a,
            b,
            k,
            eb,
            PolyFeature::FaceEdge {
                normal: mesh_a.faces[face].plane.normal,
                edge_a: line(mesh_a, ea),
                edge_b: line(mesh_b, eb),
            },
        ));
    };
    for u in 0..n {
        let next = if u == n - 1 { 0 } else { u + 1 };
        let x = mesh_b.edge_vertex(edges[u], Some(incident), 0);
        let y = mesh_b.edge_vertex(edges[next], Some(incident), 0);
        // 00b53eae: the first edge crossed below the face.
        let one = |x: usize, y: usize| mesh_a.crossing_edge(face, point(x), point(y));
        match (codes[0][u], codes[0][next]) {
            (0, 3) | (1, 2) | (2, 1) | (3, 0) => {
                if let Some(k) = one(x, y) {
                    face_edge(k, x, y);
                }
            }
            // 00b5405c: up to two edges.
            (1, 3) | (3, 1) | (3, 3) => {
                let (first, second) = mesh_a.crossing_edges(face, point(x), point(y));
                for k in [first, second].into_iter().flatten() {
                    face_edge(k, x, y);
                }
            }
            // 00b54248: the first edge crossed at any height.
            (2, 3) => {
                if let Some(k) = mesh_a.crossing_edge_any(face, point(x), point(y)) {
                    face_edge(k, x, y);
                }
            }
            (3, 2) => {
                if let Some(k) = mesh_a.crossing_edge_any(face, point(y), point(x)) {
                    face_edge(k, y, x);
                }
            }
            _ => {}
        }
    }
}

/// PolyPolyContact data: the cached pair (+0xd4) and the connectors
/// (FixedArray at +0x30), each marked when it is in the kernel.
///
/// The caller's primitive order may differ from the native one (the lower
/// collide type first, Block 2 before a Wedge's 3): `order[i]` is the
/// caller's index of native primitive i, and connector bodies use the
/// caller's indices.
#[derive(Clone, Debug)]
pub struct PolyContact<T> {
    /// Native order.
    shapes: [PolyShape; 2],
    /// BallPolyContact: native body 0 is a Ball (`shapes[0]` unused).
    ball: bool,
    order: [usize; 2],
    answer: Option<Separation>,
    connectors: Vec<Connector<T>>,
    in_kernel: Vec<bool>,
    /// Connector events since the last `take_events`.
    pub events: Vec<ContactEvent<T>>,
}

/// The collide type (Geometry vtable +0x10): Block 2, Poly 3.
fn collide_type(shape: PolyShape) -> u32 {
    match shape {
        PolyShape::Block => 2,
        PolyShape::Wedge => 3,
    }
}

impl<T: Default> PolyContact<T> {
    /// A contact between primitives of `shapes`, in the caller's order.
    pub fn new(shapes: [PolyShape; 2]) -> Self {
        let order = if collide_type(shapes[1]) < collide_type(shapes[0]) {
            [1, 0]
        } else {
            [0, 1]
        };
        Self {
            shapes: [shapes[order[0]], shapes[order[1]]],
            ball: false,
            order,
            answer: None,
            connectors: Vec::new(),
            in_kernel: Vec::new(),
            events: Vec::new(),
        }
    }
    /// A BallPolyContact (00add538): the Ball is the caller's primitive
    /// `ball` (0 or 1), the other a Poly of `shape`. It shares
    /// PolyContact's step, collision test and rebuild (the same vtable
    /// slots) and finds its connectors with 00add542.
    pub fn new_ball(ball: usize, shape: PolyShape) -> Self {
        Self {
            shapes: [PolyShape::Block, shape],
            ball: true,
            order: if ball == 0 { [0, 1] } else { [1, 0] },
            answer: None,
            connectors: Vec::new(),
            in_kernel: Vec::new(),
            events: Vec::new(),
        }
    }
    pub fn connectors(&self) -> &[Connector<T>] {
        &self.connectors
    }
    pub fn connectors_mut(&mut self) -> &mut [Connector<T>] {
        &mut self.connectors
    }
    /// A fresh contact of the same primitives (cached pair forgotten).
    pub fn reset(&mut self) {
        self.answer = None;
        self.connectors.clear();
        self.in_kernel.clear();
        self.events.clear();
    }
    /// Whether connector `index` is in the kernel.
    pub fn is_live(&self, index: usize) -> bool {
        self.in_kernel.get(index).copied().unwrap_or(false)
    }
    /// The kernel refused connector `index` (neither body in a kernel list).
    pub fn set_outside(&mut self, index: usize) {
        if let Some(flag) = self.in_kernel.get_mut(index) {
            *flag = false;
        }
    }
    pub fn take_connectors(&mut self) -> Vec<Connector<T>> {
        self.in_kernel.clear();
        std::mem::take(&mut self.connectors)
    }
    pub fn take_events(&mut self) -> Vec<ContactEvent<T>> {
        std::mem::take(&mut self.events)
    }
    /// Connector geometry refresh (virtual +0x24) for the current frames.
    pub fn refresh(&mut self, blocks: [&Block; 2]) {
        for c in &mut self.connectors {
            c.point = c.pair.compute(blocks);
        }
    }
    /// findPolyConnectors 00b52c2a with the pair search 00b52d2e.
    fn find_connectors(&mut self, blocks: [&Block; 2]) -> Vec<GeoPair> {
        let native = [blocks[self.order[0]], blocks[self.order[1]]];
        if self.ball {
            return ball_connectors(native[0], native[1], self.shapes[1])
                .map(|(kind, id, feature)| {
                    poly_pair(kind, self.order[0], self.order[1], 0, id, feature)
                })
                .into_iter()
                .collect();
        }
        let meshes: [Mesh; 2] = std::array::from_fn(|i| Mesh::new(self.shapes[i], native[i].size));
        let bodies = Bodies {
            frames: [&native[0].frame, &native[1].frame],
            meshes: [&meshes[0], &meshes[1]],
        };
        let fresh_faces = |a: usize, b: usize| Separation::Faces {
            a,
            b,
            face: 0,
            incident: None,
            alternate: None,
        };
        let mut answer = self.answer.take().unwrap_or_else(|| fresh_faces(0, 1));
        let mut best = answer.separation(&bodies);
        if !(best > 0.0) {
            let mut chosen = None;
            for mut candidate in [
                fresh_faces(0, 1),
                fresh_faces(1, 0),
                Separation::Edges {
                    a: 0,
                    b: 1,
                    edges: [0, 0],
                },
            ] {
                if answer.same_kind_and_first(&candidate) {
                    continue;
                }
                let s = candidate.separation(&bodies);
                if 0.0 < s {
                    chosen = None;
                    answer = candidate;
                    break;
                }
                if best + 0.01 < s {
                    best = s;
                    chosen = Some(candidate);
                }
            }
            if let Some(candidate) = chosen {
                answer = candidate;
            }
        }
        let mut out = Vec::new();
        answer.generate(&bodies, &mut out);
        if let Separation::Faces {
            incident,
            alternate: Some(alternate),
            ..
        } = &mut answer
            && out.is_empty()
        {
            *incident = Some(*alternate);
            answer.generate(&bodies, &mut out);
        }
        self.answer = Some(answer);
        for pair in &mut out {
            pair.body_a = self.order[pair.body_a];
            pair.body_b = self.order[pair.body_b];
        }
        out
    }
    /// The rebuild 00b520a4: new connectors, each replaced by a matching
    /// old one; the old ones left over leave the kernel and are deleted.
    fn rebuild(&mut self, blocks: [&Block; 2]) {
        let pairs = self.find_connectors(blocks);
        let mut old: Vec<(Connector<T>, bool)> = std::mem::take(&mut self.connectors)
            .into_iter()
            .zip(std::mem::take(&mut self.in_kernel))
            .collect();
        for pair in pairs {
            let found = old.iter().position(|(c, _)| {
                c.pair.id_a == pair.id_a && c.pair.id_b == pair.id_b && c.pair.kind == pair.kind
            });
            let (connector, live) = match found {
                Some(i) => old.swap_remove(i),
                None => (
                    Connector {
                        pair,
                        point: ContactPoint {
                            normal: [0.0; 3],
                            length: 0.0,
                            position: [0.0; 3],
                        },
                        state: T::default(),
                    },
                    false,
                ),
            };
            self.connectors.push(connector);
            self.in_kernel.push(live);
        }
        for (connector, live) in old {
            if live {
                self.events.push(ContactEvent::Removed {
                    connector,
                    recomputed: false,
                });
            }
        }
    }
    /// Every connector's geometry update (00b521c6's +0x24 calls, or the
    /// step's): the kernel ones redisplace. Returns the deepest overlap.
    fn compute_all(&mut self, blocks: [&Block; 2]) -> f32 {
        let mut deepest = f32::MIN;
        for (index, c) in self.connectors.iter_mut().enumerate() {
            c.point = c.pair.compute(blocks);
            if self.in_kernel[index] {
                self.events.push(ContactEvent::Kept {
                    index,
                    reused: true,
                });
            }
            let overlap = -c.point.length;
            if deepest < overlap {
                deepest = overlap;
            }
        }
        deepest
    }
    /// computeIsColliding 00b5203a with `ignored` overlap.
    pub fn compute_is_colliding(&mut self, blocks: [&Block; 2], ignored: f32) -> bool {
        if !extents_overlap(&blocks[0].extents(), &blocks[1].extents()) {
            return false;
        }
        self.rebuild(blocks);
        !self.connectors.is_empty() && ignored < self.compute_all(blocks)
    }
    /// The contact step 00b51fc4. `in_kernel` is the stage 0xf test.
    pub fn update(&mut self, blocks: [&Block; 2], in_kernel: bool) -> bool {
        if !self.compute_is_colliding(blocks, 0.0) {
            // 00b51ec4: out of the kernel, still listed.
            for index in 0..self.connectors.len() {
                if self.in_kernel[index] {
                    self.in_kernel[index] = false;
                    self.events.push(ContactEvent::Withdrawn { index });
                }
            }
            return false;
        }
        if in_kernel {
            self.compute_all(blocks);
            // 00b51f34.
            for index in 0..self.connectors.len() {
                if !self.in_kernel[index] && self.connectors[index].point.length < -0.01 {
                    self.in_kernel[index] = true;
                    self.events.push(ContactEvent::Kept {
                        index,
                        reused: false,
                    });
                }
            }
        }
        true
    }
}

/// The Poly shape of a Block or Wedge collide geometry.
pub fn poly_shape(shape: crate::material::Shape) -> Option<PolyShape> {
    match shape {
        crate::material::Shape::Block => Some(PolyShape::Block),
        crate::material::Shape::Wedge => Some(PolyShape::Wedge),
        crate::material::Shape::Ball => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    fn block(position: [f32; 3], size: [f32; 3]) -> Block {
        Block {
            frame: CoordinateFrame {
                position,
                rotation: IDENTITY,
            },
            size,
        }
    }

    #[test]
    fn ball_against_wedge_finds_plane_edge_and_vertex() {
        // An 8x4x8 Wedge at the origin: slope y = z / 2, normal (0, 2, -1)/sqrt 5.
        let wedge = block([0.0; 3], [8.0, 4.0, 8.0]);
        let root5 = 5f32.sqrt();
        let n = [0.0, 2.0 / root5, -1.0 / root5];
        let cases = [
            // Diameter 2, sunk 0.05 into the slope.
            (n.map(|v| v * 0.95), PairKind::PolyBallPlane, -0.05),
            // Past the top back edge (y 2, z 4): 0.6 out along y and z.
            (
                [0.0, 2.6, 4.6],
                PairKind::PolyBallEdge,
                0.72f32.sqrt() - 1.0,
            ),
            // Past the corner (4, 2, 4).
            (
                [4.5, 2.5, 4.5],
                PairKind::PolyBallVertex,
                0.75f32.sqrt() - 1.0,
            ),
        ];
        for (center, kind, length) in cases {
            let ball = block(center, [2.0; 3]);
            let mut contact = PolyContact::<()>::new_ball(0, PolyShape::Wedge);
            assert!(contact.update([&ball, &wedge], true), "{kind:?}");
            let c = contact.connectors();
            assert_eq!(c.len(), 1, "{c:?}");
            assert_eq!(c[0].pair.kind, kind);
            assert_eq!((c[0].pair.body_a, c[0].pair.body_b), (0, 1));
            assert!((c[0].point.length - length).abs() < 1e-5, "{c:?}");
            // The normal points from the ball into the Wedge.
            let into = sub(c[0].point.position, center);
            assert!(dot(into, c[0].point.normal) > 0.0, "{c:?}");
        }
        // The Wedge first in the caller's order: the ball is still body A.
        let ball = block(n.map(|v| v * 0.95), [2.0; 3]);
        let mut contact = PolyContact::<()>::new_ball(1, PolyShape::Wedge);
        assert!(contact.update([&wedge, &ball], true));
        assert_eq!(contact.connectors()[0].pair.body_a, 1);
        // Clear of it: nothing.
        let ball = block(n.map(|v| v * 1.5), [2.0; 3]);
        let mut contact = PolyContact::<()>::new_ball(0, PolyShape::Wedge);
        assert!(!contact.update([&ball, &wedge], true));
    }

    #[test]
    fn wedge_resting_on_block_gets_its_four_bottom_corners() {
        // A 4x1x2 Wedge sunk 0.05 into a 10x1x10 floor (top at y = 0.5).
        let floor = block([0.0, 0.0, 0.0], [10.0, 1.0, 10.0]);
        let wedge = block([0.0, 0.95, 0.0], [4.0, 1.0, 2.0]);
        let mut contact = PolyContact::<()>::new([PolyShape::Block, PolyShape::Wedge]);
        assert!(contact.update([&floor, &wedge], true));
        let c = contact.connectors();
        assert_eq!(c.len(), 4, "{c:?}");
        for connector in c {
            assert_eq!(connector.pair.kind, PairKind::FaceVertex);
            // The floor's top face, normal up from the floor to the wedge.
            assert_eq!(connector.pair.body_a, 0);
            assert!((connector.point.length + 0.05).abs() < 1e-5);
            assert!(connector.point.normal[1] > 0.999);
        }
        // All four are deeper than 0.01, so all enter the kernel.
        assert_eq!(
            contact
                .events
                .iter()
                .filter(|e| matches!(e, ContactEvent::Kept { reused: false, .. }))
                .count(),
            4
        );
    }

    #[test]
    fn separated_wedge_has_no_connectors_in_the_kernel() {
        let floor = block([0.0, 0.0, 0.0], [10.0, 1.0, 10.0]);
        let mut contact = PolyContact::<()>::new([PolyShape::Block, PolyShape::Wedge]);
        let resting = block([0.0, 0.95, 0.0], [4.0, 1.0, 2.0]);
        assert!(contact.update([&floor, &resting], true));
        contact.take_events();
        // Lifted 0.2 clear: within the extents, but separated.
        let lifted = block([0.0, 1.2, 0.0], [4.0, 1.0, 2.0]);
        assert!(!contact.update([&floor, &lifted], true));
        assert!((0..contact.connectors().len()).all(|i| !contact.is_live(i)));
    }

    #[test]
    fn block_on_wedge_slope_touches_the_slope() {
        // A 2x2x2 Block resting on the slope of a 4x4x4 Wedge, rotated to
        // the 45 degree slope and pushed 0.05 into it.
        let wedge = block([0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let rotation = [[1.0, 0.0, 0.0], [0.0, s, s], [0.0, -s, s]];
        let center = [0.0, (1.0 - 0.05) * s, -(1.0 - 0.05) * s];
        let cube = Block {
            frame: CoordinateFrame {
                position: center,
                rotation,
            },
            size: [2.0, 2.0, 2.0],
        };
        let mut contact = PolyContact::<()>::new([PolyShape::Block, PolyShape::Wedge]);
        assert!(contact.update([&cube, &wedge], true));
        let deepest = contact
            .connectors()
            .iter()
            .map(|c| -c.point.length)
            .fold(f32::MIN, f32::max);
        assert!((deepest - 0.05).abs() < 1e-4, "{deepest}");
    }
}
