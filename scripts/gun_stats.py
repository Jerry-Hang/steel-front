
import re, os, glob
total = 0
files = sorted(glob.glob(r'D:\Rust\steel-front\src\engine\guns\*.rs'))
for f in files:
    if f.endswith('mod.rs'):
        continue
    txt = open(f, encoding='utf-8').read()
    cyl = re.findall(r'cylinder\(([0-9.]+),\s*([0-9.]+),\s*([0-9]+)\)', txt)
    fru = re.findall(r'frustum\(([0-9.]+),\s*([0-9.]+),\s*([0-9.]+),\s*([0-9]+)', txt)
    sph = re.findall(r'sphere\(([0-9]+),\s*([0-9]+)\)', txt)
    tor = re.findall(r'torus_arc\([^)]*,\s*([0-9]+),\s*([0-9]+)\)', txt)
    bbox = re.findall(r'beveled_box\([^)]*,\s*([0-9]+)\)', txt)
    v = sum(int(s)*2+2 for _,_,s in cyl)
    v += sum(int(s)*2+2 for *_,s in fru)
    v += sum(int(a)*int(b) for a,b in sph)
    v += sum(int(a)*int(b) for a,b in tor)
    v += sum(int(s)*20 for s in bbox)
    nparts = len(cyl)+len(fru)+len(sph)+len(tor)+len(bbox)
    total += v
    print(f'{os.path.basename(f):22s} parts={nparts:3d}  approx_verts={v:7d}')
print('TOTAL approx verts:', total)
