# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# ① PT dispatch 后插降噪 dispatch（找 "self.pt_frame.set(self.pt_frame.get() + 1);" 后）
old1 = """                    self.pt_frame.set(self.pt_frame.get() + 1);
                }"""
new1 = """                    self.pt_frame.set(self.pt_frame.get() + 1);
                }
                // 2026-09-02 降噪后处理（SM 满载 + 去噪）：PT 输出 → dn_img
                if self.dn_pipeline != vk::Pipeline::null() {
                    let dn_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::SHADER_READ).dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.pt_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[dn_bar]);
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, self.dn_pipeline);
                    self.device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::COMPUTE, self.dn_layout, 0, &[self.dn_dset], &[]);
                    self.device.cmd_dispatch(command_buffer, (pw + 7) / 8, (ph + 7) / 8, 1);
                    let dn_bar2 = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE).dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.dn_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[dn_bar2]);
                }"""
if old1 in s:
    s = s.replace(old1, new1, 1)
    print('dn dispatch in')
# ② blit 源 pt_img→dn_img
old2 = "self.device.cmd_blit_image(command_buffer, self.pt_img, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);"
if old2 in s:
    # 只有 dn 启用时用 dn_img——直接改（dn 总是启用！）
    s = s.replace(old2, "self.device.cmd_blit_image(command_buffer, if self.dn_pipeline != vk::Pipeline::null() { self.dn_img } else { self.pt_img }, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);")
    print('blit dn source')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
