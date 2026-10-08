// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    pub(crate) fn init_render_pass(&mut self) -> Result<(), String> {
        let msaa = self.msaa_samples;
        let color_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(msaa)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE) // 经 resolve 输出，自身不保留
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_attachment_refs = [color_attachment_ref];

        // 解析附件：MSAA 颜色 → 交换链图像（TYPE_1，最终呈现）
        let resolve_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let resolve_attachment_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        // 深度附件（D32_SFLOAT，与颜色同采样数）
        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(msaa)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(2)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let resolve_attachment_refs = [resolve_attachment_ref];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs)
            .resolve_attachments(&resolve_attachment_refs)
            .depth_stencil_attachment(&depth_attachment_ref);
        let subpasses = [subpass];
        let attachments = [color_attachment, resolve_attachment, depth_attachment];

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );
        let dependencies = [dependency];

        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        self.render_pass = unsafe {
            self.device
                .create_render_pass(&render_pass_create_info, None)
                .map_err(|e| format!("创建渲染流程失败: {}", e))?
        };
        Ok(())
    }
    /// MSAA 颜色附件（每交换链图像一个，samples=msaa_samples）。
    /// 主 pass 渲染到该附件，subpass resolve 输出到交换链图像；MSAA 关闭时跳过。
    pub(crate) fn init_msaa_resources(&mut self) -> Result<(), String> {
        self.msaa_images.clear();
        self.msaa_image_memory.clear();
        self.msaa_image_views.clear();
        if self.msaa_samples == vk::SampleCountFlags::TYPE_1 {
            return Ok(());
        }
        for _ in 0..self.swapchain_images.len() {
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(self.swapchain_format)
                .extent(vk::Extent3D {
                    width: self.swapchain_extent.width,
                    height: self.swapchain_extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(self.msaa_samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = unsafe {
                self.device
                    .create_image(&image_info, None)
                    .map_err(|e| format!("创建 MSAA 颜色 Image 失败: {}", e))?
            };
            let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
            let mem_type = self.pick_memory_type(mem_reqs, true)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(mem_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配 MSAA 颜色内存失败: {}", e))?
            };
            unsafe { self.device.bind_image_memory(image, memory, 0) }
                .map_err(|e| format!("绑定 MSAA 颜色内存失败: {}", e))?;
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(self.swapchain_format)
                .components(vk::ComponentMapping {
                    r: vk::ComponentSwizzle::IDENTITY,
                    g: vk::ComponentSwizzle::IDENTITY,
                    b: vk::ComponentSwizzle::IDENTITY,
                    a: vk::ComponentSwizzle::IDENTITY,
                })
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let view = unsafe {
                self.device
                    .create_image_view(&view_info, None)
                    .map_err(|e| format!("创建 MSAA 颜色 ImageView 失败: {}", e))?
            };
            self.msaa_images.push(image);
            self.msaa_image_memory.push(memory);
            self.msaa_image_views.push(view);
        }
        Ok(())
    }
    /// 为每个交换链图像创建深度缓冲（D32_SFLOAT Image + depth aspect ImageView）
    pub(crate) fn init_depth_resources(&mut self) -> Result<(), String> {
        let depth_format = vk::Format::D32_SFLOAT;
        self.depth_images.clear();
        self.depth_images_memory.clear();
        self.depth_image_views.clear();

        for _ in 0..self.swapchain_images.len() {
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(depth_format)
                .extent(vk::Extent3D {
                    width: self.swapchain_extent.width,
                    height: self.swapchain_extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(self.msaa_samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = unsafe {
                self.device
                    .create_image(&image_info, None)
                    .map_err(|e| format!("创建深度 Image 失败: {}", e))?
            };

            let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
            let memory_type = self.pick_memory_type(mem_reqs, true)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配深度 Image 内存失败: {}", e))?
            };
            unsafe {
                self.device
                    .bind_image_memory(image, memory, 0)
                    .map_err(|e| format!("绑定深度 Image 内存失败: {}", e))?;
            }

            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(depth_format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::DEPTH)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                );
            let view = unsafe {
                self.device
                    .create_image_view(&view_info, None)
                    .map_err(|e| format!("创建深度 Image View 失败: {}", e))?
            };

            self.depth_images.push(image);
            self.depth_images_memory.push(memory);
            self.depth_image_views.push(view);
        }
        log::info!(
            "深度缓冲创建完成: {} 张 {}x{} D32_SFLOAT",
            self.depth_images.len(),
            self.swapchain_extent.width,
            self.swapchain_extent.height
        );
        Ok(())
    }

    // ============================================================
    // 新增：初始化 Descriptor（Uniform Buffer + 布局 + 池 + 分配）
    // ============================================================
    /// ⛔ 传统 VERTEX 管线【已冻结维护】（2026-08-16）：仅 WSLg/dzn 无 VK_EXT_mesh_shader
    /// 时回退使用（地形 LOD 网格 + 地面实例场）。新渲染功能一律走 mesh 路径
    /// （init_mesh_pipeline），本管线不再新增功能。
    pub(crate) fn init_pipeline(&mut self) -> Result<(), String> {
        // 2026-08-28 终极修正：使用 build.rs 内嵌 SPIR-V（OUT_DIR/shaders.rs 常量），
        // 不再加载外置 assets/triangle.*.spv（两者曾长期不同步：外置为旧版，color 通道被 UV 顶替）
        let vs_spirv = crate::shaders::VS_SPIRV.to_vec();
        let fs_spirv = crate::shaders::FS_SPIRV.to_vec();
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_entry = c"vs_main";
        let fs_entry = c"fs_main";

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(vs_entry);
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(fs_entry);
        let shader_stages = [vs_stage, fs_stage];

        let vertex_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        let vertex_attributes = [
            // location 0: position vec3
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, pos) as u32),
            // location 1: color vec3
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, color) as u32),
            // location 2: uv vec2
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(std::mem::offset_of!(Vertex, uv) as u32),
        ];

        log::info!(
            "gun-attr: stride={} pos@{} color@{} uv@{}",
            std::mem::size_of::<Vertex>(),
            std::mem::offset_of!(Vertex, pos),
            std::mem::offset_of!(Vertex, color),
            std::mem::offset_of!(Vertex, uv)
        );
        let vertex_bindings = [vertex_binding];
        let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&vertex_attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            // 2026-09-04：主管线**打开深度测试**。此前它是 false，意味着当前唯一在跑的
            // legacy 路径完全没有深度遮挡，楼与楼只按绘制顺序互相穿透（mesh 管线一直是
            // 开的，所以这个差异只有在两条管线对比时才看得出来）。
            // 枪模对"不测深度"的依赖已拆到下面的 `gun_depth_stencil`，因此这里可以安全打开。
            // 保持 LESS_OR_EQUAL：项目里有大量刻意共面/零厚度的装饰件，用 LESS 会让它们
            // 被自己先前写入的深度挡住而闪烁。
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        // 枪模专用管线：不测深度、**也不写深度**。不写是必要的——否则枪模会把自身深度
        // 留在缓冲里，之后与它重叠的 HUD/粒子反而会被一把枪挡住。
        let gun_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        let color_write_mask = vk::ColorComponentFlags::R

            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A;
        // 2026-08-15：主 pass 开启 alpha 混合——现有几何 color.a 恒为 1.0（不受影响），
        // 自发光实体（爆炸等）设 alpha<1 即实现半透明（球形火光/冲击波可透出背景）
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(color_write_mask)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let color_blend_attachments = [color_blend_attachment];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments);

        // ---- 管线布局：挂上 descriptor_set_layout ----
        let set_layouts = [self.descriptor_set_layout];
        let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&[]);

        self.pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&pipeline_layout_create_info, None)
                .map_err(|e| format!("创建管线布局失败: {}", e))?
        };

        // 动态 viewport/scissor：resize 后每帧用当前 swapchain_extent 重设，
        // 避免全屏/窗口变化后画面卡在旧尺寸左上角（2026-08-15 修复）
        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
        ];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&dynamic_states);

        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .dynamic_state(&dynamic_state)
            .color_blend_state(&color_blend_state)
            .layout(self.pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_create_info], None)
                .map_err(|(_, e)| format!("创建图形管线失败: {}", e))?
                .remove(0)
        };

        // 枪模管线：除 depth 状态外与主管线逐字段相同。必须在销毁 shader module **之前**
        // 创建——create_graphics_pipelines 是同步的，模块在返回后即可释放。
        let gun_pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&gun_depth_stencil)
            .dynamic_state(&dynamic_state)
            .color_blend_state(&color_blend_state)
            .layout(self.pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.gun_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[gun_pipeline_create_info], None)
                .map_err(|(_, e)| format!("创建枪模管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        self.create_vertex_buffer()?;
        self.create_index_buffer()?;
        self.create_far_geometry()?;
        self.create_ground_geometry()?;
        self.create_sphere_geometry()?;
        self.create_cylinder_geometry()?;
        self.create_terrain_lods()?;
        log::info!("图形管线创建完成");
        Ok(())
    }
    /// 可选网格着色器管线（VK_EXT_mesh_shader）：
    /// - 阶段 = MESH_EXT + FRAGMENT（片元着色器与主管线同一模块，原样复用）；
    /// - 无 vertex input state / input assembly state（VK_EXT_mesh_shader 要求二者为 NULL）；
    /// - rasterization（Back cull + CLOCKWISE）/ depth / blend / viewport 与主管线完全一致；
    /// - pipeline layout 复用同一 descriptor set layout，仅追加 MESH_EXT push constant
    ///   （base_slot，16 字节）；传统管线共用同一 descriptor set layout 不受影响。
    /// mesh_enabled=false（设备没有 VK_EXT_mesh_shader）时直接返回，不加载 mesh.spv、不创建任何资源。
    pub(crate) fn init_mesh_pipeline(&mut self) -> Result<(), String> {
        if !self.mesh_enabled {
            return Ok(());
        }
        // maxMeshWorkGroupCount[0]：地面场 65536 workgroup 超最低保证 65535，须分块绘制。
        let mut mesh_props = vk::PhysicalDeviceMeshShaderPropertiesEXT::default();
        let mut p2 = vk::PhysicalDeviceProperties2::default();
        p2.p_next = &mut mesh_props as *mut _ as *mut std::ffi::c_void;
        unsafe {
            self.instance
                .get_physical_device_properties2(self.physical_device, &mut p2);
        }
        self.mesh_max_wg_x = mesh_props.max_mesh_work_group_count[0].max(1);
        log::info!(
            "网格着色器 maxMeshWorkGroupCount[0] = {}（地面场 65536 按此分块）",
            self.mesh_max_wg_x
        );
        let mesh_spirv = load_spirv("assets/mesh.spv")?;
        let fs_spirv = load_spirv("assets/triangle.frag.spv")?;
        let mesh_module = self.create_shader_module(&mesh_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let mesh_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::MESH_EXT)
            .module(mesh_module)
            .name(c"mesh_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [mesh_stage, fs_stage];

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        let color_write_mask = vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A;
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(color_write_mask)
            .blend_enable(false);
        let color_blend_attachments = [color_blend_attachment];
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments);

        // 同一 descriptor set layout + MESH_EXT push constant（base_slot，16 字节）
        let set_layouts = [self.descriptor_set_layout];
        let push_constant = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::MESH_EXT)
            .offset(0)
            .size(16);
        let mesh_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(std::slice::from_ref(&push_constant));
        self.mesh_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&mesh_layout_info, None)
                .map_err(|e| format!("创建网格管线布局失败: {}", e))?
        };

        // mesh 管线：pVertexInputState / pInputAssemblyState 必须为 NULL（ash 默认即 null）
        let mesh_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let mesh_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&mesh_dynamic_states);
        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend_state)
            .dynamic_state(&mesh_dynamic_state)
            .layout(self.mesh_pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        self.mesh_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_create_info],
                    None,
                )
                .map_err(|(_, e)| format!("创建网格着色器管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(mesh_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        log::info!("网格着色器管线创建完成（VK_EXT_mesh_shader）");
        Ok(())
    }
    /// 初始化 HUD 覆盖层：自包含 pipeline（无描述符、depth off、alpha 混合）+ 独立 HOST_VISIBLE 顶点缓冲
    pub(crate) fn init_hud(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/hud.vert.spv")?;
        let fs_spirv = load_spirv("assets/hud.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"vs_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let hud_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<HudVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        let hud_attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
            // 🧊 location 2 = `uv_glass`（vec3，见 `HudVertex`）。**与 overlay 那份必须一致**。
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<HudVertex>() as u32 - std::mem::size_of::<[f32; 3]>() as u32),
        ];
        let hud_bindings = [hud_binding];
        let hud_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&hud_bindings)
            .vertex_attribute_descriptions(&hud_attributes);

        let hud_input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let hud_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let hud_scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let hud_viewports = [hud_viewport];
        let hud_scissors = [hud_scissor];
        let hud_viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&hud_viewports)
            .scissors(&hud_scissors);

        let hud_rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let hud_multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(self.msaa_samples);

        let hud_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::ALWAYS);

        let hud_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let hud_blend_attachments = [hud_blend_attachment];
        let hud_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&hud_blend_attachments);

        // 独立 pipeline layout：🧊 只带一个 set（HUD 玻璃：`glass_tex` + `glass_smp`）。
        // 🔴 **两个 HUD 管线共用这一个 layout**（主 pass 的 `hud_pipeline` 与 overlay 的），
        // 而着色器**静态**引用了 binding 0/1 ⇒ 两边都必须带这套 set（运行时 `glass=0` 也照样
        // 需要有合法描述符绑定，见 `set_hud_quads` 与两处 HUD 绘制里的 `cmd_bind_descriptor_sets`）。
        let hud_set_layouts = [self.hud_glass_set_layout];
        let hud_layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&hud_set_layouts)
            .push_constant_ranges(&[]);
        self.hud_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&hud_layout_info, None)
                .map_err(|e| format!("创建 HUD 管线布局失败: {}", e))?
        };

        let hud_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let hud_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&hud_dynamic_states);
        let hud_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&hud_vertex_input)
            .input_assembly_state(&hud_input_assembly)
            .viewport_state(&hud_viewport_state)
            .rasterization_state(&hud_rasterizer)
            .multisample_state(&hud_multisampling)
            .depth_stencil_state(&hud_depth_stencil)
            .color_blend_state(&hud_blend_state)
            .dynamic_state(&hud_dynamic_state)
            .layout(self.hud_pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);
        self.hud_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[hud_create_info], None)
                .map_err(|(_, e)| format!("创建 HUD 图形管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        // 独立 HOST_VISIBLE 顶点缓冲（容量 4096 quad × 6 顶点 × 24B）
        let hud_size =
            (self.hud_capacity_quads as usize * 6 * std::mem::size_of::<HudVertex>()) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, hud_size)?;
        self.hud_vertex_buffer = buffer;
        self.hud_vertex_buffer_memory = memory;
        self.hud_mapped = unsafe {
            self.device
                .map_memory(memory, 0, hud_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 HUD 顶点缓冲失败: {}", e))?
        };
        log::info!(
            "HUD 覆盖层初始化完成（独立 pipeline，容量 {} quads）",
            self.hud_capacity_quads
        );
        Ok(())
    }
    /// PT 覆盖后重绘 HUD 的 overlay pass（load=LOAD 保留 PT 画面！2026-09-01）
    ///
    /// 🔴 **2026-09-15 修了三处**（都由验证层在 PT 打开时抓出，见未结案 #2 的后续）：
    /// ① `initialLayout` 曾是 `PRESENT_SRC_KHR`，而调用方在 `cmd_begin_render_pass` **之前**
    ///    已经把图手动转成了 `COLOR_ATTACHMENT_OPTIMAL` ⇒
    ///    `VUID-vkCmdBeginRenderPass-initialLayout-00900`（"初始布局必须等于当前布局"）。
    ///    现在声明成 `COLOR_ATTACHMENT_OPTIMAL`，与那次手动 barrier 对齐；
    ///    `finalLayout` 仍是 `PRESENT_SRC_KHR`，由 render pass 自己做最后那次转换。
    /// ② HUD 覆盖层以前**复用主 pass 的 `hud_pipeline`**（那是 MSAA 4x + 带深度附件的管线），
    ///    而 overlay pass 是 1 采样、无深度 ⇒ `VUID-vkCmdDraw-renderPass-02684`（管线与当前
    ///    render pass 不兼容 = UB）。现在单独建一条 `hud_overlay_pipeline`。
    /// ③ 收尾那次 `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` 的 barrier 是多余的，
    ///    而且与 render pass 的 `finalLayout` 撞车 ⇒ `VUID-VkImageMemoryBarrier-oldLayout-01197`。
    ///    已删除（转换由 render pass 负责）。
    pub(crate) fn init_hud_overlay(&mut self) -> Result<(), String> {
        unsafe {
            let color_attachment = vk::AttachmentDescription::default()
                .format(self.swapchain_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
            let color_refs = [vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_refs);
            let attachments = [color_attachment];
            let subpasses = [subpass];
            let rp_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses);
            self.hud_render_pass = self.device.create_render_pass(&rp_info, None)
                .map_err(|e| format!("hud rp: {e}"))?;
        }
        self.create_hud_overlay_pipeline()?;
        self.recreate_hud_framebuffers()
    }
    /// 建 **HUD overlay 专用**图形管线：与 `hud_pipeline` 同着色器/同顶点格式/同混合，
    /// 但 `render_pass = hud_render_pass`、**1 采样、无深度附件** —— 渲染状态必须与 render pass
    /// 逐项兼容，复用主 pass 的管线就是 `VUID-vkCmdDraw-renderPass-02684`（UB）。
    /// 只借用 `hud_pipeline_layout`（同一套着色器 ⇒ 同一套布局，空描述符集 + 空 push constant）。
    pub(crate) fn create_hud_overlay_pipeline(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/hud.vert.spv")?;
        let fs_spirv = load_spirv("assets/hud.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;
        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"vs_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let hud_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<HudVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        let hud_attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
            // 🧊 location 2 = `uv_glass`（vec3）：磨砂玻璃的屏幕 UV + 标志位。
            // ⚠️ **两个 HUD 管线（主 pass 的 `hud_pipeline` 与 overlay 的 `hud_overlay_pipeline`）
            // 必须同时加**：它们共用 `hud_pipeline_layout` / 同一套着色器，只加一处 =
            // 另一处按 24B 步长解读 36B 顶点 = 静默错位的几何。
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<HudVertex>() as u32 - std::mem::size_of::<[f32; 3]>() as u32),
        ];
        let hud_bindings = [hud_binding];
        let hud_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&hud_bindings)
            .vertex_attribute_descriptions(&hud_attributes);
        let hud_input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);
        let hud_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let hud_scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        let hud_viewports = [hud_viewport];
        let hud_scissors = [hud_scissor];
        let hud_viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&hud_viewports)
            .scissors(&hud_scissors);
        let hud_rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);
        // 🔴 这一行是本次修法的关键：overlay pass 只有 1 个采样，不是主 pass 的 MSAA 数
        let hud_multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let hud_depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::ALWAYS);
        let hud_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD);
        let hud_blend_attachments = [hud_blend_attachment];
        let hud_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&hud_blend_attachments);
        let hud_dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let hud_dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&hud_dynamic_states);
        let create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&hud_vertex_input)
            .input_assembly_state(&hud_input_assembly)
            .viewport_state(&hud_viewport_state)
            .rasterization_state(&hud_rasterizer)
            .multisample_state(&hud_multisampling)
            .depth_stencil_state(&hud_depth_stencil)
            .color_blend_state(&hud_blend_state)
            .dynamic_state(&hud_dynamic_state)
            .layout(self.hud_pipeline_layout)
            .render_pass(self.hud_render_pass)
            .subpass(0);
        let result = unsafe {
            self.device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[create_info], None)
        };
        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        self.hud_overlay_pipeline =
            result.map_err(|(_, e)| format!("创建 HUD overlay 管线失败: {}", e))?.remove(0);
        Ok(())
    }
    /// 按**当前** `swapchain_image_views` 重建 HUD overlay 的 framebuffer。
    ///
    /// 🔴 **这就是未结案 #2「PT 一启动即 0xC0000005」的根因（2026-09-15 由验证层抓出）**：
    /// `hud_framebuffers` 原本只在 `init_hud_overlay`（启动时一次）里创建，而
    /// `destroy_swapchain` 会**销毁它依赖的 `swapchain_image_views`** 却不重建它们。
    /// 启动阶段就有 **5 次** swapchain 重建（resize 事件），所以这组 framebuffer 从很早就
    /// 指向**已销毁的 ImageView**；而它唯一的消费者是 **PT 通路**（PT 画完再叠 HUD），
    /// 光栅路径走 `self.framebuffers`（那次是重建过的）——
    /// ⇒ 症状正好是"**光栅一切正常、一开 PT 就崩**"，而且崩因与 PT 本身毫无关系。
    ///
    /// 验证层原话：
    /// ```text
    /// vkCmdBeginRenderPass(): pCreateInfo->pAttachments[0] VkImageView 0x70000000007 is invalid.
    /// VUID-VkRenderPassBeginInfo-framebuffer-parameter
    /// ```
    pub(crate) fn recreate_hud_framebuffers(&mut self) -> Result<(), String> {
        if self.hud_render_pass == vk::RenderPass::null() {
            return Ok(()); // HUD overlay 未启用（无 HUD 管线），无需 framebuffer
        }
        for &framebuffer in &self.hud_framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.hud_framebuffers = self
            .swapchain_image_views
            .iter()
            .map(|&iv| {
                let fbi = vk::FramebufferCreateInfo::default()
                    .render_pass(self.hud_render_pass)
                    .attachments(std::slice::from_ref(&iv))
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe { self.device.create_framebuffer(&fbi, None) }
                    .map_err(|e| format!("hud fb: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }
    pub(crate) fn init_framebuffers(&mut self) -> Result<(), String> {
        self.framebuffers = self
            .swapchain_image_views
            .iter()
            .enumerate()
            .map(|(i, &image_view)| {
                // MSAA：attachments = [msaa 颜色, 交换链（resolve 目标）, 深度]；
                // 关闭时 msaa view 即交换链本身（TYPE_1，无独立附件）
                let msaa_view = if self.msaa_samples == vk::SampleCountFlags::TYPE_1 {
                    image_view
                } else {
                    self.msaa_image_views[i]
                };
                let attachments = [msaa_view, image_view, self.depth_image_views[i]];
                let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&attachments)
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe {
                    self.device
                        .create_framebuffer(&framebuffer_create_info, None)
                        .map_err(|e| format!("创建帧缓冲失败: {e}"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }
}
