# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
i0 = s.find('        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!("dn bi: {e}"))?;')
i1 = s.find('        let dn_view = unsafe { self.device.create_image_view')
# 删除 i0 到 i1 之间的 fill 段（包括新增的 noop 块！）
seg = s[i0:i1]
new_seg = "        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!(\"dn bi: {e}\"))?;\n        "
s = s[:i0] + "        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!(\"dn bi: {e}\"))?;\n" + s[i1:]
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('noop fill removed')
