// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 阴影建筑 LOD 专用几何上传（2026-09-19 专项，PROGRESS §14）。
    /// 整体重建、无持久映射；任何失败都只把 `prop_sh_index_count` 留 0，
    /// shadow loop 自动退回全量道具几何——**退化方向是"多画三角形"，不是缺阴影**。
    pub(crate) fn set_shadow_props(
        &mut self,
        set: &crate::engine::props::PropSet,
        placements: &[crate::engine::props::PropPlacement],
    ) {
        if !self.shadow_lod || std::env::var("RV3D_NO_PROPS").as_deref() == Ok("1") {
            return;
        }
        let merged = crate::engine::props::merge_shadow_binned(
            set,
            placements,
            PROP_BIN_CELL_M,
            |x, z| terrain_height_at(x, z),
        );
        if merged.verts.is_empty() || merged.indices.is_empty() {
            return;
        }
        let need_v = merged.verts.len() as u32;
        let need_i = merged.indices.len() as u32;
        let vsize = need_v as u64 * std::mem::size_of::<Vertex>() as u64;
        let isz = need_i as u64 * 4;
        unsafe {
            // 与主缓冲同一套安全规矩：旧缓冲可能正被 GPU 引用，先等空闲再销毁
            self.wait_idle_checked();
            for (buf, mem) in [
                (self.prop_sh_vertex_buffer, self.prop_sh_vertex_memory),
                (self.prop_sh_index_buffer, self.prop_sh_index_memory),
            ] {
                if buf != vk::Buffer::null() {
                    self.device.destroy_buffer(buf, None);
                }
                if mem != vk::DeviceMemory::null() {
                    self.device.free_memory(mem, None);
                }
            }
            self.prop_sh_vertex_buffer = vk::Buffer::null();
            self.prop_sh_index_buffer = vk::Buffer::null();
        }
        let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vsize) {
            Ok(v) => v,
            Err(e) => {
                log::error!("props/shadow: 顶点缓冲创建失败，阴影退回全量几何: {e}");
                return;
            }
        };
        let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, isz) {
            Ok(v) => v,
            Err(e) => {
                unsafe {
                    self.device.destroy_buffer(vb, None);
                    self.device.free_memory(vm, None);
                }
                log::error!("props/shadow: 索引缓冲创建失败，阴影退回全量几何: {e}");
                return;
            }
        };
        let mut ok = true;
        unsafe {
            match self.device.map_memory(vm, 0, vsize, vk::MemoryMapFlags::empty()) {
                Ok(ptr) => {
                    let dst = ptr as *mut Vertex;
                    for (i, v) in merged.verts.iter().enumerate() {
                        *dst.add(i) = Vertex {
                            pos: [v[0], v[1], v[2]],
                            color: [v[8], v[9], v[10]],
                            uv: [v[6], v[7]],
                        };
                    }
                    self.device.unmap_memory(vm);
                }
                Err(e) => {
                    log::error!("props/shadow: 顶点映射失败: {e}");
                    ok = false;
                }
            }
            if ok {
                match self.device.map_memory(im, 0, isz, vk::MemoryMapFlags::empty()) {
                    Ok(ptr) => {
                        std::ptr::copy_nonoverlapping(
                            merged.indices.as_ptr() as *const u8,
                            ptr as *mut u8,
                            merged.indices.len() * 4,
                        );
                        self.device.unmap_memory(im);
                    }
                    Err(e) => {
                        log::error!("props/shadow: 索引映射失败: {e}");
                        ok = false;
                    }
                }
            }
            if !ok {
                self.device.destroy_buffer(vb, None);
                self.device.destroy_buffer(ib, None);
                self.device.free_memory(vm, None);
                self.device.free_memory(im, None);
                return;
            }
        }
        self.prop_sh_vertex_buffer = vb;
        self.prop_sh_vertex_memory = vm;
        self.prop_sh_index_buffer = ib;
        self.prop_sh_index_memory = im;
        self.prop_sh_index_count = need_i;
        self.prop_sh_bins = merged.bins.clone();
        log::info!(
            "props/shadow: 建筑盒壳几何 顶点 {} / 三角 {} / 分桶 {} 个",
            need_v,
            need_i / 3,
            self.prop_sh_bins.len()
        );
    }
    pub(crate) fn init_shadow_resources(&mut self) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        // ---- 1. 阴影图 Image + 内存 + View（静态图 + 动态图，见字段注释）----
        let (shadow_image, shadow_image_memory, shadow_image_view) =
            self.create_shadow_map_image()?;
        self.shadow_image = shadow_image;
        self.shadow_image_memory = shadow_image_memory;
        self.shadow_image_view = shadow_image_view;
        let (dyn_image, dyn_memory, dyn_view) = self.create_shadow_map_image()?;
        self.shadow_dyn_image = dyn_image;
        self.shadow_dyn_image_memory = dyn_memory;
        self.shadow_dyn_image_view = dyn_view;

        // 两张图创建后**立刻**转成 SHADER_READ_ONLY_OPTIMAL：它们的描述符（binding 5 / 10）
        // 从第一帧起就按这个布局绑定，而每张图**不一定都会在第一帧被渲染**（关掉拆分时动态图
        // 永不渲染；检视模式下两张都不渲染）。不先转布局就采样一个仍停在 UNDEFINED 的图像 =
        // VUID-vkCmdDraw-None-08114（2026-09-26 实测：RV3D_NO_SHADOW_SPLIT=1 下 11 条）。
        // 转完之后的每次阴影 pass 都以 SHADER_READ_ONLY 为旧布局（见 record_shadow_pass）。
        let maps = [self.shadow_image, self.shadow_dyn_image];
        let barrier_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        self.run_single_time_commands(|cb| unsafe {
            let barriers: Vec<vk::ImageMemoryBarrier> = maps
                .iter()
                .map(|&img| {
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(img)
                        .subresource_range(barrier_range)
                        .src_access_mask(vk::AccessFlags::empty())
                        .dst_access_mask(vk::AccessFlags::SHADER_READ)
                })
                .collect();
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
        })?;

        // ---- 2. 阴影采样器（PCF：NEAREST + CLAMP_TO_EDGE）----
        // 手动 PCF 用 textureSample 读原始深度再比较，必须是普通采样器：
        // comparison sampler + 非 Dref 采样在严格 Vulkan 验证下会报 VUID。
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false)
            .compare_enable(false)
            .compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_lod(0.0)
            .max_lod(1.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
            .unnormalized_coordinates(false);
        self.shadow_sampler = unsafe {
            self.device
                .create_sampler(&sampler_info, None)
                .map_err(|e| format!("创建阴影采样器失败: {}", e))?
        };

        // 硬件 PCF 用的比较采样器（binding 11）：`textureSampleCompare` 要求
        // `compare_enable(true)`，过滤必须是 LINEAR —— 硬件 PCF 的"2x2 免费抽头"
        // 靠的就是这个双线性比较，用 NEAREST 会退化成单点比较（白改）。
        let cmp_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false)
            .compare_enable(true)
            .compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_lod(0.0)
            .max_lod(1.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
            .unnormalized_coordinates(false);
        self.shadow_cmp_sampler = unsafe {
            self.device
                .create_sampler(&cmp_info, None)
                .map_err(|e| format!("创建阴影比较采样器失败: {}", e))?
        };

        // ---- 3. depth-only render pass（无颜色附件，clear 1.0，store 供主 pass 采样）----
        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .depth_stencil_attachment(&depth_attachment_ref);
        let subpasses = [subpass];
        let attachments = [depth_attachment];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses);
        self.shadow_render_pass = unsafe {
            self.device
                .create_render_pass(&render_pass_info, None)
                .map_err(|e| format!("创建阴影渲染流程失败: {}", e))?
        };

        // ---- 4. framebuffer（单附件：阴影图 view）——静态图 + 动态图各一个 ----
        self.shadow_framebuffer = self.create_shadow_framebuffer(self.shadow_image_view)?;
        self.shadow_dyn_framebuffer = self.create_shadow_framebuffer(self.shadow_dyn_image_view)?;

        // ---- 5. shadow descriptor layout（binding 0 = UBO，binding 2 = 实例 storage）----
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let storage_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let shadow_bindings = [ubo_binding, storage_binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&shadow_bindings);
        self.shadow_descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| format!("创建阴影 Descriptor Set Layout 失败: {}", e))?
        };

        // ---- 6. shadow UBO（每帧 slot 一份 64B mat4）+ descriptor sets（从主 pool 分配）----
        let max_frames = self.max_frames_in_flight;
        for _ in 0..max_frames {
            let (buffer, memory, mapped) = self.create_uniform_buffer(
                std::mem::size_of::<glam::Mat4>() as u64,
            )?;
            self.shadow_ubo_buffers.push(buffer);
            self.shadow_ubo_memory.push(memory);
            self.shadow_ubo_mapped.push(mapped);
        }

        let layouts: Vec<vk::DescriptorSetLayout> = (0..max_frames)
            .map(|_| self.shadow_descriptor_set_layout)
            .collect();
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        self.shadow_descriptor_sets = unsafe {
            self.device
                .allocate_descriptor_sets(&alloc_info)
                .map_err(|e| format!("分配阴影 Descriptor Sets 失败: {}", e))?
        };

        // 阴影 pass 的实例范围同样用唯一定义。注意：这里此前连枪模槽（+1）都没覆盖，
        // 只是从没有 shader 在阴影里读那些槽所以没暴露；统一后一并修正。
        let instance_range =
            std::mem::size_of::<InstanceData>() as u64 * INSTANCE_BUFFER_ELEMS;
        for i in 0..max_frames {
            let ubo_info = vk::DescriptorBufferInfo::default()
                .buffer(self.shadow_ubo_buffers[i])
                .offset(0)
                .range(std::mem::size_of::<glam::Mat4>() as u64);
            let ubo_infos = [ubo_info];
            let ubo_write = vk::WriteDescriptorSet::default()
                .dst_set(self.shadow_descriptor_sets[i])
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&ubo_infos);
            let instance_info = vk::DescriptorBufferInfo::default()
                .buffer(self.instance_buffers[i])
                .offset(0)
                .range(instance_range);
            let instance_infos = [instance_info];
            let instance_write = vk::WriteDescriptorSet::default()
                .dst_set(self.shadow_descriptor_sets[i])
                .dst_binding(2)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&instance_infos);
            let writes = [ubo_write, instance_write];
            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }

        log::info!("阴影贴图资源创建完成: {}x{} D32_SFLOAT", SHADOW_MAP_SIZE, SHADOW_MAP_SIZE);
        Ok(())
    }
    /// 阴影 depth-only 管线：与主几何共享顶点/实例布局，无颜色附件；
    /// depth bias（constant 1.25 / slope 1.75）缓解斜面 shadow acne。
    pub(crate) fn init_shadow_pipeline(&mut self) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        let vs_spirv = load_spirv("assets/shadow.vert.spv")?;
        let fs_spirv = load_spirv("assets/shadow.frag.spv")?;
        let vs_module = self.create_shader_module(&vs_spirv)?;
        let fs_module = self.create_shader_module(&fs_spirv)?;

        let vs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs_module)
            .name(c"shadow_main");
        let fs_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs_module)
            .name(c"fs_main");
        let shader_stages = [vs_stage, fs_stage];

        let vertex_binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);
        // shadow VS 只读 position（location 0）；实例变换走 storage buffer（binding 2）
        let vertex_attributes = [vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0)];
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
            .width(SHADOW_MAP_SIZE as f32)
            .height(SHADOW_MAP_SIZE as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(vk::Extent2D {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
            });
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
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .min_depth_bounds(0.0)
            .max_depth_bounds(1.0);

        // 无颜色附件：color blend state 留空（Vulkan 对该场景忽略此状态）
        let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&[]);

        let set_layouts = [self.shadow_descriptor_set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts);
        self.shadow_pipeline_layout = unsafe {
            self.device
                .create_pipeline_layout(&layout_info, None)
                .map_err(|e| format!("创建阴影管线布局失败: {}", e))?
        };

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blend_state)
            .layout(self.shadow_pipeline_layout)
            .render_pass(self.shadow_render_pass)
            .subpass(0);

        self.shadow_pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info],
                    None,
                )
                .map_err(|(_, e)| format!("创建阴影管线失败: {}", e))?
                .remove(0)
        };

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }
        log::info!("阴影 depth-only 管线创建完成");
        Ok(())
    }
    /// 记录阴影 depth-only pass：布局转换（UNDEFINED → DEPTH_STENCIL_ATTACHMENT_OPTIMAL）
    /// → 渲几何到 2048x2048 阴影图 →（DEPTH_STENCIL_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL）。
    ///
    /// 🔴 **一次调用只画一类投射者**（`draw_static` / `draw_dynamic`），目标图由 `image` +
    /// `framebuffer` 指定 —— 默认路径是"静态图 + 动态图"两张（见 `shadow_dyn_image` 字段注释）：
    /// 静态那类隔一阵子画一次，动态那类每帧画。`RV3D_NO_SHADOW_SPLIT=1` 时调用方改成
    /// "单张图、两类一起画"，与拆分前逐帧等价。
    pub(crate) fn record_shadow_pass(
        &self,
        command_buffer: vk::CommandBuffer,
        near_count: u32,
        far_count: u32,
        terrain_lod: usize,
        image: vk::Image,
        framebuffer: vk::Framebuffer,
        draw_static: bool,
        draw_dynamic: bool,
    ) -> Result<(), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;

        // 🔬 诊断门（只为测量，不影响默认行为）：把阴影 pass 的**静态**投射者与**动态**投射者
        // 分开关掉，用来回答"阴影那 18% 里，静态占多少"。
        // 依据（2026-09-26 实测）：只关静态 ≈ 关掉整个阴影 pass（`skip_static` 与 `-NoShadow`
        // 同档），只关动态几乎不变 ⇒ 静态几何才是阴影开销的来源，这正是拆两张图的理由。
        // OnceLock：env 只读一次，热路径上没有分配。
        static SKIP_STATIC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        static SKIP_DYNAMIC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        // 第三个门：只跳过**地面实例场**（near/far 实例数置 0），保留地形网格/marker/道具。
        // 目的：地面场是 65536 实例的实例化 draw（顶点量级远大于道具的 246k 三角形），
        // 而它本身就是地面 —— 地面给自己投影几乎没有视觉意义。若它真是阴影 pass 的大头，
        // 那"把地面场从阴影 pass 拿掉"就是一行改动，比两张阴影图简单得多。
        static SKIP_GROUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let skip_ground =
            *SKIP_GROUND.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_GROUND").is_ok());
        let draw_static =
            draw_static && !*SKIP_STATIC.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_STATIC").is_ok());
        let draw_dynamic = draw_dynamic
            && !*SKIP_DYNAMIC.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_DYNAMIC").is_ok());
        // 第四/五个门（同一族诊断）：只跳过**地形网格**（257² 网格，High LOD ≈13 万三角形，
        // 量级最大）或只跳过地面实例场，用来分辨"阴影 pass 的顶点量到底花在哪一层地面上"。
        static SKIP_TERRAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let skip_terrain =
            *SKIP_TERRAIN.get_or_init(|| std::env::var("RV3D_SHADOW_SKIP_TERRAIN").is_ok());
        // 静态组：地形 / 地面实例场 / marker / 道具；动态组：NPC 盒柱球 / 士兵 GLB。
        // 用"把实例数置 0"实现跳过（`draw_shadow_range` 对 0 会早退），不动绘制逻辑本身。
        let (near_count, far_count) = if draw_static && !skip_ground {
            (near_count, far_count)
        } else {
            (0, 0)
        };
        let marker_near = if draw_static { self.last_marker_near } else { 0 };
        let marker_far = if draw_static { self.last_marker_far } else { 0 };
        let npc_box_near = if draw_dynamic { self.last_npc_box_near } else { 0 };
        let npc_box_far = if draw_dynamic { self.last_npc_box_far } else { 0 };
        let npc_cyl_near = if draw_dynamic { self.last_npc_cyl_near } else { 0 };
        let npc_cyl_far = if draw_dynamic { self.last_npc_cyl_far } else { 0 };
        let npc_sph_near = if draw_dynamic { self.last_npc_sph_near } else { 0 };
        let npc_sph_far = if draw_dynamic { self.last_npc_sph_far } else { 0 };
        let soldier_drawn = if draw_dynamic { self.soldier_drawn } else { 0 };
        let skip_static = !draw_static;

        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);

        // 进入 attachment 布局：旧布局固定 SHADER_READ_ONLY_OPTIMAL —— 两张图在 init 时就被
        // 转到这个布局（见 init_shadow_resources 的一次性 barrier），而拆分之后静态图可能隔着
        // 几十帧才重画，中间那些帧主 pass 一直在采样它（FRAGMENT_SHADER 读）。写之前必须让
        // 上一次的读可见/完成：srcStage=FRAGMENT_SHADER + srcAccess=SHADER_READ。
        let old_layout = vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL;
        let src_stage = vk::PipelineStageFlags::FRAGMENT_SHADER;
        let src_access = vk::AccessFlags::SHADER_READ;
        let to_attachment = vk::ImageMemoryBarrier::default()
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .src_access_mask(src_access)
            .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE);
        let to_attachment_barriers = [to_attachment];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                src_stage,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_attachment_barriers,
            );
        }

        // ---- shadow render pass：绑 shadow pipeline + shadow descriptor set ----
        let clear_depth = [vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        }];
        let shadow_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.shadow_render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: SHADOW_MAP_SIZE,
                    height: SHADOW_MAP_SIZE,
                },
            })
            .clear_values(&clear_depth);
        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &shadow_pass_info,
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.shadow_pipeline,
            );
            let shadow_sets = [self.shadow_descriptor_sets[self.current_frame]];
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.shadow_pipeline_layout,
                0,
                &shadow_sets,
                &[],
            );
        }

        // 地形（保留 identity 实例 = INSTANCE_COUNT，与主 pass 一致）
        let terrain_mesh = if skip_static || skip_terrain {
            None
        } else {
            self.terrain_lods.get(terrain_lod)
        };
        if let Some(mesh) = terrain_mesh {
            let terrain_vertex_buffers = [mesh.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &terrain_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    mesh.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    mesh.index_count,
                    1,
                    0,
                    0,
                    INSTANCE_COUNT,
                );
            }
        }

        // 地面实例场（近/远档，与主 pass 同一槽位布局）
        self.draw_shadow_range(
            command_buffer,
            self.ground_vertex_buffer,
            self.ground_index_buffer,
            GROUND_INDICES.len() as u32,
            near_count,
            0,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.ground_vertex_buffer,
            self.ground_index_buffer,
            GROUND_INDICES.len() as u32,
            far_count,
            near_count,
        )?;
        // marker
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            marker_near,
            MARKER_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            marker_far,
            MARKER_SLOT_BASE + marker_near,
        )?;
        // NPC 盒体区（躯干/脚/枪；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_box_near,
            NPC_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_box_far,
            NPC_SLOT_BASE + npc_box_near,
        )?;
        // NPC 圆柱区（四肢；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_cyl_near,
            NPC_CYL_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_cyl_far,
            NPC_CYL_SLOT_BASE + npc_cyl_near,
        )?;
        // NPC 球体区（头；阴影以盒体近似）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            npc_sph_near,
            NPC_SPH_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            npc_sph_far,
            NPC_SPH_SLOT_BASE + npc_sph_near,
        )?;
        // 🪖 士兵 GLB（2026-09-14 补）——**这条是补我自己的回归**。
        //
        // 上面那三对 `npc_box/cyl/sph` 是 18 段箱体的阴影近似。而 `set_npc_visuals`
        // 在 `soldier_on` 时**不再生成任何箱体段** ⇒ 那三对的实例数全是 0 ⇒
        // **士兵一度完全不投影**，而阴影 pass 不报任何错、画面上只是"人浮在地上"。
        //
        // 实例矩阵不用重算：`upload_soldiers` 已经把 N 个根变换写进
        // `SOLDIER_INSTANCE_BASE` 起的槽位，阴影 pass 只要用同一段槽位再画一遍即可。
        // `soldier_drawn` 为 0（网格没上传）时 `draw_shadow_range` 自己会早退。
        self.draw_shadow_range(
            command_buffer,
            self.soldier_vertex_buffer,
            self.soldier_index_buffer,
            self.soldier_index_count,
            soldier_drawn,
            SOLDIER_INSTANCE_BASE,
        )?;
        // 🌳 道具（2026-09-14 补）—— 此前**道具完全不投影**（未结案 #14 定案）。
        //
        // 树、楼、沙袋这些本来是场景里体积最大的一批几何，没有影子会让"东西贴在地上"
        // 这件事失去线索。主 pass 的 bin 循环就在 `record_command_buffer` 里，这里复刻它。
        //
        // ⚠️ **刻意不做视锥剔除。** 主 pass 用的是 `bin_visible(bin, &self.frame_frustum, …)`
        // ——那是**相机**视锥；而阴影 pass 覆盖的是**光源**视锥，两者是不同的体积。
        // 照抄那行会把"相机看不见、但在阴影图里"的道具剔掉 ⇒ **影子缺一块**，
        // 而且缺的位置随视角移动，是最难查的那类伪影。
        // 代价是多几十次 `cmd_draw_indexed`（全城约 9×9 桶），远低于一次剔除错判的代价。
        // ⚠️ **剔除必须用光源视锥，不能用相机视锥。** 主 pass 那行用的是
        // `bin_visible(bin, &self.frame_frustum, …)` —— 那是**相机**视锥；阴影 pass 覆盖的是
        // **光源**视锥，两者是不同的体积。照抄相机会把"相机看不见、但在阴影图里"的道具剔掉
        // ⇒ 影子缺一块，而且缺的位置随视角移动（最难查的那类伪影）。
        //
        // 但也不能不剔除：全画 81 个桶实测把帧率从约 250 压到 134。
        // 正解是**用光源自己的视锥剔**（`light_view_proj` 的 6 个平面），
        // 于是"正确"和"便宜"同时成立。margin 给 2m，与主 pass 同档。
        // 🏢 阴影建筑 LOD（2026-09-19 专项）：优先盒壳几何；未建成或
        // RV3D_SHADOW_LOD=0 时退回全量——退化方向是"多画三角形"，永不缺阴影。
        let use_lod = self.shadow_lod
            && self.prop_sh_index_count > 0
            && !self.prop_sh_bins.is_empty()
            && self.prop_sh_vertex_buffer != vk::Buffer::null()
            && self.prop_sh_index_buffer != vk::Buffer::null();
        let (sh_vb, sh_ib, sh_bins): (vk::Buffer, vk::Buffer, &[crate::engine::props::PropBin]) =
            if use_lod {
                (
                    self.prop_sh_vertex_buffer,
                    self.prop_sh_index_buffer,
                    &self.prop_sh_bins,
                )
            } else if self.prop_vertex_buffer != vk::Buffer::null()
                && self.prop_index_buffer != vk::Buffer::null()
            {
                (self.prop_vertex_buffer, self.prop_index_buffer, &self.prop_bins)
            } else {
                (vk::Buffer::null(), vk::Buffer::null(), &[])
            };
        if !skip_static && sh_ib != vk::Buffer::null() {
            let light_frustum =
                Self::extract_frustum_planes_from(self.light_data.shadow.light_view_proj);
            let bind_vb = [sh_vb];
            let prop_off = [0u64];
            unsafe {
                self.device
                    .cmd_bind_vertex_buffers(command_buffer, 0, &bind_vb, &prop_off);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    sh_ib,
                    0,
                    vk::IndexType::UINT32,
                );
            }
            for bin in sh_bins {
                if bin.index_count == 0 {
                    continue;
                }
                if !crate::engine::props::bin_visible(bin, &light_frustum, 2.0) {
                    continue;
                }
                unsafe {
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        bin.index_count,
                        1,
                        bin.first_index,
                        0,
                        PROP_INSTANCE_INDEX,
                    );
                }
            }
        }
        // 自发光（爆炸闪光等）
        self.draw_shadow_range(
            command_buffer,
            self.vertex_buffer,
            self.index_buffer,
            INDICES.len() as u32,
            self.last_emissive_near,
            EMISSIVE_SLOT_BASE,
        )?;
        self.draw_shadow_range(
            command_buffer,
            self.far_vertex_buffer,
            self.far_index_buffer,
            FAR_INDICES.len() as u32,
            self.last_emissive_far,
            EMISSIVE_SLOT_BASE + self.last_emissive_near,
        )?;

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
        }

        // DEPTH_STENCIL_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL（主 pass 采样）
        let to_read = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);
        let to_read_barriers = [to_read];
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_read_barriers,
            );
        }
        Ok(())
    }
    /// shadow pass 内的一次实例区 draw（bind 顶点/索引缓冲 + draw_indexed）。
    pub(crate) fn draw_shadow_range(
        &self,
        command_buffer: vk::CommandBuffer,
        vertex_buffer: vk::Buffer,
        index_buffer: vk::Buffer,
        index_count: u32,
        instance_count: u32,
        first_instance: u32,
    ) -> Result<(), String> {
        if instance_count == 0 {
            return Ok(());
        }
        let vertex_buffers = [vertex_buffer];
        let offsets = [0u64];
        unsafe {
            self.device.cmd_bind_vertex_buffers(
                command_buffer,
                0,
                &vertex_buffers,
                &offsets,
            );
            self.device.cmd_bind_index_buffer(
                command_buffer,
                index_buffer,
                0,
                vk::IndexType::UINT32,
            );
            self.device.cmd_draw_indexed(
                command_buffer,
                index_count,
                instance_count,
                0,
                0,
                first_instance,
            );
        }
        Ok(())
    }
}
