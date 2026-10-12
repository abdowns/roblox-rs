"""Fingerprint supplied Contents without following symlinks or changing any files."""
import hashlib
import json
import os
from pathlib import Path
import sys

PROJECT = Path(__file__).resolve().parents[1]
BUNDLE = PROJECT / "RobloxStudio"
CONTENTS = BUNDLE / "Contents"
ANALYSIS = BUNDLE / "analysis"
SNAPSHOT = ANALYSIS / "bundle-before.json"


def inventory():
    entries = {}
    for directory, dirs, files in os.walk(CONTENTS, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            relative = str(path.relative_to(CONTENTS))
            if path.is_symlink():
                entries[relative] = ["symlink", os.readlink(path)]
            elif path.is_file():
                digest = hashlib.sha256()
                with path.open("rb") as source:
                    for chunk in iter(lambda: source.read(1024 * 1024), b""):
                        digest.update(chunk)
                entries[relative] = ["file", path.stat().st_size, digest.hexdigest()]
    return entries


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("record", "check"):
        raise SystemExit("Usage: python3 tools/verify_bundle.py record|check")
    if not CONTENTS.is_dir() or not CONTENTS.resolve().is_relative_to(BUNDLE.resolve()):
        raise SystemExit("Supplied Contents missing or outside RobloxStudio")
    ANALYSIS.mkdir(exist_ok=True)
    if not ANALYSIS.resolve().is_relative_to(BUNDLE.resolve()):
        raise SystemExit("Analysis directory escapes RobloxStudio")
    current = inventory()
    if sys.argv[1] == "record":
        # An existing baseline must never be silently replaced.
        with SNAPSHOT.open("x") as destination:
            json.dump(current, destination, sort_keys=True, indent=2)
        print(f"bundle_baseline_recorded entries={len(current)} path={SNAPSHOT}")
    else:
        before = json.loads(SNAPSHOT.read_text())
        changed = sorted(key for key in before.keys() | current.keys() if before.get(key) != current.get(key))
        if changed:
            print("bundle_changed " + repr(changed))
            return 1
        print(f"bundle_unchanged entries={len(current)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
