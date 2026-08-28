# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("output.color = color * 3.0; // 仪器：attr真值×3", "output.color = color * inst.tint.rgb;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('vs restored' if 'color * inst.tint.rgb' in s else 'FAIL-vs')
p2 = 'src/engine/renderer.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("            mesh_enabled: false, // 临时分离测试", "            mesh_enabled: mesh_shader_available,")
# 双写恢复：uv 回真 UV（颜色保持 v.color + uv: v.uv）
s2 = s2.replace("                *vptr.add(i) = Vertex {\n                    pos: v.pos,\n                    color: v.color,\n                    uv: [v.color[0], v.color[1]],\n                };", "                *vptr.add(i) = Vertex {\n                    pos: v.pos,\n                    color: v.color,\n                    uv: v.uv,\n                };")
# 读回探针删除
s2 = s2.replace("""        if let Some(first) = verts.first() {
            let vp = self.gun_mapped as *const Vertex;
            log::info!(
                "gun-POSTWRITE: 首色 {:?} / 映射@color {:?} @uv {:?}",
                first.color,
                unsafe { (*vp).color },
                unsafe { (*vp).uv }
            );
        }
""", "")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('renderer restored, dual removed' if 'mesh_shader_available,' in s2 else 'FAIL-renderer')
