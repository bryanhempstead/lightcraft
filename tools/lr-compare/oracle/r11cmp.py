"""r11cmp.py A.txt B.txt [filter]: two `lccmp.py` outputs side by side (mean ΔE2000 per item, before → after),
then the mean of each setting over the scenes / photos that have it (synthetic and real raws apart)."""
import re, sys
from collections import defaultdict


def load(f):
    d = {}
    for line in open(f):
        m = re.match(r"(\S+__\S+)\s+dE\s+([\d.]+)", line)
        if m:
            d[m.group(1)] = float(m.group(2))
    return d


a, b = load(sys.argv[1]), load(sys.argv[2])
flt = sys.argv[3] if len(sys.argv) > 3 else ""
groups = defaultdict(lambda: [[], []])
for k in sorted(b):
    if flt not in k or k not in a:
        continue
    print(f"{k:30s} {a[k]:6.2f} -> {b[k]:6.2f}")
    scene, setting = k.split("__", 1)
    kind = "syn" if scene in ("grid", "spatial", "land", "room", "freq1") else "real"
    groups[(kind, setting)][0].append(a[k])
    groups[(kind, setting)][1].append(b[k])
print()
for (kind, setting), (x, y) in sorted(groups.items()):
    print(f"{kind:4s} {setting:16s} n={len(x):2d}  {sum(x) / len(x):5.2f} -> {sum(y) / len(y):5.2f}")
