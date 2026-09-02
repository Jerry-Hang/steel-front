# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# Result ×10：搜 "self.device.end_command_buffer(cb);" 与同类行（.queue_submit( etc）结尾 ; 前加 let _ =
lines = s.split('\n')
out = []
for ln in lines:
    st = ln.rstrip()
    if st.endswith(';') and (
        'end_command_buffer(cb)' in st or 'end_command_buffer(cmd)' in st or
        'queue_submit(' in st and st.endswith(';') and st.strip().startswith('self.device')
    ) and not st.startswith('let _'):
        out.append('        let _ = ' + ln.strip() if ln.strip().startswith('self.') else ln)
    else:
        out.append(ln)
s = '\n'.join(out)
# 更精确：直接全覆盖 self.device.xxx(...); 模式（必 unused Result 的 VK 调用）
import re
s = re.sub(r'^(\s*)self\.device\.(end_command_buffer|queue_submit|reset_command_buffer|cmd_end_render_pass|destroy_buffer|destroy_image|destroy_pipeline|destroy_shader_module|destroy_semaphore|destroy_fence|free_memory|free_command_buffers)\(([^;]*?)\);\s*$', r'\1let _ = self.device.\2(\3);', s, flags=re.M)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('renderer Result fixed')
