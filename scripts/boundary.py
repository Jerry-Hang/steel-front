# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
i0 = s.find("    /// [TEMP-MINIMAL]")
assert i0 >= 0
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
new_fn = '''    /// [TEMP-BOUNDARY] 真实 GLB 枪网格 + 强制常量色（区分：枪数据 vs 着色）
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let path = if std::path::Path::new("assets/guns/ak12_baked.glb").exists() {
            "assets/guns/ak12_baked.glb"
        } else {
            "assets/guns/ak12.glb"
        };
        let bytes = std::fs::read(path).ok()?;
        let mesh = crate::engine::assets::parse_glb(&bytes).ok()?;
        let verts: Vec<crate::engine::meshgen::GVertex> = mesh
            .verts
            .iter()
            .map(|v| crate::engine::meshgen::GVertex {
                pos: [v[0], v[1], v[2]],
                normal: [v[3], v[4], v[5]],
                uv: [v[6], v[7]],
                color: [0.15, 0.16, 0.19],
            })
            .collect();
        Some((verts, mesh.indices))
    }'''
s = s[:i0] + new_fn + s[end:]
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('boundary injected')
