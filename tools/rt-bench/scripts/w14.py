# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace(').map_err(|e| format!("{e:?}"))).map_err(|e| format!("{e:?}"))?;', ').map_err(|e| format!("{e:?}"))?;')
s = s.replace('dev.bind_buffer_memory(buf, mem, 0)).map_err(|e| format!("{e:?}"))?;', 'dev.bind_buffer_memory(buf, mem, 0).map_err(|e| format!("{e:?}"))?;')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('dup fixed')
