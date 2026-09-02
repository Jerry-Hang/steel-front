# -*- coding: utf-8 -*-
import io
p = 'src/engine/ray_tracer.rs'
s = io.open(p, encoding='utf-8').read()
# ① i consume（宏内 i += 8; → + let _ = i）
s = s.replace("            i += 8;\n        }};", "            i += 8;\n            let _ = i;\n        }};")
# ② PT 常量 allow（按行精插——文本匹配！）
s = s.replace("pub const PT_SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.85];", "#[allow(dead_code)] // 规划特性保留：PT 太阳色常量（备用）\npub const PT_SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.85];")
s = s.replace("pub const PT_AMBIENT_COLOR: [f32; 3] = [0.5, 0.55, 0.6];", "#[allow(dead_code)] // 规划特性保留：PT 环境光颜色（备用）\npub const PT_AMBIENT_COLOR: [f32; 3] = [0.5, 0.55, 0.6];")
s = s.replace("pub const PT_AMBIENT_INTENSITY: f32 = 0.5;", "#[allow(dead_code)] // 规划特性保留：PT 环境光强度（备用）\npub const PT_AMBIENT_INTENSITY: f32 = 0.5;")
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('ray_tracer fixed (symbol match)')
