// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 创建一张带完整 mip 链的采样纹理：
    /// staging buffer → Image（SAMPLED|TRANSFER_DST|TRANSFER_SRC）→ 逐级 blit 生成 mip →
    /// ImageView。地面纹理之外的附加贴图（marker/NPC 程序化皮肤纹理、地面微细节 tile）
    /// 共用此路径，采样器复用主纹理的 texture_sampler（尺寸同规格，mip 级数一致）。
    ///
    /// `format` 决定 view 的色彩空间：皮肤图传 `R8G8B8A8_SRGB`（存的是显示编码色），
    /// 地面细节层必须传 `R8G8B8A8_UNORM`——它存的是**线性亮度调制**（纹素 = 调制/2），
    /// 用 SRGB view 会把 128 解码成 0.214，乘 2 后得 0.43 → 全场地面暗一半。
    pub(crate) fn create_sampled_image(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        format: vk::Format,
    ) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), String> {
        let image_size = (width * height * 4) as u64;
        // mip 链级别数：按长边逐次减半直至 1
        let mut mip_levels = 1u32;
        let mut largest = width.max(height);
        while largest > 1 {
            largest >>= 1;
            mip_levels += 1;
        }

        // ---- 1. staging buffer：CPU 写入像素数据 ----
        let buffer_info = vk::BufferCreateInfo::default()
            .size(image_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建纹理 staging buffer 失败: {e}"))?
        };

        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let memory_type = mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (mem_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 staging buffer）".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配纹理 staging buffer 内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定纹理 staging buffer 内存失败: {e}"))?;
            let data_ptr = self
                .device
                .map_memory(staging_memory, 0, image_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射纹理 staging buffer 失败: {e}"))?;
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                data_ptr as *mut u8,
                pixels.len(),
            );
            self.device.unmap_memory(staging_memory);
        }

        // ---- 2. Vulkan Image（SAMPLED | TRANSFER_DST | TRANSFER_SRC）----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建纹理 Image 失败: {e}"))?
        };

        let img_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let img_mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let img_memory_type = img_mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (img_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| {
                img_mem_props.memory_types.iter().enumerate().find(|(i, _)| {
                    let type_mask = 1 << i;
                    (img_reqs.memory_type_bits & type_mask) != 0
                })
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 Image）".to_string())?;

        let img_alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(img_reqs.size)
            .memory_type_index(img_memory_type);
        let image_memory = unsafe {
            self.device
                .allocate_memory(&img_alloc_info, None)
                .map_err(|e| format!("分配纹理 Image 内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, image_memory, 0)
                .map_err(|e| format!("绑定纹理 Image 内存失败: {e}"))?;
        }

        // ---- 3. 拷贝 staging buffer → Image，生成 mip 链，转 SHADER_READ_ONLY_OPTIMAL ----
        self.run_single_time_commands(|cmd| {
            let subresource_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(mip_levels)
                .base_array_layer(0)
                .layer_count(1);

            // UNDEFINED → TRANSFER_DST_OPTIMAL
            let barrier_to_transfer = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_transfer],
                );
            }

            // 拷贝像素数据
            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width, height, depth: 1 });
            unsafe {
                self.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            // 逐级生成 mip：上一级 TRANSFER_DST → TRANSFER_SRC，再 blit 缩小到本级
            for mip in 1..mip_levels {
                let src_w = (width >> (mip - 1)).max(1);
                let src_h = (height >> (mip - 1)).max(1);
                let dst_w = (width >> mip).max(1);
                let dst_h = (height >> mip).max(1);

                let level_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(mip - 1)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                // TRANSFER_DST_OPTIMAL → TRANSFER_SRC_OPTIMAL
                let barrier_to_blit_src = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(level_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                unsafe {
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier_to_blit_src],
                    );
                }

                let blit_region = vk::ImageBlit::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip - 1)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: src_w as i32, y: src_h as i32, z: 1 },
                    ])
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: dst_w as i32, y: dst_h as i32, z: 1 },
                    ]);
                unsafe {
                    self.device.cmd_blit_image(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit_region],
                        vk::Filter::LINEAR,
                    );
                }
            }

            // 全部 mip → SHADER_READ_ONLY_OPTIMAL（基级们 TRANSFER_SRC、末级 TRANSFER_DST）
            let mut read_barriers = Vec::new();
            if mip_levels > 1 {
                let read_src_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels - 1)
                    .base_array_layer(0)
                    .layer_count(1);
                read_barriers.push(
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(read_src_range)
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ),
                );
            }
            let read_last_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(mip_levels - 1)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1);
            read_barriers.push(
                vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(read_last_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            );
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &read_barriers,
                );
            }
        })?;

        // 释放 staging buffer
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }

        // ---- 4. Image View（2D 类型）----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        let view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建纹理 Image View 失败: {e}"))?
        };

        Ok((image, image_memory, view))
    }
    /// 加载 assets/textures/test.png 并创建纹理资源
    pub(crate) fn init_texture(&mut self) -> Result<(), String> {
        // 程序化地面纹理（CPU 画像素 + 烘焙高度场 AO/静态天光，零第三方依赖）。
        // 世界空间对齐：与 build.rs 片元着色器 world-space UV 一致（见 procedural.rs）。
        // RV3D_PROC_TEX=0 回退到 assets/textures/test.png（A/B 验证程序化材质效果）。
        let (width, height, pixels) = if std::env::var("RV3D_PROC_TEX").as_deref() != Ok("0") {
            let size = super::procedural::GROUND_TEXTURE_SIZE;
            let height_at = |x: f32, z: f32| terrain_height(x, z);
            (
                size,
                size,
                super::procedural::generate_city_ground_texture(size, &height_at),
            )
        } else {
            let texture_path = "assets/textures/test.png";
            let img = image::open(texture_path)
                .map_err(|e| format!("加载纹理图片失败 '{}': {}", texture_path, e))?
                .to_rgba8();
            (img.width(), img.height(), img.as_raw().clone())
        };
        let image_size = (width * height * 4) as u64;
        // mip 链级别数：按长边逐次减半直至 1
        let mut mip_levels = 1u32;
        let mut largest = width.max(height);
        while largest > 1 {
            largest >>= 1;
            mip_levels += 1;
        }

        // ---- 1. staging buffer：CPU 写入像素数据 ----
        let buffer_info = vk::BufferCreateInfo::default()
            .size(image_size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建纹理 staging buffer 失败: {}", e))?
        };

        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let memory_type = mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (mem_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 staging buffer）".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配纹理 staging buffer 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定纹理 staging buffer 内存失败: {}", e))?;
            let data_ptr = self
                .device
                .map_memory(staging_memory, 0, image_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射纹理 staging buffer 失败: {}", e))?;
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                data_ptr as *mut u8,
                pixels.len(),
            );
            self.device.unmap_memory(staging_memory);
        }

        // ---- 2. Vulkan Image（SAMPLED | TRANSFER_DST）----
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        self.texture_image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建纹理 Image 失败: {}", e))?
        };

        let img_reqs = unsafe { self.device.get_image_memory_requirements(self.texture_image) };
        let img_mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let img_memory_type = img_mem_props
            .memory_types
            .iter()
            .enumerate()
            .find(|(i, mem_type)| {
                let type_mask = 1 << i;
                (img_reqs.memory_type_bits & type_mask) != 0
                    && mem_type.property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| {
                img_mem_props.memory_types.iter().enumerate().find(|(i, _)| {
                    let type_mask = 1 << i;
                    (img_reqs.memory_type_bits & type_mask) != 0
                })
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到合适的内存类型（纹理 Image）".to_string())?;

        let img_alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(img_reqs.size)
            .memory_type_index(img_memory_type);
        self.texture_image_memory = unsafe {
            self.device
                .allocate_memory(&img_alloc_info, None)
                .map_err(|e| format!("分配纹理 Image 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_image_memory(self.texture_image, self.texture_image_memory, 0)
                .map_err(|e| format!("绑定纹理 Image 内存失败: {}", e))?;
        }

        // ---- 3. 拷贝 staging buffer → Image，并转换布局 ----
        let image = self.texture_image;
        self.run_single_time_commands(|cmd| {
            let subresource_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(mip_levels)
                .base_array_layer(0)
                .layer_count(1);

            // UNDEFINED → TRANSFER_DST_OPTIMAL
            let barrier_to_transfer = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_transfer],
                );
            }

            // 拷贝像素数据
            let region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D { width, height, depth: 1 });
            unsafe {
                self.device.cmd_copy_buffer_to_image(
                    cmd,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            // 逐级生成 mip：上一级 TRANSFER_DST → TRANSFER_SRC，再 blit 缩小到本级
            for mip in 1..mip_levels {
                let src_w = (width >> (mip - 1)).max(1);
                let src_h = (height >> (mip - 1)).max(1);
                let dst_w = (width >> mip).max(1);
                let dst_h = (height >> mip).max(1);

                let level_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(mip - 1)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);

                // TRANSFER_DST_OPTIMAL → TRANSFER_SRC_OPTIMAL
                let barrier_to_blit_src = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(level_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
                unsafe {
                    self.device.cmd_pipeline_barrier(
                        cmd,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier_to_blit_src],
                    );
                }

                let blit_region = vk::ImageBlit::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip - 1)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: src_w as i32, y: src_h as i32, z: 1 },
                    ])
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(mip)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D { x: dst_w as i32, y: dst_h as i32, z: 1 },
                    ]);
                unsafe {
                    self.device.cmd_blit_image(
                        cmd,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit_region],
                        vk::Filter::LINEAR,
                    );
                }
            }

            // 全部 mip → SHADER_READ_ONLY_OPTIMAL（基级们 TRANSFER_SRC、末级 TRANSFER_DST）
            let mut read_barriers = Vec::new();
            if mip_levels > 1 {
                let read_src_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels - 1)
                    .base_array_layer(0)
                    .layer_count(1);
                read_barriers.push(
                    vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(image)
                        .subresource_range(read_src_range)
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ),
                );
            }
            let read_last_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(mip_levels - 1)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1);
            read_barriers.push(
                vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(read_last_range)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            );
            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &read_barriers,
                );
            }
        })?;

        // 释放 staging buffer
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }

        // ---- 4. Image View（2D 类型）----
        let view_info = vk::ImageViewCreateInfo::default()
            .image(self.texture_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_SRGB)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        self.texture_image_view = unsafe {
            self.device
                .create_image_view(&view_info, None)
                .map_err(|e| format!("创建纹理 Image View 失败: {}", e))?
        };

        // ---- 5. Sampler（线性过滤）----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .anisotropy_enable(self.texture_anisotropy_enabled)
            .max_anisotropy(if self.texture_anisotropy_enabled {
                self.physical_device_properties
                    .limits
                    .max_sampler_anisotropy
                    .min(16.0)
            } else {
                1.0
            })
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .min_lod(0.0)
            .max_lod((mip_levels - 1) as f32);
        self.texture_sampler = unsafe {
            self.device
                .create_sampler(&sampler_info, None)
                .map_err(|e| format!("创建纹理 Sampler 失败: {}", e))?
        };

        let tex_src = if std::env::var("RV3D_PROC_TEX").as_deref() == Ok("0") {
            "assets/textures/test.png"
        } else {
            "程序化地面材质"
        };
        log::info!(
            "纹理初始化完成: {}x{}（{} mip，来源={}）",
            width,
            height,
            mip_levels,
            tex_src
        );

        // ---- marker/NPC 程序化皮肤纹理（CPU 画像素，零依赖）----
        // RV3D_SKIN_TEX=1 时片元着色器采样（light_data.flags.z 通知）；缺省 0 纯色回退。
        // 纹理恒创建（shader 静态引用 binding 7/8，descriptor 必须有效），仅门控采样路径。
        let skin_size = super::procedural::SKIN_TEXTURE_SIZE;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_marker_skin_texture(),
            skin_size,
            skin_size,
            vk::Format::R8G8B8A8_SRGB,
        )?;
        self.skin_marker_image = img;
        self.skin_marker_memory = mem;
        self.skin_marker_image_view = view;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_npc_skin_texture(),
            skin_size,
            skin_size,
            vk::Format::R8G8B8A8_SRGB,
        )?;
        self.skin_npc_image = img;
        self.skin_npc_memory = mem;
        self.skin_npc_image_view = view;
        log::info!(
            "程序化皮肤纹理初始化完成: {}x{}（marker=木板墙, npc=迷彩军服, RV3D_SKIN_TEX={}）",
            skin_size,
            skin_size,
            if self.skin_tex_enabled { "on" } else { "off（纯色回退）" }
        );

        // ---- 地面微细节层（binding 9；build.rs 片元 `ground_detail_tex`）----
        // ⚠ 恒创建、恒绑定，**不受任何环境变量门控**：片元是无条件采样它的，缺这个
        // 描述符不会报错、只会让驱动回吐 0，于是 `mixed *= mix(1.0, 0*2, gdetail)`
        // 把相机周边整圈地面乘成纯黑（2026-09-03 大面积黑地根因）。
        // 格式必须是 UNORM（线性）：纹素存的是「亮度调制 / 2」而不是显示编码颜色。
        // 采样器复用 texture_sampler（binding 3）：REPEAT + LINEAR/LINEAR-mip，
        // 正是平铺细节层要的（build.rs 用 textureSampleLevel 显式选 mip）。
        let detail_size = super::procedural::GROUND_DETAIL_SIZE;
        let (img, mem, view) = self.create_sampled_image(
            &super::procedural::generate_default_ground_detail_texture(),
            detail_size,
            detail_size,
            vk::Format::R8G8B8A8_UNORM,
        )?;
        self.ground_detail_image = img;
        self.ground_detail_memory = mem;
        self.ground_detail_image_view = view;
        log::info!(
            "地面微细节层初始化完成: {}x{} 覆盖 {}m（{} 纹素/米，UNORM 线性，绑定 binding {}）",
            detail_size,
            detail_size,
            super::procedural::GROUND_DETAIL_METRES,
            detail_size as f32 / super::procedural::GROUND_DETAIL_METRES,
            GROUND_DETAIL_BINDING
        );
        Ok(())
    }

    // ============================================================
    // 阴影贴图（2026-08-11）：depth-only pass 渲光空间深度，主 pass 3x3 PCF 采样
    // ============================================================
    /// 创建阴影贴图资源：2048x2048 D32_SFLOAT（DEPTH_STENCIL_ATTACHMENT | SAMPLED）、
    /// depth-compare 采样器、depth-only render pass、framebuffer、每帧 shadow UBO、
    /// shadow descriptor set layout + sets（binding 0 = shadow UBO，binding 2 = 实例 storage）。
    /// 建一张阴影图（image + 显存 + view）。静态图与动态图除用途外完全同构，
    /// 所以创建代码只留这一份（加第二张图时把它从内联收口成函数）。
    pub(crate) fn create_shadow_map_image(&self) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建阴影图 Image 失败: {}", e))?
        };
        let mem_reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = self.pick_memory_type(mem_reqs, true)?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配阴影图 Image 内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, memory, 0)
                .map_err(|e| format!("绑定阴影图 Image 内存失败: {}", e))?;
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
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
                .map_err(|e| format!("创建阴影图 Image View 失败: {}", e))?
        };
        Ok((image, memory, view))
    }
    /// 建一个阴影 pass 的 framebuffer（单附件 = 那张图的 view）。
    /// 静态图与动态图共用 `shadow_render_pass`，所以只有附件不同。
    pub(crate) fn create_shadow_framebuffer(&self, view: vk::ImageView) -> Result<vk::Framebuffer, String> {
        use crate::engine::lighting::SHADOW_MAP_SIZE;
        let attachments = [view];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(self.shadow_render_pass)
            .attachments(&attachments)
            .width(SHADOW_MAP_SIZE)
            .height(SHADOW_MAP_SIZE)
            .layers(1);
        unsafe {
            self.device
                .create_framebuffer(&info, None)
                .map_err(|e| format!("创建阴影帧缓冲失败: {}", e))
        }
    }
    /// 🧊 磨砂玻璃的"背景模糊图"（2026-09-26，未结案 #19）：一张**固定尺寸**的降采样副本。
    ///
    /// 为什么固定尺寸（不跟交换链走）：换窗口尺寸时**不用重建**它，描述符集也就不用在
    /// 交换链重建路径里重写 —— 这条路上少一个"忘了重写"的机会（对比阴影图：那是固定
    /// 2048²，同样与交换链无关）。代价只是横竖缩放比不同 ⇒ 模糊半径在两个方向上不等，
    /// 对"磨砂"这件事完全不受影响。
    ///
    /// 建立后**立刻清成深灰并转到 `SHADER_READ_ONLY_OPTIMAL`**：描述符从第一帧起就按这个
    /// 布局绑定，而菜单出现之前根本不会有人写它 ⇒ 不清就是"首帧采到未定义内容"
    /// （与 §21.38 两张阴影图必须 init 时先转布局是同一类问题）。
    pub(crate) fn init_menu_blur(&mut self) -> Result<(), String> {
        let format = self.swapchain_format;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: MENU_BLUR_W,
                height: MENU_BLUR_H,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe {
            self.device
                .create_image(&image_info, None)
                .map_err(|e| format!("创建菜单模糊图失败: {e}"))?
        };
        let reqs = unsafe { self.device.get_image_memory_requirements(image) };
        let memory_type = self.pick_memory_type(reqs, true)?;
        let memory = unsafe {
            self.device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(reqs.size)
                        .memory_type_index(memory_type),
                    None,
                )
                .map_err(|e| format!("分配菜单模糊图内存失败: {e}"))?
        };
        unsafe {
            self.device
                .bind_image_memory(image, memory, 0)
                .map_err(|e| format!("绑定菜单模糊图内存失败: {e}"))?;
        }
        let view = unsafe {
            self.device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(0)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(|e| format!("创建菜单模糊图 View 失败: {e}"))?
        };
        let sampler = unsafe {
            self.device
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(|e| format!("创建菜单模糊图采样器失败: {e}"))?
        };
        self.menu_blur_image = image;
        self.menu_blur_memory = memory;
        self.menu_blur_view = view;
        self.menu_blur_sampler = sampler;

        // 一次性：UNDEFINED → TRANSFER_DST → 清成深灰 → SHADER_READ_ONLY（之后永远可采样）
        let clear = vk::ClearColorValue {
            float32: [0.02, 0.02, 0.03, 1.0],
        };
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        self.run_single_time_commands(|cb| unsafe {
            let to_dst = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::NONE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_dst],
            );
            self.device
                .cmd_clear_color_image(cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear, &[range]);
            let to_read = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(range);
            self.device.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_read],
            );
        })?;
        Ok(())
    }
}
