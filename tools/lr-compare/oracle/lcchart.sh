#!/bin/bash
# lcchart.sh SET NAME: LightCraft render of one chart DNG -> patch means, image deleted
cd "$(dirname "$0")"; S=$1; n=$2
LIGHTCRAFT_GPU=0 ORACLE_SIZE=1024 ~/crafts/lightcraft/target/release/examples/oracle_render $S/lc $S/dng/$n.dng > /dev/null 2>&1 || echo "FAIL $n"
python3 patches.py $S lc $n
