// 由 src/engine/renderer.rs 拆出（模块化，见 docs/refactor-plan.md）。
// SSAO 合成 pass（2026-10-22）：主 pass 之后、升采样 blit 之前的一次全屏 pass。
//
// 输入：① 离屏场景图（主 pass resolve 输出，TRANSFER_SRC）② 深度 resolve 图（D32，主 pass
// 以 MIN 模式解析出来的单采样深度）。输出：每交换链图像一张 `ssao_images`（格式同交换链，
// `finalLayout = TRANSFER_SRC_OPTIMAL`），随后那条升采样 blit 改为从它取源。
//
// **默认关**（`RV3D_SSAO=1` 才启用；不可用设备自动关闭）⇒ 未开启时渲染路径与旧版逐字节一致。
use super::*;

impl Renderer {
    /// SSAO 是否启用（默认关）。三条件：显式开关 + 设备支持深度 resolve + MSAA 开启。
    pub(crate) fn ssao_active(&self) -> bool {
        // LOW 画质档不跑 SSAO（见 helpers::ssao_should_run 的文档）
        ssao_should_run(self.ssao_enabled, self.quality)
    }

    /// 初始化 / 重建 SSAO 资源（幂等：重建路径会先拆旧的再建新的）。
    pub(crate) fn init_ssao(&mut self) -> Result<(), String> {
        if !self.ssao_enabled {
            return Ok(());
        }
        // 重建：拆掉按尺寸分配的资源（管线/布局/render pass/池只建一次）
        self.destroy_ssao_swapchain_dependent()?;

        for _ in 0..self.swapchain_images.len() {
            let create = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(self.swapchain_format)
                .extent(vk::Extent3D {
                    width: self.render_extent.width,
                    height: self.render_extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = unsafe {
                self.device
                    .create_image(&create, None)
                    .map_err(|e| format!("ssao image: {e}"))?
            };
            let req = unsafe { self.device.get_image_memory_requirements(image) };
            let mt = self.pick_memory_type(req, true)?;
            let ai = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(mt);
            let memory = unsafe {
                self.device
                    .allocate_memory(&ai, None)
                    .map_err(|e| format!("ssao mem: {e}"))?
            };
            unsafe {
                self.device
                    .bind_image_memory(image, memory, 0)
                    .map_err(|e| format!("ssao bind: {e}"))?;
            }
            let vi = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.swapchain_format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                );
            let view = unsafe {
                self.device
                    .create_image_view(&vi, None)
                    .map_err(|e| format!("ssao view: {e}"))?
            };
            self.ssao_images.push(image);
            self.ssao_image_memory.push(memory);
            self.ssao_image_views.push(view);
        }

        // render pass：1 个颜色附件，出口布局 TRANSFER_SRC（给 blit 当源）
        if self.ssao_render_pass == vk::RenderPass::null() {
            let att = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
            let atts = [att];
            let color_ref = vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let color_refs = [color_ref];
            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_refs);
            let subpasses = [subpass];
            let dep = vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
            let deps = [dep];
            let ci = vk::RenderPassCreateInfo::default()
                .attachments(&atts)
                .subpasses(&subpasses)
                .dependencies(&deps);
            self.ssao_render_pass = unsafe {
                self.device
                    .create_render_pass(&ci, None)
                    .map_err(|e| format!("ssao render pass: {e}"))?
            };
        }

        // 描述符集布局：两个 SAMPLED_IMAGE（场景色 + 深度），无采样器（着色器用 textureLoad）
        if self.ssao_set_layout == vk::DescriptorSetLayout::null() {
            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            let ci = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
            self.ssao_set_layout = unsafe {
                self.device
                    .create_descriptor_set_layout(&ci, None)
                    .map_err(|e| format!("ssao set layout: {e}"))?
            };
        }
        if self.ssao_pool == vk::DescriptorPool::null() {
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(16)];
            let ci = vk::DescriptorPoolCreateInfo::default()
                .pool_sizes(&sizes)
                .max_sets(8);
            self.ssao_pool = unsafe {
                self.device
                    .create_descriptor_pool(&ci, None)
                    .map_err(|e| format!("ssao pool: {e}"))?
            };
        }
        let layouts = vec![self.ssao_set_layout; self.ssao_image_views.len()];
        let sets = unsafe {
            self.device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(self.ssao_pool)
                        .set_layouts(&layouts),
                )
                .map_err(|e| format!("ssao sets: {e}"))?
        };
        for (i, &set) in sets.iter().enumerate() {
            let scene = vk::DescriptorImageInfo::default()
                .image_view(self.scene_image_views[i])
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            let depth = vk::DescriptorImageInfo::default()
                .image_view(self.depth_resolve_views[i])
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(std::slice::from_ref(&scene)),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(std::slice::from_ref(&depth)),
            ];
            unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        }
        self.ssao_sets = sets;

        // 管线（只建一次）
        if self.ssao_pipeline == vk::Pipeline::null() {
            self.init_ssao_pipeline()?;
        }

        self.ssao_framebuffers = self
            .ssao_image_views
            .iter()
            .map(|&v| {
                let ci = vk::FramebufferCreateInfo::default()
                    .render_pass(self.ssao_render_pass)
                    .attachments(std::slice::from_ref(&v))
                    .width(self.render_extent.width)
                    .height(self.render_extent.height)
                    .layers(1);
                unsafe {
                    self.device
                        .create_framebuffer(&ci, None)
                        .map_err(|e| format!("ssao fb: {e}"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        log::info!(
            "SSAO 合成 pass 就绪: {} 张 {}x{}",
            self.ssao_framebuffers.len(),
            self.render_extent.width,
            self.render_extent.height
        );
        Ok(())
    }

    fn init_ssao_pipeline(&mut self) -> Result<(), String> {
        let spirv = crate::shaders::SSAO_SPIRV.to_vec();
        let vs = self.create_shader_module(&spirv)?;
        let fs = self.create_shader_module(&spirv)?;
        let result = (|| -> Result<vk::Pipeline, String> {
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vs)
                    .name(c"vs_main"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fs)
                    .name(c"fs_main"),
            ];
            let push = vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .offset(0)
                .size(16);
            let set_layouts = [self.ssao_set_layout];
            let layout_ci = vk::PipelineLayoutCreateInfo::default()
                .set_layouts(&set_layouts)
                .push_constant_ranges(std::slice::from_ref(&push));
            let layout = unsafe {
                self.device
                    .create_pipeline_layout(&layout_ci, None)
                    .map_err(|e| format!("ssao layout: {e}"))?
            };
            let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let dynamic_ci = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
            let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
            let input_asm = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
            let viewport_state = vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);
            let raster = vk::PipelineRasterizationStateCreateInfo::default()
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::NONE)
                .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
                .line_width(1.0);
            let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);
            let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false);
            let color_blend = vk::PipelineColorBlendStateCreateInfo::default()
                .attachments(std::slice::from_ref(&color_blend_attachment));
            let ci = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_asm)
                .viewport_state(&viewport_state)
                .rasterization_state(&raster)
                .multisample_state(&multisample)
                .color_blend_state(&color_blend)
                .dynamic_state(&dynamic_ci)
                .layout(layout)
                .render_pass(self.ssao_render_pass)
                .subpass(0);
            let pipelines = unsafe {
                self.device
                    .create_graphics_pipelines(vk::PipelineCache::null(), &[ci], None)
                    .map_err(|(_, e)| format!("ssao pipeline: {e}"))?
            };
            self.ssao_pipeline_layout = layout;
            Ok(pipelines[0])
        })();
        unsafe {
            self.device.destroy_shader_module(vs, None);
            self.device.destroy_shader_module(fs, None);
        }
        self.ssao_pipeline = result?;
        Ok(())
    }

    /// 拆除按交换链尺寸分配的资源（重建交换链/内部渲染分辨率变化时调用）。
    pub(crate) fn destroy_ssao_swapchain_dependent(&mut self) -> Result<(), String> {
        for &fb in &self.ssao_framebuffers {
            unsafe { self.device.destroy_framebuffer(fb, None) };
        }
        self.ssao_framebuffers.clear();
        for &v in &self.ssao_image_views {
            unsafe { self.device.destroy_image_view(v, None) };
        }
        self.ssao_image_views.clear();
        for (&img, &mem) in self.ssao_images.iter().zip(self.ssao_image_memory.iter()) {
            unsafe {
                self.device.destroy_image(img, None);
                self.device.free_memory(mem, None);
            }
        }
        self.ssao_images.clear();
        self.ssao_image_memory.clear();
        if !self.ssao_sets.is_empty() && self.ssao_pool != vk::DescriptorPool::null() {
            unsafe {
                self.device
                    .free_descriptor_sets(self.ssao_pool, &self.ssao_sets)
                    .map_err(|e| format!("ssao free sets: {e}"))?;
            }
        }
        self.ssao_sets.clear();
        Ok(())
    }

    /// 记录 SSAO 合成 pass（主 pass 之后、升采样 blit 之前）。
    pub(crate) fn record_ssao(&self, command_buffer: vk::CommandBuffer, image_index: usize) {
        let proj = self.frame_proj;
        // glam perspective_rh: z_axis.z = far/(near-far), w_axis.z = near*far/(near-far)
        let m22 = proj.z_axis.z;
        let m32 = proj.w_axis.z;
        let near = (m32 / m22).abs().max(1e-4);
        let far = (m22 * near / (1.0 + m22)).abs().max(near + 1.0);
        let radius = crate::syscfg::cfg("RV3D_SSAO_RADIUS")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(14.0);
        let strength = crate::syscfg::cfg("RV3D_SSAO_STRENGTH")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.6);
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let scene_img = self.scene_images[image_index];
        let depth_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            ..range
        };
        let to_read = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(scene_img)
            .subresource_range(range);
        let depth_ready = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.depth_resolve_images[image_index])
            .subresource_range(depth_range);
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_read, depth_ready],
            );
            let clear = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            };
            let begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.ssao_render_pass)
                .framebuffer(self.ssao_framebuffers[image_index])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: self.render_extent,
                })
                .clear_values(std::slice::from_ref(&clear));
            self.device
                .cmd_begin_render_pass(command_buffer, &begin, vk::SubpassContents::INLINE);
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.ssao_pipeline,
            );
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: self.render_extent.width as f32,
                height: self.render_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            self.device
                .cmd_set_viewport(command_buffer, 0, std::slice::from_ref(&viewport));
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.render_extent,
            };
            self.device
                .cmd_set_scissor(command_buffer, 0, std::slice::from_ref(&scissor));
            let params = [near, far, radius, strength];
            self.device.cmd_push_constants(
                command_buffer,
                self.ssao_pipeline_layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck_bytes(&params),
            );
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.ssao_pipeline_layout,
                0,
                std::slice::from_ref(&self.ssao_sets[image_index]),
                &[],
            );
            self.device.cmd_draw(command_buffer, 3, 1, 0, 0);
            self.device.cmd_end_render_pass(command_buffer);
        }
    }
}
