# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("        let hits_items: u32 = 1 << 22;", "        let hits_items: u32 = rays;")
old_loop = """        let iters = 200u32;
        let mut best = 0.0f64;
        for _round in 0..3 {
            dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).map_err(|e| format!("begin: {e:?}"))?;
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
            let hit_bar = vk::BufferMemoryBarrier::default().src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR).dst_access_mask(vk::AccessFlags::SHADER_WRITE).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED).buffer(hbuf).offset(0).size(vk::WHOLE_SIZE);
            dev.cmd_pipeline_barrier(cmd, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[hit_bar], &[]);
            dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
            dev.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, pl, 0, &[ds], &[]);
            let t0 = Instant::now();
            for _ in 0..iters {
                dev.cmd_dispatch(cmd, hits_items / 1024, 1, 1);
            }
            dev.end_command_buffer(cmd);
            dev.queue_submit(*queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], vk::Fence::null()).map_err(|e| format!("submit: {e:?}"))?;
            dev.queue_wait_idle(*queue).map_err(|e| format!("wait: {e:?}"))?;
            let dt = t0.elapsed().as_secs_f64();
            let mrays = hits_items as f64 * iters as f64 / dt / 1e6;
            if mrays > best { best = mrays; }
        }"""
new_loop = """        let mut best = 0.0f64;
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
        return Ok(RtResult { best_mrays: best, median_mrays, total_hits, rounds_mrays });"""
if old_loop in s:
    s = s.replace(old_loop, new_loop, 1)
    print('loop replaced')
else:
    print('loop miss')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
