# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
old = """            if show_gun {
                renderer.set_first_person_gun_mesh(&gun_mesh.0, &gun_mesh.1);
            } else {
                renderer.set_first_person_gun_mesh(&[], &[]);
            }"""
new = """            if show_gun {
                renderer.set_first_person_gun_mesh(&gun_mesh.0, &gun_mesh.1);
                // 2026-08-28：枪顶点已静态化，bob/后坐/相机跟随全走实例矩阵（GPU 端）
                renderer.set_first_person_gun_model(self.fp_gun_matrix());
            } else {
                renderer.set_first_person_gun_mesh(&[], &[]);
                renderer.set_first_person_gun_model(glam::Mat4::IDENTITY);
            }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
    print('setter called per-frame')
else:
    print('call anchor missing')
