// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    pub(crate) fn init_swapchain(&mut self) -> Result<(), String> {
        let surface_capabilities = unsafe {
            self.surface_loader
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)
                .map_err(|e| format!("获取表面能力失败: {}", e))?
        };

        let surface_formats = unsafe {
            self.surface_loader
                .get_physical_device_surface_formats(self.physical_device, self.surface)
                .map_err(|e| format!("获取表面格式失败: {}", e))?
        };
        let format = surface_formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .unwrap_or(&surface_formats[0]);

        let present_modes = unsafe {
            self.surface_loader
                .get_physical_device_surface_present_modes(self.physical_device, self.surface)
                .map_err(|e| format!("获取呈现模式失败: {}", e))?
        };
        // 呈现模式可被 RV3D_PRESENT_MODE 覆盖（immediate/mailbox/fifo），性能探针对比用。
        //
        // 🔴 2026-09-13：**补上 `mailbox`**。此前只有 immediate/fifo，而默认落在
        // IMMEDIATE —— 屏幕上是**持续撕裂**，在快速转视角时正好读成"残影/鬼影"
        // （用户 2026-09-13 报告"晃画面和跑动时枪有非常明显的残影"）。
        // 抓帧抓不到它：`PrintWindow` 拿的是已合成的完整帧，撕裂只发生在显示器上。
        //
        // 三种模式的取舍（原注释只记了前两条）：
        //   * FIFO   —— 独显直连下等不到 vblank 中断 ⇒ 主循环冻结（2026-08-23）
        //   * MAILBOX —— 不撕裂且不阻塞；原注释记的"笔记本混合切换时 device lost"
        //                是 MUX 切换场景，独显直连/手动模式下不触发
        //   * IMMEDIATE —— 最稳但撕裂
        // ⇒ 默认仍保持 IMMEDIATE（基准/压力测试要的是最稳 + 全速），
        //   **玩家路径由 `SteelFront.bat` 显式设成 mailbox**（见该文件）。
        let preferred = match self.present_mode_override {
            Some(m) => m,
            None => match std::env::var("RV3D_PRESENT_MODE").as_deref() {
                Ok("immediate") => vk::PresentModeKHR::IMMEDIATE,
                Ok("fifo") => vk::PresentModeKHR::FIFO,
                Ok("mailbox") => vk::PresentModeKHR::MAILBOX,
                // Android：可靠的基本只有 FIFO（IMMEDIATE 多数驱动不支持，
                // MAILBOX 看机型）。见 docs/HANDOFF-mobile.md 5.6。
                #[cfg(target_os = "android")]
                _ => vk::PresentModeKHR::FIFO,
                #[cfg(not(target_os = "android"))]
                _ => vk::PresentModeKHR::IMMEDIATE,
            },
        };
        let present_mode = present_modes
            .iter()
            .find(|&&m| m == preferred)
            .copied()
            .unwrap_or(vk::PresentModeKHR::FIFO);

        // 诊断（2026-10-09）：Android 上画面转了 90°，先把 surface 的真实能力打出来。
        log::info!(
            "surface: current_transform={:?} current_extent={}x{} window_extent={}x{} min={}x{} max={}x{}",
            surface_capabilities.current_transform,
            surface_capabilities.current_extent.width,
            surface_capabilities.current_extent.height,
            self.window_extent.width,
            self.window_extent.height,
            surface_capabilities.min_image_extent.width,
            surface_capabilities.min_image_extent.height,
            surface_capabilities.max_image_extent.width,
            surface_capabilities.max_image_extent.height
        );
        let extent = swapchain_extent_choice(
            surface_capabilities.current_extent,
            self.window_extent,
            surface_capabilities.min_image_extent,
            surface_capabilities.max_image_extent,
        );

        let image_count = {
            let mut count = surface_capabilities.min_image_count + 1;
            if surface_capabilities.max_image_count != 0 {
                count = count.min(surface_capabilities.max_image_count);
            }
            count
        };

        let mut queue_family_indices = vec![self.graphics_queue_family_index];
        if self.present_queue_family_index != self.graphics_queue_family_index {
            queue_family_indices.push(self.present_queue_family_index);
        }
        let sharing_mode = if queue_family_indices.len() > 1 {
            vk::SharingMode::CONCURRENT
        } else {
            vk::SharingMode::EXCLUSIVE
        };

        // COLOR_ATTACHMENT | TRANSFER_SRC：截图读回需要把 swapchain 图像作为
        // TRANSFER 源拷贝到 staging buffer（vkCmdCopyImageToBuffer 的 VUID 要求）。
        //
        // 🔴 **TRANSFER_DST（2026-09-15 补，未结案 #2 的直接证据）**：PT 实时通路把
        // `pt_img` blit 到 swapchain 图像（见本文件 PT present 段：先 barrier 到
        // `TRANSFER_DST_OPTIMAL`，再 `cmd_blit_image`），**那一步要求目标图像带 TRANSFER_DST**。
        // 缺它时开启验证层（`RV3D_VALIDATION=1`）当场报两条：
        //   * `VUID-vkCmdBlitImage-dstImage-00224`（dstImage 缺 TRANSFER_DST）
        //   * `VUID-VkImageMemoryBarrier-oldLayout-01213`（barrier 到 TRANSFER_DST_OPTIMAL）
        // 这正是"设 `pt_enable=true` 一启动就 `0xC0000005`"的来源：**非法用法驱动不报错，
        // 崩在别处**。PT 之前一直开不起来，所以这条从来没被验证层看见过。
        let mut swapchain_usage =
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
        if surface_capabilities
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_DST)
        {
            swapchain_usage |= vk::ImageUsageFlags::TRANSFER_DST;
        } else {
            log::warn!(
                "surface 不支持 TRANSFER_DST：PT 实时通路无法 blit 到交换链（PT 打开时画面会异常）"
            );
        }
        // 旋转（实机 2026-10-09）：Android 横屏时 `currentTransform` 是 ROTATE_90/270，
        // 而 `preTransform = currentTransform` 的语义是"我的图像内容**已经**转好了" ——
        // 引擎的投影并没有跟着转，于是画面整体转 90°（手机上实测）。
        // 桌面 `currentTransform` 恒为 IDENTITY，所以这条只在 Android 上生效。
        // 取 IDENTITY 让呈现引擎替我们转；它不在 supportedTransforms 里时退回原值。
        #[cfg(target_os = "android")]
        let pre_transform = if surface_capabilities
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            surface_capabilities.current_transform
        };
        #[cfg(not(target_os = "android"))]
        let pre_transform = surface_capabilities.current_transform;
        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(swapchain_usage)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        // 🔴 **传副本给驱动（2026-10-08 实测）**：NVIDIA 驱动会往这个"const"结构体里**回写**
        // `image_usage |= STORAGE`（关掉验证层后依然回写 ⇒ 不是验证层干的；见本节末尾注释），
        // 而验证层随后照被改过的结构体报 2 条 VUID（`02275` / `01778`，本机 10s 就能复现）。
        // 我们申请的用法本身合法（`COLOR_ATTACHMENT|TRANSFER_SRC[|TRANSFER_DST]`），
        // 用副本可以保证本函数下面读到的、以及日志里打印的，仍然是我们真正申请的那份。
        let create_info_for_driver = swapchain_create_info;
        self.swapchain = unsafe {
            self.swapchain_loader
                .create_swapchain(&create_info_for_driver, None)
                .map_err(|e| format!("创建交换链失败: {}", e))?
        };
        self.swapchain_images = unsafe {
            self.swapchain_loader
                .get_swapchain_images(self.swapchain)
                .map_err(|e| format!("获取交换链图像失败: {}", e))?
        };
        self.swapchain_format = format.format;
        self.swapchain_extent = extent;
        // 诊断（2026-08-15）：surface current_extent vs 最终 swapchain extent ——
        // 若 current_extent 是窗口逻辑尺寸而实际物理尺寸不同，画面会 1:1 错位
        // 🔴 这里打印 `swapchain_usage`（我们申请的），**不要**打印 create info 的字段：
        //    驱动回写后那个值会变成 `…|STORAGE`，日志就不准了（实测踩过）。
        log::info!(
            "swapchain diag: current_extent={}x{} final={}x{} flags={:?} min_images={} usage={:?}",
            surface_capabilities.current_extent.width,
            surface_capabilities.current_extent.height,
            extent.width,
            extent.height,
            create_info_for_driver.flags,
            create_info_for_driver.min_image_count,
            swapchain_usage
        );

        self.swapchain_image_views = self
            .swapchain_images
            .iter()
            .map(|&image| {
                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);
                let view_create_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.swapchain_format)
                    .subresource_range(subresource_range);
                unsafe {
                    self.device
                        .create_image_view(&view_create_info, None)
                        .map_err(|e| format!("创建图像视图失败: {e}"))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        log::info!(
            "交换链初始化完成: {}x{}, 格式: {:?}, 图像数: {}, present_mode: {:?}",
            extent.width,
            extent.height,
            format.format,
            image_count,
            present_mode
        );
        Ok(())
    }
    /// 创建并持久映射一个 HOST_VISIBLE | HOST_COHERENT 的 Uniform Buffer
    pub(crate) fn create_uniform_buffer(
        &self,
        size: u64,
    ) -> Result<(vk::Buffer, vk::DeviceMemory, *mut std::ffi::c_void), String> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
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

        let mapped = unsafe {
            self.device
                .map_memory(buffer_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 Uniform Buffer 内存失败: {}", e))?
        };

        Ok((buffer, buffer_memory, mapped))
    }
    pub(crate) fn init_command_pool(&mut self) -> Result<(), String> {
        let pool_create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.graphics_queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        self.command_pool = unsafe {
            self.device
                .create_command_pool(&pool_create_info, None)
                .map_err(|e| format!("创建命令池失败: {}", e))?
        };
        Ok(())
    }
    pub(crate) fn init_command_buffers(&mut self) -> Result<(), String> {
        // 🔴 数量 = **在飞帧数**，不是交换链图像数：命令缓冲与 `in_flight_fences[slot]`
        // 一对一，`render()` 按 `current_frame` 取（判据见
        // `command_buffer_is_indexed_by_frame_slot_not_by_swapchain_image`）。
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.max_frames_in_flight as u32);

        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配命令缓冲失败: {}", e))?
        };

        // 占位录制（每帧都会重录）：图像下标只用来选 framebuffer，取模防止
        // 交换链图像数 < 在飞帧数时越界。
        let fbs = self.framebuffers.len().max(1);
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i % fbs, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
        }
        Ok(())
    }
    pub(crate) fn init_sync_objects(&mut self) -> Result<(), String> {
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info =
            vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        // image-available 与 fence **按在飞帧**分配：围栏在每帧开头就被等到，
        // 所以轮到同一个槽位时，上一次等待它的 submit 必然已经完成 ⇒ 复用合法。
        for _ in 0..self.max_frames_in_flight {
            let image_available = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建信号量失败: {}", e))?
            };
            let fence = unsafe {
                self.device
                    .create_fence(&fence_create_info, None)
                    .map_err(|e| format!("创建围栏失败: {}", e))?
            };
            self.image_available_semaphores.push(image_available);
            self.in_flight_fences.push(fence);
        }
        // render-finished **按交换链图像**分配（理由见该函数文档）
        self.resize_render_finished_semaphores()?;
        Ok(())
    }
    /// 按**当前交换链图像数**重排 render-finished 信号量。
    ///
    /// ## 为什么不能按「在飞帧」分配（2026-09-15 由验证层抓出）
    ///
    /// 原来是 `render_finished_semaphores[current_frame]`（在飞帧 = 2），而交换链有 **3** 张图像。
    /// 打开验证层（`RV3D_VALIDATION=1`，本轮 mesh.spv 修好后才第一次真跑起来）立刻报：
    ///
    /// ```text
    /// vkQueueSubmit(): pSubmits[0].pSignalSemaphores[0] (VkSemaphore 0x910000000091) is being
    /// signaled by VkQueue ..., but it may still be in use by VkSwapchainKHR ...
    /// Most recently acquired image indices: [0], 1, 2.
    /// (Brackets mark the last use of VkSemaphore ... in a presentation operation.)
    /// VUID-vkQueueSubmit-pSignalSemaphores-00067
    /// ```
    ///
    /// 方括号标出那个信号量最后是被**图像 0** 的 present 用掉的 ——
    /// `vkQueuePresentKHR` **不保证**在 `vkQueueSubmit` 返回时就已经消费掉等待的信号量，
    /// 于是在飞帧轮回到同一槽位时会**重复 signal 一个仍被 present 持有的二值信号量**。
    ///
    /// 改成「每张交换链图像一个」之后契约才成立：`vkAcquireNextImageKHR` 返回图像 i
    /// **本身就保证**图像 i 不再被使用（那次 present 已执行完并释放它），
    /// 所以此刻重新 signal `render_finished[i]` 是合法的。
    ///
    /// ⚠ 调用前必须保证**设备已空闲**（`recreate_swapchain` 开头就 `wait_idle()`）：
    /// 销毁可能仍在被 pending present 等待的信号量同样是未定义行为。
    pub(crate) fn resize_render_finished_semaphores(&mut self) -> Result<(), String> {
        let want = self.swapchain_images.len();
        if self.render_finished_semaphores.len() == want {
            // 图像数没变：`recreate_swapchain` 已经 `wait_idle()`，旧信号量必然已无主，直接复用
            return Ok(());
        }
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        for semaphore in std::mem::take(&mut self.render_finished_semaphores) {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        for _ in 0..want {
            let semaphore = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建 render-finished 信号量失败: {}", e))?
            };
            self.render_finished_semaphores.push(semaphore);
        }
        log::info!(
            "render-finished 信号量重排为 {} 个（= 交换链图像数，不再跟在飞帧数 {} 走）",
            self.render_finished_semaphores.len(),
            self.max_frames_in_flight
        );
        Ok(())
    }

    // ============================================================
    // 画质预设 / PNG 截图（公开 API）
    // ============================================================
    /// 重建交换链（窗口尺寸变化 / `交换链过期` / 5 秒尺寸自检三条路都调它）。
    /// 🔴 开头**必须** `wait_idle()`：销毁可能仍在被 pending present 等待的信号量与
    /// framebuffer 是未定义行为（见 `resize_render_finished_semaphores` 的文档）。
    ///
    /// 🔴 2026-09-25 复查补：这个函数**先销毁再重建**，中间有 8 个可能失败的步骤，而
    /// 调用方（`main.rs` 三处）以前都 `let _ =` 把错误丢掉 ⇒ 一旦中途失败，渲染器就带着
    /// **半销毁**的状态继续每帧 acquire/提交（拿空句柄调 Vulkan，日志里只有一串含义不明的报错）。
    /// 现在失败一律置 `swapchain_broken` 降级：不再提交帧，等下一次
    /// 重建**成功**时自动恢复。判据 = `frame_suppressed` 的单测。
    ///
    /// 🔴 2026-09-26 再加一层：失败原因若是**设备丢失**，那"下一次成功"永远不会来
    /// （本引擎没有重建设备的路径），置 `device_lost` 之后 `swapchain_recovery_allowed()`
    /// 恒假 ⇒ 调用方不再重试、不再刷日志。见该字段与 `is_device_lost_error` 的文档。
    /// 现在**值不值得**再试一次交换链重建（`main.rs` 三处重建入口的判据）。
    /// 语义与判据全在纯函数 `should_retry_swapchain` 里，这里只是把"距上次多久"喂进去。
    pub(crate) fn swapchain_recovery_allowed(&self) -> bool {
        should_retry_swapchain(
            self.device_lost,
            self.swapchain_broken,
            self.last_recreate_attempt.elapsed().as_secs_f32(),
        )
    }
    /// 更新「窗口物理尺寸」（`Window::inner_size()`）。**必须在 `recreate_swapchain()`
    /// 之前调用**，否则 Wayland 下重建出来的交换链还是上一次的尺寸（`currentExtent`
    /// 未定义 ⇒ 尺寸只能来自这里，见 `window_extent` 字段与 `swapchain_extent_choice`）。
    ///
    /// 为什么不让 `init_swapchain` 自己去问窗口：`init_swapchain` 只有 `&mut self`，
    /// 拿不到 `Window`，而 Vulkan 的 surface 创建与窗口生命周期是分开的两件事
    /// （见 `Renderer::new(window: &Window)`）—— 把窗口尺寸作为**显式输入**传进来，
    /// 比让渲染器持有窗口引用更不容易出借用冲突，也让这条依赖在类型上可见。
    pub(crate) fn set_window_extent(&mut self, width: u32, height: u32) {
        self.window_extent = vk::Extent2D { width, height };
    }
    pub(crate) fn recreate_swapchain(&mut self) -> Result<(), String> {
        self.last_recreate_attempt = Instant::now();
        let r = self.try_recreate_swapchain();
        self.swapchain_broken = r.is_err();
        if let Err(e) = &r {
            if is_device_lost_error(e) {
                // 不可恢复：把结论**讲明白一次**，然后彻底停下来
                self.device_lost = true;
                log::error!(
                    "设备已丢失（VK_ERROR_DEVICE_LOST，不可恢复）：停止提交与交换链重建，需要重启进程。原因：{}",
                    e
                );
            } else {
                log::error!(
                    "重建交换链失败：{} —— 进入降级（不再提交帧；下一次重建成功即恢复）",
                    e
                );
            }
        }
        r
    }
    pub(crate) fn try_recreate_swapchain(&mut self) -> Result<(), String> {
        self.wait_idle()?;
        self.destroy_swapchain();
        self.init_swapchain()?;
        // 交换链图像数可能变了 ⇒ render-finished 信号量的个数必须跟着变
        // （此处设备已空闲，销毁/重建都安全）
        self.resize_render_finished_semaphores()?;
        // 🔴 HUD overlay 的 framebuffer 绑的是**交换链 ImageView**，必须跟着重建 ——
        // 漏掉这一步就是未结案 #2：PT 通路随后用一组指向已销毁 ImageView 的 framebuffer
        // （见 `recreate_hud_framebuffers` 的文档）
        self.recreate_hud_framebuffers()?;
        self.init_msaa_resources()?;
        self.init_depth_resources()?;
        self.init_framebuffers()?;
        self.recreate_command_buffers()?;
        // 交换链尺寸/图像已变化：截图读回资源按旧 extent 创建，作废并清掉 pending 请求
        // （下次 capture_screenshot 时惰性重建）
        self.destroy_screenshot_resources();
        self.screenshot_request = None;
        Ok(())
    }
    pub(crate) fn recreate_command_buffers(&mut self) -> Result<(), String> {
        unsafe {
            self.device
                .free_command_buffers(self.command_pool, &self.command_buffers);
        }
        // 同 `init_command_buffers`：数量按**在飞帧数**（命令缓冲与围栏槽位一对一）
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.max_frames_in_flight as u32);
        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("重新分配命令缓冲失败: {}", e))?
        };
        let fbs = self.framebuffers.len().max(1);
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i % fbs, INSTANCE_COUNT, 0, TerrainLod::High as usize)?;
        }
        Ok(())
    }
    pub(crate) fn destroy_swapchain(&mut self) {
        // HUD overlay 的 framebuffer 引用 `swapchain_image_views`，**必须先于它们销毁**
        // （2026-09-15 补：此前这里漏了它，于是每次重建都留下一组悬空句柄）
        for &framebuffer in &self.hud_framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.hud_framebuffers.clear();
        for &framebuffer in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.framebuffers.clear();
        for &image_view in &self.swapchain_image_views {
            unsafe { self.device.destroy_image_view(image_view, None) };
        }
        self.swapchain_image_views.clear();
        // 深度资源
        for &view in &self.depth_image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        self.depth_image_views.clear();
        for (&image, &memory) in self
            .depth_images
            .iter()
            .zip(self.depth_images_memory.iter())
        {
            unsafe {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }
        }
        self.depth_images.clear();
        self.depth_images_memory.clear();
        // MSAA 颜色附件
        for &view in &self.msaa_image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        self.msaa_image_views.clear();
        for (&image, &memory) in self
            .msaa_images
            .iter()
            .zip(self.msaa_image_memory.iter())
        {
            unsafe {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }
        }
        self.msaa_images.clear();
        self.msaa_image_memory.clear();
        if self.swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }
            self.swapchain = vk::SwapchainKHR::null();
        }
    }
}
