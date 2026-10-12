# Rendering evidence — Studio 0.255.0.82159

These addresses refer only to the verified i386 binary. Raw MCP output remains
untracked in `RobloxStudio/analysis/render-*-20261012.json`. The live MCP reaches
studio2016/RobloxStudio with GhidraMCP 7.0.0; no native executable was launched.

| Address | Finding | Boundary |
|---|---|---|
| 010d1850 | Part dispatch retains last direct DataModelMesh child; loaded file mesh takes priority except MeshPart; Block is shape 1. | Other dispatch branches unimplemented. |
| 010d4260 | Separate file, composited humanoid, special head and operation resource paths; SpecialMesh Head selects fonts/head.mesh. | Explicit bundled character supported; general humanoid/flag logic unimplemented. |
| 010cabd0 | Head width=min(size.x*scale.x,size.z*scale.z)*0.8, height=size.y*scale.y*0.8. | Decompiled scale helper. |
| 010c9d90 | Head Y=source.y*width +/- (height-width)*0.625, positive only for source.y>0; offset before world frame; normals rotate without inverse scaling. | i386 floating-point equivalence **not tested**. |
| 010cca80 | Blocks use size/2 and six face quads; indices 0,1,2,2,1,3. Surface subdivisions use 8-stud intervals. U extent=width/2; V extent=clamp(height/8,0,1)/4. | Character UVs implemented; subdivisions/tangents/edges unimplemented. |
| 017f0140 | Six 20-byte face records: four corner indices plus orientation word. | Read from original Mach-O at runtime. |
| 017f045c | Surface rows 0, 0.25, 0.25, 0, 0.5, 0.75; surface 0/10 uses zero UVs. | Table read at runtime; character face dimensions only. |
| 00650222 | 208 constant color registrations in 28422-byte initializer. | Bounded instruction decoder, not emulator. |
| 00659b66 | Registers RGB bytes and alpha 255. | Setter decompilation. |
| 0064e9de / 0064ea82 | Numeric BrickColor lookup and RGB conversion. | Invalid colors fail rather than inventing defaults. |
| 010ca980 | Alpha=255-trunc(effective transparency*255); flags affect reflectance/compositing/color. | Opaque, zero-reflectance character path currently used. |
| 010c3880 | Historical material specular byte intensity and gloss switch. | Full material shader behavior separate. |
| 0108dea0 | Perspective uses vertical FOV and aspect; camera -Z, depth 0..1. | Matrix construction recovered. |
| 01191d80 / 0073a94c | Render near=0.5 and far=5000; cull camera separate. | VR/asymmetric projection/culling unimplemented. |
| 0108d5e0 | View uses transposed rotation and negative rotated translation. | Source-only inverse-frame test passes. |
| 011c5ab0 | Fog reciprocal=0 if start=end, else 1/(end-start). | Shader uses clip W as depth. |
| 01206470 | RBXS signature, count, 96-byte records; name64/digest16/absolute offset/length. | Four packs, 572 records read; manifest dispatch incomplete. |
| 011d7980 | Default Technique: back cull, bias 0, LEQUAL depth/write, blend ONE/ZERO. | Used by opaque probe. |
| 010294b0 / 010296a0 / 01029570 | GL raster/depth/blend enum tables. | Recovered numeric state mapping; stencil/other states incomplete. |
| 010ba770 | Transparent flags select SRC_ALPHA/ONE_MINUS_SRC_ALPHA; alpha ONE_MINUS_DST_ALPHA/ONE; depth writes off; decal bias -16 gives slope -0.5/units -16. | Decal probe implemented; GL/Metal bias precision **not tested**. |
| 0116a2a0 | FastCluster decal flags=(part flags & 1) OR 2; fully transparent decals skipped. | Confirms blending/no depth-write selection. |
| 010c3460 | Instructions OR 0x8 for Decal, 0x10 for Texture subclass. | Decompiled text omitted this register logic. |
| 010c3150 / 010c09a0 | Empty URL skips material; diffuse stage 5, identity atlas, filter 2/address 1 for Decal. | Explicit head/empty torso paths implemented. |
| 010278a0 / 017eeff8 | Filter 2: linear mag, linear-mipmap-linear min; address 1=CLAMP_TO_EDGE. | Probe anisotropy fixed at 1. |
| 0128e602 | Native libpng 1.2.40, row order retained; no gamma or alpha premultiplication calls. | Independent RGBA8 PNG decoder. Other formats unimplemented. |
| 01131ff0 / 01134190 | Full PNG mip chain; pairwise byte average rounds up horizontally then vertically. | Implemented for square power-of-two images. |
| 010bc630 | Low-quality Plastic uses DefaultPlasticFS and studs at stage 0, filter 1/address 0. | Repeat/trilinear; outlines disabled probe. |
| 01131140 | squish color decoding: RGB565 bit replication; floor-weighted thirds when endpoint0>endpoint1, else floor midpoint and transparent black. | DXT1 native software fallback implemented; hardware compressed decoding **not tested**. |
| 011316a0 | DXT decompression copies bounded partial edge blocks. | Original stud mips include dimensions below four. Other DXT formats unimplemented. |

The bundle includes original HLSL and packed GLSL. Default shaders combine
opposing directional terms, ambient, light grid, shadows, specular and fog with
variant-dependent outlines. Plastic has separate low/high-quality paths.
Using modern Bevy materials would not establish equivalence.

`audit-geometry` currently builds Head 517 vertices/846 triangles and five blocks
with 24 vertices/12 triangles each: 637 vertices, 906 triangles. Fully transparent
root Part is skipped. Thirteen source-only tests pass. First palette run failed
at XOR EAX,EAX; instruction evidence explained it and regression/fix passed.
These checks establish mathematical/data contracts, not native execution.
Headless Metal rendering now executes the independently authored low-quality
equations with constant grid/shadow inputs. The initial back-side probe produced
6,831 covered pixels, 292 colors and repeat-identical RGBA SHA-256
`2ce85d030db6547e8106aeb47182f90c74c80c584dd612fa16f2fd348c79d4da`.
Synthetic near red/far green triangles yield center `[128,0,128,255]` with fog,
equal fog bounds yield `[0,0,255,255]`, and BGRA/RGBA targets agree.
The front-side probe retains 6,831 covered pixels with 369 colors before decals;
the bundled face changes 120 pixels and repeated textured output is identical.
Synthetic straight-alpha blending gives `[128,127,0,255]`; a later, farther blue
decal appears, verifying depth writes are disabled.

The runtime loads `PlatformContent/pc/textures/studs.dds`: 128x2048 DXT1,
174,984 bytes, twelve mips; SHA-256
`1e1177269a43a176e22f2df2495f90c741c8f1f2778d6875f18246cc3ba5b817`.
Plastic's original shader multiplies color by twice the sampled red channel,
ignoring atlas alpha. The synthetic repeat probe yields `[32,64,96,255]`.
The actual character atlas pass changes 5,518 pixels; repeat-identical final RGBA
SHA-256 is `95e162d2e69e72c48a74ef2060a8b0f7418717006b9537805d7b459f6f15d015`.
This uses the recovered software DXT1 fallback uploaded as RGBA8; original hardware
compressed pixels are **not tested**. Eighteen source-only tests pass.
Window presentation failed twice and is **not tested** after the format correction.
General decals, surface subdivisions, textures, outlines, dynamic light
grid, shadows, sky, postprocessing and Studio UI remain incomplete. Original
pixel/behavior parity is **not tested**.
