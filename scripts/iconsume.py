# -*- coding: utf-8 -*-
import io
p = 'src/engine/ray_tracer.rs'
s = io.open(p, encoding='utf-8').read()
# 在 box_triangles 最后一个 v! 调用后加 let _ = i;（找函数结束 "    }" 前）
# 用锚：倒数第 2 个 v! 调用后是 "    }" —— 在文件精确：找 "v!(cx + hx, cy - hy, cz + hz, 1.0, 0.0, 0.0, 0.0, 0.0);"（最后一个！）后插入
last = 'v!(cx + hx, cy - hy, cz + hz, 1.0, 0.0, 0.0, 0.0, 0.0);'
if last in s:
    s = s.replace(last, last + '\n    let _ = i; // 消费最后赋值（消除 unused-assignment 警告）', 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('i consumed')
else:
    print('miss last v')
