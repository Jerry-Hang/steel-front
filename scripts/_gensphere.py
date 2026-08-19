import math
SEGS, RINGS = 8, 2
pos = []; uv = []
for j in range(RINGS + 1):
    phi = math.pi * j / RINGS
    sp, cp = math.sin(phi), math.cos(phi)
    for i in range(SEGS + 1):
        theta = 2 * math.pi * i / SEGS
        st, ct = math.sin(theta), math.cos(theta)
        pos.append((sp * ct, cp, sp * st))
        uv.append((i / SEGS, 1.0 - j / RINGS))
tri = []
for j in range(RINGS):
    for i in range(SEGS):
        a = j * (SEGS + 1) + i
        b, c, d = a + 1, a + SEGS + 1, a + SEGS + 2
        tri.append((a, c, b))
        tri.append((b, c, d))
# emit WGSL: 3 per line for pos, 6 per line for uv, 8 per line for tri
def fmt(vals, per, fmtf):
    lines = []
    for k in range(0, len(vals), per):
        chunk = vals[k:k+per]
        lines.append('    ' + ', '.join(fmtf(v) for v in chunk) + ',')
    return '\n'.join(lines)
w = 'const SPHERE_POS: array<vec3<f32>, 27> = array<vec3<f32>, 27>(\n'
w += fmt(pos, 3, lambda v: 'vec3<f32>(%.6f, %.6f, %.6f)' % v) + '\n);\n'
w += 'const SPHERE_UV: array<vec2<f32>, 27> = array<vec2<f32>, 27>(\n'
w += fmt(uv, 6, lambda v: 'vec2<f32>(%.6f, %.6f)' % v) + '\n);\n'
w += 'const SPHERE_TRI: array<vec3<u32>, 32> = array<vec3<u32>, 32>(\n'
w += fmt(tri, 8, lambda v: 'vec3<u32>(%du, %du, %du)' % v) + '\n);\n'
print(w)