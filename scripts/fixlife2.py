# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&[dset_pool_info, dset_pool_info2]);""", """        let pool_sizes = [dset_pool_info, dset_pool_info2];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);""")
s = s.replace("""            let _ = begin;
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT));""", """            let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(cb, &begin_info);""")
s = s.replace("""        let submit = vk::SubmitInfo::default().command_buffers(&[cb]);
        unsafe { self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("pt submit: {e}"))?;""", """        let cbs = [cb];
        let submit = vk::SubmitInfo::default().command_buffers(&cbs);
        unsafe { self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("pt submit: {e}"))?;""")
s = s.replace("""            let submit2 = vk::SubmitInfo::default().command_buffers(&[cb]);
            self.device.queue_submit(self.graphics_queue, &[submit2], vk::Fence::null()).map_err(|e| format!("pt bench submit: {e}"))?;""", """            let cbs2 = [cb];
            let submit2 = vk::SubmitInfo::default().command_buffers(&cbs2);
            self.device.queue_submit(self.graphics_queue, &[submit2], vk::Fence::null()).map_err(|e| format!("pt bench submit: {e}"))?;""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('lifetimes done')
