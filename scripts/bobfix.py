# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
old = """        let bob = (self.anim_clock * 10.0).sin() * 0.012 * (1.0 - self.ads_blend * 0.9);
        anchor.x += bob;"""
new = """        // 持枪摆动（2026-08-28 修复残影/大幅高频晃动）：
        // ① 仅移动时摆动（站立/原地不晃——消除残影来源）；
        // ② 双相摆动：左右 x（7.5Hz 小幅）+ 上下 y（0.8 相位偏移升降）；
        // ③ 开镜（ads_blend→1）与开火后 0.25s 内阻尼到 ~15%（射击/瞄准时轻微）。
        let speed = self.game.player_body.vel.length();
        let moving = speed > 0.6;
        let since_shot = self.anim_clock - self.last_shot_at;
        let fire_damp = if since_shot >= 0.0 && since_shot < 0.25 { 0.15 } else { 1.0 };
        let damp = (1.0 - self.ads_blend * 0.985) * fire_damp;
        let amp = 0.009 * (1.0 - self.ads_blend * 0.9);
        if moving {
            let bob_x = (self.anim_clock * 7.5).sin() * amp;
            let bob_y = (self.anim_clock * 7.5 * 0.5 + 0.8).sin() * amp * 0.9;
            anchor.x += bob_x * damp;
            anchor.y += bob_y * damp;
        }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
    print('bob rewritten')
else:
    print('anchor missing')
