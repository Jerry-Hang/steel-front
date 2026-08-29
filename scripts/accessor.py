# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/game.rs', encoding='utf-8').read()
# 在 player_body 字段旁或 Game impl 加访问器——找 add 一个简单方法
anchor = "    pub fn advance_level(&mut self, player: &glam::Vec3) -> bool {"
if anchor in s:
    s = s.replace(anchor, """    /// 玩家水平速度（持枪摆动/动画驱动用）
    pub fn player_speed(&self) -> f32 {
        self.player_body.vel.length()
    }

    pub fn advance_level(&mut self, player: &glam::Vec3) -> bool {""", 1)
    io.open(p if False else 'src/engine/game.rs', 'w', encoding='utf-8', newline='').write(s)
    print('accessor added')
else:
    print('anchor missing')
