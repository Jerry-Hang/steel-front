# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
i0 = s.find("    /// [TEMP-BOUNDARY]")
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
new_fn = '''    /// [TEMP-BOUNDARY2] 复用原 load 全流程（归一化+光照）+ 末尾强制常量色
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let path = if std::path::Path::new("assets/guns/ak12_baked.glb").exists() {
            "assets/guns/ak12_baked.glb"
        } else {
            "assets/guns/ak12.glb"
        };
        let bytes = std::fs::read(path).ok()?;
        let mesh = crate::engine::assets::parse_glb(&bytes).ok()?;
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for v in &mesh.verts {
            for k in 0..3 {
                mn[k] = mn[k].min(v[k]);
                mx[k] = mx[k].max(v[k]);
            }
        }
        let ext = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
        let long = ext[0].max(ext[1]).max(ext[2]);
        let scale = 1.35 / long.max(1e-4);
        let center = [(mn[0] + mx[0]) * 0.5, (mn[1] + mx[1]) * 0.5, (mn[2] + mx[2]) * 0.5];
        let light = glam::Vec3::new(-0.45, 0.8, -0.3).normalize();
        let verts: Vec<crate::engine::meshgen::GVertex> = mesh
            .verts
            .iter()
            .map(|v| {
                let p = glam::Vec3::new(v[0] - center[0], v[1] - center[1], v[2] - center[2]) * scale;
                let n = glam::Vec3::from_slice(&v[3..6]).normalize_or_zero();
                let ndl = n.dot(light).max(0.0);
                crate::engine::meshgen::GVertex {
                    pos: [p.x, p.y, p.z],
                    normal: [n.x, n.y, n.z],
                    uv: [v[6], v[7]],
                    color: [0.15, 0.16, 0.19],
                }
            })
            .collect();
        Some((verts, mesh.indices))
    }'''
s = s[:i0] + new_fn + s[end:]
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('boundary2 injected')
