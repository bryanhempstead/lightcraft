#!/bin/bash
# hdr_lcrun.sh SET OUT: LightCraft's side of an HDR oracle set (make_hdr_real.py / make_hdr_syn.py):
# every item rendered by `oracle_render` (HDR edits as their float HDR master, ORACLE_DEPTH=hdr), CPU,
# with the scene-EV dump (tone input EV, saturation) as SET/OUT-dump/<name>.npy; synthetic ramps also
# get their SDR twin (HDR off) in SET/sdrtwin. ORACLE_SIZE=1536 for real raws (Camera Raw's minimum).
cd "$(dirname "$0")"
S=$1; O=$2; BIN=${ORACLE_BIN:-../../../target/release/examples/oracle_render}
mkdir -p $S/$O $S/$O-dump $S/sdrtwin
python3 -c "
import json; m=json.load(open('$S/meta.json'))['items']
for k,v in m.items(): print(k, 'hdr' if v['settings'].get('HDREditMode')=='1' else 'sdr')
" | while read n d; do
  f=$(ls $S/v/$n.* | grep -v '\.xmp$' | head -1)
  if [ "$d" = hdr ] && [[ $n == *_ramp__* ]] && [ ! -f $S/sdrtwin/$n.f32 ]; then
    LIGHTCRAFT_GPU=0 ORACLE_SDR=1 $BIN $S/sdrtwin $f > /dev/null || echo "FAIL twin $n"
  fi
  [ -f $S/$O/$n.f32 ] && continue
  if [ "$d" = hdr ]; then dep=hdr; else dep=; fi
  LIGHTCRAFT_GPU=0 ORACLE_DEPTH=$dep LIGHTCRAFT_LR_DUMP=$PWD/$S/$O-dump/$n.bin $BIN $S/$O $f > /dev/null 2>&1 || echo "FAIL $n"
  [ -f $S/$O-dump/$n.bin ] && python3 -c "
import numpy as np
b=np.fromfile('$S/$O-dump/$n.bin','<f4'); w,h,n=int(b[0]),int(b[1]),int(b[2])
d=b[9:9+w*h*n].reshape(h,w,n); np.save('$S/$O-dump/$n.npy', d[...,[6,13]].astype(np.float32))
" && rm -f $S/$O-dump/$n.bin
done
