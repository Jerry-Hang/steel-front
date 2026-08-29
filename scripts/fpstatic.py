# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# ③ first_person_gun_mesh 的 GLB fp-path：直接返回缓存（无每帧重变换）
old_fp = """            // 第一人称：与程序化枪共用 fp_gun_matrix（世界空间 + 每帧跟随相机）
            let m = self.fp_gun_matrix();
            let moved: Vec<crate::engine::meshgen::GVertex> = verts
                .iter()
                .map(|v| crate::engine::meshgen::GVertex {
                    pos: {
                        let p = m.transform_point3(glam::Vec3::from(v.pos));
                        [p.x, p.y, p.z]
                    },
                    normal: {
                        let n = m.transform_vector3(glam::Vec3::from(v.normal));
                        [n.x, n.y, n.z]
                    },
                    ..*v
                })
                .collect();
            return (moved, indices);
        }"""
new_fp = """            // 第一人称：顶点已在加载时静态化到「视空间基座」，每帧仅由实例矩阵驱动
            // （2026-08-28 残影修复：消除每帧 3MB CPU 重变换）
            return (verts, indices);
        }"""
if old_fp in s:
    s = s.replace(old_fp, new_fp, 1)
    print('fp static')
else:
    print('fp anchor missing')
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
