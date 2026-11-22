#!/bin/bash
# lcrun.sh SET OUT [filter]: LightCraft (this checkout's release `oracle_render`, CPU unless LIGHTCRAFT_GPU is
# set) renders every SET/dng/*.dng whose name contains `filter` into SET/OUT/*.f32, J (4) at a time, and every
# real-raw clone in SET/v (with its sidecar) at Camera Raw's minimum size (1536 px long edge).
HERE="$(cd "$(dirname "$0")" && pwd)"
S=$1; O=$2; f=${3:-.}
BIN="${HERE%/tools/lr-compare/oracle}/target/release/examples/oracle_render"
GPU=${LIGHTCRAFT_GPU:-0}
mkdir -p "$S/$O"
[ -d "$S/dng" ] && ls "$S/dng" | grep -e "$f" | grep '\.dng$' | xargs -P "${J:-4}" -I{} env LIGHTCRAFT_GPU=$GPU "$BIN" "$S/$O" "$S/dng/{}" > /dev/null
[ -d "$S/v" ] && ls "$S/v" | grep -e "$f" | grep -v '\.xmp$' | xargs -P "${J:-4}" -I{} env LIGHTCRAFT_GPU=$GPU ORACLE_SIZE=1536 "$BIN" "$S/$O" "$S/v/{}" > /dev/null
exit 0
