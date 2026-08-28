# -*- coding: utf-8 -*-
import io
p = 'src/engine/game.rs'
s = io.open(p, encoding='utf-8').read()
blk = """            obstacles: &game.map.obstacles,
            squad_wps: &[],"""
# 找所有匹配，取第二个（6198 那个），若其后没有 spectator 则补
import re
idxs = [m.start() for m in re.finditer(re.escape(blk), s)]
print('matches', idxs)
for i in idxs:
    after = s[i:i+200]
    if 'spectator' not in after:
        s = s[:i+len(blk)] + """
            spectator: false,
            fallback_targets: &[],""" + s[i+len(blk):]
        break
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed');
