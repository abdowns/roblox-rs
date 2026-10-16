# AGENTS.md

## Project
This is a 1:1 rewrite of the 2016 Roblox Studio MacOS client in Rust and built on bevy game engine (minimally relying on it). Reverse engineer Roblox Studio macOS 0.255.0.82159 (Provided under RobloxStudio/) and rewrite its offline core client in Rust/Bevy, targeting indistinguishable behavior and appearance. 

## Hard rules, never break these

1. **Never let game assets or extracted game files leave the RobloxStudio directory.** They stay in that directory, untracked. If you need to read game data, read it from the install path at runtime or extract it to a gitignored folder.
2. **`.gitignore` is a whitelist.** It ignores everything and includes only
   source files. Do not switch it to a normal ignore list.
3. **Use git to track and commit changes from the beginning.**
4. **Do not touch anything outside this project folder** unless I explicitly name
   the path. This includes my game installs: read them, never write to them.
6. **Never put credentials in the repo or in any file you can read:** no API keys,
   no tokens, no passwords.

## How to work

- **Keep the design notes as you go.** Once the shape of something is clear, write it into
  `docs/DESIGN.md`. Do not stop and produce a plan before starting: get one thing working,
  then write down how it works.
- **One thing at a time.** Do not bundle unrelated changes. I want to be able to
  revert a single step.
- **Always keep the public function signatures stable.**. Keep track of your findings from reverse engineering, decompilation, and Ghidra
- **Log, don't look.** You cannot see the game. Instrument instead: write
  positions, counts, timings, and state transitions to a log file so you can
  verify from the numbers. Do not try to visually inspect the game.
- **Tell me how to test it.** After every change, say the exact command to run and
  what I should see. "Done" without a test procedure is not done.
- **Show it in the demo.** Whenever a change is plausibly demonstrable in the
  viewport baseplate demo (`viewport-baseplate`, `Workspace::loose_parts_demo`),
  update the demo to show it off, in the same change or the one right after.
- **Ask before large refactors.** If you think the architecture is wrong, say so
  and explain, then wait for me.

## Honesty

- If something is not tested, write **"not tested"**. Never imply you verified
  something you didn't.
- If you are not sure, say you are not sure. A confident wrong answer costs me
  hours.
- If you hit something you cannot solve after two real attempts, stop and write
  up `STATUS.md` (see `STATUS-handoff.md`) instead of trying variations at random.
- Record failures alongside successes. A dead end I can see is worth more than a
  dead end I have to watch you repeat.

## Keep these files updated

- `MODLOG.md`: add an entry after every change. Template in
  `MODLOG-template.md`.
- `docs/DESIGN.md`: how the project works, in plain language. Update when the
  architecture changes, not on every commit.
- `README.md`: short and public facing. A minimal status, install/build/run and
  acknowledgements. Test before you claim something works.
- `docs/FEATURES.md`: the detailed "what works / what doesn't work" list, with
  the test command and expected output for each entry.
- `docs/RUNNING.md`: the viewports and diagnostics and their expected output.

## Environment

- OS: MacOS Tahoe 26.5.1, Apple M3
- Language: Rust
- Agent: Codex
