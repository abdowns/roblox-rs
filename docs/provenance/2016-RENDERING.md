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
| 010cca80 | Blocks use size/2 and six face quads; indices 0,1,2,2,1,3. Surface subdivisions use 8-stud intervals. | Character sizes need one quad/face. Surface UVs/tangents/edges unimplemented. |
| 017f0140 | Six 20-byte face records: four corner indices plus orientation word. | Read from original Mach-O at runtime. |
| 00650222 | 208 constant color registrations in 28422-byte initializer. | Bounded instruction decoder, not emulator. |
| 00659b66 | Registers RGB bytes and alpha 255. | Setter decompilation. |
| 0064e9de / 0064ea82 | Numeric BrickColor lookup and RGB conversion. | Invalid colors fail rather than inventing defaults. |
| 010ca980 | Alpha=255-trunc(effective transparency*255); flags affect reflectance/compositing/color. | Opaque, zero-reflectance character path currently used. |
| 010c3880 | Historical material specular byte intensity and gloss switch. | Full material shader behavior separate. |
| 0108dea0 | Perspective uses vertical FOV and aspect; camera -Z, depth 0..1. | Matrix construction recovered. |
| 01191d80 / 0073a94c | Render near=0.5 and far=5000; cull camera separate. | VR/asymmetric projection/culling unimplemented. |
| 0108d5e0 | View uses transposed rotation and negative rotated translation. | Source-only inverse-frame test passes. |
| 011c5ab0 | Fog reciprocal=0 if start=end, else 1/(end-start). | Shader uses clip W as depth. |
| 01206470 | ShaderManager loads packed shaders and shaders.json variant manifest. | Pack loader not yet implemented. |

The bundle includes original HLSL and packed GLSL. Default shaders combine
opposing directional terms, ambient, light grid, shadows, specular and fog with
variant-dependent outlines. Plastic has separate low/high-quality paths.
Using modern Bevy materials would not establish equivalence.

`audit-geometry` currently builds Head 517 vertices/846 triangles and five blocks
with 24 vertices/12 triangles each: 637 vertices, 906 triangles. Fully transparent
root Part is skipped. Thirteen source-only tests pass. First palette run failed
at XOR EAX,EAX; instruction evidence explained it and regression/fix passed.
These checks establish mathematical/data contracts, not native execution.
Rendering, decals, surface UVs, textures, light grid, shadows, sky, postprocessing
and Studio UI are unimplemented at this checkpoint. Full parity is **not tested**.
