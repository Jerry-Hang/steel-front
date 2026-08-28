# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("            let npc_visuals: Vec<engine::renderer::NpcVisual> = self", "            let mut npc_visuals: Vec<engine::renderer::NpcVisual> = self", 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('mut ok')
