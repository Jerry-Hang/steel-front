# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/game.rs'
s = io.open(p, encoding='utf-8').read()

# 每个 AiStepCtx { ... } 块（测试与主代码都处理：块内已含 obstacles: 行）
# 策略：对所有 "obstacles: ..." 之后的块尾 "};" 插入缺失字段（若该块内没有）
pat = re.compile(r'(\n)([ \t]+)obstacles: [^\n]*\n(\s+)};')
def fix(m, cache={}):
    indent = m.group(2)
    cur = m.group(0)
    if 'squad_wps' in cur:
        return cur
    if 'fallback_targets' in cur:
        return cur
    add = indent + 'squad_wps: &[],\n' + indent + 'spectator: false,\n' + indent + 'fallback_targets: &[],\n'
    return m.group(1) + m.group(2) + 'obstacles:' + m.group(0).split('obstacles:')[1].split('\n')[0] + '\n' + add + m.group(3) + '};'
s2 = pat.sub(fix, s)
io.open(p, 'w', encoding='utf-8', newline='').write(s2)
print('patched ctx blocks')
