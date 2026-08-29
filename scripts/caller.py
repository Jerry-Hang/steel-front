# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
s = s.replace("        let speed = self.game.player_body.vel.length();", "        let speed = self.game.player_speed();")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('caller fixed')
