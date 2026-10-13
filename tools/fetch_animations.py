#!/usr/bin/env python3
"""Download the default R6 animation clips referenced by the shipped Animate
script into RobloxStudio/assets/ (untracked, beside the read-only install).

The ids are read from Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe2.rbxm
(the script the 0.255 loader selects for R6). Each asset's last-updated date
must predate the build's content files (2016-08-04), so the current version is
the one the August 2016 client would have downloaded. Nothing in Contents/ is
written. Run from the project root: python3 tools/fetch_animations.py
"""
import gzip
import hashlib
import json
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "RobloxStudio"
SCRIPT = ROOT / "Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe2.rbxm"
OUT = ROOT / "assets"
BUILD_DATE = "2016-08-04"


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": "roblox-rs-fetch"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read()


def main():
    source = SCRIPT.read_text(encoding="utf-8", errors="replace")
    ids = sorted(set(re.findall(r"asset/\?id=(\d+)", source)), key=int)
    if not ids:
        sys.exit("no animation ids found in the Animate script")
    OUT.mkdir(exist_ok=True)
    manifest = {}
    for asset in ids:
        details = json.loads(get(f"https://economy.roblox.com/v2/assets/{asset}/details"))
        updated = details.get("Updated", "")
        if not updated or updated[:10] >= BUILD_DATE:
            sys.exit(f"{asset}: updated {updated!r} is not before {BUILD_DATE}; version choice unknown")
        data = get(f"https://assetdelivery.roblox.com/v1/asset/?id={asset}")
        if data[:2] == b"\x1f\x8b":
            data = gzip.decompress(data)
        if not data.startswith(b"<roblox"):
            sys.exit(f"{asset}: unexpected payload")
        (OUT / asset).write_bytes(data)
        manifest[asset] = {
            "name": details.get("Name"),
            "creator": details.get("Creator", {}).get("Name"),
            "created": details.get("Created"),
            "updated": updated,
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }
        print(asset, manifest[asset]["name"], updated, len(data), manifest[asset]["sha256"][:16])
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(f"fetched {len(ids)} assets into {OUT}")


if __name__ == "__main__":
    main()
