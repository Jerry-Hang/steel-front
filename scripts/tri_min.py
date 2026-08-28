# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
anchor = """    /// 导入枪模：优先 assets/guns/ak12_baked.glb（Blender 顶点色烘焙版），"""
new = """    /// [TEMP-MINIMAL] 3 顶点三角形最小实验（验证 GPU 属性喂养）
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let verts = vec![
            crate::engine::meshgen::GVertex { pos: [-0.3, -0.2, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
            crate::engine::meshgen::GVertex { pos: [0.3, -0.2, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
            crate::engine::meshgen::GVertex { pos: [0.0, 0.3, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
        ];
        Some((verts, vec![0u32, 1, 2]))
    }

    /// 导入枪模：优先 assets/guns/ak12_baked.glb（Blender 顶点色烘焙版），"""
if anchor in s:
    s = s.replace(anchor, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('minimal triangle injected')
else:
    print('anchor missing')
