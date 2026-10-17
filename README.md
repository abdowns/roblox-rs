# roblox-rs

roblox-rs is an open source rewrite of the 2016 Roblox Studio client for macOS
(version **0.255.0.82159**), written in Rust with [Bevy](https://bevy.org/).
Point it at a copy of that Studio build and it loads the build's own data
(character, meshes, scripts, sky, materials) into a new engine. The goal is
offline behavior and appearance that match the original. Each system is
reverse engineered from the original binary.

## Status

Early and incomplete. Working today, with an automated audit for each item:

- **Play Solo on the Studio Baseplate:** the 2016 R6 character spawns, falls,
  lands, walks, jumps and plays the August 2016 animations.
- **Camera:** the bundled ClassicCamera, PopperCam and
  TransparencyController scripts.
- **Physics:**
  - the original Humanoid state machine and forces;
  - collisions with anchored Parts;
  - unanchored Parts that fall, stack, get pushed, carry the character and
    fall asleep at rest, including Ball Parts that roll and Wedge Parts;
  - welded Parts that move as one, glued Parts that break under load,
    hinges, motor hinges, Motors and Motor6Ds.
- **Rendering:** the character, the studded Baseplate, block, Ball and
  Wedge Parts, Decals and tiled Textures on blocks, all 20 Part materials at
  the original's low quality level (textures, Neon, reflective Plastic), the
  default sky and lighting.
- **Model files:** binary .rbxm and XML .rbxmx read and written byte for
  byte as the original writes them. The demo place and the test places are
  .rbxm files in `places/`.

Not there yet:
- the Studio editor UI;
- a general Lua runtime;
- loading whole places (.rbxl with services, scripts, Models), and the
  rest of the reflection database (the port has only the property types
  that a format conversion needs);
- the remaining joint kinds (VelocityMotor, ManualGlue, surface joints);
- Cylinders, CornerWedges and the other Part shapes;
- decals and textures on Balls, Wedges and meshes (blocks have them),
  shadows, outlines and the high-quality material shaders.

Matching the original is not tested pixel for pixel or step for step. See
[docs/FEATURES.md](docs/FEATURES.md) for the full list and the test for each
item.

## Install, build and run

macOS is the only tested platform (Apple Silicon, macOS 26). You need
[rustup](https://rustup.rs/), Python 3 and Git.

1. Clone the repository and install the pinned toolchain:

   ```bash
   git clone https://github.com/abdowns/roblox-rs.git
   cd roblox-rs
   rustup toolchain install 1.97.1
   ```

2. Download Roblox Studio **0.255.0.82159** (August 5, 2016) from the
   [Roblonium archive](https://archive.roblonium.com/Client/Mac/Studio/2016/8.2016/).
   Extract it, then copy the Studio app bundle into the repository as
   `RobloxStudio/`. When it's in place, this path exists:
   `RobloxStudio/Contents/MacOS/RobloxStudio`.

   ```bash
   cp -R "/path/to/RobloxStudio.app" RobloxStudio
   ```

   roblox-rs only reads this copy. It never modifies it, and `RobloxStudio/`
   is never committed.

3. Optional: record a checksum baseline of the install, and download the 23
   original animation clips. Without the clips the character stays in its
   rest pose.

   ```bash
   python3 tools/verify_bundle.py record
   python3 tools/fetch_animations.py
   ```

4. Build and run Play Solo on the Baseplate:

   ```bash
   sh tools/cargo.sh run --locked -- viewport-baseplate
   ```

   Controls:
   - **WASD:** move.
   - **Space:** jump.
   - **Right-drag or ←/→:** turn the camera.
   - **Mouse wheel or I/O:** zoom.
   - **R:** respawn.
   - **Escape:** quit.

`tools/cargo.sh` keeps the Cargo cache, temporary files and build output
inside the project folder. Runtime data loading makes no network requests.

For development there is an optional dev panel with stats and controls:
`sh tools/cargo.sh run --locked --features dev -- viewport-baseplate` (F1
toggles it). See [RUNNING](docs/RUNNING.md).

## Documentation

- [Running the client and audits](docs/RUNNING.md): every viewport and
  diagnostic, with its expected output.
- [Features](docs/FEATURES.md): what works, what doesn't, and how each part
  is tested.
- [Design](docs/DESIGN.md): how the engine is built, and the reverse
  engineering behind it.
- [Game data and rollback](docs/DATA.md), [tool research](docs/RESEARCH.md)
  and the [change log](MODLOG.md).

This project is written by LLM coding agents.

## Acknowledgements and license

- [Roblonium](https://archive.roblonium.com/) preserves the historical client
  builds this project targets.
- [Ghidra](https://github.com/NationalSecurityAgency/ghidra) and
  [ghidra-mcp](https://github.com/bethington/ghidra-mcp) were used to inspect
  the original binary.
- [Bevy](https://bevy.org/) provides the engine foundation.

### License

The roblox-rs source code is licensed under the
[Apache License, Version 2.0](LICENSE). The license covers only the code in
this repository.

It grants no rights to Roblox Studio or to any Roblox binaries, assets,
meshes, scripts or data. roblox-rs distributes none of those files. It reads
them at runtime from a copy of Studio 0.255.0.82159 that you supply yourself.

### Disclaimer

Roblox and Roblox Studio are trademarks of Roblox Corporation. roblox-rs is an
independent project. It is not affiliated with, sponsored by or endorsed by
Roblox Corporation.
