// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.wait_idle_checked();

            // 2026-08-29 显存纪律：PT 常驻资源（AS/管线/图像）显式销毁——退出后驱动立刻回收！
            self.destroy_pt_resident();

            // 释放截图读回资源（staging buffer + fence）
            self.destroy_screenshot_resources();

            // 释放同步对象
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }
            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }

            // 释放命令池
            self.device.destroy_command_pool(self.command_pool, None);

            // 释放帧缓冲
            for &framebuffer in &self.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            // HUD overlay 的 framebuffer（只被 PT 通路消费，见 recreate_hud_framebuffers）
            for &framebuffer in &self.hud_framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }

            // 释放管线
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
            }
            for (b, m) in [
                (self.soldier_vertex_buffer, self.soldier_vertex_buffer_memory),
                (self.soldier_index_buffer, self.soldier_index_buffer_memory),
            ] {
                if b != vk::Buffer::null() { self.device.destroy_buffer(b, None); }
                if m != vk::DeviceMemory::null() { self.device.free_memory(m, None); }
            }
            if self.gun_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.gun_pipeline, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.pipeline_layout, None);
            }
            // 释放可选网格着色器管线（mesh_enabled=false 时为 null，直接跳过）
            if self.mesh_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.mesh_pipeline, None);
            }
            if self.mesh_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.mesh_pipeline_layout, None);
            }
            // 释放 HUD 覆盖层（独立 pipeline / 顶点缓冲）
            if self.hud_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.hud_pipeline, None);
            }
            if self.hud_overlay_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.hud_overlay_pipeline, None);
            }
            if self.hud_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.hud_pipeline_layout, None);
            }
            if self.hud_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.hud_vertex_buffer, None);
            }
            if self.hud_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.hud_vertex_buffer_memory, None);
            }
            // 🧊 释放 HUD 磨砂玻璃的那套资源（2026-09-26 补）：图像 / 内存 / 视图 / 采样器 /
            // 描述符池 + set layout。**这五样是当天新加的，当时没进释放清单** ——
            // 是 `tools/audit_vk_resources.py` 在它自己上线两小时后抓出来的
            // （"no release call found: 5"，全部指向这几个字段）。
            // 顺序：先释放依赖资源的池/layout，再拆 view/sampler，最后 image + memory。
            if self.hud_glass_pool != vk::DescriptorPool::null() {
                self.device.destroy_descriptor_pool(self.hud_glass_pool, None);
                self.hud_glass_pool = vk::DescriptorPool::null();
                self.hud_glass_set = vk::DescriptorSet::null();
            }
            if self.hud_glass_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.hud_glass_set_layout, None);
                self.hud_glass_set_layout = vk::DescriptorSetLayout::null();
            }
            if self.menu_blur_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.menu_blur_view, None);
                self.menu_blur_view = vk::ImageView::null();
            }
            if self.menu_blur_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.menu_blur_sampler, None);
                self.menu_blur_sampler = vk::Sampler::null();
            }
            if self.menu_blur_image != vk::Image::null() {
                self.device.destroy_image(self.menu_blur_image, None);
                self.menu_blur_image = vk::Image::null();
            }
            if self.menu_blur_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.menu_blur_memory, None);
                self.menu_blur_memory = vk::DeviceMemory::null();
            }
            // 释放第一人称枪模缓冲
            if self.gun_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.gun_vertex_buffer, None);
            }
            if self.gun_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.gun_vertex_buffer_memory, None);
            }
            if self.gun_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.gun_index_buffer, None);
            }
            if self.gun_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.gun_index_buffer_memory, None);
            }
            // 道具合并网格缓冲（先解映射再释放内存，顺序不能反）
            if self.prop_mapped != std::ptr::null_mut() {
                self.device.unmap_memory(self.prop_vertex_memory);
                self.prop_mapped = std::ptr::null_mut();
            }
            if self.prop_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_vertex_buffer, None);
            }
            if self.prop_vertex_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_vertex_memory, None);
            }
            if self.prop_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_index_buffer, None);
            }
            if self.prop_index_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_index_memory, None);
            }
            if self.prop_sh_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_sh_vertex_buffer, None);
            }
            if self.prop_sh_vertex_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_sh_vertex_memory, None);
            }
            if self.prop_sh_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_sh_index_buffer, None);
            }
            if self.prop_sh_index_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_sh_index_memory, None);
            }
            // 🏢 PT 道具逐三角属性表（道具进 BLAS 专项）：渲染器自有的 device-local
            //   缓冲，teardown 必须销毁，否则验证层在设备销毁时报泄漏
            if self.prop_attr_buf != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_attr_buf, None);
            }
            if self.prop_attr_mem != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_attr_mem, None);
            }
            if self.render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.render_pass, None);
            }

            // ---- 新增：释放 Descriptor 和 Uniform Buffer ----
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device.destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);
            }
            for &mapped in &self.uniform_mapped {
                // unmap 不需要判断 null，ash 会处理
                if !mapped.is_null() {
                    // 注意：ash 的 unmap 需要 DeviceMemory，我们逐个处理
                }
            }
            for (i, &buffer) in self.uniform_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.uniform_buffers_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }

            // 释放光照 Uniform Buffer
            for (i, &buffer) in self.light_uniform_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.light_uniform_buffers_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }

            // 释放阴影贴图资源（framebuffer 先于 render pass；descriptor sets 随 pool 释放）
            if self.shadow_framebuffer != vk::Framebuffer::null() {
                self.device.destroy_framebuffer(self.shadow_framebuffer, None);
            }
            if self.shadow_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.shadow_pipeline, None);
            }
            if self.shadow_pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.shadow_pipeline_layout, None);
            }
            if self.shadow_render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.shadow_render_pass, None);
            }
            if self.shadow_descriptor_set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.shadow_descriptor_set_layout, None);
            }
            for (i, &buffer) in self.shadow_ubo_buffers.iter().enumerate() {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
                if let Some(&mem) = self.shadow_ubo_memory.get(i) {
                    if mem != vk::DeviceMemory::null() {
                        self.device.free_memory(mem, None);
                    }
                }
            }
            if self.shadow_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.shadow_sampler, None);
            }
            if self.shadow_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.shadow_image_view, None);
            }
            if self.shadow_image != vk::Image::null() {
                self.device.destroy_image(self.shadow_image, None);
            }
            if self.shadow_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.shadow_image_memory, None);
            }
            // 动态阴影图（第二张）：与静态图同一套顺序（framebuffer → view → image → memory）
            if self.shadow_dyn_framebuffer != vk::Framebuffer::null() {
                self.device.destroy_framebuffer(self.shadow_dyn_framebuffer, None);
            }
            if self.shadow_dyn_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.shadow_dyn_image_view, None);
            }
            if self.shadow_dyn_image != vk::Image::null() {
                self.device.destroy_image(self.shadow_dyn_image, None);
            }
            if self.shadow_dyn_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.shadow_dyn_image_memory, None);
            }

            // 释放图像视图
            for &image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(image_view, None);
            }

            // 释放深度资源
            for &view in &self.depth_image_views {
                self.device.destroy_image_view(view, None);
            }
            for (&image, &memory) in self
                .depth_images
                .iter()
                .zip(self.depth_images_memory.iter())
            {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }

            // 释放交换链
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader.destroy_swapchain(self.swapchain, None);
            }

            // 释放顶点缓冲
            if self.vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.vertex_buffer, None);
            }
            if self.vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.vertex_buffer_memory, None);
            }
            if self.index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.index_buffer, None);
            }
            if self.index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.index_buffer_memory, None);
            }
            if self.far_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.far_vertex_buffer, None);
            }
            if self.far_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.far_vertex_buffer_memory, None);
            }
            if self.far_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.far_index_buffer, None);
            }
            if self.far_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.far_index_buffer_memory, None);
            }
            if self.ground_vertex_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.ground_vertex_buffer, None);
            }
            if self.ground_vertex_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.ground_vertex_buffer_memory, None);
            }
            if self.ground_index_buffer != vk::Buffer::null() {
                self.device.destroy_buffer(self.ground_index_buffer, None);
            }
            if self.ground_index_buffer_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.ground_index_buffer_memory, None);
            }
            // NPC 近似几何：球（头）与圆柱（四肢）。这两对缓冲由 `create_sphere_geometry` /
            // `create_cylinder_geometry` 在 init 时各建一次，**此前不在任何释放表里**
            // （2026-09-26 用 tools/audit_vk_resources.py 扫出来的）：一次性小泄漏，但退出路径
            // 不完整就会被后来照抄这段的人继续放大。
            for (b, m) in [
                (self.sphere_vertex_buffer, self.sphere_vertex_buffer_memory),
                (self.sphere_index_buffer, self.sphere_index_buffer_memory),
                (self.cylinder_vertex_buffer, self.cylinder_vertex_buffer_memory),
                (self.cylinder_index_buffer, self.cylinder_index_buffer_memory),
            ] {
                if b != vk::Buffer::null() {
                    self.device.destroy_buffer(b, None);
                }
                if m != vk::DeviceMemory::null() {
                    self.device.free_memory(m, None);
                }
            }
            // 释放地形 LOD 网格（顶点/索引缓冲）
            for mesh in &self.terrain_lods {
                if mesh.vertex_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(mesh.vertex_buffer, None);
                }
                if mesh.vertex_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(mesh.vertex_memory, None);
                }
                if mesh.index_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(mesh.index_buffer, None);
                }
                if mesh.index_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(mesh.index_memory, None);
                }
            }
            for &buffer in &self.instance_buffers {
                if buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(buffer, None);
                }
            }
            for &memory in &self.instance_buffers_memory {
                if memory != vk::DeviceMemory::null() {
                    self.device.free_memory(memory, None);
                }
            }

            // 释放纹理资源
            if self.texture_sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.texture_sampler, None);
            }
            if self.texture_image_view != vk::ImageView::null() {
                self.device.destroy_image_view(self.texture_image_view, None);
            }
            if self.texture_image != vk::Image::null() {
                self.device.destroy_image(self.texture_image, None);
            }
            if self.texture_image_memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.texture_image_memory, None);
            }
            // 释放 marker/NPC 程序化皮肤纹理 + 地面微细节层
            for (img, mem, view) in [
                (
                    self.skin_marker_image,
                    self.skin_marker_memory,
                    self.skin_marker_image_view,
                ),
                (
                    self.skin_npc_image,
                    self.skin_npc_memory,
                    self.skin_npc_image_view,
                ),
                (
                    self.ground_detail_image,
                    self.ground_detail_memory,
                    self.ground_detail_image_view,
                ),
            ] {
                if view != vk::ImageView::null() {
                    self.device.destroy_image_view(view, None);
                }
                if img != vk::Image::null() {
                    self.device.destroy_image(img, None);
                }
                if mem != vk::DeviceMemory::null() {
                    self.device.free_memory(mem, None);
                }
            }

            // 释放逻辑设备
            self.device.destroy_device(None);

            // 释放表面
            self.surface_loader.destroy_surface(self.surface, None);

            // 释放调试回调
            if let (Some(ref debug_utils), Some(messenger)) =
                (&self.debug_utils, self.debug_messenger)
            {
                debug_utils.destroy_debug_utils_messenger(messenger, None);
            }

            // 释放实例
            self.instance.destroy_instance(None);
        }
    }
}
