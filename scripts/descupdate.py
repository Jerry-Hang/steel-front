# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 布局 binding0 类型
s = s.replace("""        let set_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);""", """        let set_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);""")
# pool 类型
s = s.replace("""        let dset_pool_info = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1);""", """        let dset_pool_info = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1);""")
# 描述符写入：地址缓冲
s = s.replace("""        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
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
        };""", """        // 加速结构地址缓冲（规范路径：地址→OpConvertUToAccelerationStructureKHR）
        let tlas_addr = unsafe {
            let a = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
            let info = vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(assets.tlas);
            a.get_acceleration_structure_device_address(&info)
        };
        let (addr_buf, addr_mem) = self
            .create_host_buffer(vk::BufferUsageFlags::STORAGE_BUFFER, 8)
            .map_err(|e| format!("addr buf: {e}"))?;
        unsafe {
            let m = self.device.map_memory(addr_mem, 0, 8, vk::MemoryMapFlags::empty()).map_err(|e| format!("addr map: {e}"))?;
            std::ptr::copy_nonoverlapping(&tlas_addr as *const u64 as *const u8, m as *mut u8, 8);
            self.device.unmap_memory(addr_mem);
        }
        let addr_info = vk::DescriptorBufferInfo {
            buffer: addr_buf,
            offset: 0,
            range: 8,
        };
        let write0 = vk::WriteDescriptorSet {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
            p_next: std::ptr::null(),
            dst_set: dset,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
            p_image_info: std::ptr::null(),
            p_buffer_info: std::slice::from_ref(&addr_info).as_ptr(),
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('renderer desc updated')
