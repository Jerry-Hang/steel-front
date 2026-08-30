# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
# 显示双口径
s = s.replace('println!("  {} : {:.2} {} ({})", name_, v, unit, explain);', 'println!("  {} : {:.1} GFMA/s = {:.2} TFLOPS (2x) ({})", name_, v / 2.0, v / 1000.0, explain);')
# ops 计算已有；结果 now = GFLOPS (2x) —— 用 v 保留 GFLOPS；显示改如上
# 计时改 fence？—— 保留（wait_idle 已足够精确）；增加休眠后 UI 说明
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('dual unit display')
# rt 显示也改双口径
s2 = io.open(p, encoding='utf-8').read()
s2 = s2.replace('println!("  RT  : {:.1} Mrays/s (1M射线 x 200次迭代)", rt_val);', 'println!("  RT  : {:.1} Mrays/s ({:.1} G rays/s, 纯compute 无验证层)", rt_val, rt_val / 1000.0);')
io.open(p, 'w', encoding='utf-8', newline='').write(s2)
print('rt display dual')
