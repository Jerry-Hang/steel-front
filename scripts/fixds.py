# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
            .acceleration_structure_count(1)
            .p_acceleration_structures(&[assets.tlas]);
        let write0 = vk::WriteDescriptorSet::default()
            .dst_set(dset)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1)
            .p_next(&accel_write as *const _ as *const std::ffi::c_void);
        let buf_info = vk::DescriptorBufferInfo::default()
            .buffer(hits_buf)
            .offset(0)
            .range((n * 4) as u64);
        let write1 = vk::WriteDescriptorSet::default()
            .dst_set(dset)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .p_buffer_info(&[buf_info]);
        unsafe { self.device.update_descriptor_sets(&[write0, write1], &[]) };"""
new = """        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: &[assets.tlas],
            _marker: std::marker::PhantomData,
        };
        let write0 = vk::WriteDescriptorSet {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
            p_next: &accel_write as *const _ as *const std::ffi::c_void,
            dst_set: dset,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
            p_image_info: std::ptr::null(),
            p_buffer_info: std::ptr::null(),
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };
        let buf_info = vk::DescriptorBufferInfo {
            buffer: hits_buf,
            offset: 0,
            range: (n * 4) as u64,
        };
        let write1 = vk::WriteDescriptorSet {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
            p_next: std::ptr::null(),
            dst_set: dset,
            dst_binding: 1,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
            p_image_info: std::ptr::null(),
            p_buffer_info: &[buf_info],
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };
        unsafe { self.device.update_descriptor_sets(&[write0, write1], &[]) };"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('ds writes fixed')
else:
    print('anch miss')
