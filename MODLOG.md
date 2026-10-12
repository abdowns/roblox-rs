# MODLOG

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
