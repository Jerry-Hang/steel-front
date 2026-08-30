# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('println!("  {} : {:.1} GFMA/s（FMA指令率） ≈ {:.2} TFLOPS(2x口径) ({})", name_, v / 2.0, v / 1000.0, explain);', 'println!("  {} : {:.1} GFMA/s ≈ {:.2} TFLOPS(2x) [编译优化敏感·仅供参考] ({})", name_, v / 2.0, v / 2000.0, explain);')
# fp_test 调用：只做一轮（round==0 已取）
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('label fixed')
