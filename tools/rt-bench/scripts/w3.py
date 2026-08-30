# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("// ---------- RT 压测完整实现 ----------", "// ---------- RT 压测完整实现 ----------\nuse ash::vk;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('use added')
