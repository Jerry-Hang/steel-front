// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// PT 场景热替换：重写 BLAS 内容并重建加速结构（关卡加载/据点变色时一次）。
    /// 与光栅化共用同一批 WorldMarker 矩阵 => PT 与画面几何逐米一致。
    pub(crate) fn pt_set_scene_markers(&mut self, markers: &[WorldMarker]) -> Result<(), String> {
        use crate::engine::ray_tracer::PT_MAX_BOXES;
        if self.pt_resident.is_none() || !self.pt_live_enabled {
            return Ok(());
        }
        let mut boxes: Vec<crate::engine::ray_tracer::PtBox> =
            Vec::with_capacity(markers.len() + 1);
        let mut albedos: Vec<[f32; 3]> = Vec::with_capacity(markers.len() + 1);
        // 盒 0 = 地面大盒（游戏地形中央压平，PT 用平面盒近似，烘焙参照足够）
        // 🏪 它的 boxMats 反照率自 2026-09-29 起不再被着色器读取：地面改采样与光栅
        //   同一张程序化纹理（binding 5，见 pt_panorama.glsl）——单颗均匀沥青表不出
        //   分区（道路区曾比光栅偏亮 22%）。保留条目只为盒序号与 marker 一一对应。
        boxes.push(crate::engine::ray_tracer::PtBox {
            center: [0.0, -1.0, 0.0],
            half: [400.0, 1.0, 400.0],
            material: 0,
        });
        albedos.push([0.115, 0.120, 0.128]);
        // 🔴 容量比对挪到 take **之前**（2026-09-19）：take 截断让 build_pt_as 里的告警闩
        //   永远不触发——marker=1789 > 旧容量 1024 静默丢 765 个就是从这里漏出去的
        //   （#10 这一族坑的第三次复发，容量现已提到 2048）。
        if markers.len() + 1 > PT_MAX_BOXES && !self.pt_box_cap_warned {
            self.pt_box_cap_warned = true;
            log::warn!(
                "PT: marker 数 {} + 地面盒超过盒容量 {} ⇒ 超出部分正被 take 截断，请提高 PT_MAX_BOXES",
                markers.len(),
                PT_MAX_BOXES - 1
            );
        }
        for m in markers.iter().take(PT_MAX_BOXES - 1) {
            let c = m.model.w_axis;
            // 🔴 实例缩放 = 真实半尺寸 ÷ 模板半幅（`obstacle_model` 的 `half / tmpl`），
            // 所以还原半尺寸要**乘回模板半幅**，不是乘 0.5。
            // 模板半幅只有圆柱的 Y 是 0.5（单位圆柱 y∈[−0.5,0.5]），其余形状三轴都是 1.0
            // （立方体/球模板是 **±1**，见本文件 `VERTICES`）。
            // 旧代码一律 `* 0.5` 是 **2026-09-17 之前**的约定——那时渲染盒是 AABB 的 2 倍
            // （`half / tmpl` 之前写的是 `2*half / tmpl`，`* 0.5` 恰好抵消）。
            // 9-17 把渲染盒改成与碰撞盒逐轴同尺寸时，这里没跟着改 ⇒ **PT 的 marker 盒
            // 整体小了一半**（只有圆柱的高度因为模板半幅正好是 0.5 而恰好正确）。
            // 后果与取证见 docs/PROGRESS.md §22.14。
            let shape = crate::engine::geom::Shape::from_tag(m.tint[3]);
            let hx = m.model.x_axis.length() * shape.template_half_extent(0);
            let hy = m.model.y_axis.length() * shape.template_half_extent(1);
            let hz = m.model.z_axis.length() * shape.template_half_extent(2);
            if !(hx > 0.01 && hy > 0.01 && hz > 0.01) {
                continue;
            }
            boxes.push(crate::engine::ray_tracer::PtBox {
                center: [c.x, c.y, c.z],
                half: [hx, hy, hz],
                material: 1,
            });
            albedos.push([m.tint[0], m.tint[1], m.tint[2]]);
        }
        let sig = pt_scene_sig(&boxes);
        // 🏢 道具几何句柄/三角数变了 ⇒ BLAS 的尺寸与引用都变 ⇒ 必须整体重建
        //（就地 rebuild 只重写盒体内容，改不了 AS 大小）。
        let prop_key = (
            ash::vk::Handle::as_raw(self.prop_vertex_buffer),
            ash::vk::Handle::as_raw(self.prop_attr_buf),
            self.prop_index_count,
        );
        let props_changed = prop_key != self.pt_prop_key;
        if sig == self.pt_scene_sig && !props_changed {
            return Ok(());
        }
        self.pt_scene_sig = sig;
        self.pt_prop_key = prop_key;
        let n = boxes.len();
        if props_changed {
            // 顺序：静默 → 建新（双几何尺寸查询+创建+填充）→ 重写描述符 → 构建+静默 → 销毁旧。
            // 帧内顺序（set_props → 本函数 → render）保证新旧之间没有 dispatch 引用旧缓冲。
            unsafe {
                self.wait_idle_checked();
            }
            let old = self.pt_resident.take();
            let fresh = match self.build_pt_as(&boxes) {
                Ok(a) => a,
                Err(e) => {
                    self.pt_resident = old;
                    return Err(e);
                }
            };
            let fresh_prop_tris = fresh.prop_tris;
            self.pt_resident = Some(Box::new(fresh));
            self.pt_refresh_dset()?;
            let res = self.pt_scene_rebuild(
                self.pt_resident.as_ref().unwrap(),
                &boxes,
                &albedos,
                n,
            );
            if let Some(o) = old {
                unsafe { self.pt_destroy_assets(&o) };
            }
            res?;
            self.pt_box_count = n;
            self.pt_frame.set(0);
            self.pt_reset.set(true);
            log::info!(
                "PT-SCENE: 道具几何变化 → BLAS 整体重建：盒 {} + 道具三角 {}",
                n,
                fresh_prop_tris
            );
            return Ok(());
        }
        // 取出 assets（避免 &mut self.pt_resident 与随后的 &self 方法调用冲突）
        let assets = match self.pt_resident.take() {
            Some(a) => a,
            None => return Ok(()),
        };
        let res = self.pt_scene_rebuild(&assets, &boxes, &albedos, n);
        self.pt_resident = Some(assets);
        res?;
        self.pt_box_count = n;
        // 场景换了，旧累积全部作废
        self.pt_frame.set(0);
        self.pt_reset.set(true);
        log::info!("PT-SCENE: 盒 {} 个（WorldMarker 同源）", n);
        Ok(())
    }
    /// PT 参考帧渲染（2026-08-29 里程碑1/2）：相机射线 + 命中着色 + 图像输出 → PNG
    pub(crate) fn run_pt_view(
        &mut self,
        boxes: &[crate::engine::ray_tracer::PtBox],
        size: u32,
    ) -> Result<(), String> {
        let assets = self.build_pt_as(boxes)?;
        let vs_module = self
            .create_shader_module(&crate::shaders::PT_FRAME_SPV.to_vec())
            .map_err(|e| format!("PT_FRAME module: {e}"))?;
        let as_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let img_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let mat_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let acc_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏢 binding 4 = 道具逐三角属性表（device-local，2×u32/三角）——道具进 BLAS 专项
        let propv_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(4)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🏪 binding 5 = 程序化地面纹理（与 init_pt_resident 同布局：着色器静态引用了它，
        // 不绑 = UB；玩具场景的"地面"会显示市心广场的纹理像素，无害）
        let ground_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(5)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        // 🧱 binding 6 = marker 砌块皮肤（与 init_pt_resident 同布局：着色器静态引用了它）
        let skin_layout = vk::DescriptorSetLayoutBinding::default()
            .binding(6)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE);
        let set_bindings = [as_layout, img_layout, mat_layout, acc_layout, propv_layout, ground_layout, skin_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&set_bindings);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("PT set: {e}"))?;
        let pipe_layouts = [set_layout_handle];
        let pc_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(112)];
        let pipe_create = vk::PipelineLayoutCreateInfo::default().set_layouts(&pipe_layouts).push_constant_ranges(&pc_ranges);
        let pipe_layout = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }
            .map_err(|e| format!("PT layout: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE).module(vs_module).name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default().stage(stage_info).layout(pipe_layout);
        let pipelines = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("PT pipe: {:?}", e.1))?
        };
        let compute_pipeline = pipelines[0];
        // 输出存储图像（rgba8, size×size）
        let img_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D { width: size, height: size, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&img_info, None) }
            .map_err(|e| format!("PT img: {e}"))?;
        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_type = self.pick_memory_type(img_reqs, true)?;
        let img_alloc = vk::MemoryAllocateInfo::default().allocation_size(img_reqs.size).memory_type_index(img_type);
        let img_mem = unsafe { self.device.allocate_memory(&img_alloc, None) }
            .map_err(|e| format!("PT img mem: {e}"))?;
        unsafe { self.device.bind_image_memory(image, img_mem, 0) }
            .map_err(|e| format!("PT img bind: {e}"))?;
        let img_view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let view = unsafe { self.device.create_image_view(&img_view_info, None) }
            .map_err(|e| format!("PT view: {e}"))?;
        // 累积图像（RGBA32F）：参考帧一次派发多帧 spp，输出收敛结果而非 1 spp 噪声图
        let acc_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .extent(vk::Extent3D { width: size, height: size, depth: 1 })
            .mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let acc_image = unsafe { self.device.create_image(&acc_info, None) }
            .map_err(|e| format!("PT acc: {e}"))?;
        let acc_reqs = unsafe { self.device.get_image_memory_requirements(acc_image) };
        let acc_type = self.pick_memory_type(acc_reqs, true)?;
        let acc_alloc = vk::MemoryAllocateInfo::default().allocation_size(acc_reqs.size).memory_type_index(acc_type);
        let acc_mem = unsafe { self.device.allocate_memory(&acc_alloc, None) }
            .map_err(|e| format!("PT acc mem: {e}"))?;
        unsafe { self.device.bind_image_memory(acc_image, acc_mem, 0) }
            .map_err(|e| format!("PT acc bind: {e}"))?;
        let acc_view_info = vk::ImageViewCreateInfo::default()
            .image(acc_image).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::R32G32B32A32_SFLOAT)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
        let acc_view = unsafe { self.device.create_image_view(&acc_view_info, None) }
            .map_err(|e| format!("PT acc view: {e}"))?;
        // 描述符
        let pool_sizes = [
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(1),
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2),
            // STORAGE_BUFFER ×2：binding 2（盒材质）+ binding 4（道具逐三角属性表）
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(2),
            // COMBINED_IMAGE_SAMPLER ×2：binding 5（程序化地面纹理）+ binding 6（marker 皮肤）
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER).descriptor_count(2),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&pool_sizes);
        let dpool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("PT pool: {e}"))?;
        let dset_layouts = [set_layout_handle];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default().descriptor_pool(dpool).set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }
            .map_err(|e| format!("PT dset: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
            s_type: vk::StructureType::WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR,
            p_next: std::ptr::null(),
            acceleration_structure_count: 1,
            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),
            _marker: std::marker::PhantomData,
        };
        let img_info_desc = vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: view,
            image_layout: vk::ImageLayout::GENERAL,
        };
        let acc_info_desc = vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: acc_view,
            image_layout: vk::ImageLayout::GENERAL,
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
        let ground_desc = vk::DescriptorImageInfo {
            sampler: self.texture_sampler,
            image_view: self.texture_image_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        };
        // 🧱 binding 6 = marker 砌块皮肤（与光栅 binding 7 同一张图、同一个采样器）
        let skin_desc = vk::DescriptorImageInfo {
            sampler: self.texture_sampler,
            image_view: self.skin_marker_image_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
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
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 1, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&img_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 2, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&mat_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 3, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_IMAGE,
                p_image_info: std::slice::from_ref(&acc_info_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 4, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::STORAGE_BUFFER,
                p_image_info: std::ptr::null(), p_buffer_info: std::slice::from_ref(&propv_buf_info).as_ptr(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 5, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                p_image_info: std::slice::from_ref(&ground_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
            // 🧱 binding 6 = marker 砌块皮肤
            vk::WriteDescriptorSet {
                s_type: vk::StructureType::WRITE_DESCRIPTOR_SET,
                p_next: std::ptr::null(),
                dst_set: dset, dst_binding: 6, dst_array_element: 0, descriptor_count: 1,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                p_image_info: std::slice::from_ref(&skin_desc).as_ptr(), p_buffer_info: std::ptr::null(),
                p_texel_buffer_view: std::ptr::null(), _marker: std::marker::PhantomData,
            },
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        // 命令：AS 构建 + dispatch + 拷贝回读
        let alloc = vk::CommandBufferAllocateInfo::default().command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1);
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("PT cb: {e}"))?[0];
        let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device.begin_command_buffer(cb, &begin_info).map_err(|e| format!("PT cb begin: {e}"))?;
            self.record_pt_build(cb, &assets, boxes.len())?;
            let img_bar = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[img_bar]);
            let accel_bar = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[accel_bar], &[], &[]);
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            // 累积图像进 GENERAL（一次性；old_layout 用 UNDEFINED 只在首帧合法）
            let acc_bar0 = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(acc_image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[acc_bar0]);
            // 一次派发多帧：帧索引逐帧推进 => 采样去相关 => 输出收敛参考帧而非 1 spp 噪声图
            let spp = std::env::var("RV3D_PT_SPP")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| (1..=4096).contains(v))
                .unwrap_or(64u32);
            let self_dep = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE);
            for i in 0..spp {
                let pc = self.pt_params.pack(size, size, i, i == 0, spp, 0.0, (self.pt_box_count * 12) as u32);
                self.device.cmd_push_constants(cb, pipe_layout, vk::ShaderStageFlags::COMPUTE, 0, bytemuck_bytes(&pc));
                self.device.cmd_dispatch(cb, (size + 7) / 8, (size + 7) / 8, 1);
                if i + 1 < spp {
                    // 相邻 dispatch 读写同一累积像素，必须 compute->compute 自依赖 barrier
                    self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[self_dep], &[], &[]);
                }
            }
            log::info!("PT-VIEW: spp={}", spp);
            // 回读缓冲
            let (read_buf, read_mem) = self.create_host_buffer(vk::BufferUsageFlags::TRANSFER_DST, (size * size * 4) as u64)?;
            let img_bar2 = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .old_layout(vk::ImageLayout::GENERAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[img_bar2]);
            let cpy_regions = [vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width: size, height: size, depth: 1 })];
            self.device.cmd_copy_image_to_buffer(cb, image, vk::ImageLayout::GENERAL, read_buf, &cpy_regions);
            self.device.end_command_buffer(cb).map_err(|e| format!("PT cb end: {e}"))?;
            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("PT submit: {e}"))?;
            // 2026-08-29 修复：AS 构建必须在 dispatch 前完成（wait 保障 TLAS 可见）
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("PT wait: {e}"))?;
            // 读回 + PNG
            let m = self.device.map_memory(read_mem, 0, (size * size * 4) as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!("PT map: {e}"))?;
            let px: Vec<u8> = std::slice::from_raw_parts(m as *const u8, (size * size * 4) as usize).to_vec();
            self.device.unmap_memory(read_mem);
            log::info!("PT-VIEW px: [{},{},{}] [{},{},{}] [{},{},{}]", px[0], px[1], px[2], px[64*4], px[64*4+1], px[64*4+2], px[10*64*4+20*4], px[10*64*4+20*4+1], px[10*64*4+20*4+2]);
            // BMP 落盘（24bit，程序化写出，无依赖）
            {
                let row = size * 3;
                let pad = (4 - row % 4) % 4;
                let data_len = (row + pad) as usize * size as usize;
                let file_len = 54 + data_len;
                let mut bmp = Vec::with_capacity(file_len);
                bmp.extend_from_slice(b"BM");
                bmp.extend_from_slice(&(file_len as u32).to_le_bytes());
                bmp.extend_from_slice(&[0u8; 4]);
                bmp.extend_from_slice(&(54u32).to_le_bytes());
                bmp.extend_from_slice(&(40u32).to_le_bytes());
                bmp.extend_from_slice(&(size as i32).to_le_bytes());
                bmp.extend_from_slice(&(size as i32).to_le_bytes());
                bmp.push(1); bmp.push(24); bmp.push(0); bmp.push(0);
                bmp.extend_from_slice(&[0u8; 24]);
                for y in (0..size).rev() {
                    for x in 0..size {
                        let i = ((y * size + x) * 4) as usize;
                        // 逐像素 3 次 push 会被 clippy::same_item_push 误判成"重复推同一个
                        // 项"，而且这里本来就是"搬一段连续字节"，写成切片拷贝更贴原意。
                        // 字节序与顺序保持完全不变（BMP 那 3 个字节仍是 px 的前三分量）。
                        bmp.extend_from_slice(&px[i..i + 3]);
                    }
                    for _ in 0..pad { bmp.push(0); }
                }
                std::fs::write("screenshots/pt_ref.bmp", &bmp).map_err(|e| format!("PT bmp: {e}"))?;
            }
        }
        // 清理
        unsafe {
            self.device.destroy_pipeline(compute_pipeline, None);
            self.device.destroy_pipeline_layout(pipe_layout, None);
            self.device.destroy_descriptor_set_layout(set_layout_handle, None);
            self.device.destroy_descriptor_pool(dpool, None);
            self.device.destroy_shader_module(vs_module, None);
            self.device.free_command_buffers(self.command_pool, &[cb]);
            self.device.destroy_image_view(view, None);
            self.device.destroy_image(image, None);
            self.device.free_memory(img_mem, None);
            self.device.destroy_image_view(acc_view, None);
            self.device.destroy_image(acc_image, None);
            self.device.free_memory(acc_mem, None);
            let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
            ext.destroy_acceleration_structure(assets.tlas, None);
            ext.destroy_acceleration_structure(assets.blas, None);
        }
        Ok(())
    }
    /// RT 核心 纯求交吞吐基准（2026-08-29）：RT_BENCH_SPV 全遍历 × iterations
    /// 返回 (每秒射线 M, 命中数)
    pub(crate) fn run_pt_bench(
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
        let set_bindings = [set_layout, hits_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&set_bindings);
        let set_layout_handle = unsafe { self.device.create_descriptor_set_layout(&set_create, None) }
            .map_err(|e| format!("RT set layout: {e}"))?;
        let pipe_layouts = [set_layout_handle];
        let pc_ranges: [vk::PushConstantRange; 0] = [];
        let pipe_create = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&pipe_layouts)
            .push_constant_ranges(&pc_ranges);
        let pipe_layout = unsafe { self.device.create_pipeline_layout(&pipe_create, None) }
            .map_err(|e| format!("RT pipe layout: {e}"))?;
        let stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(vs_module)
            .name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage_info)
            .layout(pipe_layout);
        let pipelines = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("RT compute pipeline: {:?}", e.1))?
        };
        let compute_pipeline = pipelines[0];
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
        let pool_sizes = [dset_pool_info, dset_pool_info2];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let dpool = unsafe { self.device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("RT pool: {e}"))?;
        let dset_layouts = [set_layout_handle];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(dpool)
            .set_layouts(&dset_layouts);
        let dset = unsafe { self.device.allocate_descriptor_sets(&dset_alloc) }
            .map_err(|e| format!("RT dset: {e}"))?[0];
        let accel_write = vk::WriteDescriptorSetAccelerationStructureKHR {
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
            p_buffer_info: std::slice::from_ref(&buf_info).as_ptr(),
            p_texel_buffer_view: std::ptr::null(),
            _marker: std::marker::PhantomData,
        };
        unsafe { self.device.update_descriptor_sets(&[write0, write1], &[]) };
        // 5) 一次性构建命令（AS 构建）
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // 使用帧命令池外的一个独立分配
        let cb = unsafe { self.device.allocate_command_buffers(&alloc) }.map_err(|e| format!("pt cb: {e}"))?[0];
        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device.begin_command_buffer(cb, &begin_info).map_err(|e| format!("pt cb begin: {e}"))?;
            self.record_pt_build(cb, &assets, boxes.len())?;
            self.device.end_command_buffer(cb).map_err(|e| format!("pt cb end: {e}"))?;
        }
        let cbs = [cb];
        let submit = vk::SubmitInfo::default().command_buffers(&cbs);
        unsafe { self.device.queue_submit(self.graphics_queue, &[submit], vk::Fence::null()).map_err(|e| format!("pt submit: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("pt wait: {e}"))?;
        }
        // 6) 计时迭代：dispatch × iterations（单独 cmd，等待后计时）
        let t0 = std::time::Instant::now();
        unsafe {
            self.device.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).map_err(|e| format!("pt reset: {e}"))?;
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default()).map_err(|e| format!("pt bench begin: {e}"))?;
            self.device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, compute_pipeline);
            self.device.cmd_bind_descriptor_sets(cb, vk::PipelineBindPoint::COMPUTE, pipe_layout, 0, &[dset], &[]);
            for _ in 0..iterations {
                self.device.cmd_dispatch(cb, (n as u32 + 63) / 64, 1, 1);
            }
            self.device.end_command_buffer(cb).map_err(|e| format!("pt bench end: {e}"))?;
            let cbs2 = [cb];
            let submit2 = vk::SubmitInfo::default().command_buffers(&cbs2);
            self.device.queue_submit(self.graphics_queue, &[submit2], vk::Fence::null()).map_err(|e| format!("pt bench submit: {e}"))?;
            self.device.queue_wait_idle(self.graphics_queue).map_err(|e| format!("pt bench wait: {e}"))?;
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
    /// 记录 BLAS/TLAS 构建命令（一次性：命令缓冲执行）
    pub(crate) fn record_pt_build(
        &self,
        cmd: vk::CommandBuffer,
        assets: &crate::engine::ray_tracer::PtAssets,
        box_count: usize,
    ) -> Result<(), String> {
        let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);
        // scratch 归 PtAssets 所有（旧实现每次 record 都新建 2MB 且从不释放 = 显存泄漏源）；
        // BLAS 用前段、TLAS 用后段，两次构建不再共享同一地址。
        let scratch_base = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.scratch_buf); self.device.get_buffer_device_address(&i) };
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
        tri.max_vertex = (crate::engine::ray_tracer::PT_MAX_BOXES * 24 - 1) as u32;
        tri.vertex_data = vk::DeviceOrHostAddressConstKHR { device_address: vaddr };
        tri.vertex_stride = 32;
        tri.index_type = vk::IndexType::UINT32;
        tri.index_data = vk::DeviceOrHostAddressConstKHR { device_address: iaddr };
        tri.transform_data = vk::DeviceOrHostAddressConstKHR { device_address: 0 };
        let mut b_geo = vk::AccelerationStructureGeometryKHR::default();
        b_geo.geometry_type = vk::GeometryTypeKHR::TRIANGLES;
        b_geo.geometry = vk::AccelerationStructureGeometryDataKHR { triangles: tri };
        b_geo.flags = vk::GeometryFlagsKHR::OPAQUE;
        // 🏢 道具几何与 build_pt_as 创建 BLAS 时同一套引用（pt_prop_key 保证句柄/三角数
        //   一致，见 pt_set_scene_markers 的整体重建分支）
        let mut geos = vec![b_geo];
        let mut ranges = vec![
            vk::AccelerationStructureBuildRangeInfoKHR {
                primitive_count: (box_count * 12) as u32,
                primitive_offset: 0,
                first_vertex: 0,
                transform_offset: 0,
            },
        ];
        if assets.prop_tris > 0 {
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
            ranges.push(vk::AccelerationStructureBuildRangeInfoKHR {
                primitive_count: assets.prop_tris,
                primitive_offset: 0,
                first_vertex: 0,
                transform_offset: 0,
            });
        }
        b_geom.geometry_count = geos.len() as u32;
        b_geom.p_geometries = geos.as_ptr();
        b_geom.dst_acceleration_structure = assets.blas;
        b_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_base };
        b_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        // TLAS
        let mut t_geom = vk::AccelerationStructureBuildGeometryInfoKHR::default();
        t_geom.ty = vk::AccelerationStructureTypeKHR::TOP_LEVEL;
        t_geom.flags = vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;
        t_geom.geometry_count = 1;
        let mut inst_geo_data = vk::AccelerationStructureGeometryInstancesDataKHR::default();
        inst_geo_data.array_of_pointers = vk::FALSE;
        let inst_addr = unsafe { let i = vk::BufferDeviceAddressInfo::default().buffer(assets.inst_buf); self.device.get_buffer_device_address(&i) };
        inst_geo_data.data = vk::DeviceOrHostAddressConstKHR { device_address: inst_addr };
        let mut t_geo = vk::AccelerationStructureGeometryKHR::default();
        t_geo.geometry_type = vk::GeometryTypeKHR::INSTANCES;
        t_geo.geometry = vk::AccelerationStructureGeometryDataKHR { instances: inst_geo_data };
        t_geom.p_geometries = &t_geo;
        t_geom.dst_acceleration_structure = assets.tlas;
        t_geom.scratch_data = vk::DeviceOrHostAddressKHR { device_address: scratch_base + assets.scratch_blas };
        t_geom.mode = vk::BuildAccelerationStructureModeKHR::BUILD;
        let range_t = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 1, primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
        unsafe {
            let rbs: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [ranges.as_slice()];
            ext.cmd_build_acceleration_structures(cmd, &[b_geom], &rbs);
            // BLAS 写完 -> TLAS 读几何/引用其结果，两次构建之间必须有执行依赖
            let bb = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags::SHADER_READ);
            self.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &[bb],
                &[],
                &[],
            );
            let rt: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [range_t];
            let rts: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&rt];
            ext.cmd_build_acceleration_structures(cmd, &[t_geom], &rts);
        }
        Ok(())
    }
}
