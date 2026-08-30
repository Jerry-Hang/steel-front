# -*- coding: utf-8 -*-
import io
# 批量替换 0u → 1u
for name in ['fp32', 'fp16', 'fp8', 'fp4']:
    p = r'shaders\\' + name + '.comp'
    s = io.open(p, encoding='utf-8').read()
    s = s.replace('atomicAdd(o[g], 0u);', 'atomicAdd(o[g], 1u);')
    io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('atomicAdd 1u done')
