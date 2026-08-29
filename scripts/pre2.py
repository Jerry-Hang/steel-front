# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# 在 if let Some(renderer) = &mut self.renderer 前插预计算
anchor = "        if let Some(renderer) = &mut self.renderer {"
pre = """        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前——防借用冲突 + 每帧一次）
        let fp_gun_pre = {
            let show = self.inspect_weapon.is_some()
                || (self.game.state() == GameState::Playing
                    && self.camera.mode == CameraMode::FirstPerson);
            if show { self.fp_gun_matrix() } else { glam::Mat4::IDENTITY }
        };
""" 
if anchor in s:
    s = s.replace(anchor, pre + anchor, 1)
    io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
    print('pre computed')
else:
    print('anchor missing')
