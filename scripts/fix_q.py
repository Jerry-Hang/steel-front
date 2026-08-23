# -*- coding: utf-8 -*-
import io
p = 'src/llm_cmd.rs'
s = io.open(p, encoding='utf-8').read()
# 修复：.replace('"', "\"") → .replace('"', "\\\"") （JSON 引号必须带反斜杠）
old = '.replace(\'"\', "\\"")'
new = '.replace(\'"\', "\\\\\\"")'
assert old in s, 'not found'
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed quote escape')
