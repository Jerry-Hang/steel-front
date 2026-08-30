# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace(')).map_err(|e| format!("{e:?}"))?;', '))?;'.replace("))", ")"))  # 先转义问题——用两段
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('x')
