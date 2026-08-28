# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
start_marker = "    /// 导入枪模：优先 assets/guns/ak12_baked.glb"
i0 = s.find(start_marker)
assert i0 >= 0
# 找函数结尾：fn load_gun_glb() -> ... { 与匹配的 }
fn_start = s.find("fn load_gun_glb()", i0)
brace = s.find("{", fn_start)
depth = 0
i = brace
while True:
    if s[i] == '{': depth += 1
    elif s[i] == '}':
        depth -= 1
        if depth == 0: break
    i += 1
end = i + 1
new_fn = '''    /// [TEMP-MINIMAL] 3 顶点三角形最小实验：验证 GPU 属性喂养
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let verts = vec![
            crate::engine::meshgen::GVertex { pos: [-0.3, -0.2, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
            crate::engine::meshgen::GVertex { pos: [0.3, -0.2, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
            crate::engine::meshgen::GVertex { pos: [0.0, 0.3, -0.5], normal: [0.0, 0.0, 1.0], uv: [0.0, 0.0], color: [0.15, 0.16, 0.19] },
        ];
        Some((verts, vec![0u32, 1, 2]))
    }'''
s = s[:i0] + new_fn + s[end:]
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('injected clean')
