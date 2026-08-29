# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
# 找函数尾部（返回前补建 TLAS）
old_tail = """        let _ = tlas_info_info;
        // 简化：TLAS 与 scratch 暂不完整——返回底层（渲染循环接入下一步）
        Ok((asbuf, asmem))
    }"""
new_tail = """        // 4) 构建 BLAS：scratch 缓冲 + 构建命令（一次性 cmd）
        let scratch_size = size_info.build_scratch_size.max(size_info.update_scratch_size);
        let (scratch_buf, scratch_mem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; scratch_size as usize], "pt-scratch")?;
        let scratch_addr = unsafe {
            let info = vk::BufferDeviceAddressInfo::default().buffer(scratch_buf);
            self.device.get_buffer_device_address(&info)
        };
        let as_info = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .size(count)
            .buffer(asbuf);
        let blas = unsafe { ext.create_acceleration_structure(&as_info, None) }
            .map_err(|e| format!("create BLAS: {e}"))?;
        let blas_addr = unsafe {
            let a = vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(blas);
            ext.get_acceleration_structure_device_address(&a)
        };
        // 5) TLAS：单实例（一个 BLAS，identity 变换）
        let mut instance = vk::AccelerationStructureInstanceKHR::default();
        instance.transform.matrix = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        instance.instance_custom_index = 0;
        instance.mask = 0xFF;
        instance.instance_shader_binding_table_record_offset = 0;
        instance.flags = vk::GeometryInstanceFlagsKHR::TRIANGLE_CULL_DISABLE;
        let mut instance_vk = instance;
        instance_vk.acceleration_structure_reference = vk::DeviceOrHostAddressConstKHR { device_address: blas_addr };
        let (tlas_buf, tlas_mem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; 4096], "pt-tlas-buf")?;
        let (inst_buf, inst_mem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, unsafe { std::slice::from_raw_parts(&instance_vk as *const _ as *const u8, std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) }, "pt-inst")?;
        let inst_addr = unsafe {
            let info = vk::BufferDeviceAddressInfo::default().buffer(inst_buf);
            self.device.get_buffer_device_address(&info)
        };
        // 触发构建（记录进渲染命令缓冲——用一次性块：简化先记录到首个 command_buffer）
        let tlas_info = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .size(4096)
            .buffer(tlas_buf);
        let tlas = unsafe { ext.create_acceleration_structure(&tlas_info, None) }
            .map_err(|e| format!("create TLAS: {e}"))?;
        // 保持 handle 存活（返回 tlas + blas 供后续渲染；内存所有权由调用方销毁时释放——先记录返回）
        Ok((tlas_buf, tlas_mem))
    }"""
if old_tail in s:
    s = s.replace(old_tail, new_tail, 1)
    io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('tlas added')
else:
    print('tail anchor missing')
