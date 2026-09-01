use ash::vk;
use std::time::Instant;

pub struct RtResult {
    pub best_mrays: f64,
    pub median_mrays: f64,
    pub total_hits: u64,
    pub rounds_mrays: Vec<f64>,
}

fn box_triangles(c: [f32; 3], h: [f32; 3], v: &mut [f32; 192]) {
    let (x, y, z) = (c[0], c[1], c[2]);
    let (hx, hy, hz) = (h[0], h[1], h[2]);
    let mut w = |i: usize, vx: f32, vy: f32, vz: f32, nx: f32, ny: f32, nz: f32, u: f32, t: f32| {
        let o = i * 8;
        v[o] = vx; v[o + 1] = vy; v[o + 2] = vz;
        v[o + 3] = nx; v[o + 4] = ny; v[o + 5] = nz;
        v[o + 6] = u; v[o + 7] = t;
    };
    w(0, x + hx, y - hy, z - hz, 1.0, 0.0, 0.0, 0.0, 0.0);
    w(1, x + hx, y + hy, z - hz, 1.0, 0.0, 0.0, 1.0, 0.0);
    w(2, x + hx, y + hy, z + hz, 1.0, 0.0, 0.0, 1.0, 1.0);
    w(3, x + hx, y - hy, z + hz, 1.0, 0.0, 0.0, 0.0, 1.0);
    w(4, x - hx, y - hy, z - hz, -1.0, 0.0, 0.0, 0.0, 0.0);
    w(5, x - hx, y - hy, z + hz, -1.0, 0.0, 0.0, 1.0, 0.0);
    w(6, x - hx, y + hy, z + hz, -1.0, 0.0, 0.0, 1.0, 1.0);
    w(7, x - hx, y + hy, z - hz, -1.0, 0.0, 0.0, 0.0, 1.0);
    w(8, x - hx, y + hy, z - hz, 0.0, 1.0, 0.0, 0.0, 0.0);
    w(9, x + hx, y + hy, z - hz, 0.0, 1.0, 0.0, 1.0, 0.0);
    w(10, x + hx, y + hy, z + hz, 0.0, 1.0, 0.0, 1.0, 1.0);
    w(11, x - hx, y + hy, z + hz, 0.0, 1.0, 0.0, 0.0, 1.0);
    w(12, x - hx, y - hy, z - hz, 0.0, -1.0, 0.0, 0.0, 0.0);
    w(13, x + hx, y - hy, z - hz, 0.0, -1.0, 0.0, 1.0, 0.0);
    w(14, x + hx, y - hy, z + hz, 0.0, -1.0, 0.0, 1.0, 1.0);
    w(15, x - hx, y - hy, z + hz, 0.0, -1.0, 0.0, 0.0, 1.0);
    w(16, x - hx, y - hy, z + hz, 0.0, 0.0, 1.0, 0.0, 0.0);
    w(17, x + hx, y - hy, z + hz, 0.0, 0.0, 1.0, 1.0, 0.0);
    w(18, x + hx, y + hy, z + hz, 0.0, 0.0, 1.0, 1.0, 1.0);
    w(19, x - hx, y + hy, z + hz, 0.0, 0.0, 1.0, 0.0, 1.0);
    w(20, x - hx, y - hy, z - hz, 0.0, 0.0, -1.0, 0.0, 0.0);
    w(21, x + hx, y - hy, z - hz, 0.0, 0.0, -1.0, 1.0, 0.0);
    w(22, x + hx, y + hy, z - hz, 0.0, 0.0, -1.0, 1.0, 1.0);
    w(23, x - hx, y + hy, z - hz, 0.0, 0.0, -1.0, 0.0, 1.0);
}

fn box_indices() -> [u32; 36] {
    let mut idx = [0u32; 36];
    for f in 0..6u32 {
        let b = f * 4;
        idx[(f * 6) as usize] = b;
        idx[(f * 6 + 1) as usize] = b + 1;
        idx[(f * 6 + 2) as usize] = b + 2;
        idx[(f * 6 + 3) as usize] = b;
        idx[(f * 6 + 4) as usize] = b + 2;
        idx[(f * 6 + 5) as usize] = b + 3;
    }
    idx
}

// ---------- 内存 helper：统一的 DEVICE_ADDRESS flag ----------
#[allow(clippy::too_many_arguments)]
fn mem_alloc_ex(
    dev: &ash::Device,
    instance: &ash::Instance,
    phys: vk::PhysicalDevice,
    buf: vk::Buffer,
    size: u64,
    host_visible: bool,
) -> Result<(vk::DeviceMemory, u64), String> {
    unsafe {
        let req = dev.get_buffer_memory_requirements(buf);
        let mprops = instance.get_physical_device_memory_properties(phys);
        let mut idx = 0u32;
        for (i, t) in mprops.memory_types.iter().enumerate() {
            if req.memory_type_bits & (1 << i) != 0 {
                let prop = if host_visible {
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
                } else {
                    vk::MemoryPropertyFlags::DEVICE_LOCAL
                };
                if t.property_flags.contains(prop) { idx = i as u32; break; }
            }
        }
        let _ = size;
        let mut ai = vk::MemoryAllocateInfo::default();
        ai.allocation_size = req.size;
        ai.memory_type_index = idx;
        let mut fl = vk::MemoryAllocateFlagsInfo::default();
        fl.flags = vk::MemoryAllocateFlags::DEVICE_ADDRESS;
        ai.p_next = &fl as *const _ as *const std::ffi::c_void;
        let mem = dev.allocate_memory(&ai, None).map_err(|e| format!("alloc: {e:?}"))?;
        dev.bind_buffer_memory(buf, mem, 0).map_err(|e| format!("bind: {e:?}"))?;
        Ok((mem, req.size))
    }
}

fn create_buf(dev: &ash::Device, size: u64, usage: vk::BufferUsageFlags) -> Result<vk::Buffer, String> {
    unsafe {
        dev.create_buffer(
            &vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        ).map_err(|e| format!("create_buffer: {e:?}"))
    }
}

fn buf_addr(dev: &ash::Device, buf: vk::Buffer) -> vk::DeviceAddress {
    unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(buf); dev.get_buffer_device_address(&i) }
}

pub fn rt_test(
    dev: &ash::Device,
    queue: &vk::Queue,
    instance: &ash::Instance,
    phys: vk::PhysicalDevice,
    rays: u32,
    iters: u32,
    rounds: u32,
) -> Result<RtResult, String> {
    unsafe {
        let small = [
            ([0.0f32, -0.5, 0.0], [50.0f32, 0.5, 50.0]),
            ([1.0f32, 1.0, 0.0], [2.0f32, 2.0, 1.0]),
            ([-4.0f32, 1.5, -2.0], [1.5f32, 1.5, 1.5]),
            ([0.5f32, 1.0, 5.0], [0.8f32, 0.8, 0.8]),
        ];
        let mut boxes: Vec<([f32; 3], [f32; 3])> = small.to_vec();
        // 大场景：1024 伪随机盒（深 BVH + 多次命中 = 真实游戏型负载！）
        let mut seed = 0x12345678u32;
        let mut rnd = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 8) as f32 / 16777216.0 };
        for _ in 0..1000 {
            let cx = (rnd() - 0.5) * 60.0;
            let cy = rnd() * 12.0;
            let cz = (rnd() - 0.5) * 60.0;
            let h = 0.4 + rnd() * 2.0;
            boxes.push(([cx, cy, cz], [h, h, h]));
        }
        let mut verts: Vec<u8> = Vec::new();
        let mut idxs: Vec<u32> = Vec::new();
        let boxidx = box_indices();
        for (k, (c, h)) in boxes.iter().enumerate() {
            let mut v = [0.0f32; 192];
            box_triangles(*c, *h, &mut v);
            for p in v.chunks_exact(8) { for f in p { verts.extend_from_slice(&f.to_le_bytes()); } }
            let base = (k as u32) * 24;
            for &i in boxidx.iter() { idxs.push(i + base); }
        }

        let vbuf = create_buf(dev, verts.len() as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR)?;
        let ibuf = create_buf(dev, idxs.len() as u64 * 4,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR)?;
        let (vmem, _) = mem_alloc_ex(dev, instance, phys, vbuf, verts.len() as u64, true)?;
        let (imem, _) = mem_alloc_ex(dev, instance, phys, ibuf, idxs.len() as u64 * 4, true)?;
        let vp = dev.map_memory(vmem, 0, verts.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("{e:?}"))?;
        std::ptr::copy_nonoverlapping(verts.as_ptr(), vp as *mut u8, verts.len());
        dev.unmap_memory(vmem);
        let ip = dev.map_memory(imem, 0, idxs.len() as u64 * 4, vk::MemoryMapFlags::empty()).map_err(|e| format!("{e:?}"))?;
        std::ptr::copy_nonoverlapping(idxs.as_ptr(), ip as *mut u32, idxs.len());
        dev.unmap_memory(imem);

        let vaddr = buf_addr(dev, vbuf);
        let iaddr = buf_addr(dev, ibuf);
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = boxes.len() as u32 * 24 - 1;
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
        let ext = ash::khr::acceleration_structure::Device::new(instance, dev);
        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &geom, &[(boxes.len() * 12) as u32], &mut size_info);
        let asbuf = create_buf(dev, size_info.acceleration_structure_size,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS)?;
        let (asmem, _) = mem_alloc_ex(dev, instance, phys, asbuf, size_info.acceleration_structure_size, false)?;
        let blas = ext.create_acceleration_structure(&vk::AccelerationStructureCreateInfoKHR::default().ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL).size(size_info.acceleration_structure_size).buffer(asbuf), None).map_err(|e| format!("blas: {e:?}"))?;
        let blas_addr = { let a = vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(blas); ext.get_acceleration_structure_device_address(&a) };

        let instance_khr = vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR { matrix: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] },
            instance_custom_index_and_mask: vk::Packed24_8::new(0xFFu32, 0xFFu8),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0, 0),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: blas_addr },
        };
        let inst_bytes: &[u8] = std::slice::from_raw_parts(&instance_khr as *const _ as *const u8, std::mem::size_of::<vk::AccelerationStructureInstanceKHR>());
        let ibuf2 = create_buf(dev, inst_bytes.len() as u64,
            vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR)?;
        let (imem2, _) = mem_alloc_ex(dev, instance, phys, ibuf2, inst_bytes.len() as u64, true)?;
        let ip2 = dev.map_memory(imem2, 0, inst_bytes.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("{e:?}"))?;
        std::ptr::copy_nonoverlapping(inst_bytes.as_ptr(), ip2 as *mut u8, inst_bytes.len());
        dev.unmap_memory(imem2);
        let inst_addr = buf_addr(dev, ibuf2);

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
        ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &tgeom, &[1], &mut tsize);
        let tbuf = create_buf(dev, tsize.acceleration_structure_size,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS)?;
        let (tmem, _) = mem_alloc_ex(dev, instance, phys, tbuf, tsize.acceleration_structure_size, false)?;
        let tlas = ext.create_acceleration_structure(&vk::AccelerationStructureCreateInfoKHR::default().ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL).size(tsize.acceleration_structure_size).buffer(tbuf), None).map_err(|e| format!("tlas: {e:?}"))?;

        let sbuf = create_buf(dev, 0x400000,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS)?;
        let (smem, _) = mem_alloc_ex(dev, instance, phys, sbuf, 0x400000, false)?;
        let scratch_addr = buf_addr(dev, sbuf);

        let hits_items: u32 = rays;
        let hbuf = create_buf(dev, hits_items as u64 * 4, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let (hmem, _) = mem_alloc_ex(dev, instance, phys, hbuf, hits_items as u64 * 4, true)?; // host-visible 回读！

        let spv = include_bytes!("../assets/rt.spv");
        let module = dev.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(std::slice::from_raw_parts(spv.as_ptr() as *const u32, spv.len() / 4)), None).map_err(|e| format!("module: {e:?}"))?;
        let as_layout = vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let hit_layout = vk::DescriptorSetLayoutBinding::default().binding(1).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let dsl = dev.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&[as_layout, hit_layout]), None).map_err(|e| format!("dsl: {e:?}"))?;
        let pl = dev.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&[dsl]), None).map_err(|e| format!("pl: {e:?}"))?;
        let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(module).name(c"main");
        let pipe = dev.create_compute_pipelines(vk::PipelineCache::null(), &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(pl)], None).map_err(|e| format!("pipe: {:?}", e.1))?[0];
        let pool = dev.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&[
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1),
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1),
        ]), None).map_err(|e| format!("pool: {e:?}"))?;
        let ds = dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl])).map_err(|e| format!("ds: {e:?}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(), acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let hbi = vk::DescriptorBufferInfo { buffer: hbuf, offset: 0, range: vk::WHOLE_SIZE };
        let mut w_as = vk::WriteDescriptorSet::default();
        w_as.dst_set = ds;
        w_as.dst_binding = 0;
        w_as.descriptor_type = vk::DescriptorType::ACCELERATION_STRUCTURE_KHR;
        w_as.descriptor_count = 1;
        w_as.p_next = &accel_write as *const _ as *const std::ffi::c_void;
        let mut w_hit = vk::WriteDescriptorSet::default();
        w_hit.dst_set = ds;
        w_hit.dst_binding = 1;
        w_hit.descriptor_type = vk::DescriptorType::STORAGE_BUFFER;
        w_hit.descriptor_count = 1;
        w_hit.p_buffer_info = &hbi;
        dev.update_descriptor_sets(&[w_as, w_hit], &[]);

        let cpool = dev.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(0), None).map_err(|e| format!("cpool: {e:?}"))?;
        let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).command_buffer_count(1)).map_err(|e| format!("cmd: {e:?}"))?[0];

        let blocks: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: (boxes.len() * 12) as u32, primitive_offset: 0, first_vertex: 0, transform_offset: 0 }];
        let blk_refs: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&blocks];
        let tblocks: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 1, primitive_offset: 0, first_vertex: 0, transform_offset: 0 }];
        let blk_refs_t: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&tblocks];

        let mut best = 0.0f64;
        let mut rounds_mrays: Vec<f64> = Vec::new();
        let mut total_hits: u64 = 0;
        for _round in 0..rounds {
            dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).map_err(|e| format!("begin: {e:?}"))?;
            // ① 每轮 hits 清零（vkCmdFillBuffer = 确定性重置，避免命中累积与驱动快进！）
            dev.cmd_fill_buffer(cmd, hbuf, 0, vk::WHOLE_SIZE, 0);
            let mut bg = geom.clone();
            bg.dst_acceleration_structure = blas;
            bg.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_addr };
            ext.cmd_build_acceleration_structures(cmd, &[bg], &blk_refs);
            let mut tg = tgeom.clone();
            tg.dst_acceleration_structure = tlas;
            tg.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_addr };
            ext.cmd_build_acceleration_structures(cmd, &[tg], &blk_refs_t);
            let accel_bar = vk::MemoryBarrier::default().src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR).dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
            dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[accel_bar], &[], &[]);
            let hit_bar = vk::BufferMemoryBarrier::default().src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::SHADER_WRITE).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED).buffer(hbuf).offset(0).size(vk::WHOLE_SIZE);
            dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[hit_bar], &[]);
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
            dev.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pl, 0, &[ds], &[]);
            let t0 = Instant::now();
            for _ in 0..iters {
                dev.cmd_dispatch(cmd, hits_items / 1024, 1, 1);
            }
            dev.end_command_buffer(cmd);
            // ② fence 严格同步（等待 GPU 完整完成！）
            let fence = dev.create_fence(&vk::FenceCreateInfo::default(), None).map_err(|e| format!("fence: {e:?}"))?;
            dev.queue_submit(*queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence).map_err(|e| format!("submit: {e:?}"))?;
            dev.wait_for_fences(&[fence], true, u64::MAX).map_err(|e| format!("wait: {e:?}"))?;
            dev.destroy_fence(fence, None);
            let dt = t0.elapsed().as_secs_f64();
            let mrays = hits_items as f64 * iters as f64 / dt / 1e6;
            rounds_mrays.push(mrays);
            if mrays > best { best = mrays; }
            // ③ 读回命中计数（验证射线遍历真实发生）
            let p = dev.map_memory(hmem, 0, hits_items as u64 * 4, vk::MemoryMapFlags::empty()).map_err(|e| format!("map: {e:?}"))?;
            let arr = std::slice::from_raw_parts(p as *const u32, hits_items as usize);
            let mut sum: u64 = 0;
            for v in arr.iter().take(4096) { sum = sum.wrapping_add(*v as u64); }
            dev.unmap_memory(hmem);
            if sum > 0 { total_hits += sum; }
        }
        rounds_mrays.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_mrays = rounds_mrays[rounds_mrays.len() / 2];
        return Ok(RtResult { best_mrays: best, median_mrays, total_hits, rounds_mrays });

        for b in [vbuf, ibuf, asbuf, tbuf, sbuf, hbuf, ibuf2] { dev.destroy_buffer(b, None); }
        for m in [vmem, imem, asmem, tmem, smem, hmem, imem2] { if m != vk::DeviceMemory::null() { dev.free_memory(m, None); } }
        ext.destroy_acceleration_structure(blas, None);
        ext.destroy_acceleration_structure(tlas, None);
        dev.destroy_pipeline(pipe, None);
        dev.destroy_pipeline_layout(pl, None);
        dev.destroy_descriptor_set_layout(dsl, None);
        dev.destroy_descriptor_pool(pool, None);
        dev.destroy_shader_module(module, None);
        dev.destroy_command_pool(cpool, None);
        Ok(RtResult { best_mrays: best, median_mrays: best, total_hits: 0, rounds_mrays: vec![best] })
    }
}

// ---------- RT 显存容量探测（分配到失败！） ----------
pub fn vram_probe(dev: &ash::Device, instance: &ash::Instance, phys: vk::PhysicalDevice) -> Result<(), String> {
    unsafe {
        let mprops = instance.get_physical_device_memory_properties(phys);
        let total: u64 = mprops.memory_heaps.iter().filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL)).map(|h| h.size).sum();
        println!("显存总量 (DEVICE_LOCAL): {:.2} GB", total as f64 / 1e9);
        // 类型索引
        let mut dev_idx = 0u32;
        for (i, t) in mprops.memory_types.iter().enumerate() {
            if t.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL) { dev_idx = i as u32; break; }
        }
        // ① 纯缓冲区最大可分配（倍增法！）
        let mut cur = 1u64 << 20; // 1MB 起步
        let mut last_ok = 0u64;
        let mut bufs: Vec<vk::Buffer> = Vec::new();
        let mut mems: Vec<vk::DeviceMemory> = Vec::new();
        loop {
            let b = dev.create_buffer(&vk::BufferCreateInfo::default().size(cur).usage(vk::BufferUsageFlags::STORAGE_BUFFER).sharing_mode(vk::SharingMode::EXCLUSIVE), None);
            match b {
                Ok(buf) => {
                    let r = dev.get_buffer_memory_requirements(buf);
                    let ai = vk::MemoryAllocateInfo::default().allocation_size(r.size).memory_type_index(dev_idx);
                    match dev.allocate_memory(&ai, None) {
                        Ok(mem) => {
                            let _ = dev.bind_buffer_memory(buf, mem, 0);
                            bufs.push(buf); mems.push(mem);
                            last_ok = cur;
                            cur = cur.saturating_mul(2);
                            if cur > (1u64 << 33) { break; } // 8GB 封顶
                        }
                        Err(_) => { dev.destroy_buffer(buf, None); break; }
                    }
                }
                Err(_) => break,
            }
        }
        println!("最大连续单块缓冲: {:.2} GB ({:.0} MiB)", last_ok as f64 / 1e9, last_ok as f64 / 1048576.0);
        // 多块合计（4MB 小片直到失败 = 实际可用总量！）
        let mut sum_all = last_ok;
        loop {
            let b = dev.create_buffer(&vk::BufferCreateInfo::default().size(4u64 << 20).usage(vk::BufferUsageFlags::STORAGE_BUFFER).sharing_mode(vk::SharingMode::EXCLUSIVE), None);
            match b {
                Ok(buf) => {
                    let r = dev.get_buffer_memory_requirements(buf);
                    let ai = vk::MemoryAllocateInfo::default().allocation_size(r.size).memory_type_index(dev_idx);
                    match dev.allocate_memory(&ai, None) {
                        Ok(mem) => { let _ = dev.bind_buffer_memory(buf, mem, 0); bufs.push(buf); mems.push(mem); sum_all += r.size; }
                        Err(_) => { dev.destroy_buffer(buf, None); break; }
                    }
                }
                Err(_) => break,
            }
        }
        println!("多块合计可分配: {:.2} GB（实际可用显存）", sum_all as f64 / 1e9);
        println!("  -> 射线命中计数 (4B/ray): {:.2} 亿射线", sum_all as f64 / 4.0 / 1e8);
        println!("  -> 顶点数据 (32B/vert): {:.2} 亿顶点", sum_all as f64 / 32.0 / 1e8);
        println!("  -> TLAS 实例 (64B/inst): {:.0} 万实例", sum_all as f64 / 64.0 / 10000.0);
        println!("  -> 射线命中计数器 (4B/ray): {:.2} 亿射线", last_ok as f64 / 4.0 / 1e8);
        println!("  -> 顶点数据 (pos+normal+uv 32B/vert): {:.1} 亿顶点", last_ok as f64 / 32.0 / 1e8);
        // ② AS 几何容量（每盒 AS 存储 ~1344B 估算来自 4盒=5376B；hits 规模测）
        let per_box_as = 5376u64 / 4;
        let as_box_cap = last_ok / per_box_as;
        println!("  -> 加速结构几何容量 (BLAS 存储, ~{:.0} B/盒): ~{:.0} 万盒 = ~{:.1} 亿三角", per_box_as as f64, as_box_cap as f64 / 10000.0, as_box_cap as f64 * 12.0 / 1e8);
        println!("  -> TLAS 实例容量 (64B/实例): {:.0} 万实例", last_ok as f64 / 64.0 / 10000.0);
        // ③ 每帧建议（参考：24 位深 BVH 中每射线 ~100B 工作集）
        let per_ray_budget = 100.0f64;
        println!("  -> 每帧射线预算 (100B/射线工作集): {:.1} 亿射线/帧", last_ok as f64 / per_ray_budget / 1e8);
        println!("  -> 1080p 全屏 1 bounce: ~200 万射线 => 可缓存 {:.0} 帧", last_ok as f64 / (2073600.0 * 100.0));
        // 清理
        for b in bufs { dev.destroy_buffer(b, None); }
        for m in mems { dev.free_memory(m, None); }
        Ok(())
    }
}
