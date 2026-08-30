# -*- coding: utf-8 -*-
import io
for name in ['fp32', 'fp16', 'fp8', 'fp4']:
    p = r'shaders\\' + name + '.comp'
    s = io.open(p, encoding='utf-8').read()
    s = s.replace('uint n = 4096u + (o[g] & 7u);', 'uint n = 16384u + (o[g] & 7u);')
    io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('n=16384')
# main.rs: items 1<<20, iters 8, 轮数 3, 显示 GFMA/s 正确（v=GFLOPS, FMA/s=v/2000 T?）
p2 = 'src/main.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace('match fp_test(&device, &queue, phys, &inst, spv, 1 << 18) {', 'match fp_test(&device, &queue, phys, &inst, spv, 1 << 20) {')
s2 = s2.replace('let iters = 32u32; // 重复计数取均', 'let iters = 8u32; // 重复计数取均')
s2 = s2.replace('for _ in 0..4 {', 'for _ in 0..3 {')
# ops 公式用 loop 16384（占位——改公式）
s2 = s2.replace('let ops = items as f64 * 4096.0 * 2.0 * iters as f64; // FMA=2 ops', 'let ops = items as f64 * 16384.0 * 2.0 * iters as f64; // FMA=2 ops')
# 显示修正：v=GFLOPS。GFMA/s = v/2（显示格式对）
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('params heavy')
