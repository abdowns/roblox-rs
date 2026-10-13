# Game data, verification and rollback

The installation is read-only. Game assets and extracted files remain under the
ignored `RobloxStudio/` directory and are never committed. No game-file changes
are made, so no installation backup/restore is needed for this step. Ghidra's
analysis copies, generated traces and logs remain in `RobloxStudio/analysis/`;
the original `Contents/` is never modified.

Git tracks source and research findings through a whitelist. Each implementation
step is committed separately; use `git revert <commit>` to undo a source step.

Baseline check: `git status --short` should be empty after committing;
`git check-ignore RobloxStudio/Contents/MacOS/RobloxStudio` should print the path.

```sh
python3 tools/verify_bundle.py check
```

Expected here: `bundle_unchanged entries=2606`. The initial manifest is kept at
`RobloxStudio/analysis/bundle-before.json`. On a fresh copy, run
`python3 tools/verify_bundle.py record` once before analysis; it refuses to overwrite
an existing baseline. This manifest verifies content, not ownership or authenticity.

[Design](DESIGN.md) · [Current tool research](RESEARCH.md) ·
[Change log](../MODLOG.md)
