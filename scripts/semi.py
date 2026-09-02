# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
ls = io.open(p, encoding='utf-8').read().split('\n')
# 精确：行 80 = '    };\n' 改为 '    }\n' ——行号 1-based 79？前面显示 [79]='    };'（0-based!）→ 79 0-based = 行 80
if ls[79].strip() == '};' or ls[79].strip() == '};':
    ls[79] = '    }'
io.open(p, 'w', encoding='utf-8', newline='\n').write('\n'.join(ls))
print('semicolon removed:', repr(ls[79]))
