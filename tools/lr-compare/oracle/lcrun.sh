#!/bin/bash
# lcrun.sh SET OUT [filter]: LightCraft (this checkout's release `oracle_render`, CPU) renders every
# SET/dng/*.dng whose name contains `filter` into SET/OUT/*.f32, J (4) at a time.
HERE="$(cd "$(dirname "$0")" && pwd)"
S=$1; O=$2; f=${3:-.}
BIN="${HERE%/tools/lr-compare/oracle}/target/release/examples/oracle_render"
mkdir -p "$S/$O"
ls "$S/dng" | grep -e "$f" | grep '\.dng$' | xargs -P "${J:-4}" -I{} env LIGHTCRAFT_GPU=0 "$BIN" "$S/$O" "$S/dng/{}" > /dev/null
