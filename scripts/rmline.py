# -*- coding: utf-8 -*-
import io
lines = io.open('build.rs', encoding='utf-8').read().split('\n')
out = []
i = 0
skip = False
while i < len(lines):
    if 'RQ_PROBE' in lines[i]:
        # 跳过到 r#" 的结束 "#
        j = i
        while j < len(lines) and not lines[j].strip().endswith('"#;'):
            j += 1
        i = j + 1
        continue
    out.append(lines[i])
    i += 1
io.open('build.rs', 'w', encoding='utf-8', newline='').write('\n'.join(out))
print('done', 'RQ_PROBE' not in '\n'.join(out))
