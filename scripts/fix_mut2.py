# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("            let mut npc_visuals: Vec<engine::renderer::NpcVisual> = if net_mode {", "            let npc_visuals: Vec<engine::renderer::NpcVisual> = if net_mode {")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('mut removed')
