# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = """            dev.queue_submit(*queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], vk::Fence::null()).map_err(|e| format!("submit: {:?}", e))?;
            dev.queue_wait_idle(*queue).map_err(|e| format!("wait: {:?}", e))?;"""
new = """            let fence = dev.create_fence(&vk::FenceCreateInfo::default(), None).map_err(|e| format!("fence: {:?}", e))?;
            dev.queue_submit(*queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence).map_err(|e| format!("submit: {:?}", e))?;
            dev.wait_for_fences(&[fence], true, u64::MAX).map_err(|e| format!("wait: {:?}", e))?;
            dev.destroy_fence(fence, None);"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('fence sync')
else:
    print('miss')
