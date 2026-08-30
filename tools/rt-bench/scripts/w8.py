# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
import re
# 全局修复 "None)).map_err" → "None).map_err" 与 "None).map_err" 的正确形态
s = s.replace("None)).map_err", "None).map_err")
s = s.replace("None).map_err|e|", "None).map_err(|e|")
# 还有一些 "None, None))?" 之类
s = s.replace("dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).command_buffer_count(1), None))?[0];", "dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).command_buffer_count(1), None).map_err(|e| format!(\"{e:?}\"))?[0];")
s = s.replace("], None)?;", "], None).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("], None))?;", "], None).map_err(|e| format!(\"{e:?}\"))?;")
s = s.replace("], None))?[0];", "], None).map_err(|e| format!(\"{e:?}\"))?[0];")
s = s.replace("], None))?[0];", "], None).map_err(|e| format!(\"{e:?}\"))?[0];")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('paren sweep')
