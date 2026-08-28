# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
old = """    #[test]
    fn glb_parses_real_ak12() {"""
new = """    #[test]
    fn glb_baked_color_and_index() {
        let p = "assets/guns/ak12_baked.glb";
        if std::path::Path::new(p).exists() {
            let m = parse_glb(&std::fs::read(p).unwrap()).unwrap();
            assert!(m.verts[0][8] < 0.5, "首网格顶点色应深色, 实际 {:?}", &m.verts[0][8..11]);
            let max_idx = m.indices.iter().take(1000).copied().max().unwrap_or(0);
            assert!(max_idx < m.verts.len() as u32, "首索引越界 {}", max_idx);
        }
    }

    #[test]
    fn glb_parses_real_ak12() {"""
if old not in s:
    print('missing anchor')
else:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('test inserted')
