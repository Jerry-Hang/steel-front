# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 1) WriteDescriptorSet: 手动构造（保留 p_next 字段）
old_w = """        dev.update_descriptor_sets(&[
            vk::WriteDescriptorSet::default().dst_set(ds).dst_binding(0).descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).p_next(&accel_write as *const _ as *const std::ffi::c_void),
            vk::WriteDescriptorSet::default().dst_set(ds).dst_binding(1).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).buffer_info(&[hbi]),
        ], &[]);"""
new_w = """        let mut w_as = vk::WriteDescriptorSet::default();
        w_as.dst_set = ds;
        w_as.dst_binding = 0;
        w_as.descriptor_type = vk::DescriptorType::ACCELERATION_STRUCTURE_KHR;
        w_as.descriptor_count = 1;
        w_as.p_next = &accel_write as *const _ as *const vk::BaseOutStructure;
        let mut w_hit = vk::WriteDescriptorSet::default();
        w_hit.dst_set = ds;
        w_hit.dst_binding = 1;
        w_hit.descriptor_type = vk::DescriptorType::STORAGE_BUFFER;
        w_hit.descriptor_count = 1;
        w_hit.p_buffer_info = &hbi;
        dev.update_descriptor_sets(&[w_as, w_hit], &[]);"""
if old_w in s:
    s = s.replace(old_w, new_w, 1)
    print('write desc fixed')
else:
    print('writedesc miss')
# 2) allocate_command_buffers 单参
s = s.replace("dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).command_buffer_count(1), None)?[0];", "dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).command_buffer_count(1)).map_err(|e| format!(\"{e:?}\"))?[0];")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('cmd alloc fixed')
