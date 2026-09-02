# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """                            let mut d = 0.0f32;
                            for k in 0..3 { let dc = c1[k] - c0[k]; d += dc * dc; let df = f1[k] - f0[k]; d += df * df * 36.0; }
                            d = d.sqrt();
                            self.pt_move_base_cam.set(c1);
                            self.pt_move_base_fwd.set(f1);
                            (d * 20.0).min(1.0)"""
new = """                            let mut d = 0.0f32;
                            for k in 0..3 { let dc = c1[k] - c0[k]; d += dc * dc; let df = f1[k] - f0[k]; d += df * df * 100.0; }
                            d = d.sqrt();
                            self.pt_move_base_cam.set(c1);
                            self.pt_move_base_fwd.set(f1);
                            // 2026-09-01v2：≥0.03m 位移/帧 => 运动满档（跑跳/走路即触发；极低速才回落）
                            if d > 0.03 { 1.0 } else { (d / 0.03).min(1.0) }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('move detection hardened (0.03m threshold => full)')
else:
    print('miss')
