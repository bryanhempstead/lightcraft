"""Minimal reader for uncompressed 16-bit chunky RGB TIFFs (Photoshop saves), + its ICC profile."""
import struct, numpy as np
def read(path):
    b = open(path, 'rb').read()
    e = '<' if b[:2] == b'II' else '>'
    off = struct.unpack(e + 'I', b[4:8])[0]
    n = struct.unpack(e + 'H', b[off:off + 2])[0]
    tags = {}
    size = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 16: 8}
    for i in range(n):
        t, typ, cnt = struct.unpack(e + 'HHI', b[off + 2 + 12 * i: off + 10 + 12 * i])
        raw = b[off + 10 + 12 * i: off + 14 + 12 * i]
        nbytes = size.get(typ, 1) * cnt
        data = raw[:nbytes] if nbytes <= 4 else b[struct.unpack(e + 'I', raw)[0]:][:nbytes]
        if typ == 3: v = list(struct.unpack(e + 'H' * cnt, data))
        elif typ == 4: v = list(struct.unpack(e + 'I' * cnt, data))
        else: v = data
        tags[t] = v
    w, h = tags[256][0], tags[257][0]
    assert tags.get(259, [1])[0] == 1 and tags[258][0] == 16, (tags.get(259), tags[258])
    spp = tags[277][0]
    offs, cnts = tags[273], tags[279]
    raw = b''.join(b[o:o + c] for o, c in zip(offs, cnts))
    a = np.frombuffer(raw, dtype=e + 'u2')[:w * h * spp].reshape(h, w, spp)[..., :3].astype(np.float64) / 65535
    icc = tags.get(34675, b'')
    desc = ''
    if icc:
        i = icc.find(b'desc')
        if i > 0:
            o = struct.unpack('>I', icc[i + 4:i + 8])[0]
            desc = icc[o + 12:o + 60].split(b'\0')[0].decode('latin1', 'ignore')
    return a, desc
