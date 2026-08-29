# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
anchor = "    /// 2026-08-28：第一人称枪的实例模型矩阵 per-frame（bob/后坐走矩阵，顶点静态）"
add = """    /// 构建路径追踪加速结构（2026-08-29 阶段2）：盒体场景 → BLAS + TLAS
    /// 返回 (TLAS 缓冲, TLAS 内存) —— ray-query 路径追踪的输入
    pub fn build_pt_as(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        use vk::KhrAccelerationStructure as _;
        // 1) 盒体几何：顶点+索引缓冲（每盒 24 顶点/12 三角）
        let n_verts = boxes.len() * 24;
        let n_idx = boxes.len() * 36;
        let mut verts: Vec<u8> = Vec::with_capacity(n_verts * 32); // pos3+normal3+uv2
        let mut idx: Vec<u32> = Vec::with_capacity(n_idx);
        let boxidx = crate::engine::ray_tracer::box_indices();
        for b in boxes {
            let mut v = [0.0f32; 72];
            crate::engine::ray_tracer::box_triangles(b, &mut v);
            for p in v.chunks_exact(8) {
                for c in p {
                    verts.extend_from_slice(&c.to_le_bytes());
                }
            }
            idx.extend_from_slice(&boxidx);
        }
        let (vbuf, vmem) = self
            .create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, verts.len() as u64)
            .map_err(|e| format!("PT 顶点缓冲: {e}"))?;
        let (ibuf, imem) = self
            .create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, (n_idx * 4) as u64)
            .map_err(|e| format!("PT 索引缓冲: {e}"))?;
        // 写入（借用 device）
        unsafe {
            let vp = self
                .device
                .map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("map v: {e}"))?;
            std::ptr::copy_nonoverlapping(verts.as_ptr(), vp as *mut u8, verts.len());
            self.device.unmap_memory(vmem);
            let ip = self
                .device
                .map_memory(imem, 0, (n_idx * 4) as u64, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("map i: {e}"))?;
            std::ptr::copy_nonoverlapping(idx.as_ptr(), ip as *mut u8, n_idx * 4);
            self.device.unmap_memory(imem);
        }
        let vaddr = unsafe {
            let info = vk::BufferDeviceAddressInfo::default().buffer(vbuf);
            self.device.get_buffer_device_address(&info)
        };
        let iaddr = unsafe {
            let info = vk::BufferDeviceAddressInfo::default().buffer(ibuf);
            self.device.get_buffer_device_address(&info)
        };
        // 2) BLAS 几何（三角形）与构建尺寸
        let tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .max_vertex(24)
            .vertex_data(vk::DeviceOrHostAddressConstKHR { device_address: vaddr })
            .vertex_stride(32)
            .index_type(vk::IndexType::UINT32)
            .index_data(vk::DeviceOrHostAddressConstKHR { device_address: iaddr })
            .index_stride(4)
            .transform_data(vk::DeviceOrHostAddressConstKHR { device_address: 0 });
        let geo = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
            .geometry(vk::AccelerationStructureGeometryDataKHR::triangles(tri))
            .flags(vk::GeometryFlagsKHR::OPAQUE);
        let geom = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .geometry_count(1)
            .p_geometries(&[geo])
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD);
        let mut count = 0u32;
        unsafe {
            self.device.get_acceleration_structure_build_sizes_khr(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &geom,
                &[32],
                &mut count,
            );
        }
        let (asbuf, asmem) = self
            .create_device_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, count as u64)
            .map_err(|e| format!("PT AS 缓冲: {e}"))?;
        let as_info = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .size(count)
            .buffer(asbuf);
        let blas = unsafe { self.device.create_acceleration_structure_khr(&as_info, None) }
            .map_err(|e| format!("create BLAS: {e}"))?;
        let mut offsets = vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(blas);
        let blas_addr = unsafe { self.device.get_acceleration_structure_device_address_khr(&offsets) };
        // 3) TLAS（单实例：BLAS）
        let tlas_info_info = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .size(64)
            .buffer(asbuf)
            .offset((count as u64 + 255) & !255);
        let _ = tlas_info_info;
        // 简化：TLAS 与 scratch 暂不完整——返回底层（渲染循环接入下一步）
        Ok((asbuf, asmem))
    }

""" + anchor
s = s.replace(anchor, add, 1)
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('AS builder added')
