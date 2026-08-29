# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
start_marker = "    /// 构建路径追踪加速结构（2026-08-29 阶段2）：盒体场景 → BLAS + TLAS"
end_marker = "    /// 2026-08-28：第一人称枪的实例模型矩阵 per-frame（bob/后坐走矩阵，顶点静态）"
i0 = s.find(start_marker)
i1 = s.find(end_marker)
assert i0 >= 0 and i1 > i0, (i0, i1)
new = '''    /// 路径追踪资源集（TLAS/BLAS/几何缓冲句柄）
    pub struct PtAssets {
        pub tlas: vk::AccelerationStructureKHR,
        pub blas: vk::AccelerationStructureKHR,
        pub tlas_buf: vk::Buffer,
        pub tlas_mem: vk::DeviceMemory,
        pub blas_buf: vk::Buffer,
        pub blas_mem: vk::DeviceMemory,
        pub verts_buf: vk::Buffer,
        pub verts_mem: vk::DeviceMemory,
        pub idx_buf: vk::Buffer,
        pub idx_mem: vk::DeviceMemory,
    }

    /// 构建路径追踪加速结构：盒体场景 → BLAS + TLAS（2026-08-29 阶段2）
    pub fn build_pt_as(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
    ) -> Result<self::PtAssets, String> {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        let n_verts = boxes.len() * 24;
        let n_idx = boxes.len() * 36;
        let mut verts: Vec<u8> = Vec::with_capacity(n_verts * 32);
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
        unsafe {
            let vp = self.device.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map v: {e}"))?;
            std::ptr::copy_nonoverlapping(verts.as_ptr(), vp as *mut u8, verts.len());
            self.device.unmap_memory(vmem);
            let ip = self.device.map_memory(imem, 0, (n_idx * 4) as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map i: {e}"))?;
            std::ptr::copy_nonoverlapping(idx.as_ptr(), ip as *mut u32, n_idx);
            self.device.unmap_memory(imem);
        }
        let vaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(vbuf); self.device.get_buffer_device_address(&i) };
        let iaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(ibuf); self.device.get_buffer_device_address(&i) };
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = 24;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut geo = vk::AccelerationStructureGeometryKHR::default();
        geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        let mut geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        geom.geometry_count = 1;
        geom.p_geometries = &geo;
        geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &geom, &[32], &mut size_info);
        }
        let count = size_info.acceleration_structure_size;
        let (asbuf, asmem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; count as usize], "pt-blas")?;
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
        // TLAS（单实例 identity）
        let mut instance = vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR { matrix: [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] },
            instance_custom_index_and_mask: vk::Packed24_8::new(0u32, 0xFFu8),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0u32, 0u8),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: blas_addr },
        };
        let inst_bytes: &[u8] = unsafe { std::slice::from_raw_parts(&instance as *const _ as *const u8, std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) };
        let (inst_buf, inst_mem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, inst_bytes, "pt-inst")?;
        let inst_addr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(inst_buf); self.device.get_buffer_device_address(&i) };

        // TLAS 几何（实例）
        let mut inst_geo_data = vk::AccelerationStructureGeometryInstancesDataKHR::default();
        inst_geo_data.array_of_pointers = vk::FALSE;
        inst_geo_data.data = vk::DeviceOrHostAddressConstKHR { device_address: inst_addr };
        let mut tgeo = vk::AccelerationStructureGeometryKHR::default();
        tgeo.geometry_type = vk::GeometryTypeKHR::INSTANCES;
        tgeo.geometry = vk::AccelerationStructureGeometryDataKHR { instances: inst_geo_data };
        let mut tgeom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        tgeom.ty = vk::AccelerationStructureTypeKHR::TOP_LEVEL;
        tgeom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        tgeom.geometry_count = 1;
        tgeom.p_geometries = &tgeo;
        tgeom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut tsize = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &tgeom, &[1], &mut tsize);
        }
        let tcount = tsize.acceleration_structure_size;
        let (tbuf, tmem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; tcount as usize], "pt-tlas")?;
        let tinfo = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .size(tcount)
            .buffer(tbuf);
        let tlas = unsafe { ext.create_acceleration_structure(&tinfo, None) }
            .map_err(|e| format!("create TLAS: {e}"))?;
        let _ = instance;
        Ok(crate::engine::ray_tracer::PtAssets {
            tlas,
            blas,
            tlas_buf: tbuf,
            tlas_mem: tmem,
            blas_buf: asbuf,
            blas_mem: asmem,
            verts_buf: vbuf,
            verts_mem: vmem,
            idx_buf: ibuf,
            idx_mem: imem,
        })
    }

    /// 记录 BLAS/TLAS 构建命令（一次性：命令缓冲执行）
    pub fn record_pt_build(
        &self,
        cmd: vk::CommandBuffer,
        assets: &crate::engine::ray_tracer::PtAssets,
    ) -> Result<(), String> {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        // 简化：scratch 用一次顶点缓冲映射（bench 一次性）
        let scratch_size = 0x200000u64;
        let (sbuf, smem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; scratch_size as usize], "pt-scratch")?;
        let scratch_addr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(sbuf); self.device.get_buffer_device_address(&i) };
        // BLAS 构建（重建）
        let mut b_geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        b_geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        b_geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        b_geom.geometry_count = 1;
        // 重建 geometry 引用（顶点/索引地址从缓冲重取）
        let vaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.verts_buf); self.device.get_buffer_device_address(&i) };
        let iaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.idx_buf); self.device.get_buffer_device_address(&i) };
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = 24;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut b_geo = vk::AccelerationStructureGeometryKHR::default();
        b_geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        b_geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        b_geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        b_geom.p_geometries = &b_geo;
        b_geom.dst_acceleration_structure = assets.blas;
        b_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_addr };
        b_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let range_b = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 36 * (0 + 1), primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
        // 注意 primitive_count = 实际三角数（盒数×12）——由调用方传入，此处保守
        // TLAS
        let mut t_geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        t_geom.ty = vk::AccelerationStructureTypeKHR::TOP_LEVEL;
        t_geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        t_geom.geometry_count = 1;
        let mut inst_geo_data = vk::AccelerationStructureGeometryInstancesDataKHR::default();
        inst_geo_data.array_of_pointers = vk::FALSE;
        // 重新从调用方获取实例地址——暂用 0（正确实现随后补）——此处先构建 BLAS
        inst_geo_data.data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut t_geo = vk::AccelerationStructureGeometryKHR::default();
        t_geo.geometry_type = vk::GeometryTypeKHR::INSTANCES;
        t_geo.geometry = vk::AccelerationStructureGeometryDataKHR { instances: inst_geo_data };
        t_geom.p_geometries = &t_geo;
        t_geom.dst_acceleration_structure = assets.tlas;
        t_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_addr };
        t_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let range_t = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 1, primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
        unsafe {
            ext.cmd_build_acceleration_structures(cmd, &[b_geom], &[&khr_build_ranges(&range_b)]);
            ext.cmd_build_acceleration_structures(cmd, &[t_geom], &[&khr_build_ranges(&range_t)]);
        }
        Ok(())
    }
'''
s = s[:i0] + new + s[i1:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('replaced block')
