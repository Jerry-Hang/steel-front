# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/ai_command.rs'
s = io.open(p, encoding='utf-8').read()

# 1) 删 SquadOrder 结构（65-68 附近，含前面的注释行）
m = re.search(r'///[^\n]*班命令[^\n]*\n(?:#\[[^\n]*\]\n)*pub struct SquadOrder \{[^}]*\}\n', s)
if m:
    s = s[:m.start()] + s[m.end():]
    print('deleted SquadOrder')

# 2) 删 squad_of（210-212）
m2 = re.search(r'    pub fn squad_of\(&self, npc_id: usize\) -> Option<usize> \{\n        self\.soldier_slot\.get\(&npc_id\)\.map\(\|\(s, _\)\| \*s\)\n    \}\n\n', s)
if m2:
    s = s[:m2.start()] + s[m2.end():]
    print('deleted squad_of')

# 3) 删 strength（224-228 附近）
m3 = re.search(r'    pub fn strength\(&self, npcs: &\[crate::engine::game::Npc\], side: Team\) -> f32 \{[^}]*\}\n\n', s)
if m3:
    s = s[:m3.start()] + s[m3.end():]
    print('deleted strength')

# 4) squad_waypoint 去 npc_pos 参数 + 体内 let _ = npc_pos;
s = s.replace('pub fn squad_waypoint(&self, npc_id: usize, npc_pos: [f32; 3]) -> Option<[f32; 2]> {',
              'pub fn squad_waypoint(&self, npc_id: usize) -> Option<[f32; 2]> {')
s = s.replace('        let _ = npc_pos;\n', '')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('waypoint sig fixed')
