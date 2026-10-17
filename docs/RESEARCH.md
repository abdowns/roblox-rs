# Current tool and reference checks — 2026-10-12

This project targets only the supplied **Roblox Studio macOS 0.255.0.82159**
offline client. The local plist, Mach-O header and executable digest were checked.
The [August archive listing](https://archive.roblonium.com/Client/Mac/Studio/2016/8.2016/)
names “August 5 (0.255.0.82159)”. No replacement client was downloaded. The digest
identifies our exact input; it does not establish archive authenticity.

## Engine and reverse-engineering tools

- [Bevy 0.20](https://bevy.org/news/bevy-0-20/) was released October 8, 2026.
  Its [v0.20.0 manifest](https://raw.githubusercontent.com/bevyengine/bevy/v0.20.0/Cargo.toml)
  requires Rust 1.97.1. Both are pinned here; that Rust toolchain was already
  installed, and the headless dependency set builds on this Apple Silicon host.
- [Ghidra latest](https://github.com/NationalSecurityAgency/ghidra/releases/latest)
  resolves to 12.1.4. The local headless tool reports 12.1.4 and imported/decompiled
  this image with language/compiler x86:LE:32:default:gcc.
- [ghidra-mcp releases](https://github.com/bethington/ghidra-mcp/releases) mark
  6.0.0 stable/latest and 7.0.0 prerelease when checked. The configured MCP's
  running extension now reports 7.0.0. Instance discovery/reconnection restored
  live CodeBrowser access; STATUS preserves the earlier project-state failure.
  No system tool or extension was installed or upgraded.
- [wgpu 30.0.1](https://docs.rs/wgpu/30.0.1/wgpu/),
  [winit 0.30.13](https://docs.rs/winit/0.30.13/winit/), and
  [image 0.25.10](https://docs.rs/image/latest/image/) were checked against current
  official API/release documentation and pinned. Direct Metal execution and
  bounded PNG decoding pass locally; native historical parity remains separate.

## Community file tools

- [rbx-dom](https://github.com/rojo-rbx/rbx-dom) supplies independent Rust instance
  and file tooling. The [rbx_xml changelog](https://raw.githubusercontent.com/rojo-rbx/rbx-dom/master/rbx_xml/CHANGELOG.md)
  lists 3.0.1 on October 1, 2026. Its reflection/migration behavior is not verified
  against exact 2016 defaults. This first importer reads explicit historical XML
  fields without a modern reflection database.
- [RobloxAPI's binary format specification](https://github.com/RobloxAPI/spec/blob/master/formats/rbxl.md)
  describes the XML/binary distinction and chunk structure. It is a future reference;
  no binary-place reader exists here yet.
- [Rojo latest](https://github.com/rojo-rbx/rojo/releases/latest) resolves to 7.7.1
  (October 2, 2026). Compatibility with this 2016 client is **not tested**. We have
  not installed its Studio plugin. This standalone rewrite does not currently
  need an injected client loader or a plugin inside the original installation.

## Reference projects

[IW4L's current public README](https://github.com/vladtrc/iw4L) describes a
Rust/Bevy runtime reading the user's MW2 installation, native data readers and
incomplete gameplay. The pattern adopted here is original data kept local,
observed contracts documented and claims bounded by tests. No source or assets
were copied. Raw main-branch manifest and GitHub API commit lookups failed in web
access; an exact current revision was not established, and no dependency on IW4L
is used.

The explicitly named `/Users/abd/Programming/2015roblox-rs` was read for loose
reference. Its target is 0.224.0.67185. All 37 matching mesh payloads match the
2016 installation by SHA-256; the character gains 14 Attachments, and the old Rust
reader's acceptance rules differ. No code was copied. Its simulation, material,
joint and input behavior remains unverified for 2016 and must be traced before reuse.

The local guide index, Rust rewrite guide, IW4L case study and supplied decompile
system map informed the workflow. No installation patching, online services,
authentication or community client loader execution was needed for this step.

## Window lifecycle check — same pinned releases

The cached primary source for wgpu-hal 30.0.1 Metal surface acquisition checks
NSWindow occlusion before acquiring a drawable. winit 0.30.13 set_visible orders
the window forward; focus_window also activates NSApplication. These APIs were
verified against the dependency sources actually built here. Online access to
the linked wgpu issue and versioned winit docs was unavailable during this check;
no system tool or dependency was changed. Numeric motion smoke confirms recovery
from a startup Occluded result, not complete macOS lifecycle equivalence.

## Poly geometry and PolyPolyContact (raw notes; ported in poly.rs and poly_contact.rs)

## createContact 00ae714a (non-Bullet)
types ordered low first. 3-3 and 2-3 -> PolyPolyContact (ctor 00b52b82->00b52b0a, 0xd8 bytes, vtable 01c87858)
1-3 BallPolyContact 00add538 (0xd4). 2-2 BlockBlock, 1-2 BallBlock, 1-1 BallBall. type 4 (Bullet) -> 00c565f8 or 0080aa0e.

## addPartImpl 010d1850 part type (vtable+0xa4): 0 Ball addSphere, 1 Block, 2 Cyl 010da0d0, 3 Truss, 4 Wedge 010dc290, 9 CornerWedge 010e14b0, 0xb Op, 0xc MeshPart

## WedgePoly vtable 01c88878 (typeinfo 01c88900, base 01c877e0 = Poly?)
slots: 0 ba4f36,1 ba4f58,2 ba4fee,3 ba5096,4 af55f0,5 ba49c2,6 d1b9dc,7 b67f90,8 b516d0,9 b4fd90,10 b4fee0,11 ba4c64(surface cframe),12 b4ffb8,13 b5011a,14 af5600,15 b5005e,16 b50258,17 b502e4,18 ba4dc4(face->surface table 017ab950),19 ba50aa,20 b50970,21 b50c50,22 b50cd8,23 b50ff4,24 ba4de6(extents),25 ba4c20(CoM offset (0,-y/6,z/6)),26 ba4aac(inertia(mass)),27 b0ab80,28 b4fbd6,29 b4fbfc,30 b50536,31 b64660,32 ba4950(buildMesh)
- ba4950 buildMesh: mesh cache ba50b2 keyed by size -> new 0x2c obj ba536e -> b31f14(mesh,size)
- b31f14 wedge mesh: h=size/2; v0(+,+,+) v1(+,-,+) v2(+,-,-) v3(-,+,+) v4(-,-,+) v5(-,-,-)
  quad(0,3,4,1) quad(3,0,2,5) tri(0,1,2) tri(3,5,4) quad(5,2,1,4). reserve v6 f5 e9
- Mesh: +0 vertices vec (0x1c each: id, pos[3], vec<Edge*>), +0xc edges vec, +0x18 faces vec (0x24 each)
- Face 0x24: +0 id, +4 vec<Edge*>, +0x10 G3D::Plane (vptr, normal +0x14, dist +0x20). plane from getVertex(face,0) of edges 0,1,2 (b33cee)
- quad face b2fd00: edges via getOrCreateEdge b33b08(a,b),(b,c),(c,d),(d,a); edge->setFace b35542 (forward +0xc first, else backward +0x10)
- edge->getVertex(face,i) = d1baf8

## PolyContact (base) — PolyPolyContact vtable 01c87858
slots: 6 b5304c(->ae27b2 make params +0x2c 16B, then vt+0x2c ae26d0), 7 b51ea8 (b51d88 delete all), 8 b51fc4 step, 9 ae26a6 touch, 10 b5203a computeIsColliding(ignored), 12 b55480 count, 13 b51e98 connector(i), 16 b52c2a findPolyConnectors(newList)
- connectors FixedArray<Connector*,40> at +0x30, count +0xd0 (assert <=10). +0xd4 answer (pair obj)
- b5203a: extents overlap aec6e4 else false; b520a4 rebuild; colliding = count!=0 && ignored < max(-conn.length(+0xe8)) [b521c6 calls conn vt+0x24 first]
- b520a4: new list via vt+0x40; b522b2 match (b52304/b52366: same ids +0x110,+0x114 and same vt+0x28 type -> old replaces new, old swap-removed); b51d88 old leftovers: b51ec4 remove from kernel, delete; memcpy new
- b51fc4 step: if !vt+0x28(0): b51ec4 (kernel remove, keep list) ret 0. else if inKernel(009e1220 ,0xf): each conn vt+0x24; b51f34: not in kernel && length < -0.01 -> kernel insert a95f22. ret 1
- b52c2a: both Poly (dyncast) -> b52d2e; answer vt+0x14(newList); if EdgeEdgePair? (cast to 01c878a0 FaceFacePair) && no new && ans+0x24 -> ans+0x20=ans+0x24, vt+0x14 again
- b52d2e: answer default FaceFacePair(A,B). if ans.sep(vt+0x10) <= 0: try [FFP(A,B), FFP(B,A), EEP(A,B)] skipping same type&&same first prim; first >0 wins (clone vt+0xc); else best with > best+0.01 (start best = current sep)

## FaceFacePair vtable 01c878b4 (name FaceFacePair): 0 b55496,1 b553da,2 b554a0,3 b53058 clone,4 b530b2 sep,5 b53872 gen
fields +4 primA +8 primB +0xc..0x18 params +0x1c faceA* +0x20 incident faceB* +0x24 ?
- b530b2: rel = inv(frameA)*frameB; verts of B in A space (b53392, FixedArray<V3,40>); sep=b534ee(face cur); if<=0 scan all faces (skip cur) keep max; early exit >0 (+0x20=0); end: +0x20=b535a0(vertIdx)
## EdgeEdgePair vtable 01c878d4: 3 b54a0c clone, 4 b54a60 sep, 5 b550ec gen. fields +7: edgesA, +8 edgesB

## Mesh details
- Vertex 0x1c: +0 id, +4 pos, +0x10 vec<Edge*> (edges in creation order, pushed by b356a8 when edge created)
- Edge 0x14: +0 id, +4 v0, +8 v1, +0xc forward face, +0x10 backward face. created b33e3e(mesh,v0,v1) pushes to v0 and v1 edge lists
- getOrCreateEdge b33b08(a,b): find in a's edges one containing b (b33d70, first match in a's list); else create a->b. (found one must be b->a with forward set, no backward)
- setFace b35542: forward if empty else backward
- getVertex(face,i) d1baf8: face==0||face==forward -> v[i] else v[(i+1)%2]
- vertex->face(i) b3427c -> b3574e(edge i, v): v==edge.v0 ? forward : backward
- face plane d1fbd0 = face+0x10 (Plane: +4 normal(+0x14 abs), +0x10 dist (+0x20 abs))
- Face plane = G3D::Plane(p0,p1,p2) with p_k = getVertex(face,0) of edge k

## FaceFacePair helpers
- b53392 verts of B into A space: ((R00 x + R01 y)+R02 z)+t ? (decompile printed z-first; CHECK asm)
- b534ee(face, verts, &idx): min over verts of (dot(n,v) - dist) strict <, idx=vertex ptr; FLT_MAX if none
- b535a0(vertB): +0x24=0; plane of faceA -> B space (rel=inv(B)*A, toWorldSpace(plane)); over vertB edges i: f=face(i); a = -(nf.y*pn.y) - nf.x*pn.x - nf.z*pn.z (CHECK asm order); best starts -2, strict >; if new best and old best >0.3 and |new-old|<0.4 -> +0x24 = old best face. returns best face

## FaceFacePair::generate b53872 (if +0x20 incident face)
- codes[2] FixedArray<u32,40>; frames[2]
- b53c40: for i in 0,1: frames[i] = inv(frame prim i) * frame prim (i+1)%2; if b543f6(i, codes[i], frames[i], out) -> return true (stop all)
- b543f6(i): ref face = i==0 ? +0x1c(A) : +0x20(B); test face = i==0 ? +0x20 : +0x1c. For each edge k of test face: v=getVertex(face,0); code=b544de(v, refFace, frame) push; if code==2 -> b545c8(prim[i], prim[(i&1)^1], v, refFace) FaceVertex connector; else allIn=false. return allIn
- b544de: p = frame*v.pos; below = d1fc5a(plane,p); inside=b553e0(face,p); code = below ? (inside?2:3) : (inside?0:1)
- b545c8 FaceVertexConnector(0x138): body0=prim[i].body(+0x58) (face), body1 vertex body, params this+0xc; ids +0x110 face id, +0x114 vertex id; +0x118 plane(local of face body: vptr,n,d); +0x12c vertex local pos
- if !done: for each edge u of incident face B (+0x20): a=getVertex(edge u,face,0), b = next edge's v0; c0=codes[0][u], c1=codes[0][next]
  (0,3)->E1(a,b); (1,3)->E2(a,b); (1,2)->E1; (2,3)->E3(a,b); (2,1)->E1; (3,0)->E1; (3,1)->E2; (3,2)->E3(b,a); (3,3)->E2
  E1=b53eae: pa,pb = frames[0]*pos; i=b344aa(faceA,pa,pb) (first edge k with b344fa true) -> b54788(i,a,b)
  E2=b5405c: b347aa -> up to two edges (b344fa) -> b54788 each
  E3=b54248: b345e8 (first k with b346f6) -> b54788
- b54788 FaceEdgeConnector (0x178): body0 A, body1 B; ids +0x110 = A face edge index k, +0x114 = B edge id (b34fea(a,b) find edge); +0x118 A face plane; +0x12c d1fe1e(faceA,k) plane; +0x140 segment(A face edge k v0,v1) [b5554c, vptr 01cb4c58]; +0x15c segment(B edge v0,v1)

## Connectors (PolyConnectors.cpp) vtables (+0x24 compute, +0x28 type)
FaceVertex 01c85ee8: aa9fe2, 6 | FaceEdge 01c85f48: aaa11e, 8 | EdgeEdge 01c85fa8: aaa48a, 7
BallVertex 01c86008: aa9ea4, 9 | BallEdge 01c86068: aa9c2a, 10 | BallPlane 01c860c8: aa99b2, 11
- contact point: +0xdc normal, +0xe8 length, +0xec position; bodies +0x94 (0), +0x98 (1); body frame at body+0xb4 (rot), +0xd8 (pos)
- FaceVertex aa9fe2: plane->world(body0); p = body1*vertex; n=plane n; length = dot(p,n)-d; then a9457c
- normal convention: body0 -> body1 (same as port GeoPair)

## More (end of session 2026-10-15)
- G3D Plane(p0,p1,p2) 01257274: n = dir(cross(p1-p0,p2-p0)), d = n.p0. Plane(n,pt) 0125737e: n=dir(n), d=n.pt
- 0129c468 halfSpaceContains: d <= n.clamp(p) (normal side); d1fc5a: n.clamp(p) <= d (back side)
- Line::fromTwoPoints b5554c: dir=(p1-p0)*1/sqrt, then normalized again. Line::intersection 01254c28: rate=dir.n; t=-(n.p - dist)/rate; p+dir*t; rate==0 -> inf
- side plane d1fe1e(face,k): Plane(cross(edgeDir d200a0, faceN), getVertex(edge k,face,0))
- b344fa crossing: side plane k separates p0,p1 (halfSpaceContains xor), X=Line(p0,p1)∩side; below face plane(X); b348d6 X between edge endpoints (planes ±cross(faceN,sideN) through edge vertex) [CHECK getVertex arg via asm]
- b346f6 same w/o face-plane test. b3584c pointInInternalExtrusion = below face plane && b553e0
- b553e0 inside face: project p to plane; b33fd6: below every adjacent face's plane (b357cc other face across edge)
- FaceEdge compute aaa11e: lines A,B to world; closest pts (aaa994 line-line, parallel if 1-c^2<=1e-6); n=unitize(qA-qB,1e-20), len=-|..|, pos=qA; if n.(R0*faceN)<=0 flip, len=+; parallel: n=R0*faceN, len 0, pos=pointA
- EdgeEdge compute aaa48a: same with unitize eps 1e-6, flip test n.(p1-p0 body centers)<=0; parallel: n=dirA, len 0, pos=pointA
- EdgeEdgePair sep b54a60: support verts b34046 (argmax dot, strict >) toward other center (Rᵀ(pOther-p)); iterate A edges from cached id (wrap) touching support A, B edges from cached touching support B; axis=unitize(dA x dB,1e-6) must be >0.001; project meshes (b55032 min/max of n.v-d); sep=max(minA,minB)-min(maxA,maxB); best starts -FLT_MAX, strict >, store edges; return at first >0
- EdgeEdge connector b5517a (0x150): ids edgeA id, edgeB id; +0x118 Line(A v0->v1), +0x134 Line(B v0->v1)
- clones: FFP b53058 (0x28, copies +0x1c,+0x20,+0x24), EEP b54a0c (0x24)
- Block geometry vtable 01c866e8 (Block derives Poly): slot 24 adf0a0 extents, 25 b4fc7e CoM, 26 adf960 inertia, 32 adec1a buildMesh -> mesh cache ae0138 (mesh builder not yet decompiled)
- BallPolyContact vtable ends ...add542 (+0x40 findPolyConnectors); ctor add538
- TODO (still open): b55496/b554aa pair type, BallPolyContact add542, Ball* connectors aa9ea4/aa9c2a/aa99b2, Wedge render 010dc290. Block mesh 00b2f962 and the Wedge mass were recovered in the port session (see poly.rs header).

## Poly ray hit test (Wedge port session, 2026-10-15)
- Primitive hitTest 00b631de: Body PV (a8eabe), CoordinateFrame::toObjectSpace(RbxRay), geometry vt+0x70 (point, normal) in part space, then point = R p + t and normal = R n. Shared by every geometry (the port's `part_space_hit`).
- WedgePoly vt+0x70 = 00b4fbd6 -> 00b340c0(mesh at geometry +0x18, ray, &point, &normal).
- 00b340c0: best = FLT_MAX, count = 0; for each face i (0x24 stride): 00b33f22(ray, face, &p); on hit: d = ((dx*dx + dy*dy) + dz*dz) from ray origin (+4); count++; if best > d (UCOMISS best,d / JBE): point = p, normal = face +0x14 (plane normal). Returns count > 0. Assert "tempDistance >= 0.0f" Mesh.cpp:940.
- 00b33f22: p = RbxRay::intersectionPlane(face plane d1fbd0); p == Vector3::inf -> false; else 00b33fd6(face, p) (behind every neighbouring face plane via otherFace b357cc + d1fc5a). It does NOT project p onto the face plane first (Face::inside 00b553e0 does).
- RbxRay layout: +4 origin, +0x10 direction. Plane: +4 normal, +0x10 distance.
- RbxRay::intersectionPlane 0129b702: rate = (n.x d.x + n.y d.y) + n.z d.z; if 1e-6 > |rate| -> inf (NaN rate falls through); t = (dist - ((n.x o.x + n.y o.y) + n.z o.z)) / rate; t >= 0 (JNC, NaN -> inf) -> o + d t, else inf.
- Wedge drawing 010dc290 (GeometryGenerator.cpp): addBlock's layout. Corners 0..7 as the block (bit 4 +X, 2 +Y, 1 +Z) from (center - size/2). Face table 017f0744, 0x14 rows: f0 7,7,4,5 / f1 3,7,3,7 (skipped) / f2 7,3,5,1 / f3 3,3,1,0 / f4 0,4,1,5 / f5 3,7,0,4. Strip lengths local_2c = [y, z, y, y, z, |(y,z)|]. Normals: +X, +Y, +Z, -X, -Y, unit(R1 z/y - R2) (slope). local_1ec per face 8 floats: step a, step b, u rate a (+0x18), u rate b (+0x1c): f0 (-R1 - k R2, -R1, k, 0), f2 (-R1, -R1), f3 (-R1, -R1 - k R2, 0, k), f4 (R2, R2), f5 (unit(-(y R1 + z R2)) twice). Stud u: a starts width/2 (0 if rate a), b starts 0 (width/2 if rate b); per full strip u_a += 4 rate_a, u_b -= 4 rate_b (4 only when studded); last strip times the fraction. Quad limit 0x2000 over faces 0,2..5; assert faceCounter == quads (line 1648).
- BallPolyContact (BallPolyContact.cpp): ctor 00add538 -> 00add416, vtable 01c86678. It equals PolyPolyContact 01c87858 except slots 0/1 (dtors), 6 (00addf68 -> ae27b2, the same as b5304c) and 16 findPolyConnectors 00add542. Ball = prim +0x10 (native body 0), Poly = +0x14.
- 00add542: p = R_polyᵀ (c_ball - t_poly); r = ball radius (geometry vt+0x20). face = 00add796 (max over faces of n.p - d, strict >, from -FLT_MAX, first on ties). If r < dist: none. dist <= 0 -> BallPlane(face). Else Face::inside b553e0(face, p) -> BallPlane; else 00adda14 (face edges: Line::fromTwoPoints(edge v0, v1), distance |closestPoint - p| (00addfd8), keep if (double)d < best AND 00b3507c) -> if r < d none, else BallEdge; else 00addc50 (nearest edge line, no span test) -> 00addd4e (nearer of getVertex(edge, null, 0/1), strict <) -> if r < d none, else BallVertex.
- 00b3507c: e = v1 - v0; 0 <= (p - v0).e && e.(p - v1) <= 0.
- Line::closestPoint 01254d30: t = ((p-q).dir, x+y+z); q + dir t.
- Connectors body0 = ball, body1 = poly; ids +0x110 = 0, +0x114 = face/edge/vertex id; +0x118 radius. BallPlane 00add8aa (0x134, type 11): +0x11c getVertex(face's first edge, face, 0), +0x128 face normal. BallEdge 00addadc (0x134, type 10): +0x11c getVertex(edge, null, 0), +0x128 d200a0(edge, null). BallVertex 00adde38 (0x128, type 9): +0x11c vertex.
- BallPlane compute aa99b2 (asm checked): n = -(R n_local); p = R p_local + t; s = (p - c).n (x, then - y, - z); position = c + n s; length = sqrt((nx s)^2 + (ny s)^2 + (nz s)^2) - r.
- BallEdge compute aa9c2a: a = R dir; d = (R p + t) - c; d -= a (d.a); position = d + c; length = unitize(d, 1e-6) - r; normal = d.
- BallVertex compute aa9ea4: p = R v + t; position p; normal = p - c; length = unitize(normal, 1e-6) - r.
- Primitive stiffness 00b6312e: 00a92640(size, geometryType (b62c78 = geometry vt+0xc) == 1). Only the Ball (geometry type 1) takes the Ball path; a Wedge (type 4) takes the Block one. Geometry types 10/11 use size (4,4,4).

## SerializerBinary (rbxm port session, 2026-10-16)

- Load: 00baf78a (SerializerV2::load) and 00bafe64 (loadInstances) check
  "<roblox!" and call the binary reader 00e1179e/00e117bb; other files go
  to XML.
- Reader 00e117bb:
  - the header is "<roblox!" plus the 6 bytes at 017d17a9
    (89 ff 0d 0a 1a 0a); version u16 0 ("Unrecognized version %d");
  - each chunk is name, compressed size, size and 4 reserved bytes;
    compressed 0 means stored; a size mismatch after LZ4 is
    "Malformed data";
  - INST: type index, class, format 0 or 1 (1 adds a byte per instance),
    count, referents. An instance is only made if its class has
    ClassDescriptor +0x130 bit 4;
  - PROP: the property is looked up in the class and read only if the
    descriptor has +0x24 bit 8 (canXmlRead), by 00e08b14. Unknown
    properties are skipped;
  - PRNT: format 0, count, children, parents (-1 means the load root);
  - END stops; unknown chunks are skipped.
- Writer 00e10866(stream=ECX, root=EDX, instances, flags):
  - 00e1293e collects archivable (+0x20) instances of creatable classes in
    pre-order (ids) and post-order (PRNT);
  - the class map 00e1382e is keyed by Name sortIndex
    (`**(desc+4)`), so classes come out alphabetically;
  - properties: those of class +0x18..+0x1c that pass vtable+8 false and
    +0x24 bit 0x10 (canXmlWrite), sorted by Name sortIndex (00e13d85
    introsort, then 00e13ece insertion sort);
  - the INST format byte is 1 if the first instance is a Service, with one
    byte per instance (parent == root);
  - the END chunk holds "</roblox>" with flag 2 (stored).
- Callers: 00e11780 passes flags 0 (00340782 and the others pass 0).
  00e1075f (save the children of a root through the filter 00e12ede)
  also passes 0.
- Chunk writer 00e12a36: flag 2 means stored. Otherwise LZ4 through the
  pointer at 01c3de00 (00e07889) or, with flag 1, the one at 01c3de04
  (00e08a02, presumably HC; never used by the save paths).
- LZ4_compress 00e07889: isize <= 0x1000a goes to 00e07430 (u16 table
  [8192], hash >> 19, empty slot = offset 0), else 00e0703b (pointer table
  [4096], hash >> 20). Both:
  - first step attempts 0x44 >> 6;
  - a match needs ref >= ip - 0xffff;
  - catch-up, then MINMATCH 4 and the match count up to iend-5;
  - fill table at ip-2, test ip for an immediate next match.
- Value writer 00e0cb3a, type bytes:
  - 01 string (Content and ProtectedString too, u32 length + bytes);
  - 02 bool (byte), 03 int (zigzag 00e103c9), 04 float (00e104d9);
  - 05 double (8 LE bytes), 06 UDim (floats then ints);
  - 07 UDim2 (sx, sy floats, ox, oy ints), 08 Ray (24 bytes);
  - 09 Faces and 0a Axes (a byte each), 0b BrickColor (00e105ce,
    interleaved raw);
  - 0c Color3, 0d Vector2, 0e Vector3 (float arrays);
  - 0f Vector2int16 (raw), 14 Vector3int16 (6 raw bytes);
  - 10 CFrame (0x11 with flag 4, quaternions): an id byte per value, then
    the 36-byte matrix if the id is 0, then x[], y[], z[];
  - 12 enum (00e105ce), 13 Ref (delta referents 00e106bb, -1 for null);
  - 15 NumberSequence (count + 12-byte keys), 16 ColorSequence (count +
    20-byte keys), 17 NumberRange (8 bytes), 18 Rect2D (4 float arrays);
  - 19 PhysicalProperties (a flag byte, then 5 floats if custom);
  - anything else throws "Unknown property type for property %s".
- CFrame ids: 006b10dc accepts exact 0/±0/±1 entries with one ±1 per row and
  column (no handedness check). 006b11a7 returns
  6*NormalId(col0)+NormalId(col1), and the writer adds 1.
- Descriptor functionality (the argument to PropDescriptor ctors) maps to
  +0x24 as `(f<<1)&0x7c | f&1 | 2` (00562476). So bit 4 means XML read and
  bit 8 means XML write. Examples:
  - 0x1f is the standard set;
  - 0x15 is UI/script only, e.g. BasePart Position, Rotation, Size, Color,
    brickColor and Motor CurrentAngle;
  - 0x0e is "streaming", e.g. Part `size` and JointInstance
    Surface0/Surface1.
- BasePart props 00999b11; JointInstance, Glue, DynamicRotate and Motor
  props 0092aba0:
  - Part0 and Part1 (0x1f; `part1` is an alias with 0x10);
  - Surface0 and Surface1 (0x0e);
  - C0 and C1;
  - F0..F3 and BaseAngle;
  - MaxVelocity and DesiredAngle (0x1f), CurrentAngle (0x15).

## XML serializer (rbxmx port session, 2026-10-16)

- Atoms (00bb6dcb): the tag and attribute names are interned once
  ("roblox", "Item", "Properties", "referent", "External", X..R22, XS/XO/
  YS/YO, faces, axes, origin, direction, min, max, CustomPhysics, Density,
  Friction, Elasticity, FrictionWeight, ElasticityWeight, ...). "S" and "O"
  have no reader or writer: no UDim XML form.
- Root (00bb070a): `roblox` with xmlns:xmime, xmlns:xsi,
  xsi:noNamespaceSchemaLocation and version="4" (plus one more attribute
  when a string argument is non-empty), then `<External>null</External>`
  and `<External>nil</External>`.
- Instance::writeXml (00ac2362): `Item` with class and referent, then
  Properties (00ac227e walks the class's descriptor list; files show it is
  name-sorted), then the children (00ac21b2). A property element
  (0056260e) is the type's tag with a `name` attribute, filled by the
  descriptor's vtable +0x34 writer.
- Referents (00bb5ea8 case 8): the instance's cached id (+0x10) if the
  writer accepts it, otherwise 0067202c: "RBX" + a GUID string with the
  braces and dashes erased. The loader stores the file's referent on the
  instance (00bb0cbe -> 00bb6300).
- Value strings (00bb5ea8): bool "true"/"false", int %d, token %u, float
  %.9g, double %.20g, with INF/-INF/NAN.
- Text output: 00bb8640 writes CDATA when the string has "\n" and no "]]>"
  (with strlen, so a NUL ends it), else 00bb872c escapes per byte. Bytes
  >= 0x7f and control bytes other than \n and \r become `&#N;`.
- Element output: 00bb8884 (open with attributes), 00bba128 (value; a
  Content value is `<url>..</url>` or `<null></null>`), 00bba086 (children
  one tab deeper on their own lines), 00bb8a60 (close).
- Reader: TextXmlParser::parse 00bb92c0, text 00bb8068 (CDATA, then skip to
  '<'), entities 00bb7a56 (lt gt amp quot apos, nbsp as space, &#N; as a
  byte via atoi, &#x is an error, unknown kept).
- Type writers and readers (00ab4000..00ab9400 and elsewhere):
  - CFrame 0098b48a: X Y Z then R00..R22, floats;
  - Vector3 00ab4d08, Vector2 00ab4b4c, Rect2D 00ab84fe (min, max);
  - Vector3int16 00ab7cc6 and Vector2int16 00ab7a00: ints, sign extended;
  - Region3int16 (00ab4322 reads min/max): its helper 00ab4508 reads each
    16-bit field with a 32-bit load, so X prints as x | y << 16 and Z picks
    up two bytes past the value. Not ported (no binary type);
  - UDim2 00ab87d6/00ab88f2: XS float, XO int, YS, YO;
  - Ray 00ab92d8: origin, direction;
  - Faces/Axes 00ab8b74: a `faces`/`axes` int child;
  - Color3 00ab900e: 0xFF000000 | r<<16 | g<<8 | b as %u, the bytes from
    G3D::Color3uint8 (0124ff24): min(255, (int)floorf(c * 256.0f)), no
    lower clamp. The reader 00ab8ef2 takes that uint (bytes / 255.0f,
    0124fc10) or R/G/B children;
  - PhysicalProperties 00ab813c / 00ab7e9e: CustomPhysics, then Density,
    Friction, Elasticity, FrictionWeight, ElasticityWeight. The reader's
    defaults are 1, 0, 0, 1, 1, and it clamps density to [0.01, 100],
    friction to [0, 2], elasticity to [0, 1] and both weights to [0, 100];
  - NumberRange 0087248a, NumberSequence 00c5ce95, ColorSequence 007e79a5:
    ostream doubles, each followed by a space;
  - BinaryString 00ab9d5e -> 00ac088c: base64, '=' padded, a newline after
    every 18 full groups.
- Property types for format conversion (reflection.rs, 2026-10-16). The
  formats lose four types (XML BrickColor is `<int>`; binary string, Content,
  ProtectedString and BinaryString are all 0x01), so the table holds those:
  - Type singletons, built on first use under a cxa guard:
    BrickColor 0223cc88 (00ab9362; Type ctor 00ac047e with name "BrickColor"
    and tag "int"), Content 0223ca08 (00ab5d26; ctor 00abec3c, tag = name),
    ProtectedString 0222c7b0 (006b922e), BinaryString 0223ccd8 (00ab9a3e).
    ContentId subtypes (AnimationId, TextureId, ...) use one-call getters
    that return the Content Type (e.g. 0062d1e0), so XML has one Content
    tag for all of them;
  - property descriptors: every one is built by 00562476 (this, class
    descriptor, Type, name, category, ...), called from 202 typed
    constructors. The ones for the four types: 008ae190 (BrickColor),
    00cb2838, 006e7c38, 00902d08, 00c4eaa8, 00dcb532 (Content), 00cb25ec
    (ProtectedString), 00cfd9d0 (BinaryString). Per-class wrappers
    (e.g. 006e7aae for Animation) call those, and the static initializers
    (e.g. 006e88b4) pass the property name as a literal;
  - ClassDescriptor ctor 0055ea3e (this, base, name); the name is a literal
    or `*PTR_PTR_x` (a pointer to the class's `sName` string). 347 classes;
  - result: 69 (class, property) pairs on 44 classes, and the base chain
    for the 86 classes that have or inherit one. Same-named properties of
    other types exist (IntValue.Value is int, BrickColorValue.Value is
    BrickColor, BinaryStringValue.Value is BinaryString), so the lookup is
    by class. The traversal ran through the ghidra-mcp HTTP API (inline
    Ghidra scripts are disabled in this setup); script and output are
    RobloxStudio/analysis/reflection-types-20261016.{py,txt}, the checks
    (every Type argument resolved; none of the 22 other Type getters reach
    the four) in reflection-types-checks-20261016.txt;
  - with the table, binary -> XML -> binary of characterR15_2.rbxm,
    humanoidAnimateR15_2.rbxm and the demo place is byte-identical.
- Which install files the 0.255 writer made: rewriting all 19 XML models,
  7 come out identical. Rocket.rbxm and characterR15.rbxm print
  `-1.1920929e-007` (three exponent digits: MSVC, a Windows writer). Others
  indent with spaces, omit referents (humanoidStatic, humanoidExtra,
  SlingshotPellet) or escape CR/LF in Source instead of CDATA.

## Decals and Textures (decal port session, 2026-10-16)

- RBX::Decal ctor 007ffb8e: Texture string +0x5c, Specular +0x60 = 0,
  Shiny +0x64 = 20, Transparency +0x68 = 0, LocalTransparencyModifier
  +0x6c = 0; class descriptor 01c98270. FaceInstance ctor 0081d6ae: Face
  +0x58 = 5 (Front). Texture ctor 00800428: StudsPerTileU/V +0x70/+0x74 = 2;
  class 01d6beb0; setter 00800398 ignores values <= 0.
- Property init 00802341 (category Appearance): Texture, Transparency,
  StudsPerTileU/V with flags 0x1f; Specular and Shiny 0x15;
  LocalTransparencyModifier 0x10. Native XML (character3.rbxm) saves a
  Decal as Face, Name, Texture, Transparency only.
- Effective transparency 007ffd24: 1 - (1 - T)(1 - LTM).
- FastCluster::updateGeometry 0116a2a0: the Part draws if its own
  transparency < 1. If the Part has decal children (+0x94 bit 0), every
  child of class 01c98270 (Decal or Texture) with effective transparency
  < 1 is added with flags (partFlags & 1) | 2. The render queue is 4 for an
  opaque decal, 3 for a translucent one, 7 when the Part is translucent.
- createMaterial 010c3460: a decal adds flag 0x10 for a Texture, 0x8 for a
  Decal, then createMaterialForDecal 010c3150 (no texture -> no material).
  createTexturedMaterial 010c09a0: sampler filter = 2 when flags & 0x18,
  address = (flags >> 3) & 1, so Decal clamps and Texture wraps.
- addBlock 010cca80 with a Decal: one quad (quad count 1, no strips). Face
  = Decal +0x58; corners from table 017f01b8 (four u32 per face):
  6 7 4 5 / 3 7 2 6 / 7 3 5 1 / 3 2 1 0 / 5 1 4 0 / 2 6 0 4. Normal = the
  block face normal. UV per vertex (U,0) (0,0) (U,V) (0,V); uv2 = 0; edge
  distances 10.0. Tiling is 1 (getTiling is skipped for decals). The
  vertex color is 010ca980 with the decal: white, alpha
  255 - (int)(min(t,1) * 255), 255 when t <= 0.
- FUN_010e4850(out, decal, size, flag) at 010cd5d1: (1, 1) if flag or not a
  Texture; else per Face: 0 (z, y), 1 (-x, z), 2 (x, y), 3 (-z, y),
  4 (x, -z), 5 (-x, y), each divided by (StudsPerTileU, StudsPerTileV).
- ObjectExporter::processPart 0110f6a0 (OBJ export, not the renderer)
  pushes decal vertices 0.02 along the normal; the renderer does not move
  them (decal pass depth bias instead).

## Materials (material port session, 2026-10-17)

- createDefaultMaterial 010c0e00: reflection flag =
  `(material & ~0x10) == 0x100 && Reflectance > 0.015` (Plastic and
  SmoothPlastic only).
- createRenderMaterial 010bc630 (material, PartMaterial, reflection): cache
  index per material 0..0x14 (0x100 -> 0, 0x110 -> 2 | refl, 0x120 -> 0x15,
  0x200 -> 4 ... 0x600 -> 0x14); any other code returns no material. Name
  strings "Plastic".."Ice"; `name + ("Reflection" if refl)`. Low-quality FS:
  0x100/0x110 -> "DefaultPlasticFS" or "Default" + name + "Reflection" +
  "FS" (so DefaultSmoothPlasticReflectionFS); 0x120 -> "DefaultNeonFS";
  0x370 -> "LowQMaterialWangFS" (or "LowQMaterialWangFallbackFS" when that
  program fails); else "LowQMaterialFS". Slot 0 = studs.dds sampler
  (Linear=1, Wrap). Non-Plastic/Neon: slot 5 =
  "rbxasset://textures/" + lowercase(name) + "/diffuse" + DAT_0224dde0
  (".dds"), sampler (1, 0, 0); slot 8 = wang index (MaterialGenerator +0x944*4)
  with sampler (0, 0, 0) for Cobblestone. LqmatFarTilingFactor (f, f, 1, 1):
  0.25 for 800 (Slate), 0x330, 0x340, 0x430, 0x500, 0x600; 0.5 for 0x410;
  1 otherwise. Neon passes `material == 0x120` to FUN_010bbf40 (blend
  state); Plastic/Neon call FUN_010bc540, the rest set the textures.
- shaders.json: plastic.hlsl and smoothplastic.hlsl have the same non-HQ
  code (`#define PIN_PLASTIC` + default.hlsl), so the two reflection
  programs differ only in name at low quality.
- getTiling 010c3c40: a switch (jump table at 010c3e65), read at runtime
  with follow_switch; values Plastic 1.3, SmoothPlastic/Neon 1, Wood,
  WoodPlanks, Diamondplate, Metal 0.2, Marble, Slate, Granite, Pebble, Rust,
  Aluminum, Sand, Ice 0.1, Concrete, Grass, Fabric 0.15, Brick 0.125,
  Cobblestone 0.0675.
- UV origin (addPartImpl 010d1850): seed = part + (part >> 3) (heap
  address); bits 3..8 and 9..14 index the 64 floats at 017f0218. Brick and
  WoodPlanks use the table values directly; the rest use
  ((T + T) + -1) * ((x + y) + z) * 0.5. UVs are multiplied by the tiling.
- addBlock 010cca80 material UVs: u = origin + width at the first corner,
  origin at the second (width = Z on X faces, X otherwise); v = start, +8 per
  strip, the last strip ends at start + rest (unclamped). Wedge (fc54): the
  strip's v end is 8 + v_off, rate 8 * z / y per strip. addSphere: u starts
  each row at origin + |column * size|, falls by width * fraction; v grows by
  |row * size| * fraction per row, skipping each strip's repeated last row.
- Vertex reflectance byte = (int)(min(r, 1) * 0.8 * 255), 0 when r <= 0.
- Glow factor 2.2: SceneManager ctor immediate at 011c084e (`C7 40 04`),
  used when the PostFX level is below 1 (011c2200).
- EnvMap 010a7310 / renderFace 010a8030: 256x256 cube with mips; each face
  draws Sky::render without stars from the origin with
  perspective(pi/2, 1, 0.5, 3000); the face view matrices come from a static
  initializer and match the wgpu cube face order. EnvMap::update refreshes
  faces over time (not ported).
- BlendState layout {colorMask, srcColor, dstColor, srcAlpha, dstAlpha};
  factor table 017ef0cc: 0 ONE, 1 ZERO, 2 DST_COLOR, 3 SRC_ALPHA,
  4 INV_SRC_ALPHA, 5 DST_ALPHA, 6 INV_DST_ALPHA. Neon is opaque.
- FastCluster passes geometry Options flags 0, so every material keeps the
  stud coordinates in the second UV set.
- squish DXT5: alpha uses integer palettes; DXT5 color always takes the
  4-color branch.

## CustomPhysicalProperties (2026-10-17)

- Property registration at 0099a9e4 (string 018af7d7): getter 00981938
  returns Primitive(+0x8c) +0xc4, the custom record pointer; the setter is
  a virtual member pointer (0x105 = vtable +0x104), not followed.
- 00a43dbe Primitive::getPhysicalProperties: if the record's first byte
  (enabled) is 0, PhysicalProperties(material +0xc8) 00a4331a; else copy
  the 0x18-byte record. 00a43fb4 / 00a44104 (contact params) inline the
  same choice for both primitives.
- Record layout (Lua ctor 0058125c, 5 args density, friction, elasticity,
  frictionWeight, elasticityWeight): +0 enabled, +4 density clamp
  [0.01, 100], +8 elasticity [0, 1], +0xc friction [0, 2], +0x10 friction
  weight [0, 100], +0x14 elasticity weight [0, 100]. The 3-arg form sets
  both weights to 1; the 1-arg form takes a Material (00a4331a) and is not
  custom (enabled 0).
- Floor friction: 00a43e10 (custom) = 00a43e7e(+0x10, +0xc, material
  weight, material friction), the same weighting as 00a43f4c.
- Not found: whether the binary reader (type 0x19) clamps on load.
