# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 精准：所有 "self.device.begin_command_buffer(cb,...);" 且行内无 let _（处理 5 处）
s = re.sub(r'(\n\s*)self\.device\.begin_command_buffer\(cb, ([^;]*?)\);', r'\1let _ = self.device.begin_command_buffer(cb, \2);', s)
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('begin_command_buffer let-_ (all)')
