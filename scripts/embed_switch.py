# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """        let vs_spirv = load_spirv("assets/triangle.vert.spv")?;
        log::info!("shader: triangle.vert.spv 字节数 {}", vs_spirv.len());
        let fs_spirv = load_spirv("assets/triangle.frag.spv")?;"""
new = """        // 2026-08-28 终极修正：使用 build.rs 内嵌 SPIR-V（OUT_DIR/shaders.rs 常量），
        // 不再加载外置 assets/triangle.*.spv（两者曾长期不同步：外置为旧版，color 通道被 UV 顶替）
        let vs_spirv = crate::shaders::VS_SPIRV.to_vec();
        let fs_spirv = crate::shaders::FS_SPIRV.to_vec();"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('embedded switch installed')
else:
    print('anchor missing')
