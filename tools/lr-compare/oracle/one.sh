#!/bin/bash
# one.sh SET FILE: LightCraft render + dump of one oracle file, both kept only at stride 3
cd "$(dirname "$0")"; S=$1; f=$2; n=$(basename "$f"); n=${n%.*}
[ -f $S/dump/$n.npy ] && [ -f $S/lcs/$n.npy ] && exit 0
mkdir -p $S/lc $S/lcs $S/dump
LIGHTCRAFT_GPU=0 ORACLE_SIZE=1536 LIGHTCRAFT_LR_DUMP=$PWD/$S/dump/$n.bin ~/crafts/lightcraft/target/release/examples/oracle_render $S/lc $S/v/$f > /dev/null 2>&1 || echo "FAIL $f"
[ -f $S/dump/$n.bin ] && python3 shrink.py $S/dump/$n.bin $S/dump/$n.npy
[ -f $S/lc/$n.f32 ] && python3 shrinkf32.py $S/lc/$n.f32 $S/lcs/$n.npy
exit 0
