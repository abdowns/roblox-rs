# MODLOG

## 2026-10-12 Execute static rendering on Metal

**Changed:** Direct Metal renderer with Bevy extraction, independently authored
low-quality WGSL, native shader-pack reader, numeric readback and experimental
viewport commands. Source whitelist now includes WGSL.
**Why:** Execute recovered geometry, projection, lighting/fog and depth contracts.
**Tested how:** 14 source-only tests; actual M3 Metal readback; repeat comparison;
synthetic depth/fog and RGBA/BGRA checks; Contents fingerprint; formatting/Clippy.
**Result:** 6,831 covered pixels/292 colors; repeat-identical output; four packs,
572 records. Depth/fog/format checks pass; 2,606 original entries unchanged.
**Still broken / not tested:** Initial wgpu API type errors fixed. Window smoke
failed twice (hidden surface stalled, then warm-up RGBA/BGRA mismatch). Stopped
GUI retries and recorded STATUS. Offscreen format correction passes; window after
that fix is **not tested**. Texture/material completeness and native parity are
**not tested**.
**Next:** Recover decal resource/sampler/geometry contracts from native calls.

## 2026-10-12 Recover static geometry and camera

**Changed:** Native-data palette/topology reader, character head/block geometry,
material specular quantization, camera matrices, audit-geometry CLI and evidence.
**Why:** Establish the 2016 geometry/projection contract before GPU rendering.
**Tested how:** Live MCP decompilation/instructions, 13 source-only tests, actual
character diagnostic, formatting and Clippy with warnings denied.
**Result:** Live MCP restored through instance discovery. 208 native colors; six
draws, 637 vertices, 906 triangles.
**Still broken / not tested:** First palette run omitted XOR-zero; native
instructions identified the cause and regression/fix passed. First Clippy run
requested rfind; corrected. Surface UVs, decals and rendering remain absent.
Original execution and full parity are **not tested**.
**Next:** Connect static geometry to a bounded GPU probe.

## 2026-10-12 Verified 2016 mesh and character import

**Changed:** Added pinned Rust/Bevy package and lockfile; confined bundle access,
independent mesh v2 reader, explicit XML Part/instance subset, headless diagnostic,
local Ghidra utilities and Contents fingerprint check. Added design, native
provenance, current tool research and STATUS; updated README.
**Why:** Prove a data boundary from the exact 2016 binary before rendering or reuse
of 2015 behavior.
**Tested how:** Eight synthetic tests; original-bundle diagnostic over 120 Bevy
updates; native decompilation/instruction tracing; whole Contents hash comparison;
formatting and Clippy with warnings denied.
**Result:** 37 meshes / 10,992 vertices / 12,896 triangles; 26 instances, including
14 Attachments and seven Parts (six visible); `static_import_pass updates=120`.
All 2,606 original files/symlinks unchanged. Matching 2015 mesh payloads are identical,
but the old reader's acceptance differs; no source was transplanted. Logs remain
inside RobloxStudio/analysis.
**Still broken / not tested:** MCP project state failed twice; headless fallback
succeeded. Broad analysis timed out after 180 seconds, saving 48,563 functions.
First compile type/map errors were fixed and later checks passed. Rendering,
simulation, scripting, editing/saving and full native parity are not implemented /
**not tested**. Remote IW4L manifest/commit lookups failed.
**Next:** Recover exact 2016 character geometry/resource selection before rendering.

## 2026-10-12 Source baseline

**Changed:** Created the source whitelist, README and Git baseline; preserved the
provided guidance and decompilation map.
**Why:** Establish reversible source history before implementing the rewrite.
**Tested how:** Inspected Info.plist; checked Git's tracked list and ignore rule.
**Result:** Bundle version is 0.255.0.82159; installation excluded from Git.
**Still broken / not tested:** No Rust runtime; native client execution and parity
are not tested. Ghidra MCP initially reports no open programs.
**Next:** Import the exact executable into a local analysis copy and implement
the smallest data-loading boundary supported by native evidence.
