# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
lines = io.open(p, encoding='utf-8').read().split('\n')
# 找 "pub mod gdi_img {" 行
i = next(k for k, l in enumerate(lines) if l.strip() == 'pub mod gdi_img {')
# 删除从 i 到文件末（390 行 = i 之后整个）
new = lines[:i]
s = '\n'.join(new)
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('gdi_img module deleted, remaining lines:', len(new))
