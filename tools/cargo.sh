#!/bin/sh
# Keep downloaded dependencies, temporary files and builds inside this project.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
mkdir -p .cache/cargo .cache/tmp
toolchain_bin=$(dirname -- "$(rustup which --toolchain 1.97.1 cargo)")
export PATH="$toolchain_bin:$PATH"
export CARGO_HOME="$project_dir/.cache/cargo"
export CARGO_TARGET_DIR="$project_dir/target"
export TMPDIR="$project_dir/.cache/tmp"
exec "$toolchain_bin/cargo" "$@"
