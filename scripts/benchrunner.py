# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = "    /// 记录 BLAS/TLAS 构建命令（一次性：命令缓冲执行）"
add = '''    /// RT 核心 纯求交吞吐基准（2026-08-29）：RT_BENCH_SPV 全遍历 × iterations
    /// 返回 (每秒射线 M, 命中数)
    pub fn run_pt_bench(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
        rays: u32,
        iterations: u32,
    ) -> Result<(f64, u32), String> {
        // 1) AS
        let assets = self.build_pt_as(boxes)?;
        // 2) compute 管线：RT_BENCH_SPV（内嵌!）
        let vs_module = self
            .create_shader_module(&crate::shaders::RT_BENCH_SPV.to_vec())
            .map_err(|e| format!("RT_BENCH module: {e}"))?;
        let set_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let hits_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_create = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&[set_layout, hits_layout]);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("RT set layout: {e}"))?;
        let pipe_create = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&[set_layout_handle])
            .push_constant_ranges(&[]);
        let pipe_layout = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }
            .map_err(|e| format!("RT pipe layout: {e}"))?;
        let compute_info = vk::ComputePipelineCreateInfo::default()
            .stage(vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(vs_module)
                .name(c"main"))
            .layout(pipe_layout);
        let compute_pipeline = unsafe {
            self.device
                .create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("RT compute pipeline: {e}"))?
        }[0];
        // 3) hits 缓冲（N u32，host 可见回读）
        let n = rays as usize;
        let (hits_buf, hits_mem) = self
            .create_host_buffer(vk::BufferUsageFlags::STORAGE_BUFFER, (n * 4) as u64)
            .map_err(|e| format!("hits: {e}"))?;
        let hits_mapped = unsafe {
            self.device
                .map_memory(hits_mem, 0, (n * 4) as u64, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("hits map: {e}"))?
        };
        unsafe {
            std::ptr::write_bytes(hits_mapped, 0, n * 4);
        }
        // 4) 描述符集（accel + hits）
        let dset_pool_info = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1);
        let dset_pool_info2 = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1);
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&[dset_pool_info, dset_pool_info2]);
        let dpool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("RT pool: {e}"))?;
        let dset_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(dpool)
            .set_layouts(&[set_layout_handle]);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }
            .map_err(|e| format!("RT dset: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
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
        unsafe { self.device.update_descriptor_sets(&[write0, write1], &[]) };
        // 5) 一次性构建命令（AS 构建）
        let alloc = vk::CommandBufferAllocateInfo::default()
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // 使用帧命令池外的一个独立分配
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("pt cb: {e}"))?[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            let _ = begin;
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT));
            self.record_pt_build(cb, &assets)?;
            self.device.end_command_buffer(cb);
        }
        let submit = vk::SubmitInfo::default().command_buffers(&[cb]);
        unsafe { self.device.queue_submit(self.queue, &[submit], vk::Fence::null()).map_err(|e| format!("pt submit: {e}"))?;
            self.device.queue_wait_idle(self.queue).map_err(|e| format!("pt wait: {e}"))?;
        }
        // 6) 计时迭代：dispatch × iterations（单独 cmd，等待后计时）
        let t0 = std::time::Instant::now();
        unsafe {
            self.device.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).map_err(|e| format!("pt reset: {e}"))?;
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default());
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            for _ in 0..iterations {
                self.device.cmd_dispatch(cb, (n as u32 + 63) / 64, 1, 1);
            }
            self.device.end_command_buffer(cb);
            let submit2 = vk::SubmitInfo::default().command_buffers(&[cb]);
            self.device.queue_submit(self.queue, &[submit2], vk::Fence::null()).map_err(|e| format!("pt bench submit: {e}"))?;
            self.device.queue_wait_idle(self.queue).map_err(|e| format!("pt bench wait: {e}"))?;
        }
        let elapsed = t0.elapsed().as_secs_f64();
        // 7) 回读命中
        let mut hits = 0u32;
        let hp = hits_mapped as *const u32;
        for i in 0..n {
            hits += unsafe { *hp.add(i) };
        }
        let total_rays = (rays as f64) * (iterations as f64);
        let mrays = total_rays / elapsed / 1_000_000.0;
        // 清理（基准一次性：简单释放）
        unsafe {
            self.device.unmap_memory(hits_mem);
            self.device.destroy_buffer(hits_buf, None);
            self.device.free_memory(hits_mem, None);
            self.device.destroy_pipeline(compute_pipeline, None);
            self.device.destroy_pipeline_layout(pipe_layout, None);
            self.device.destroy_descriptor_set_layout(set_layout_handle, None);
            self.device.destroy_descriptor_pool(dpool, None);
            self.device.destroy_shader_module(vs_module, None);
            self.device.free_command_buffers(self.command_pool, &[cb]);
            let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
            ext.destroy_acceleration_structure(assets.tlas, None);
            ext.destroy_acceleration_structure(assets.blas, None);
        }
        Ok((mrays, hits))
    }

''' + anchor
s = s.replace(anchor, add, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('bench runner added')
