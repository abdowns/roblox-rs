#!/usr/bin/env python3
"""List the exported symbols of the shipped i386 RobloxStudio executable.

The 0.255 binary keeps an export trie (LC_DYLD_INFO_ONLY) with ~9.5k C++
names, mostly RBX::Graphics plus G3D and a few RBX helpers. Printed addresses
are Ghidra/Mach-O virtual addresses (trie offset + __TEXT vmaddr 0x1000).
Read-only. Usage: python3 tools/macho_exports.py [regex]
"""
import re
import struct
import sys
from pathlib import Path

BINARY = Path(__file__).resolve().parent.parent / "RobloxStudio/Contents/MacOS/RobloxStudio"


def uleb(data, p):
    result = shift = 0
    while True:
        b = data[p]
        p += 1
        result |= (b & 0x7F) << shift
        shift += 7
        if b < 0x80:
            return result, p


def main():
    data = BINARY.read_bytes()
    if struct.unpack_from("<I", data, 0)[0] != 0xFEEDFACE:
        sys.exit("expected a thin i386 Mach-O")
    ncmds = struct.unpack_from("<I", data, 16)[0]
    off, export, text = 28, None, None
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, off)
        if cmd in (0x22, 0x80000022):
            export = struct.unpack_from("<10I", data, off + 8)[8:10]
        if cmd == 1 and data[off + 8 : off + 24].rstrip(b"\0") == b"__TEXT":
            text = struct.unpack_from("<I", data, off + 24)[0]
        off += size
    if export is None or text is None:
        sys.exit("missing export trie or __TEXT")
    base = export[0]
    names, stack = [], [(0, b"")]
    while stack:
        node, prefix = stack.pop()
        p = base + node
        terminal, p = uleb(data, p)
        if terminal:
            _flags, q = uleb(data, p)
            address, _ = uleb(data, q)
            names.append((address + text, prefix.decode("latin1")))
        p += terminal
        children = data[p]
        p += 1
        for _ in range(children):
            end = data.index(b"\0", p)
            label = data[p:end]
            child, p = uleb(data, end + 1)
            stack.append((child, prefix + label))
    pattern = re.compile(sys.argv[1] if len(sys.argv) > 1 else "")
    for address, name in sorted(names):
        if pattern.search(name):
            print(f"{address:#010x} {name}")


if __name__ == "__main__":
    main()
