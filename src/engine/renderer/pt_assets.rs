// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 构建路径追踪加速结构：盒体场景 → BLAS + TLAS（2026-08-29 阶段2）
    /// PT 实时 v2（2026-08-29 常驻化）：首帧构建 AS/管线/图像，后帧只 dispatch+blit
    /// 启动时构建 PT 常驻资源（2026-08-29：与 run_pt_view 同时空——已验证可跑！）
    pub(crate) fn init_pt_resident(&mut self, w: u32, h: u32) -> Result<(), String> {
        if self.pt_resident.is_some() {
            return Ok(());
        }
        let boxes = vec![
            crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [50.0, 0.5, 50.0], material: 0 },
            crate::engine::ray_tracer::PtBox { center: [1.0, 1.0, 0.0], half: [2.0, 2.0, 1.0], material: 1 },
            crate::engine::ray_tracer::PtBox { center: [-4.0, 1.5, -2.0], half: [1.5, 1.5, 1.5], material: 2 },
            crate::engine::ray_tracer::PtBox { center: [0.5, 1.0, 5.0], half: [0.8, 0.8, 0.8], material: 3 },
        ];
        let assets = self.build_pt_as(&boxes)?;
        let vs_module = self.create_shader_module(&crate::shaders::PT_FRAME_SPV.to_vec()).map_err(|e| format!("PT m: {e}"))?;
        let as_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0).descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let img_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let mat_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(2).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let acc_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(3).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏢 binding 4 = 道具逐三角属性表（device-local，2×u32/三角）——道具进 BLAS 专项
        let propv_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(4).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏪 binding 5 = 程序化地面纹理（与光栅同一张）：PT 地面盒按 world-space UV 采样，
        // 参照帧的地面反照率不再是一颗均匀沥青（单值表不出分区，道路区曾偏亮 22%）。
        let ground_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(5).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🧱 binding 6 = marker 砌块皮肤（与光栅 binding 7 同一张纹理）：PT 参照帧里
        // 围墙/花坛/护栏不再死平一块纯色。布局两处副本（这里 + run_pt_view）必须同步。
        let skin_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(6).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout, propv_layout, ground_layout, skin_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let sl = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }.map_err(|e| format!("PT sl: {e}"))?;
        let pipe_layouts = [sl];
        // push constants：7×vec4 = 112B（pt_panorama.glsl 的 PC 块 a..g）
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(112)];
        let pipe_create = vk::PipelineLayoutCreateInfo::default().set_layouts(&pipe_layouts).push_constant_ranges(&pc_ranges);
        let pl = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }.map_err(|e| format!("PT pl: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(vs_module).name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default().stage(stage_info).layout(pl);
        let pipelines = unsafe { self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None).map_err(|e| format!("PT pipe {:?}", e.1))? };
        let pipeline = pipelines[0];
        // 🔴 **图像格式必须与 GLSL 里声明的 `rgba8` 逐位一致**（2026-09-15 修正）。
        //
        // 原来这里是 `B8G8R8A8_UNORM`，而 `assets/rt/pt_panorama.glsl` 写的是
        // `layout(set=0, binding=1, rgba8) uniform writeonly image2D OutImg;` ——
        // 两者**兼容但不相等**，于是验证层（`RV3D_VALIDATION=1`）报：
        //
        // ```text
        // vkCmdDispatch(): the storage image descriptor [... variable "OutImg"] is accessed by a
        // OpTypeImage that has a Format operand Rgba8 (VK_FORMAT_R8G8B8A8_UNORM) which doesn't match
        // the VkImageView format (VK_FORMAT_B8G8R8A8_UNORM). Any loads or stores with the variable
        // will produce undefined values to the whole image (not just the texel being accessed).
        // While the formats are compatible, Storage Images must exactly match.
        // ```
        //
        // ⇒ 一句话：**PT 一直在往这张图里写"未定义值"**，不崩、不报错、只是画面发灰发脏 ——
        // 这正是本项目一直在防的那一类「静默 UB」（同教训 15 的越界读）。修法是让图像跟着着色器走
        // （而不是改着色器去迁就图像）：blit 到 B8G8R8A8_SRGB 交换链时驱动会做通道映射，
        // 两者属于同一 format compatibility class，颜色不会错位。
        let pt_img_format = vk::Format::R8G8B8A8_UNORM;
        // 明确查一次：STORAGE_IMAGE 对具体格式是**可选**能力，不支持就大声失败，
        // 不要留下一张"能创建但写不进"的图（那又会退回静默 UB）
        let pt_fmt_props = unsafe {
            self.instance
                .get_physical_device_format_properties(self.physical_device, pt_img_format)
        };
        if !pt_fmt_props
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::STORAGE_IMAGE)
        {
            return Err(format!(
                "设备不支持 {pt_img_format:?} 的 STORAGE_IMAGE —— PT 需要它来匹配 GLSL 的 rgba8"
            ));
        }
        let img_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D).format(pt_img_format)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 }).mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&img_info, None) }.map_err(|e| format!("PT i: {e}"))?;
        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_type = self.pick_memory_type(img_reqs, true).map_err(|e| format!("PT mt: {e}"))?;
        let img_alloc = vk::MemoryAllocateInfo::default().allocation_size(img_reqs.size).memory_type_index(img_type);
        let img_mem = unsafe { self.device.allocate_memory(&img_alloc, None) }.map_err(|e| format!("PT im: {e}"))?;
        unsafe { self.device.bind_image_memory(image, img_mem, 0) }.map_err(|e| format!("PT ib: {e}"))?;
        let img_view_info = vk::ImageViewCreateInfo::default()
            .image(image).view_type(vk::ImageViewType::TYPE_2D).format(pt_img_format)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let view = unsafe { self.device.create_image_view(&img_view_info, None) }.map_err(|e| format!("PT iv: {e}"))?;
        // 时域累积图像：RGBA32F（rgb=Σ线性样本，a=已累积 spp）。必须 STORAGE 且常驻，
        // 每帧只累加不丢弃 => 布局转换只在创建时做一次，逐帧 barrier 用 GENERAL->GENERAL。
        let acc_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .extent(vk::Extent3D { width: w, height: h, depth: 1 }).mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
        let acc_image = unsafe { self.device.create_image(&acc_info, None) }.map_err(|e| format!("PT acc: {e}"))?;
        let acc_reqs = unsafe { self.device.get_image_memory_requirements(acc_image) };
        let acc_type = self.pick_memory_type(acc_reqs, true).map_err(|e| format!("PT acc mt: {e}"))?;
        let acc_alloc = vk::MemoryAllocateInfo::default().allocation_size(acc_reqs.size).memory_type_index(acc_type);
        let acc_mem = unsafe { self.device.allocate_memory(&acc_alloc, None) }.map_err(|e| format!("PT acc mem: {e}"))?;
        unsafe { self.device.bind_image_memory(acc_image, acc_mem, 0) }.map_err(|e| format!("PT acc bind: {e}"))?;
        let acc_view_info = vk::ImageViewCreateInfo::default()
            .image(acc_image).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let acc_view = unsafe { self.device.create_image_view(&acc_view_info, None) }.map_err(|e| format!("PT acc view: {e}"))?;
        let pool_sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1),
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2),
            // STORAGE_BUFFER ×2：binding 2（盒材质）+ binding 4（道具逐三角属性表）
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(2),
            // COMBINED_IMAGE_SAMPLER ×2：binding 5（程序化地面纹理）+ binding 6（marker 砌块
            // 皮肤）。🔴 布局加了就得同步计数，否则 allocate_descriptor_sets 直接失败。
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(2),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes);
        let pool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }.map_err(|e| format!("PT dp: {e}"))?;
        let dset_layouts = [sl];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }.map_err(|e| format!("PT ds: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(), acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let img_info_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: view, image_layout: vk::ImageLayout::GENERAL };
        let acc_info_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: acc_view, image_layout: vk::ImageLayout::GENERAL };
        let ground_desc = vk::DescriptorImageInfo {
            sampler: self.texture_sampler,
            image_view: self.texture_image_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        };
        // 🧱 PT 的 marker 皮肤与光栅共用同一张图与同一个采样器（光栅绑在 binding 7）
        let skin_desc = vk::DescriptorImageInfo {
            sampler: self.texture_sampler,
            image_view: self.skin_marker_image_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        };
        let mat_buf_info = vk::DescriptorBufferInfo {
            buffer: assets.mat_buf,
            offset: 0,
            range: (crate::engine::ray_tracer::PT_MAX_BOXES * 16) as u64,
        };
        // 🏢 binding 4 = 道具逐三角属性表（device-local）；表未就绪时占位 verts_buf——
        // 那时 BLAS 没有道具几何，着色器道具分支按几何索引必然不可达
        let propv_buf_info = vk::DescriptorBufferInfo {
            buffer: if self.prop_attr_tris > 0 && self.prop_attr_buf != vk::Buffer::null() {
                self.prop_attr_buf
            } else {
                assets.verts_buf
            },
            offset: 0,
            range: vk::WHOLE_SIZE,
        };
        let writes = [
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: &accel_write as *const _ as *const std::ffi::c_void,
                dst_set: dset, dst_binding: 0, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                p_image_info: std::ptr::null(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 1, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&img_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 2, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&mat_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 3, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&acc_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 4, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&propv_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            // 🏪 binding 5 = 光栅同一张程序化地面纹理（init_texture 先于本函数跑完，
            // 图像已在 SHADER_READ_ONLY_OPTIMAL；常驻资源不随场景重建换，故 pt_refresh_dset 不重写它）
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 5, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                p_image_info: std::slice::from_ref(&ground_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            // 🧱 binding 6 = marker 砌块皮肤（同为常驻资源，pt_refresh_dset 不重写）
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET, p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 6, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                p_image_info: std::slice::from_ref(&skin_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // AS 一次性构建 + 等待（与 run_pt_view 同款——已验证路径！）
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        unsafe {
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(|e| format!("PT cb begin: {e}"))?;
            // 累积图像只做一次 UNDEFINED->GENERAL：之后每帧 barrier 必须是 GENERAL->GENERAL，
            // old_layout 用 UNDEFINED 等于告诉驱动"内容可丢弃" = 累积白做
            let acc_bar = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(acc_image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[acc_bar]);
            self.record_pt_build(cb, &assets, boxes.len())?;
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT sc: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("PT sw: {e}"))?;
            self.device.free_command_buffers(self.command_pool, &[cb]);
        }
        self.pt_resident = Some(Box::new(assets));
        self.pt_img = image;
        self.pt_img_mem = img_mem;
        self.pt_view = view;
        self.pt_pipeline = pipeline;
        self.pt_layout = pl;
        self.pt_setl = sl;
        self.pt_pool = pool;
        self.pt_dset = dset;
        self.pt_module = vs_module;
        self.pt_acc = acc_image;
        self.pt_acc_mem = acc_mem;
        self.pt_acc_view = acc_view;
        self.pt_size = (w, h);
        // RV3D_PT_SPP 覆盖的是**累积帧数**（不是每帧样本数！默认 256）。
        //
        // 🔴 设小值 = 改曝光，不是只改速度。片元末尾的时域累积是**指数滑动平均**
        // （`acc = mix(acc.rgb, lum, 1/a)`），而显示时又按"求和的样本数"再除一次
        // （`outc = acc.rgb / acc.a`）——双重归一化的后果是 `acc` 从 0 起步的**暂态被直接
        // 显示出来**：按稳态窗口 64 帧估，第 N 帧只到稳值的 1−(63/64)^N
        // ⇒ 16 帧 = 22.3%、64 帧 = 63.5%、256 帧（默认）= 98.2%。
        // 实测（同机位 `fly:60,1.5,-208:0,4`，全局灰度均值）：16 帧 90.7、64 帧 114.1。
        // ⇒ **拿 PT 做定量对照（与光栅比亮度、比反照率、比砖纹对比度）必须用默认 256**，
        // 小值只可用于"看个大概构图"。2026-09-29 我就是照旧注释把 16/64 当同图对比，
        // 得到了一条假缺陷（PROGRESS §22.11 → §22.11b 更正）。
        self.pt_spp_target = std::env::var("RV3D_PT_SPP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| (1..=4096).contains(v))
            .unwrap_or(256);
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        self.pt_view_sig.set(0);
        log::info!(
            "PT-RESIDENT: {}x{} 累积目标 {} 帧（时域 EMA，未到 256 帧时画面偏暗，见上方注释）",
            w,
            h,
            self.pt_spp_target
        );
        Ok(())
    }
    pub(crate) fn destroy_pt_resident(&mut self) {
        if self.pt_resident.is_none() {
            return;
        }
        unsafe {
            if self.pt_pipeline != vk::Pipeline::null() { self.device.destroy_pipeline(self.pt_pipeline, None); }
            if self.pt_layout != vk::PipelineLayout::null() { self.device.destroy_pipeline_layout(self.pt_layout, None); }
            if self.pt_setl != vk::DescriptorSetLayout::null() { self.device.destroy_descriptor_set_layout(self.pt_setl, None); }
            if self.pt_pool != vk::DescriptorPool::null() { self.device.destroy_descriptor_pool(self.pt_pool, None); }
            if self.pt_module != vk::ShaderModule::null() { self.device.destroy_shader_module(self.pt_module, None); }
            if self.pt_view != vk::ImageView::null() { self.device.destroy_image_view(self.pt_view, None); }
            if self.pt_img != vk::Image::null() { self.device.destroy_image(self.pt_img, None); }
            if self.pt_img_mem != vk::DeviceMemory::null() { self.device.free_memory(self.pt_img_mem, None); }
            if self.pt_acc_view != vk::ImageView::null() { self.device.destroy_image_view(self.pt_acc_view, None); }
            if self.pt_acc != vk::Image::null() { self.device.destroy_image(self.pt_acc, None); }
            if self.pt_acc_mem != vk::DeviceMemory::null() { self.device.free_memory(self.pt_acc_mem, None); }
            if let Some(assets) = self.pt_resident.take() {
                let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
                ext.destroy_acceleration_structure(assets.tlas, None);
                ext.destroy_acceleration_structure(assets.blas, None);
                self.device.destroy_buffer(assets.verts_buf, None);
                self.device.free_memory(assets.verts_mem, None);
                self.device.destroy_buffer(assets.idx_buf, None);
                self.device.free_memory(assets.idx_mem, None);
                self.device.destroy_buffer(assets.inst_buf, None);
                self.device.free_memory(assets.inst_mem, None);
                self.device.destroy_buffer(assets.mat_buf, None);
                self.device.free_memory(assets.mat_mem, None);
                self.device.destroy_buffer(assets.scratch_buf, None);
                self.device.free_memory(assets.scratch_mem, None);
                self.device.destroy_buffer(assets.tlas_buf, None);
                self.device.free_memory(assets.tlas_mem, None);
                self.device.destroy_buffer(assets.blas_buf, None);
                self.device.free_memory(assets.blas_mem, None);
            }
        }
        self.pt_pipeline = vk::Pipeline::null();
        self.pt_layout = vk::PipelineLayout::null();
        self.pt_setl = vk::DescriptorSetLayout::null();
        self.pt_pool = vk::DescriptorPool::null();
        self.pt_module = vk::ShaderModule::null();
        self.pt_acc = vk::Image::null();
        self.pt_acc_mem = vk::DeviceMemory::null();
        self.pt_acc_view = vk::ImageView::null();
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        self.pt_view_sig.set(0);
        self.pt_view = vk::ImageView::null();
        self.pt_img = vk::Image::null();
        self.pt_img_mem = vk::DeviceMemory::null();
    }
    pub(crate) fn build_pt_as(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
    ) -> Result<crate::engine::ray_tracer::PtAssets, String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        let n = boxes.len().min(PT_MAX_BOXES);
        // 🔴 2026-09-14：**把静默截断变成可诊断的一次告警**（与 `warn_npc_cap_once` 同一形态）。
        //
        // 上面那行 `.min()` 超出容量时什么都不说 —— 后果是"PT 画面里少了几栋楼"。
        // 而 PT 是低 spp 的噪点图，**少几个盒子肉眼根本看不出来**：
        // 未结案 #10 记的实测值就是 `marker=547 > PT_MAX_BOXES=512`，每次丢 35 个。
        //
        // 用闩而不是每次都记：这个函数在**场景重建**时调用，而重建由相机位移触发
        // （`signature()` 量化到 ~0.5m），移动时一秒能重建好几次 ⇒ 会刷屏，
        // 把 PT 那些真正有用的行淹掉（教训 26 的反面：噪声会训练人忽略日志）。
        if boxes.len() > PT_MAX_BOXES && !self.pt_box_cap_warned {
            self.pt_box_cap_warned = true;
            log::warn!(
                "PT: 盒数 {} 超过 PT_MAX_BOXES={} => 超出部分被静默丢弃，PT 画面里会少几何。\
                 两条出路：提高该常量（BLAS 按容量分配 ⇒ 显存同比上涨），\
                 或在 CPU 侧按视锥裁剪后再传进来。",
                boxes.len(),
                PT_MAX_BOXES
            );
        }
        // 顶点/索引/材质缓冲一次性按 PT_MAX_BOXES 分配（换场景只重写内容，句柄不动）
        let vb_len = PT_MAX_BOXES * 24 * 32;
        let ib_len = PT_MAX_BOXES * 36 * 4;
        let mb_len = PT_MAX_BOXES * 16;
        let (vbuf, vmem) = self
            .create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, vb_len as u64)
            .map_err(|e| format!("PT 顶点缓冲: {e}"))?;
        let (ibuf, imem) = self
            .create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, ib_len as u64)
            .map_err(|e| format!("PT 索引缓冲: {e}"))?;
        let (mbuf, mmem) = self
            .create_host_buffer(vk::BufferUsageFlags::STORAGE_BUFFER, mb_len as u64)
            .map_err(|e| format!("PT 材质缓冲: {e}"))?;
        let albedos: Vec<[f32; 3]> = boxes.iter().take(n).map(|b| pt_albedo_of(b)).collect();
        let mut assets = crate::engine::ray_tracer::PtAssets {
            tlas: vk::AccelerationStructureKHR::null(),
            blas: vk::AccelerationStructureKHR::null(),
            tlas_buf: vk::Buffer::null(),
            tlas_mem: vk::DeviceMemory::null(),
            blas_buf: vk::Buffer::null(),
            blas_mem: vk::DeviceMemory::null(),
            verts_buf: vbuf,
            verts_mem: vmem,
            idx_buf: ibuf,
            idx_mem: imem,
            inst_buf: vk::Buffer::null(),
            inst_mem: vk::DeviceMemory::null(),
            mat_buf: mbuf,
            mat_mem: mmem,
            scratch_buf: vk::Buffer::null(),
            scratch_mem: vk::DeviceMemory::null(),
            scratch_blas: 0,
            prop_tris: 0,
        };
        self.pt_fill_geom(&mut assets, &boxes[..n], &albedos)?;
        // 🏢 道具进 BLAS（2026-09-19，用户决策）：第二个三角形几何**零拷贝**引用道具主
        //   VB/IB——但**只在 BLAS 构建期被驱动读取**（烘进 BVH）。着色器命中时绝不读它们：
        //   那是 HOST_VISIBLE 内存，每命中随机读 = PCIe 风暴（pt3 实测 126fps→1.5fps）。
        //   道具的法线/颜色走 binding 4 的 device-local 逐三角属性表（set_props 构建）。
        //   分流按 rayQueryGetIntersectionGeometryIndexEXT（0=盒、1=道具）：ray query 的
        //   图元索引是**几何内局部**编号，不跨几何连续（pt3 灰树冠事故证伪了旧
        //   "hitPrim 全局连续 + pc.g.x 边界"假设——道具最前 21480 三角被当成盒查 boxMats）。
        // 🏢 道具几何只有在**属性表就绪**时才进 BLAS：着色器道具路径读的就是这张表，
        //   没有它分流就无意义（属性表失败时 set_props 已回退为不进）。
        let prop_tris = if self.prop_attr_tris > 0
            && self.prop_attr_tris * 3 == self.prop_index_count
            && self.prop_vertex_buffer != vk::Buffer::null()
            && self.prop_index_buffer != vk::Buffer::null()
        {
            self.prop_attr_tris
        } else {
            0
        };
        assets.prop_tris = prop_tris;
        let vaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.verts_buf); self.device.get_buffer_device_address(&i) };
        let iaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.idx_buf); self.device.get_buffer_device_address(&i) };
        let mut tri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
        tri.vertex_format = vk::Format::R32G32B32_SFLOAT;
        tri.max_vertex = (PT_MAX_BOXES * 24 - 1) as u32;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut geo = vk::AccelerationStructureGeometryKHR::default();
        geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        let mut geos = vec![geo];
        // 尺寸查询按**容量**算盒、按**当前**算道具：盒数波动就地重建够用；道具三角形数
        // 变化会换 `pt_prop_key` ⇒ 走整体重建，不会撑爆这块 AS 存储。
        let mut counts: Vec<u32> = vec![(PT_MAX_BOXES * 12) as u32];
        if prop_tris > 0 {
            let pvaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_vertex_buffer); self.device.get_buffer_device_address(&i) };
            let piaddr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(self.prop_index_buffer); self.device.get_buffer_device_address(&i) };
            let mut ptri = vk::AccelerationStructureGeometryTrianglesDataKHR::default();
            ptri.vertex_format = vk::Format::R32G32B32_SFLOAT;
            ptri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: pvaddr };
            ptri.vertex_stride = 32;
            ptri.max_vertex = self.prop_vertex_count.saturating_sub(1);
            ptri.index_type = vk::IndexType::UINT32;
            ptri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: piaddr };
            ptri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
            let mut pgeo = vk::AccelerationStructureGeometryKHR::default();
            pgeo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
            pgeo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: ptri };
            pgeo.flags = vk::GeometryFlagsKHR::OPAQUE;
            geos.push(pgeo);
            counts.push(prop_tris);
        }
        let mut geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        geom.ty = vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL;
        geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        geom.geometry_count = geos.len() as u32;
        geom.p_geometries = geos.as_ptr();
        geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            // 尺寸按 PT_MAX_BOXES 容量算（不是当前盒数）：换场景只重建 BLAS，
            // 若按初始 4 盒分配，塞进 512 盒会越界写 AS 缓冲 -> device lost
            ext.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &geom, &counts, &mut size_info);
        }
        let count = size_info.acceleration_structure_size;
        log::info!("PT-BLAS: size={} scratch_build={} prims={}", count, size_info.build_scratch_size, n * 12);
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
        // TLAS（单实例 identity：整场盒体合并在一个 BLAS 内，实例数与场景规模无关）
        let instance = vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR { matrix: [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] },
            instance_custom_index_and_mask: vk::Packed24_8::new(0u32, 0xFFu8),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0u32, 0u8),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: blas_addr },
        };
        let inst_bytes: &[u8] = unsafe { std::slice::from_raw_parts(&instance as *const _ as *const u8, std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) };
        let (inst_buf, inst_mem) = self
            .create_host_buffer(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR, inst_bytes.len() as u64)
            .map_err(|e| format!("PT 实例缓冲: {e}"))?;
        unsafe {
            let ip = self.device.map_memory(inst_mem, 0, inst_bytes.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map inst: {e}"))?;
            std::ptr::copy_nonoverlapping(inst_bytes.as_ptr(), ip as *mut u8, inst_bytes.len());
            self.device.unmap_memory(inst_mem);
        }
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
        log::info!("PT-TLAS: size={} scratch_build={}", tcount, tsize.build_scratch_size);
        let (tbuf, tmem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; tcount as usize], "pt-tlas")?;
        let tinfo = vk::AccelerationStructureCreateInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .size(tcount)
            .buffer(tbuf);
        let tlas = unsafe { ext.create_acceleration_structure(&tinfo, None) }
            .map_err(|e| format!("create TLAS: {e}"))?;
        // scratch 自有常驻：BLAS 用前段、TLAS 用后段（同地址连用两次构建 = 资源冲突）
        let align = 256u64;
        let b_scr = (size_info.build_scratch_size.max(align) + align - 1) & !(align - 1);
        let t_scr = (tsize.build_scratch_size.max(align) + align - 1) & !(align - 1);
        let (sbuf, smem) = self
            .create_device_local_buffer(vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, &vec![0u8; (b_scr + t_scr) as usize], "pt-scratch")?;
        assets.tlas = tlas;
        assets.blas = blas;
        assets.tlas_buf = tbuf;
        assets.tlas_mem = tmem;
        assets.blas_buf = asbuf;
        assets.blas_mem = asmem;
        assets.inst_buf = inst_buf;
        assets.inst_mem = inst_mem;
        assets.scratch_buf = sbuf;
        assets.scratch_mem = smem;
        assets.scratch_blas = b_scr;
        self.pt_box_count = n;
        Ok(assets)
    }
    /// 把盒体几何/索引/材质写进已分配的容量缓冲（句柄不变，换场景只重写内容）
    pub(crate) fn pt_fill_geom(
        &self,
        assets: &crate::engine::ray_tracer::PtAssets,
        boxes: &[crate::engine::ray_tracer::PtBox],
        albedos: &[[f32; 3]],
    ) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let boxidx = crate::engine::ray_tracer::box_indices();
        let mut verts = vec![0f32; PT_MAX_BOXES * 24 * 8];
        let mut idx = vec![0u32; PT_MAX_BOXES * 36];
        let mut mats = vec![0f32; PT_MAX_BOXES * 4];
        for (k, b) in boxes.iter().enumerate().take(PT_MAX_BOXES) {
            let mut v = [0.0f32; 192];
            crate::engine::ray_tracer::box_triangles(b, &mut v);
            verts[k * 192..k * 192 + 192].copy_from_slice(&v);
            let base = (k as u32) * 24;
            // 每盒索引加 base-vertex 偏移（否则所有盒都引用盒 0 顶点）
            for (j, &i) in boxidx.iter().enumerate() {
                idx[k * 36 + j] = i + base;
            }
            let a = albedos.get(k).copied().unwrap_or([0.5; 3]);
            mats[k * 4] = a[0];
            mats[k * 4 + 1] = a[1];
            mats[k * 4 + 2] = a[2];
            // 🔴 `.a` = 该盒的**最长世界轴跨度（米）**，不是 0/1 开关。
            //
            // 为什么把跨度传进着色器而不是在这里判完：光栅侧那条门（§22.7）除了
            // "够不够大所以是不是砌体"，还要给皮肤算一个按面尺寸收敛的 `detail` 因子
            // （`base = mix(color, skin, 0.45 * (0.25 + 0.75*detail))`）。跨度给出去，
            // PT 才能把同一个表达式抄过来；在这里压成 0/1 就只剩"有/没有"，
            // 远处会一直按 0.45 混合一张 mip 模糊过的灰图 ⇒ 越远越偏灰，与实机不符。
            // 玻璃/树冠的排除放在着色器里做（它手上就有 tint，与光栅同一条判据）。
            //
            // 尺寸门在这里先过一遍（着色器还会再判同样的阈值，两侧各自成立）：
            // 不合格的写 0.0，着色器拿到 0 必然落不进砌体分支。
            // 盒 0 = 地面大盒，跨度 800 会看着"像砌体"，但着色器对 boxIdx==0 走
            // GroundTex 分支、根本到不了这里，故无需特判。
            let span = 2.0 * (b.half[0].max(b.half[1]).max(b.half[2]));
            mats[k * 4 + 3] = if span >= crate::engine::ray_tracer::MASONRY_MIN_SPAN {
                span
            } else {
                0.0
            };
        }
        unsafe {
            let vb = verts.len() * 4;
            let p = self.device.map_memory(assets.verts_mem, 0, vb as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map v: {e}"))?;
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, p as *mut u8, vb);
            self.device.unmap_memory(assets.verts_mem);
            let ib = idx.len() * 4;
            let p = self.device.map_memory(assets.idx_mem, 0, ib as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map i: {e}"))?;
            std::ptr::copy_nonoverlapping(idx.as_ptr() as *const u8, p as *mut u8, ib);
            self.device.unmap_memory(assets.idx_mem);
            let mb = mats.len() * 4;
            let p = self.device.map_memory(assets.mat_mem, 0, mb as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("map m: {e}"))?;
            std::ptr::copy_nonoverlapping(mats.as_ptr() as *const u8, p as *mut u8, mb);
            self.device.unmap_memory(assets.mat_mem);
        }
        Ok(())
    }
    /// 重写几何 + 重建加速结构（一次性提交并等队列空闲——关卡加载级别的一次性开销）
    pub(crate) fn pt_scene_rebuild(
        &self,
        assets: &crate::engine::ray_tracer::PtAssets,
        boxes: &[crate::engine::ray_tracer::PtBox],
        albedos: &[[f32; 3]],
        n: usize,
    ) -> Result<(), String> {
        self.pt_fill_geom(assets, boxes, albedos)?;
        unsafe {
            let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
                .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
            let cb = self.device.allocate_command_buffers(&alloc).map_err(|e| format!("PT cb: {e}"))?[0];
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(|e| format!("PT cb begin: {e}"))?;
            self.record_pt_build(cb, assets, n)?;
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT scene submit: {e}"))?;
            // 必须排空**整设备**在飞工作：上一帧的 PT dispatch 仍在读同一批 TLAS/顶点缓冲，
            // 只等 graphics queue 不够（重写正在被读的 AS 输入 = device lost / TDR）
            self.device.device_wait_idle().map_err(|e| format!("PT scene wait: {e}"))?;
            self.device.free_command_buffers(self.command_pool, &[cb]);
        }
        Ok(())
    }
    /// BLAS 整体重建后重写 PT 常驻描述符集：binding 0（新 TLAS）/ 2（新 mat_buf）/
    /// 4（道具 VB）。**必须在设备静默后调用**（更新在飞中的 set = UB）——调用方
    /// （pt_set_scene_markers 重建分支）已先 device_wait_idle。
    /// binding 1/3 指渲染器自有的输出/累积图像，句柄跨重建不变，不用动。
    pub(crate) fn pt_refresh_dset(&self) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        let assets = self.pt_resident.as_ref().ok_or("PT 未常驻")?;
        if self.pt_dset == vk::DescriptorSet::null() {
            return Ok(());
        }
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let mat_info = vk::DescriptorBufferInfo {
            buffer: assets.mat_buf,
            offset: 0,
            range: (PT_MAX_BOXES * 16) as u64,
        };
        let propv_info = vk::DescriptorBufferInfo {
            buffer: if assets.prop_tris > 0 { self.prop_attr_buf } else { assets.verts_buf },
            offset: 0,
            range: vk::WHOLE_SIZE,
        };
        let writes = [
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: &accel_write as *const _ as *const std::ffi::c_void,
                dst_set: self.pt_dset,
                dst_binding: 0,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: self.pt_dset,
                dst_binding: 2,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::slice::from_ref(&mat_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: self.pt_dset,
                dst_binding: 4,
                dst_array_element: 0,
                descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(),
                p_buffer_info: std::slice::from_ref(&propv_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(),
                _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }
    /// 只销毁 PtAssets 内部的 GPU 资源（管线/图像归渲染器所有，本函数不动）。
    /// 供道具几何变化触发的整体重建在**新资源就绪后**释放旧的一份。
    pub(crate) unsafe fn pt_destroy_assets(&self, a: &crate::engine::ray_tracer::PtAssets) {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        if a.tlas != vk::AccelerationStructureKHR::null() {
            ext.destroy_acceleration_structure(a.tlas, None);
        }
        if a.blas != vk::AccelerationStructureKHR::null() {
            ext.destroy_acceleration_structure(a.blas, None);
        }
        for (buf, mem) in [
            (a.verts_buf, a.verts_mem),
            (a.idx_buf, a.idx_mem),
            (a.inst_buf, a.inst_mem),
            (a.mat_buf, a.mat_mem),
            (a.scratch_buf, a.scratch_mem),
            (a.tlas_buf, a.tlas_mem),
            (a.blas_buf, a.blas_mem),
        ] {
            if buf != vk::Buffer::null() {
                self.device.destroy_buffer(buf, None);
            }
            if mem != vk::DeviceMemory::null() {
                self.device.free_memory(mem, None);
            }
        }
    }
    /// 每帧取景参数（相机 + 太阳 + 曝光）
    pub(crate) fn set_pt_params(&mut self, p: crate::engine::ray_tracer::PtParams) {
        self.pt_params = p;
    }
}
