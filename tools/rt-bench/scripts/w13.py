# -*- coding: utf-8 -*-
import io, re
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 逐行：任何以 ?; 结尾且含 dev. 或 vk_asext. 的行 → 包 map_err
lines = s.split('\n')
out = []
for ln in lines:
    st = ln.rstrip()
    if st.endswith('?;') and ('dev.' in ln or 'vk_asext.' in ln):
        # 末尾 ?; 替换成 ).map_err(|e| format!("{e:?}"))?;
        ln2 = ln.replace('?;', ').map_err(|e| format!("{e:?}"))?;')
        out.append(ln2)
    else:
        out.append(ln)
s = '\n'.join(out)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('swept ?;')
