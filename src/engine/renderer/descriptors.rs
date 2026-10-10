// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    pub(crate) fn init_descriptors(&mut self) -> Result<(), String> {
        let max_frames = self.max_frames_in_flight;

        // ---- 1. 创建 Descriptor Set Layout ----
        // 描述：binding=0, 类型=UNIFORM_BUFFER, 阶段=VERTEX（mesh 路径额外 +MESH_EXT）
        let ubo_stage_flags = if self.mesh_enabled {
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT
        } else {
            vk::ShaderStageFlags::VERTEX
        };
        let ubo_layout_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(ubo_stage_flags);
        // 纹理采样（贴图 binding=1，采样器 binding=3，均只在 Fragment 阶段使用）
        let sampled_image_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 实例 storage buffer（binding=2，Vertex 阶段读取；mesh 路径额外 +MESH_EXT）
        let storage_stage_flags = if self.mesh_enabled {
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT
        } else {
            vk::ShaderStageFlags::VERTEX
        };
        let storage_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(storage_stage_flags);
        let sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 光照 Uniform（binding=4，Fragment 阶段读取；默认全零 = 关闭）
        let light_ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(LIGHT_UBO_BINDING)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 阴影贴图（binding=5 SAMPLED_IMAGE、binding=6 SAMPLER，均 Fragment 阶段采样）
        let shadow_map_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(5)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let shadow_sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(6)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 硬件 PCF 的**比较采样器**（binding=11 SAMPLER，Fragment）。
        // 为什么另开一个而不是改 binding 6：调试视图要读原始深度，比较采样器给不了。
        // 🔴 加它的同时**必须**把池的 SAMPLER 计数 +1（见下面 pool_sizes），
        //    否则该 set 分配失败 = 启动即报错。
        let shadow_cmp_sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(11)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // marker/NPC 程序化皮肤纹理（binding=7/8 SAMPLED_IMAGE，Fragment 采样；
        // RV3D_SKIN_TEX=1 启用，缺省 0 纯色回退。绑定号必须与 build.rs WGSL 同步）
        let marker_skin_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(7)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let npc_skin_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(8)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 地面微细节层（binding=9；build.rs 片元 `ground_detail_tex`，**无条件采样**，
        // 不受 RV3D_SKIN_TEX 门控）。漏掉这个绑定 = 采样恒 0 = 相机周边地面纯黑，
        // 详见字段 `ground_detail_image` 的注释。
        let ground_detail_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(GROUND_DETAIL_BINDING)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        // 动态阴影图（binding=10；build.rs 片元 `shadow_dyn_map`，静态图是 binding 5）
        let shadow_dyn_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(SHADOW_DYN_BINDING)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [
            ubo_layout_binding,
            sampled_image_binding,
            storage_binding,
            sampler_binding,
            light_ubo_binding,
            shadow_map_binding,
            shadow_sampler_binding,
            shadow_cmp_sampler_binding,
            marker_skin_binding,
            npc_skin_binding,
            ground_detail_binding,
            shadow_dyn_binding,
        ];

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&bindings);

        self.descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| format!("创建 Descriptor Set Layout 失败: {}", e))?
        };
        // binding 0 = view/proj UBO；binding 1 = 贴图；binding 2 = 实例 storage buffer；
        // 原采样器 binding 2 顺延到 binding 3（与 WGSL 一致）。
        log::info!(
            "Descriptor Set Layout: binding 0 = UBO(view/proj), binding 1 = 贴图, binding 2 = 实例 STORAGE_BUFFER, binding 3 = 采样器"
        );

        // ---- 2. 创建 Uniform Buffer（每帧一个）----
        let buffer_size = std::mem::size_of::<CameraUniform>() as u64;

        for _ in 0..max_frames {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(buffer_size)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);

            let buffer = unsafe {
                self.device
                    .create_buffer(&buffer_info, None)
                    .map_err(|e| format!("创建 Uniform Buffer 失败: {}", e))?
            };

            let mem_requirements = unsafe {
                self.device.get_buffer_memory_requirements(buffer)
            };

            let mem_properties = unsafe {
                self.instance
                    .get_physical_device_memory_properties(self.physical_device)
            };

            let memory_type = mem_properties
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (mem_requirements.memory_type_bits & type_mask) != 0
                        && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                        && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
                })
                .map(|(i, _)| i as u32)
                .ok_or_else(|| "没有找到合适的内存类型（Uniform Buffer）".to_string())?;

            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_requirements.size)
                .memory_type_index(memory_type);

            let buffer_memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配 Uniform Buffer 内存失败: {}", e))?
            };

            unsafe {
                self.device
                    .bind_buffer_memory(buffer, buffer_memory, 0)
                    .map_err(|e| format!("绑定 Uniform Buffer 内存失败: {}", e))?;
            }

            // 持久映射（map 一次，之后每帧直接写）
            let mapped = unsafe {
                self.device
                    .map_memory(buffer_memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                    .map_err(|e| format!("映射 Uniform Buffer 内存失败: {}", e))?
            };

            self.uniform_buffers.push(buffer);
            self.uniform_buffers_memory.push(buffer_memory);
            self.uniform_mapped.push(mapped);
        }

        // ---- 2b. 创建光照 Uniform Buffer（每帧一份，默认全零 = 光照关闭）----
        let light_ubo_size = std::mem::size_of::<LightUniform>() as u64;
        for _ in 0..max_frames {
            let (buffer, buffer_memory, mapped) = self.create_uniform_buffer(light_ubo_size)?;
            self.light_uniform_buffers.push(buffer);
            self.light_uniform_buffers_memory.push(buffer_memory);
            self.light_uniform_mapped.push(mapped);
        }

        // ---- 3. 创建 Descriptor Pool ----
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count((max_frames * 3) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                // binding 1 地面贴图 + binding 5 静态阴影图 + binding 7/8 marker/NPC 皮肤纹理
                // + binding 9 地面微细节层 + binding 10 动态阴影图（缺一个 = 该 set 分配失败
                // → 启动即报错）。加采样图绑定**必须同时改这个数**。
                .descriptor_count((max_frames * 6) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count((max_frames * 2) as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                // binding 3 贴图采样器 + binding 6 阴影普通采样器 + **binding 11 阴影比较采样器**
                .descriptor_count((max_frames * 3) as u32),
        ];

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets((max_frames * 2) as u32);

        self.descriptor_pool = unsafe {
            self.device
                .create_descriptor_pool(&pool_info, None)
                .map_err(|e| format!("创建 Descriptor Pool 失败: {}", e))?
        };

        // ---- 4. 分配 Descriptor Sets ----
        let layouts: Vec<vk::DescriptorSetLayout> = (0..max_frames)
            .map(|_| self.descriptor_set_layout)
            .collect();

        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);

        self.descriptor_sets = unsafe {
            self.device
                .allocate_descriptor_sets(&alloc_info)
                .map_err(|e| format!("分配 Descriptor Sets 失败: {}", e))?
        };

        // ---- 5. 更新 Descriptor Sets（把 buffer 绑到 set 上）----
        for i in 0..max_frames {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(self.uniform_buffers[i])
                .offset(0)
                .range(buffer_size);
            let buffer_infos = [buffer_info];

            let descriptor_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_infos);
            let descriptor_writes = [descriptor_write];

            unsafe {
                self.device.update_descriptor_sets(&descriptor_writes, &[]);
            }

            // 实例 storage buffer（每帧 set 指向该帧自己的 buffer，消除读写竞态）
            let instance_info = vk::DescriptorBufferInfo::default()
                .buffer(self.instance_buffers[i])
                .offset(0)
                // 范围必须覆盖到最高槽位（道具 identity 槽），否则 shader 读该 slot 会
                // 越界——驱动不报错，只返回全零，几何会静默消失。见 INSTANCE_BUFFER_ELEMS。
                .range(std::mem::size_of::<InstanceData>() as u64 * INSTANCE_BUFFER_ELEMS);
            let instance_infos = [instance_info];
            let instance_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(2)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&instance_infos);
            let instance_writes = [instance_write];
            unsafe {
                self.device.update_descriptor_sets(&instance_writes, &[]);
            }

            // 光照 Uniform（默认全零 = 关闭）
            let light_info = vk::DescriptorBufferInfo::default()
                .buffer(self.light_uniform_buffers[i])
                .offset(0)
                .range(light_ubo_size);
            let light_infos = [light_info];
            let light_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(LIGHT_UBO_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&light_infos);
            let light_writes = [light_write];
            unsafe {
                self.device.update_descriptor_sets(&light_writes, &[]);
            }
        }

        log::info!("Descriptor 初始化完成（{} 帧）", max_frames);
        Ok(())
    }
    /// 上传 HUD quad 列表（屏幕像素坐标 → NDC 顶点）；render 前调用，随主 command buffer 绘制
    pub(crate) fn set_hud_quads(&mut self, quads: &[crate::ui::Quad]) {
        let (w, h) = (
            self.swapchain_extent.width.max(1) as f32,
            self.swapchain_extent.height.max(1) as f32,
        );
        let count = quads.len().min(self.hud_capacity_quads as usize);
        // 2026-09-22 复查补：HUD 是全仓**最后一处静默截断** —— NPC / 尸体 / 道具那几处
        // 早就有一次性告警闩（`warn_npc_cap_once` 形态），只有这里超容不提示。
        // 仍按容量截断（行为不变，防越界写映射内存），但把"少画了东西"变成可诊断的一行日志。
        if quads.len() > self.hud_capacity_quads as usize {
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "HUD quad 超容：需要 {} 个，容量 {}，超出部分本帧不绘制（一次性告警）",
                    quads.len(),
                    self.hud_capacity_quads
                );
            }
        }
        self.hud_vertex_count = (count * 6) as u32;
        // 🧊 本帧有没有磨砂玻璃面板：有 ⇒ `record_command_buffer` 会把 HUD 挪到
        // overlay pass、并在它之前把交换链降采样成模糊图（见 `MENU_BLUR_W`）。
        // 判定放在这里（唯一的上传入口），main.rs / ui.rs 都不需要额外接线。
        //
        // 🔴 PT 模式下**一律当没有玻璃**：那条路把 PT 图 blit 上来再叠 HUD，没有"面板背后的
        // 光栅画面"这回事；更要紧的是 PT 的 HUD 绘制与主 pass 的 HUD 绘制互斥，
        // 若这里留真值，主 pass 那份 HUD 会被跳过而 PT 那份又采不到正确的模糊图。
        let glass_on = self.menu_glass_enabled && !self.pt_live_enabled;
        self.hud_has_glass = glass_on && quads.iter().take(count).any(|q| q.glass);
        if count == 0 || self.hud_mapped.is_null() {
            return;
        }
        let mut verts: Vec<HudVertex> = Vec::with_capacity(count * 6);
        for q in quads.iter().take(count) {
            let x0 = q.rect.x / w * 2.0 - 1.0;
            let y0 = 1.0 - q.rect.y / h * 2.0;
            let x1 = (q.rect.x + q.rect.w) / w * 2.0 - 1.0;
            let y1 = 1.0 - (q.rect.y + q.rect.h) / h * 2.0;
            let color = [q.color.r, q.color.g, q.color.b, q.color.a];
            // 屏幕归一化 UV（与 NDC 同一套换算，只是不乘 2 也不减 1）—— 模糊图是整个屏幕的
            // 降采样副本，所以 uv 直接就是"这一点在屏幕上的位置"。
            let (u0, v0) = (q.rect.x / w, q.rect.y / h);
            let (u1, v1) = ((q.rect.x + q.rect.w) / w, (q.rect.y + q.rect.h) / h);
            // PT 模式（或总开关关掉）时把标志位写成 0 —— 顶点格式恒定，只有这个分量在变
            let g = if q.glass && glass_on { 1.0 } else { 0.0 };
            for (px, py, u, v) in [
                (x0, y0, u0, v0),
                (x1, y0, u1, v0),
                (x0, y1, u0, v1),
                (x1, y0, u1, v0),
                (x1, y1, u1, v1),
                (x0, y1, u0, v1),
            ] {
                verts.push(HudVertex {
                    pos: [px, py],
                    color,
                    uv_glass: [u, v, g],
                });
            }
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                verts.as_ptr() as *const u8,
                self.hud_mapped as *mut u8,
                verts.len() * std::mem::size_of::<HudVertex>(),
            );
        }
    }
    /// 🧊 HUD 玻璃描述符集（`glass_tex` + `glass_smp`）：**只有一份**，不在在飞帧之间复制 ——
    /// 图像与采样器建好后永不改动，也就不存在"写一个正在被读的 set"。
    pub(crate) fn init_hud_glass_set(&mut self) -> Result<(), String> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        self.hud_glass_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(|e| format!("创建 HUD 玻璃 set layout 失败: {e}"))?
        };
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(1),
        ];
        self.hud_glass_pool = unsafe {
            self.device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .pool_sizes(&pool_sizes)
                        .max_sets(1),
                    None,
                )
                .map_err(|e| format!("创建 HUD 玻璃 pool 失败: {e}"))?
        };
        let layouts = [self.hud_glass_set_layout];
        self.hud_glass_set = unsafe {
            self.device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(self.hud_glass_pool)
                        .set_layouts(&layouts),
                )
                .map_err(|e| format!("分配 HUD 玻璃 set 失败: {e}"))?
        }[0];
        let image_info = [vk::DescriptorImageInfo::default()
            .image_view(self.menu_blur_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let sampler_info = [vk::DescriptorImageInfo::default()
            .sampler(self.menu_blur_sampler)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(self.hud_glass_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_info),
            vk::WriteDescriptorSet::default()
                .dst_set(self.hud_glass_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler_info),
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }
    /// 把纹理 Image View 和 Sampler 写入每个 DescriptorSet（binding 1 / 3），
    /// 并把阴影贴图 View + depth-compare Sampler 写入 binding 5 / 6。
    pub(crate) fn update_texture_descriptor_sets(&mut self) -> Result<(), String> {
        for i in 0..self.descriptor_sets.len() {
            let image_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.texture_image_view)
                .sampler(self.texture_sampler);
            let image_infos = [image_info];

            let sampled_image_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(1)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_infos);

            let sampler_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(3)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&image_infos);

            // 阴影贴图（binding 5 = SAMPLED_IMAGE，binding 6 = SAMPLER；depth-compare 采样）
            let shadow_image_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.shadow_image_view)
                .sampler(self.shadow_sampler);
            let shadow_image_infos = [shadow_image_info];
            let shadow_map_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(5)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&shadow_image_infos);
            let shadow_sampler_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(6)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&shadow_image_infos);

            // marker/NPC 程序化皮肤纹理（binding 7/8；RV3D_SKIN_TEX=1 时片元采样，缺省纯色回退）
            // binding 11：硬件 PCF 的比较采样器（两张阴影图共用同一个）
            let shadow_cmp_infos = [vk::DescriptorImageInfo::default()
                .sampler(self.shadow_cmp_sampler)];
            let shadow_cmp_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(11)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&shadow_cmp_infos);
            let marker_skin_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.skin_marker_image_view)
                .sampler(self.texture_sampler);
            let marker_skin_infos = [marker_skin_info];
            let marker_skin_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(7)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&marker_skin_infos);
            let npc_skin_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.skin_npc_image_view)
                .sampler(self.texture_sampler);
            let npc_skin_infos = [npc_skin_info];
            let npc_skin_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(8)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&npc_skin_infos);
            // 地面微细节层（binding 9）：**必须写**，片元无条件采样它。
            // 采样器字段对 SAMPLED_IMAGE 写入无意义（真正的采样器走 binding 3 那条
            // SAMPLER 写入），与其它贴图保持一致填 texture_sampler。
            let ground_detail_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.ground_detail_image_view)
                .sampler(self.texture_sampler);
            let ground_detail_infos = [ground_detail_info];
            let ground_detail_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(GROUND_DETAIL_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&ground_detail_infos);
            // 动态阴影图（binding 10）：片元在静态图之外再采一张，两张取 max（互不覆盖）。
            // 采样器仍走 binding 6 那条 SAMPLER 写入（两张图共用同一个 sampled-depth 采样器）。
            let shadow_dyn_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(self.shadow_dyn_image_view)
                .sampler(self.shadow_sampler);
            let shadow_dyn_infos = [shadow_dyn_info];
            let shadow_dyn_write = vk::WriteDescriptorSet::default()
                .dst_set(self.descriptor_sets[i])
                .dst_binding(SHADOW_DYN_BINDING)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&shadow_dyn_infos);

            let writes = [
                sampled_image_write,
                sampler_write,
                shadow_map_write,
                shadow_sampler_write,
                shadow_cmp_write,
                marker_skin_write,
                npc_skin_write,
                ground_detail_write,
                shadow_dyn_write,
            ];
            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }
        Ok(())
    }
}
