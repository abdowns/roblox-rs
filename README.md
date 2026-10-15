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
    fall asleep at rest, including Ball Parts that roll;
  - welded Parts that move as one, glued Parts that break under load,
    hinges, motor hinges, Motors and Motor6Ds.
- **Rendering:** the character, the studded Baseplate, the default sky and
  lighting.

Not there yet:
- the Studio editor UI;
- a general Lua runtime;
- place loading and saving;
- the remaining joint kinds (VelocityMotor, ManualGlue, surface joints);
- Cylinders, Wedges and the other Part shapes (Balls work but render as
  blocks);
- textures and decals beyond the character, shadows and outlines.

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

No license has been chosen for the source yet. Until a `LICENSE` file is added,
all rights are reserved. roblox-rs distributes no Roblox files or assets. Roblox
and Roblox Studio are trademarks of Roblox Corporation, and roblox-rs is not
affiliated with or endorsed by Roblox.
