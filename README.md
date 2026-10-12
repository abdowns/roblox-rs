# Studio 2016 rewrite

An offline Rust/Bevy recreation targeting the supplied Roblox Studio macOS
**0.255.0.82159**. The goal is identical behavior and appearance; this is the
beginning of the project and no equivalence is claimed. Unaffiliated with Roblox.

## What works / what does not

The supplied bundle's Info.plist has been inspected and identifies the target
version. There is no runtime yet. Rendering, simulation, scripting, Studio editing,
and visual/behavioral equivalence are **not tested** and not implemented.

## Data and rollback

The installation is read-only. Game assets and extracted files remain under the
ignored `RobloxStudio/` directory and are never committed. No game-file changes
are made, so no installation backup/restore is needed for this step.

Git tracks source and research findings through a whitelist. Each implementation
step is committed separately; use `git revert <commit>` to undo a source step.

Baseline check: `git status --short` should be empty after committing;
`git check-ignore RobloxStudio/Contents/MacOS/RobloxStudio` should print the path.
