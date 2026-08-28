# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 删顶端 pre-read
old_top = """        if let Some(first) = verts.first() {
            let vp = self.gun_mapped as *const Vertex;
            log::info!(
                "gun: 写入首色 {:?} / 映射@color {:?} @uv {:?}",
                first.color,
                unsafe { (*vp).color },
                unsafe { (*vp).uv }
            );
        }"""
assert old_top in s
s = s.replace(old_top, "", 1)
# 在写入循环之后插 readback（找到转换结束 + 索引上传之前）
anchor = """        // 索引上传（独立映射窗口，用一次性的暂存：直接再 map 索引内存）"""
add = """        if let Some(first) = verts.first() {
            let vp = self.gun_mapped as *const Vertex;
            log::info!(
                "gun-POSTWRITE: 首色 {:?} / 映射@color {:?} @uv {:?}",
                first.color,
                unsafe { (*vp).color },
                unsafe { (*vp).uv }
            );
        }
""" + anchor
s = s.replace(anchor, add, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('post-write readback OK')
