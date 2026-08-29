# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/game.rs', encoding='utf-8').read()
s = s.replace("        self.player_body.vel.length()", "        (self.player_body.vel.x * self.player_body.vel.x + self.player_body.vel.z * self.player_body.vel.z).sqrt()")
io.open('src/engine/game.rs', 'w', encoding='utf-8', newline='').write(s)
print('fixed')
