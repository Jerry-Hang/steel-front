// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 当前交换链尺寸（main.rs 每帧与窗口尺寸比对，不一致即重建——防 DPI/全屏错位）
    pub(crate) fn swapchain_size(&self) -> (u32, u32) {
        (self.swapchain_extent.width, self.swapchain_extent.height)
    }
    /// 上一帧统计：near/far 可见实例数与地形 LOD 名（供 HUD / 日志）
    pub(crate) fn last_stats(&self) -> (u32, u32, &'static str) {
        (
            self.last_near_count,
            self.last_far_count,
            self.last_terrain_lod_name,
        )
    }
    pub(crate) fn perf_snapshot(&self) -> PerfSnapshot {
        PerfSnapshot {
            frame_us: self.last_frame_us,
            cull_us: self.last_cull_us,
            terrain_us: self.stage_terrain_us,
            wait_fence_us: self.stage_wait_fence_us,
            acquire_us: self.stage_acquire_us,
            record_us: self.stage_record_us,
            submit_us: self.stage_submit_us,
            present_us: self.stage_present_us,
        }
    }
    /// GPU 设备名（性能日志头部用）
    pub(crate) fn gpu_name(&self) -> String {
        self.device_name.clone()
    }
    /// 更新光照 uniform（每帧渲染前调用；默认全零 = 光照关闭）
    pub(crate) fn set_lights(&mut self, lights: &LightUniform) {
        self.light_data = *lights;
        // 动态阴影图是否参与采样（两张图取 max）。放在这里统一置位，免得依赖
        // game.rs 构造 LightUniform 时是否知道"阴影拆了两张"这件事。
        self.light_data.shadow.config.z = if self.shadow_split
            && self.shadow_dyn_image_view != vk::ImageView::null()
        {
            1.0
        } else {
            0.0
        };
        // 片元成本对比开关（2026-10-09，性能定位用）：shadow.config.w
        //   0 = 正常（默认，逐位不变）
        //   1 = 跳过逐像素阴影 PCF
        //   2 = apply_lighting 直接返回底色（光照全关）
        //   3 = 片元直出常量色（纯填充/几何下限）
        // 实机：setprop debug.sf.shadow_mode 1
        self.light_data.shadow.config.w = crate::syscfg::cfg("RV3D_SHADOW_MODE")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.0);
        // RV3D_DEBUG_SHADOW=1：片元直出 shadow_factor 灰度（阴影诊断）
        if std::env::var("RV3D_DEBUG_SHADOW").as_deref() == Ok("1") {
            self.light_data.shadow.config.y = 1.0;
            // D3诊断：仅打印一次实际传入GPU的light_view_proj矩阵（列主序16元素）
            static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let m = self.light_data.shadow.light_view_proj.to_cols_array();
                log::info!(
                    "D3 light_view_proj (col-major): [{:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}, {:.6}]",
                    m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7],
                    m[8], m[9], m[10], m[11], m[12], m[13], m[14], m[15]
                );
            }
        }
    }
    /// 惰性创建截图读回资源：按 max_frames_in_flight 双缓冲 HOST_VISIBLE staging buffer + fence，
    /// 避免与 in-flight 帧竞态（capture_screenshot 首次调用时创建；交换链重建后作废重建）。
    pub(crate) fn init_screenshot_resources(&mut self) -> Result<(), String> {
        let size = (self.swapchain_extent.width as u64) * (self.swapchain_extent.height as u64) * 4;
        if size == 0 {
            return Err("交换链尺寸为 0，无法创建截图缓冲".to_string());
        }
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        for _ in 0..self.max_frames_in_flight {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(vk::BufferUsageFlags::TRANSFER_DST)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = match unsafe { self.device.create_buffer(&buffer_info, None) } {
                Ok(b) => b,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("创建截图 staging buffer 失败: {}", e));
                }
            };
            self.screenshot_buffers.push(buffer);

            let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
            let memory_type = mem_props
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (mem_reqs.memory_type_bits & type_mask) != 0
                        && mem_type
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
                        && mem_type
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::HOST_COHERENT)
                })
                .map(|(i, _)| i as u32)
                .ok_or_else(|| "没有找到合适的内存类型（截图 staging buffer）".to_string())
                .map_err(|e| {
                    self.destroy_screenshot_resources();
                    e
                })?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = match unsafe { self.device.allocate_memory(&alloc_info, None) } {
                Ok(m) => m,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("分配截图 staging buffer 内存失败: {}", e));
                }
            };
            self.screenshot_buffers_memory.push(memory);

            if let Err(e) = unsafe { self.device.bind_buffer_memory(buffer, memory, 0) } {
                self.destroy_screenshot_resources();
                return Err(format!("绑定截图 staging buffer 内存失败: {}", e));
            }

            let fence = match unsafe {
                self.device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            } {
                Ok(f) => f,
                Err(e) => {
                    self.destroy_screenshot_resources();
                    return Err(format!("创建截图围栏失败: {}", e));
                }
            };
            self.screenshot_fences.push(fence);
        }
        Ok(())
    }
    /// 销毁截图读回资源（交换链重建 / Drop 时调用；字段归零，下次截图惰性重建）
    pub(crate) fn destroy_screenshot_resources(&mut self) {
        for (&buffer, &memory) in self
            .screenshot_buffers
            .iter()
            .zip(self.screenshot_buffers_memory.iter())
        {
            if buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(buffer, None) };
            }
            if memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(memory, None) };
            }
        }
        self.screenshot_buffers.clear();
        self.screenshot_buffers_memory.clear();
        for &fence in &self.screenshot_fences {
            if fence != vk::Fence::null() {
                unsafe { self.device.destroy_fence(fence, None) };
            }
        }
        self.screenshot_fences.clear();
    }
    /// 读回当前帧 swapchain 图像并保存 PNG。
    /// 在 render() 提交渲染之后、present 之前调用：此时图像内容已确定，
    /// 且 render_finished 信号量尚未被 present 消费，主机侧等待不会死锁。
    /// 流程：等待信号量 → 一次性命令（布局转换 + 拷贝）→ wait fence → map 读取 → 保存。
    pub(crate) fn do_screenshot_readback(&mut self, image: vk::Image) -> Result<(), String> {
        let path = match self.screenshot_request.take() {
            Some(p) => p,
            None => return Ok(()),
        };
        let width = self.swapchain_extent.width;
        let height = self.swapchain_extent.height;
        let format = self.swapchain_format;
        let slot = self.current_frame;
        let buffer = *self
            .screenshot_buffers
            .get(slot)
            .ok_or_else(|| "截图缓冲未初始化".to_string())?;
        let memory = *self
            .screenshot_buffers_memory
            .get(slot)
            .ok_or_else(|| "截图缓冲内存未初始化".to_string())?;
        let fence = *self
            .screenshot_fences
            .get(slot)
            .ok_or_else(|| "截图围栏未初始化".to_string())?;
        let buffer_size = (width as u64) * (height as u64) * 4;

        // 1. 主机侧等待本帧渲染完成：vkWaitSemaphores 只接受 timeline 信号量，
        //    这里复用 in_flight_fence（本帧 queue_submit 已提交，等待不会死锁）。
        unsafe {
            self.device
                .wait_for_fences(
                    &[self.in_flight_fences[slot]],
                    true,
                    SCREENSHOT_WAIT_TIMEOUT_NS,
                )
                .map_err(|e| format!("等待渲染完成围栏失败: {}", e))?;
        }

        // 2. 一次性命令缓冲：PRESENT_SRC_KHR → TRANSFER_SRC_OPTIMAL → 拷贝 → 回 PRESENT_SRC_KHR
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd_buffer = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配截图命令缓冲失败: {}", e))?
        }[0];
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let barrier_to_transfer = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
        let barrier_to_present = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty());
        let copy_region = vk::BufferImageCopy::default()
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
            self.device
                .begin_command_buffer(cmd_buffer, &begin_info)
                .map_err(|e| format!("开始截图命令缓冲失败: {}", e))?;
            self.device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_to_transfer],
            );
            self.device.cmd_copy_image_to_buffer(
                cmd_buffer,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[copy_region],
            );
            self.device.cmd_pipeline_barrier(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_to_present],
            );
            self.device
                .end_command_buffer(cmd_buffer)
                .map_err(|e| format!("结束截图命令缓冲失败: {}", e))?;
        }

        // 3. 提交拷贝命令（独立 fence），等待完成后再释放命令缓冲
        unsafe {
            self.device
                .reset_fences(&[fence])
                .map_err(|e| format!("重置截图围栏失败: {}", e))?;
        }
        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("提交截图命令失败: {}", e))?;
            // 🔴 超时必须有限（`u64::MAX` = 无限等 ⇒ 铁律 B 那个"静默卡死"的同一形态；
            // 2026-09-25 复查时这句是**漏网的一处**，判据 `no_unbounded_wait_on_vulkan_calls`）。
            // ⚠️ 超时后**故意不释放**这条命令缓冲：它可能仍在 pending（释放 = 未定义行为），
            // 代价是每次超时漏一条一次性命令缓冲 —— 比 UB 便宜得多。同理那条围栏仍是 pending，
            // 下一次截图 `reset_fences` 会踩 UB；但这一路径只在 GPU 已经卡住时才可达
            // （那种情况下主循环的围栏超时判定会先 `gpu_stalled`，画面本来就不再更新）。
            self.device
                .wait_for_fences(&[fence], true, SCREENSHOT_WAIT_TIMEOUT_NS)
                .map_err(|e| format!("等待截图围栏失败（限时 {}s）: {}", SCREENSHOT_WAIT_TIMEOUT_NS / 1_000_000_000, e))?;
            self.device.free_command_buffers(self.command_pool, &[cmd_buffer]);
        }

        // 4. map 读取像素 → 格式转换 → 保存 PNG
        let data_ptr = unsafe {
            self.device
                .map_memory(memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射截图缓冲失败: {}", e))?
        };
        let mut raw = vec![0u8; (width * height * 4) as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(data_ptr as *const u8, raw.as_mut_ptr(), raw.len());
        }
        unsafe {
            self.device.unmap_memory(memory);
        }
        let mut rgba = vec![0u8; raw.len()];
        convert_pixels_to_rgba(format, &raw, &mut rgba)?;
        let img = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "创建 RGBA 图像失败".to_string())?;
        img.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| format!("保存截图失败 '{}': {}", path.display(), e))?;
        log::info!("截图已保存: {}", path.display());
        Ok(())
    }
    /// mesh 路径单次 draw：写入 base_slot push constant 后调用 vkCmdDrawMeshTasksEXT。
    /// count=0 直接返回（Vulkan 允许 group_count=0，这里避免无意义调用）。
    pub(crate) fn draw_mesh_range(
        &self,
        command_buffer: vk::CommandBuffer,
        mesh: &MeshShaderDevice,
        base_slot: u32,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        // push constant = (base_slot + chunk_start, 0, 0, 0)：
        // workgroup_id.x 从 0 起，槽位 = base + wg.x；每次下发不超过 maxMeshWorkGroupCount[0]。
        let chunk = self.mesh_max_wg_x.max(1);
        let mut drawn = 0u32;
        while drawn < count {
            let n = (count - drawn).min(chunk);
            let push: [u32; 4] = [base_slot + drawn, 0, 0, 0];
            let push_bytes = unsafe {
                std::slice::from_raw_parts(
                    push.as_ptr() as *const u8,
                    std::mem::size_of::<[u32; 4]>(),
                )
            };
            unsafe {
                self.device.cmd_push_constants(
                    command_buffer,
                    self.mesh_pipeline_layout,
                    vk::ShaderStageFlags::MESH_EXT,
                    0,
                    push_bytes,
                );
                mesh.cmd_draw_mesh_tasks(command_buffer, n, 1, 1);
            }
            drawn += n;
        }
    }
    /// 设置画质预设（纯 CPU 侧参数：地形 LOD 切换距离 + 实例近/远档分界距离等，
    /// 不触碰 pipeline/shader/swapchain 创建路径）。由外部（main.rs）按需调用。

    pub(crate) fn set_quality(&mut self, preset: QualityPreset) {
        self.quality = preset;
        log::info!("画质预设已切换: {}", preset.label());
    }
    /// 当前画质预设
    pub(crate) fn quality(&self) -> QualityPreset {
        self.quality
    }
    /// 请求截图：置 pending 标记，本帧渲染完成后读回 swapchain 图像并保存 PNG。
    /// 支持 B8G8R8A8 / R8G8B8A8 的 UNORM/SRGB 像素格式；一切失败返回 Err（不 panic）。

    pub(crate) fn capture_screenshot(&mut self, path: &std::path::Path) -> Result<(), String> {
        if self.screenshot_buffers.is_empty() {
            self.init_screenshot_resources()?;
        }
        self.screenshot_request = Some(path.to_path_buf());
        Ok(())
    }

    // ============================================================
    // 渲染循环
    // ============================================================
    pub(crate) fn render(&mut self, view: glam::Mat4, proj: glam::Mat4) -> Result<(), String> {
        // GPU 侧已被判定卡死（连续 N 次围栏超时）：不再等待/提交/呈现 —— 否则每帧都要
        // 卡满 5 秒超时，主循环形同僵死。保持响应、把结论留在日志里，交给上层决定。
        // 同理 `swapchain_broken`（见 `recreate_swapchain`）：重建**中途**失败时
        // `swapchain` 等句柄可能已经被销毁，再 acquire/提交就是拿空句柄调 Vulkan。
        // `device_lost` 包含 `swapchain_broken`（置位那一拍就是重建失败），这里并列写出来
        // 是为了让"不可恢复 ⇒ 不提交"这条读起来是显式的。
        if frame_suppressed(self.gpu_stalled, self.swapchain_broken || self.device_lost) {
            return Ok(());
        }
        let frame_start = Instant::now();
        let fence = self.in_flight_fences[self.current_frame];
        let t0 = Instant::now();
        match unsafe {
            // 🔴 超时有限（5s）：`u64::MAX` 会让"GPU 侧再也不会 signal"变成**静默死锁**
            // （日志停住、无 panic、无 VUID）。超时给出可诊断的错误。
            self.device
                .wait_for_fences(&[fence], true, FENCE_WAIT_TIMEOUT_NS)
        } {
            Ok(()) => {
                self.fence_timeouts = 0;
            }
            Err(e) => {
                self.fence_timeouts += 1;
                let n = self.fence_timeouts;
                if n == 1 {
                    log::error!(
                        "等待围栏超时（{:.0}s 内这一帧没完成）—— GPU 侧没有 signal；\
                         旧写法在这里用 u64::MAX 无限等 ⇒ 整个进程静默卡死",
                        FENCE_WAIT_TIMEOUT_NS as f32 / 1e9
                    );
                }
                if fence_stall_due(n) {
                    self.gpu_stalled = true;
                    log::error!(
                        "连续 {} 次围栏超时（≈{:.0}s 无任何一帧完成）⇒ 判定 GPU 侧卡死：\
                         后续帧不再等待/提交/呈现（画面会静止，但进程与输入保持响应）",
                        n,
                        n as f32 * FENCE_WAIT_TIMEOUT_NS as f32 / 1e9
                    );
                }
                return Err(format!("等待围栏失败: {}", e));
            }
        }
        self.stage_wait_fence_us = t0.elapsed().as_micros() as u64;

        let t0 = Instant::now();
        // 有限超时 + 分类重试（判据见 `classify_acquire_err`）：呈现引擎不给图像时
        // 不能无限等 —— 先记日志，再降级到 mailbox 重建，最后才报错交出这一帧。
        let (image_index, suboptimal) = loop {
            let r = unsafe {
                self.swapchain_loader.acquire_next_image(
                    self.swapchain,
                    ACQUIRE_TIMEOUT_NS,
                    self.image_available_semaphores[self.current_frame],
                    vk::Fence::null(),
                )
            };
            match r {
                Ok(v) => break v,
                Err(e) => match classify_acquire_err(e) {
                    AcquireOutcome::Retry => {
                        self.acquire_timeouts += 1;
                        let n = self.acquire_timeouts;
                        if n == 1 || n % 5 == 0 {
                            log::warn!(
                                "获取交换链图像超时（已连续 {} 次，累计 {:.1}s）—— 呈现引擎没有交出图像；\
                                 隐藏/被遮挡的窗口 + IMMEDIATE 是已知诱因",
                                n,
                                t0.elapsed().as_secs_f32()
                            );
                        }
                        if n >= ACQUIRE_STALL_FALLBACK
                            && self.present_mode_override != Some(vk::PresentModeKHR::MAILBOX)
                        {
                            // 自动恢复：换 mailbox 重建交换链（mailbox 会丢弃待呈现图像，
                            // 不会像 IMMEDIATE 那样把图像全留在呈现引擎手里）
                            log::error!(
                                "连续 {} 次拿不到交换链图像 ⇒ 把呈现模式降级为 MAILBOX 并重建交换链",
                                n
                            );
                            self.present_mode_override = Some(vk::PresentModeKHR::MAILBOX);
                            return Err("交换链过期".to_string());
                        }
                        if n >= ACQUIRE_STALL_MAX {
                            log::error!(
                                "连续 {} 次拿不到交换链图像（{:.1}s）—— 放弃这一帧",
                                n,
                                t0.elapsed().as_secs_f32()
                            );
                            return Err(format!("交换链长时间无可用图像（连续 {} 次超时）", n));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    AcquireOutcome::RecreateSwapchain => {
                        log::warn!("获取交换链图像返回 {:?}，重建交换链...", e);
                        return Err("交换链过期".to_string());
                    }
                    AcquireOutcome::Failed => {
                        return Err(format!("获取交换链图像失败: {:?}", e));
                    }
                },
            }
        };
        self.acquire_timeouts = 0;
        self.stage_acquire_us = t0.elapsed().as_micros() as u64;

        // 🔴 `suboptimal` **只登记，不许在这里 return**（2026-09-25 深夜复查）：
        // acquire 已经成功 ⇒ `image_available_semaphores[current_frame]` 已被 signal，而该信号量
        // 不会随交换链重建而重建 ⇒ 提前 return 会让它留在 signaled 状态被下一帧复用 = UB 且静默。
        // 这一帧照常走完（record/submit/present），重建统一放到 present 之后，判据见 `frame_action`。
        if suboptimal {
            log::warn!("交换链 SUBOPTIMAL（acquire）—— 本帧照常呈现，随后重建交换链");
        }

        unsafe {
            if let Err(e) = self.device.reset_fences(&[fence]) {
                // 同一条不变式（见 `frame_action` 文档）：这里也已经 acquire 成功过，
                // 直接 `?` 会把 `image_available_semaphores[current_frame]` 留在 signaled 状态
                // 被下一帧复用。既然连围栏都重置不了，设备事实上已经不可用 ⇒ 走既有的
                // `gpu_stalled` 降级：`render()` 之后直接返回，那个信号量**永不再被使用**。
                self.gpu_stalled = true;
                log::error!("重置围栏失败（{}）⇒ 判定 GPU 侧不可用，停止渲染循环", e);
                return Err(format!("重置围栏失败: {}", e));
            }
        }

        // ---- 每帧视锥剔除：可见实例压缩上传到当前帧 slot 的 HOST_VISIBLE buffer ----
        // 相机世界位置（view 为刚体变换，其逆矩阵的平移列即相机坐标），每帧只算一次
        let cam_pos = view.inverse().w_axis.truncate();
        let cull_start = Instant::now();
        let (near_count, far_count) = if self.void_mode {
            // 虚空检视模式：跳过世界几何剔除/上传（仅枪模）
            (0, 0)
        } else if self.mesh_enabled {
            // mesh 路径：地面实例场静态一次性上传（见 create_instance_buffer），
            // 完全跳过 CPU SIMD 剔除/压缩——剔除与顶点变换全部移到 GPU mesh shader。
            // 性能日志 visible 语义 = 已上传槽位数（INSTANCE_COUNT）。
            (INSTANCE_COUNT, 0)
        } else {
            self.cull_and_upload(view, proj, cam_pos)
        };
        // ---- 世界障碍 marker：独立槽位上传（见 MARKER_SLOT_BASE），计数供 draw call 使用 ----
        // RV3D_NO_MARKERS=1：A/B 用 —— 跳过**障碍标记实例**（`marker=` 那个计数）。
        // 与 `RV3D_NO_TERRAIN_FIELD` / `RV3D_NO_PROPS` 同类的对照开关：
        // 第 37 轮量出道具 3.2ms + 地形场 0.87ms 只占 9.44ms 帧的 43%，
        // 剩下的未知要靠逐个开关消掉，而不是靠猜。
        let (marker_near, marker_far) = if self.void_mode
            || std::env::var("RV3D_NO_MARKERS").is_ok()
        {
            (0, 0)
        } else {
            self.upload_markers(cam_pos)
        };
        self.last_marker_near = marker_near;
        self.last_marker_far = marker_far;
        // ---- NPC 士兵段：独立槽位上传（见 NPC_SLOT_BASE），计数供 draw call 使用 ----
        let ((box_near, box_far), (cyl_near, cyl_far), (sph_near, sph_far)) = if self.void_mode {
            ((0, 0), (0, 0), (0, 0))
        } else {
            self.upload_npcs(cam_pos)
        };
        self.last_npc_box_near = box_near;
        self.last_npc_box_far = box_far;
        self.last_npc_cyl_near = cyl_near;
        self.last_npc_cyl_far = cyl_far;
        self.last_npc_sph_near = sph_near;
        self.last_npc_sph_far = sph_far;
        // 🪖 士兵 GLB 实例上传（在 NPC 实例同一批里做完，避免多开一次遍历）
        //
        // 🔴 2026-09-13：**顺便打一个无歧义的计数**（`RV3D_SOLDIER_STATS=1` 才开，默认静默）。
        //
        // 存在的理由：我今晚**两次**用没验证过的指标下结论 —— 一次是单次 A/B（方差比效应大），
        // 一次是把 HUD 的 `npc: I{} P{} C{} A{}` 读成 `npc={}`（读丢了 `I`，那其实是
        // Idle 人数，与渲染毫无关系）。两次都是"先有结论、再找一个看起来支持它的数字"。
        //
        // ⇒ 这个计数**没有歧义**：左边是真正提交的 GLB 士兵实例数，右边是箱体实例数。
        //   GLB 生效时右边应当是 **0**（活体每人 1 个实例、尸体也是 1 个），
        //   所以"右边不为 0"就是回退路径被走到的**直接证据**，不必再靠别的字段推断。
        let soldiers = self.upload_soldiers();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_SOLDIER_STATS").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0
            {
                log::info!(
                    "soldier-stats: GLB 实例 {} / 箱体 {}（活体+尸体；GLB 生效时箱体应为 0）",
                    soldiers,
                    self.last_npc_box_near + self.last_npc_box_far
                );
            }
        }
        // ---- 自发光实体（爆炸闪光等）：独立槽位上传（见 EMISSIVE_SLOT_BASE）----
        let (emissive_near, emissive_far) = if self.void_mode { (0, 0) } else { self.upload_emissive(cam_pos) };
        self.last_emissive_near = emissive_near;
        self.last_emissive_far = emissive_far;
        let cull_us = cull_start.elapsed().as_micros() as u64;
        self.last_cull_us = cull_us;
        // RV3D_NO_TERRAIN_FIELD=1：A/B 用 —— 跳过**地形实例场**（近档 + 远档）的绘制。
        //
        // 目的（第 37 轮）：日志里 `visible=65536/65536 near=65536 far=0` 说明
        // 65,536 个地形实例**每帧全部进管线、一个都没被剔除**。与其继续猜 near/far
        // 的语义，不如直接量它值多少毫秒 —— 清零这两个计数即跳过对应 draw call。
        // 与 `RV3D_NO_PROPS` / `RV3D_NO_SHADOW` 同类的对照开关。
        let (near_count, far_count) = if std::env::var("RV3D_NO_TERRAIN_FIELD").is_ok() {
            (0, 0)
        } else {
            (near_count, far_count)
        };
        self.last_near_count = near_count;
        self.last_far_count = far_count;

        // ---- 地形网格 LOD：按相机到地形中心地面距离选级，过渡带内 morph 高度 ----
        let terrain_dist = (cam_pos.x * cam_pos.x + cam_pos.z * cam_pos.z).sqrt();
        let quality = quality_params(self.quality);
        let (terrain_lod, terrain_blend) = terrain_lod_blend_with_params(terrain_dist, quality);
        self.last_terrain_lod_name = terrain_lod.name();
        let t0 = Instant::now();
        if !self.void_mode {
            self.update_terrain_lod_morph(terrain_lod, terrain_blend);
        }
        let terrain_lod_index = if self.void_mode { 0 } else { terrain_lod as usize };
        self.stage_terrain_us = t0.elapsed().as_micros() as u64;

        // ---- 性能日志（1 次/秒）：visible / cull_us / fps ----
        self.frame_count += 1;
        if self.last_perf_log.elapsed().as_secs_f32() >= 1.0 {
            let window_secs = self.perf_window_start.elapsed().as_secs_f32();
            let fps = if window_secs > 0.0 {
                self.frame_count as f32 / window_secs
            } else {
                0.0
            };
            log::info!(
                "visible={}/{} near={} far={} fps={:.1} frame_us={} cull_us={} terrain_us={} wait_fence_us={} acquire_us={} record_us={} submit_us={} present_us={} terrain_lod={} blend={:.3} quality={} marker={} npc={}",
                near_count + far_count,
                INSTANCE_COUNT,
                near_count,
                far_count,
                fps,
                self.last_frame_us,
                cull_us,
                self.stage_terrain_us,
                self.stage_wait_fence_us,
                self.stage_acquire_us,
                self.stage_record_us,
                self.stage_submit_us,
                self.stage_present_us,
                terrain_lod.name(),
                terrain_blend,
                self.quality().label(),
                self.last_marker_near
                    + self.last_marker_far
                    + self.last_emissive_near
                    + self.last_emissive_far,
                self.last_npc_box_near
                    + self.last_npc_box_far
                    + self.last_npc_cyl_near
                    + self.last_npc_cyl_far
                    + self.last_npc_sph_near
                    + self.last_npc_sph_far
            );
            self.frame_count = 0;
            self.perf_window_start = Instant::now();
            self.last_perf_log = Instant::now();
        }

        // ---- 每帧把 view/proj 写进 Uniform Buffer（按 frame-in-flight 多份）----
        // 扩展字段（planes / cam_pos）仅网格着色器读取；传统顶点着色器只读前 144 字节。
        let (planes, cam_pos_w) = if self.mesh_enabled {
            let near_sq = quality_params(self.quality).instance_lod_distance;
            (Self::extract_frustum_planes(view, proj), near_sq * near_sq)
        } else {
            ([[0.0f32; 4]; 6], 0.0)
        };
        // 道具分桶剔除用的平面：**必须与 mesh 路径无关地存下来**。
        // 上面那个三元只在 mesh 路径算平面，而道具走的是传统顶点管线、在 mesh_enabled
        // 为 false 时也要能剔除；全零平面会让 bin_visible 恒真（退化为不剔除，安全但无效），
        // 所以这里无条件算一次。extract_frustum_planes 是纯算术，成本可忽略。
        self.frame_frustum = Self::extract_frustum_planes(view, proj);
        // 相机位置与视锥同处填：`RV3D_PROP_STATS=1` 的距离直方图要用它（见字段注释）
        self.frame_cam_pos = view.inverse().w_axis.truncate();
        let ubo = CameraUniform {
            view,
            proj,
            // x/w = 地形 LOD 切换距离（shader 未读取，仅 CPU 侧语义），y/z = 实例淡出区间
            lod_params: [
                quality.terrain_lod_high_end,
                FADE_START,
                FADE_END,
                quality.terrain_lod_med_end,
            ],
            planes,
            cam_pos: [cam_pos.x, cam_pos.y, cam_pos.z, cam_pos_w],
        };
        if let Some(&ptr) = self.uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &ubo as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<CameraUniform>(),
                );
            }
        }

        // ---- 光照 Uniform：写入 game 每帧更新的 light_data（默认全零 = 光照关闭）----
        let mut light_ubo = self.light_data;
        // RV3D_SKIN_TEX=1：flags.z 置 1 通知片元着色器启用 marker/NPC 程序化皮肤纹理
        // （缺省 0 保持纯色路径，冒烟基线不变；flags.x/y 语义不变）
        if self.skin_tex_enabled {
            light_ubo.flags.z = 1.0;
        }
        // flags.w：通知片元"地面微细节层（binding 9）真的存在，可以采样"。
        // 这个门是 build.rs 侧后加的防御：在它之前，binding 9 从未进过描述符集布局，
        // 未绑定描述符采样恒返回 0，而地面分支是乘性的（`mixed *= mix(1.0, g*2, gdetail)`），
        // 于是相机周边近处整圈地面被乘成纯黑。以图像句柄非空为条件是必要的——万一
        // init_texture 建图失败，这里保持 0，着色器就退回"没有细节层"而不是回到黑地。
        //
        // 🔴 `RV3D_NO_GROUND_TEX=1` 关掉这一层（A/B 诊断门）。**这个开关本仓早就写在
        // `renderer.rs:6134` 的注释里**（"与 RV3D_NO_SHADOW / RV3D_NO_GROUND_TEX 同一套惯例"），
        // 但**全仓从未实现过它** —— 拿它做 A/B 会得到"两边完全相同"的假结论
        // （§55 判别时就差点这样把 H1 误判为已否证）。现在补上，并读一次缓存住。
        if self.ground_detail_image_view != vk::ImageView::null() && !no_ground_detail_tex() {
            light_ubo.flags.w = 1.0;
        }
        if let Some(&ptr) = self.light_uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &light_ubo as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<LightUniform>(),
                );
            }
        }

        // ---- 阴影 UBO：写入光空间 view-proj（每帧 slot 独立，避免 in-flight 竞态）----
        if let Some(&ptr) = self.shadow_ubo_mapped.get(self.current_frame) {
            let shadow_vp = self.light_data.shadow.light_view_proj;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &shadow_vp as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<glam::Mat4>(),
                );
            }
        }

        // 每帧重录 command buffer（instance_count 随剔除结果变化）
        let t0 = Instant::now();
        // 🔴 **命令缓冲按「在飞帧槽位」取，不按 `image_index`**（2026-09-25 验证层实测，
        // 判据见 `command_buffer_is_indexed_by_frame_slot_not_by_swapchain_image`）：
        // 围栏 `in_flight_fences[current_frame]` 只保证**这个槽位**的上一次提交已完成，
        // 而 `image_index` 与槽位是两套编号 —— 同一张图像可以被连续两帧 acquire
        // （mailbox 下很常见），那时按图像取就会重录一条**仍 pending** 的命令缓冲
        // （VUID-vkBeginCommandBuffer-commandBuffer-00049 / VUID-vkQueueSubmit-pCommandBuffers-00071）。
        // 图像下标只用来选 framebuffer（见 `record_command_buffer` 的入参）。
        let cmd_buffer = self.command_buffers[self.current_frame];
        // 阴影图隔帧重画（`shadow_every`）：本帧画不画在这里定，`record_command_buffer` 只读。
        // ⚠️ 跳帧时阴影图**保持上一帧的内容**（render pass 的 initialLayout=UNDEFINED + CLEAR
        // 只在真画的那一帧发生），主 pass 照常采样 —— 布局上从 DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        // 到采样所需的 SHADER_READ_ONLY_OPTIMAL 之间那道 barrier 在 `record_shadow_pass` 末尾，
        // 跳帧时图像就停在 SHADER_READ_ONLY_OPTIMAL，主 pass 读它是合法状态。
        self.shadow_frame = shadow_due(self.frame_seq, self.shadow_every, self.void_mode);
        // 静态图单独一条节奏（默认 30 帧一次）：它装的是不动的东西，没必要每帧重画。
        self.shadow_static_frame =
            shadow_static_due(self.frame_seq, self.shadow_static_every, self.void_mode, self.shadow_split);
        // 性能测量（2026-10-09）：RV3D_SHADOW_PASS_OFF=1 跳过阴影图重画，
        // 用来单独量「阴影 map pass」在帧时间里的占比。画面会变成旧影子，仅用于测量。
        if crate::syscfg::flag("RV3D_SHADOW_PASS_OFF") {
            self.shadow_frame = false;
            self.shadow_static_frame = false;
        }
        self.frame_seq = self.frame_seq.wrapping_add(1);
        self.record_command_buffer(
            cmd_buffer,
            image_index as usize,
            near_count,
            far_count,
            terrain_lod_index,
        )?;
        self.stage_record_us = t0.elapsed().as_micros() as u64;

        let wait_semaphores = [self.image_available_semaphores[self.current_frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        // ⚠ **按下标 `image_index` 取，不是 `current_frame`** —— 理由见
        // `resize_render_finished_semaphores` 的文档（VUID-vkQueueSubmit-pSignalSemaphores-00067）。
        let render_finished = *self
            .render_finished_semaphores
            .get(image_index as usize)
            .ok_or_else(|| {
                format!(
                    "render-finished 信号量缺失：image_index={}，共 {} 个（应等于交换链图像数 {}）",
                    image_index,
                    self.render_finished_semaphores.len(),
                    self.swapchain_images.len()
                )
            })?;
        let signal_semaphores = [render_finished];
        let cmd_buffers = [cmd_buffer];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_buffers)
            .signal_semaphores(&signal_semaphores);

        let t0 = Instant::now();
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("提交队列失败: {}", e))?;
        }
        self.stage_submit_us = t0.elapsed().as_micros() as u64;

        // ---- 截图：本帧若已请求，在 present 前读回 swapchain 图像并保存 PNG ----
        // （图像内容已确定；render_finished 信号量尚未被 present 消费，主机等待不会死锁）
        // 读回失败不跳过 present：未呈现的 swapchain 图像不会被回收，连续失败会耗尽图像导致卡死
        let mut screenshot_err: Option<String> = None;
        if self.screenshot_request.is_some() {
            if let Some(&image) = self.swapchain_images.get(image_index as usize) {
                if let Err(e) = self.do_screenshot_readback(image) {
                    screenshot_err = Some(e);
                }
            } else {
                self.screenshot_request = None;
                screenshot_err = Some("交换链图像索引越界".to_string());
            }
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let t0 = Instant::now();
        let present_result = unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        };
        self.stage_present_us = t0.elapsed().as_micros() as u64;

        // 🔴 2026-09-22 复查补：呈现结果**必须每条都处理**（分类见 `classify_present`）。
        // 旧写法只处理 OUT_OF_DATE 与 SUBOPTIMAL，其余 Err（SURFACE_LOST / DEVICE_LOST）
        // 落空 ⇒ 主循环以为呈现成功，继续按"一切正常"跑下去。
        //
        // 2026-09-25 补：本帧的处置交给纯函数 `frame_action` —— 它**必须**拿到 present 结果，
        // 于是"成功 acquire 之后先 present 再决定"由签名保证（理由见该函数文档）。
        match frame_action(suboptimal, classify_present(present_result)) {
            FrameAction::Presented => {}
            FrameAction::RecreateAfterPresent => {
                // Android（2026-10-09 实机）：swapchain.rs 为修横屏旋转刻意用
                // `preTransform = IDENTITY`，而 surface 的 `currentTransform` 是 ROTATE_90
                // ⇒ Adreno 驱动**每帧**都回 SUBOPTIMAL。画面本身是对的（OCR 实测），
                // 但每帧重建交换链会把帧率拖死（实测 8 秒 236 次重建、GPU 利用率上不去）。
                // 故 Android 上把「Ok(true)」视为正常，只有真正过期/丢失才重建。
                #[cfg(target_os = "android")]
                {
                    if matches!(present_result, Ok(true)) {
                        static ONCE: std::sync::Once = std::sync::Once::new();
                        ONCE.call_once(|| {
                            log::info!(
                                "呈现 SUBOPTIMAL（Android preTransform=IDENTITY 的正常现象），不重建交换链"
                            )
                        });
                    } else {
                        log::warn!("呈现 {:?}，重建交换链...", present_result);
                        return Err("交换链过期".to_string());
                    }
                }
                #[cfg(not(target_os = "android"))]
                {
                    log::warn!("呈现 {:?}，重建交换链...", present_result);
                    return Err("交换链过期".to_string());
                }
            }
            FrameAction::Fail => {
                log::error!("呈现失败（{:?}）—— 不能当成成功", present_result);
                return Err(format!("呈现失败: {:?}", present_result));
            }
        }

        // 🔴 **2026-09-29：`queue_present` 是最后一个没有上界的 Vulkan 等待。**
        //
        // acquire 有 1s 超时、围栏有 5s 超时，但 present **没有** ——
        // 而 Wayland 下 FIFO（合成器不提供 `wp_fifo_v1` 时）**就是在 present 里阻塞等
        // frame callback**，窗口隐藏/最小化时那个回调不会来。后果与 acquire 那次同类：
        // 日志停住、无 panic、无 VUID、无 `has been lost`，从外面看就是"游戏死了"。
        //
        // `vkQueuePresentKHR` 的签名里没有超时参数，加不了超时 ⇒ 判据只能是**耗时**：
        // 超阈值就数一次，连续到阈值就按与 acquire **完全相同**的方式降级到 mailbox 并重建。
        // 放在 `frame_action` **之后**是刻意的：呈现结果的处置是硬不变量，
        // 不能被这里的提前 return 跳过。
        let consecutive = next_stall_count(self.present_stall_frames, self.stage_present_us);
        self.present_stall_frames = consecutive;
        match present_stall(self.stage_present_us, consecutive) {
            PresentStall::Ok => {}
            PresentStall::Warn => {
                if consecutive == 1 {
                    log::error!(
                        "单次呈现耗时 {}ms（阈值 {}ms）—— vkQueuePresentKHR 没有超时参数，\
                         卡在这里主循环就停了。Wayland FIFO 等一个不会来的 frame callback\
                         （窗口不可见时）是已知诱因。连续 {} 次后会降级为 MAILBOX 并重建交换链。",
                        self.stage_present_us / 1000,
                        PRESENT_STALL_US / 1000,
                        PRESENT_STALL_FALLBACK
                    );
                }
            }
            PresentStall::Degrade => {
                if self.present_mode_override != Some(vk::PresentModeKHR::MAILBOX) {
                    log::error!(
                        "连续 {} 次呈现卡顿 ⇒ 降级为 MAILBOX 并重建交换链\
                         （mailbox 会丢弃待呈现图像，不像 FIFO 那样等 frame callback）",
                        consecutive
                    );
                    self.present_mode_override = Some(vk::PresentModeKHR::MAILBOX);
                    return Err("交换链过期".to_string());
                }
            }
        }

        if let Some(e) = screenshot_err {
            return Err(format!("截图失败: {}", e));
        }

        self.last_frame_us = frame_start.elapsed().as_micros() as u64;
        self.current_frame = (self.current_frame + 1) % self.max_frames_in_flight;
        Ok(())
    }
}
