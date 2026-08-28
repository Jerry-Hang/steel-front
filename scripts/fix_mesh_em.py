# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
n1 = s.replace("if (slot >= EMISSIVE_INSTANCE_BASE) {", "if (slot >= EMISSIVE_INSTANCE_BASE && slot < EMISSIVE_INSTANCE_BASE + 64u) {")
n2 = n1.replace("is_foliage(inst.tint) || slot >= EMISSIVE_INSTANCE_BASE;", "is_foliage(inst.tint) || (slot >= EMISSIVE_INSTANCE_BASE && slot < EMISSIVE_INSTANCE_BASE + 64u);")
io.open(p, 'w', encoding='utf-8', newline='').write(n2)
print('mesh fixed', n2 != n1, n1 != s)
