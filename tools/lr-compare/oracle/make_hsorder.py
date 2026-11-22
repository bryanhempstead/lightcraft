"""make_hsorder.py SET ACC STEM,…: is Highlights / Shadows applied before Exposure / Contrast / Whites / Blacks?
Split-A raws with H −100 S +65 alone, under Contrast −65, under Exposure +1.5 and under a whole tone recipe
(R02; `hsorder.py` reads the renders)."""
import sys, os, json, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dng
SET, ACC = sys.argv[1], sys.argv[2]
PH = sys.argv[3].split(',')
for d in ('acr', 'v', 'src'):
    os.makedirs(f'{SET}/{d}', exist_ok=True)
G = {'E': 'Exposure2012', 'C': 'Contrast2012', 'H': 'Highlights2012', 'S': 'Shadows2012', 'W': 'Whites2012', 'K': 'Blacks2012'}
SETS = {'base': {}, 'H-100': {'H': '-100'}, 'S+65': {'S': '+65'}, 'HS': {'H': '-100', 'S': '+65'},
        'g': {'E': '-1.41', 'C': '-65', 'W': '-5', 'K': '+36'}, 'gH': {'E': '-1.41', 'C': '-65', 'W': '-5', 'K': '+36', 'H': '-100'},
        'gS': {'E': '-1.41', 'C': '-65', 'W': '-5', 'K': '+36', 'S': '+65'}, 'gHS': {'E': '-1.41', 'C': '-65', 'W': '-5', 'K': '+36', 'H': '-100', 'S': '+65'},
        'C': {'C': '-65'}, 'CHS': {'C': '-65', 'H': '-100', 'S': '+65'}, 'Ep': {'E': '+1.5'}, 'EpHS': {'E': '+1.5', 'H': '-100', 'S': '+65'}}
sample = {os.path.splitext(os.path.basename(p['path']))[0]: p for p in json.load(open(f'{ACC}/sample.json'))}
man, meta = [], {}
for stem in PH:
    p = sample[stem]; ext = os.path.splitext(p['path'])[1]
    src = f'{SET}/src/{stem}{ext}'
    if not os.path.exists(src):
        subprocess.run(['/bin/cp', p['path'], src], check=True)
    for n, st in SETS.items():
        s = dict(dng.BASE); s.update({G[k]: v for k, v in st.items()})
        name = f'{stem}__{n}'; dst = f'{SET}/v/{name}{ext}'
        if not os.path.exists(dst):
            subprocess.run(['/bin/cp', '-c', src, dst], check=True)
        open(f'{SET}/v/{name}.xmp', 'w').write(dng.xmp_packet(s, curves=dng.LINEAR))
        man.append(f'{os.path.abspath(dst)}\t{os.path.abspath(SET)}/acr/{name}.tif\tmin'); meta[name] = {'photo': stem, 'camera': p['model'], 'settings': st}
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'manifest.txt'), 'w').write('\n'.join(man) + '\n')
json.dump({'items': meta}, open(f'{SET}/meta.json', 'w'), indent=0)
print(len(man))
