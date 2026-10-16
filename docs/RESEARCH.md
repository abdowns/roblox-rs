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
- Primitive stiffness 00b6312e: 00a92640(size, geometryType (b62c78 = geometry vt+0xc) == 1). Only the Ball (geometry type 1) takes the Ball path; a Wedge (type 4) takes the Block one. Geometry types 10/11 use size (4,4,4).

