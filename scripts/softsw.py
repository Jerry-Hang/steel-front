# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """                let sig = self.pt_params.signature();
                // 2026-09-01：sig 已量化 0.5m 位移——只有大于该步幅才重置（指数平均吸收细微移动）
                if sig != self.pt_view_sig.get() {
                    self.pt_view_sig.set(sig);
                    self.pt_reset.set(true);
                    self.pt_frame.set(0);
                }"""
new = """                let sig = self.pt_params.signature();
                // 2026-09-02v2：**不再因移动重置**（消除"白→暗→正常"分段！）；指数窗口自会平滑过渡
                // 仅首帧/光照变化时重置一次
                if sig != self.pt_view_sig.get() {
                    self.pt_view_sig.set(sig);
                    self.pt_reset.set(true);
                    self.pt_frame.set(0);
                }"""
if old in s:
    s = s.replace(old, new, 1)
    print('reset kept (comment fix)')
else:
    print('s1 miss')
# ② **真删 reset 触发**：把 reset 只在"首帧/(光照变化极重大)"——维持但**加速换过渡**：改为**软切换（不清零，设 frame=0 仍不清 acc! shader reset flag false！）**
old2 = """                    self.pt_reset.set(true);
                    self.pt_frame.set(0);"""
new2 = """                    self.pt_reset.set(false); // 2026-09-02：软切换——不清累积（避免白闪段）
                    self.pt_frame.set(0);"""
if old2 in s:
    s = s.replace(old2, new2, 1)
    print('soft switch (no reset-on-move)')
else:
    print('s2 miss')
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
