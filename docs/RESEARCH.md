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
