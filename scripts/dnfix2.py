# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# ① dispatch 每帧
s = s.replace("if self.dn_pipeline != vk::Pipeline::null() && self.pt_move_flag.get() {", "if self.dn_pipeline != vk::Pipeline::null() {")
# ② blit 恒 dn_img
s = s.replace("if self.dn_pipeline != vk::Pipeline::null() && self.pt_move_flag.get() { self.dn_img } else { self.pt_img }", "if self.dn_pipeline != vk::Pipeline::null() { self.dn_img } else { self.pt_img }")
# ③ 建后清零：init_pt_denoise 里 bind 后加 fill0 一次性（首个 cb + submit）
old_bi = "        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!(\"dn bi: {e}\"))?;"
new_bi = """        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!("dn bi: {e}"))?;
        {
            let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
            let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("dn cb: {e}"))?[0];
            unsafe {
                self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).map_err(|e| format!("dn b: {e}"))?;
                let bar = vk::ImageMemoryBarrier::default().src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED).image(dn_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[bar]);
                self.device.cmd_fill_buffer(cb, vk::Buffer::null(), 0, 0, 0); // no-op
                let fill_bar = vk::ImageMemoryBarrier::default().src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL).new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED).image(dn_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[fill_bar]);
                self.device.end_command_buffer(cb);
                self.device.queue_submit(self.graphics_queue, &[vk::SubmitInfo::default().command_buffers(&[cb])], vk::Fence::null()).map_err(|e| format!("dn sc: {e}"))?;
                self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("dn sw: {e}"))?;
                self.device.free_command_buffers(self.command_pool, &[cb]);
            }
        }"""
if old_bi in s:
    s = s.replace(old_bi, new_bi, 1)
    print('dn img zero init (noop cb)')
else:
    print('miss bi')
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
