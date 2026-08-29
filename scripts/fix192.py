# -*- coding: utf-8 -*-
import io
p = 'src/engine/ray_tracer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("pub fn box_triangles(b: &PtBox, out_verts: &mut [f32; 72]) {", "pub fn box_triangles(b: &PtBox, out_verts: &mut [f32; 192]) {")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
p2 = 'src/engine/renderer.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("            let mut v = [0.0f32; 72];", "            let mut v = [0.0f32; 192];")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('192 fixed')
