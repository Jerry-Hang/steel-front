# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('match fp_test(&device, &queue, phys, &inst, spv, 1 << 20) {', 'match fp_test(&device, &queue, phys, &inst, spv, 1 << 18) {')
s = s.replace('let iters = 8u32; // 重复计数取均', 'let iters = 32u32; // 重复计数取均')
s = s.replace('for _ in 0..3 {', 'for _ in 0..4 {')
# ops 公式 16384→4096（n=4096+7）
s = s.replace('let ops = items as f64 * 16384.0 * 2.0 * iters as f64; // FMA=2 ops', 'let ops = items as f64 * 4096.0 * 2.0 * iters as f64; // FMA=2 ops')
# 显示：FMA率为主 + TFLOPS 括号说明口径
s = s.replace('println!("  {} : {:.1} GFMA/s = {:.2} TFLOPS (2x) ({})", name_, v / 2.0, v / 1000.0, explain);', 'println!("  {} : {:.1} GFMA/s（FMA指令率） ≈ {:.2} TFLOPS(2x口径) ({})", name_, v / 2.0, v / 1000.0, explain);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('final display')
