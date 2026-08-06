//! Vulkan 渲染器模块
//!
//! 使用 ash 0.38 初始化 Vulkan，渲染一个彩色三角形。
//! 包含完整的 Vulkan 管线生命周期管理。
//! 已接入 Uniform Buffer（相机矩阵），当前写入单位矩阵。

use std::ffi::CStr;
use std::time::Instant;
use std::fs::File;
use ash::{
    ext::debug_utils::Instance as DebugUtils,
    khr::{surface::Instance as Surface, swapchain::Device as Swapchain},
    util, vk, Device, Entry, Instance,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

// ============================================================
// 数据类型
// ============================================================

/// 相机 Uniform 数据（4x4 矩阵，64 字节）
#[repr(C)]
#[derive(Copy, Clone)]
struct CameraUniform {
    mvp: [[f32; 4]; 4],
}

impl CameraUniform {
    #[allow(dead_code)]
    /// 单位矩阵（不旋转、不缩放、不平移）
    fn identity() -> Self {
        Self {
            mvp: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

/// 三角形顶点数据
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Vertex {
    pos: [f32; 2],
    color: [f32; 3],
}

/// 三角形三个顶点（红、绿、蓝）
const VERTICES: [Vertex; 3] = [
    Vertex { pos: [-0.5, -0.5], color: [1.0, 0.0, 0.0] },
    Vertex { pos: [ 0.5, -0.5], color: [0.0, 1.0, 0.0] },
    Vertex { pos: [ 0.0,  0.5], color: [0.0, 0.0, 1.0] },
];

/// 远档十字 quad 地面距离淡出区间（地平线处自然消失）
/// FADE_END=900 保证任何可达机位（|x|,|z|<=600）最近场点距离 <=486 < 900，
/// 场外不再“实例全灭”；远角 1210 > 900 仍自然淡出（地平线无硬边）。
const FADE_START: f32 = 400.0;
const FADE_END: f32 = 900.0;

// ============================================================
// 渲染器
// ============================================================

pub struct Renderer {
    _entry: Entry,
    instance: Instance,
    #[allow(dead_code)]
    debug_utils: Option<DebugUtils>,
    #[allow(dead_code)]
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    surface_loader: Surface,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    #[allow(dead_code)]
    physical_device_properties: vk::PhysicalDeviceProperties,
    graphics_queue_family_index: u32,
    present_queue_family_index: u32,
    device: Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    swapchain_loader: Swapchain,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    swapchain_image_views: Vec<vk::ImageView>,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    current_frame: usize,
    max_frames_in_flight: usize,
    first_frame_done: bool,
    vertex_buffer: vk::Buffer,
    vertex_buffer_memory: vk::DeviceMemory,
    // ---- 新增：Uniform / Descriptor 相关 ----
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_sets: Vec<vk::DescriptorSet>,
    uniform_buffers: Vec<vk::Buffer>,
    uniform_buffers_memory: Vec<vk::DeviceMemory>,
    uniform_mapped: Vec<*mut std::ffi::c_void>,
    start_time: Instant,
}

fn load_spirv(path: &str) -> Result<Vec<u32>, String> {
    let mut file = File::open(path).map_err(|e| format!("打开着色器文件失败 '{}': {}", path, e))?;
    util::read_spv(&mut file).map_err(|e| format!("读取 SPIR-V 文件失败 '{}': {}", path, e))
}

impl Renderer {
    pub fn new(window: &Window) -> Result<Self, String> {
        let mut renderer = Self::init_instance(window)?;
        renderer.init_swapchain()?;
        renderer.init_render_pass()?;
        renderer.init_descriptors()?;       // ← 新增
        renderer.init_pipeline()?;
        renderer.init_framebuffers()?;
        renderer.init_command_buffers()?;
        renderer.init_sync_objects()?;
        Ok(renderer)
    }

    // ============================================================
    // 初始化步骤
    // ============================================================

    fn init_instance(window: &Window) -> Result<Self, String> {
        let entry =
            unsafe { Entry::load().map_err(|e| format!("无法加载 Vulkan 库: {}", e))? };

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"Steel Front")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"Steel Front Engine")
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let window_extensions = {
            let display_handle = window
                .display_handle()
                .map_err(|e| format!("获取显示句柄失败: {:?}", e))?
                .as_raw();
            ash_window::enumerate_required_extensions(display_handle)
                .map_err(|e| format!("无法获取窗口所需扩展: {:?}", e))?
        };
        let mut required_extensions: Vec<*const i8> = window_extensions.to_vec();
        required_extensions.push(c"VK_EXT_debug_utils".as_ptr());
        let ext_names = required_extensions.as_slice();

        let layer_names = [c"VK_LAYER_KHRONOS_validation"];
        let layers: Vec<*const i8> = layer_names.iter().map(|l| l.as_ptr()).collect();

        let layer_properties = unsafe {
            entry
                .enumerate_instance_layer_properties()
                .map_err(|e| format!("无法枚举实例层属性: {}", e))?
        };
        let has_validation = layer_properties.iter().any(|prop| {
            let name = unsafe { CStr::from_ptr(prop.layer_name.as_ptr()) };
            name.to_bytes_with_nul() == b"VK_LAYER_KHRONOS_validation\0"
        });
        if has_validation {
            log::info!("验证层可用，已启用");
        } else {
            log::warn!("验证层不可用，将不使用");
        }

        let instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(ext_names)
            .enabled_layer_names(if has_validation { &layers } else { &[] });

        let instance = unsafe {
            entry
                .create_instance(&instance_create_info, None)
                .map_err(|e| format!("创建 Vulkan 实例失败: {}", e))?
        };

        let debug_utils = if has_validation {
            let debug_utils_loader = DebugUtils::new(&entry, &instance);
            let debug_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR

                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL

                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(vulkan_debug_callback));
            let messenger = unsafe {
                debug_utils_loader
                    .create_debug_utils_messenger(&debug_create_info, None)
                    .expect("创建调试报告器失败")
            };
            Some((debug_utils_loader, messenger))
        } else {
            None
        };

        let surface = {
            let raw_handle = window
                .window_handle()
                .map_err(|e| format!("获取窗口句柄失败: {:?}", e))?
                .as_raw();
            let display_handle = window
                .display_handle()
                .map_err(|e| format!("获取显示句柄失败: {:?}", e))?
                .as_raw();
            unsafe {
                ash_window::create_surface(&entry, &instance, display_handle, raw_handle, None)
                    .map_err(|e| format!("创建 Vulkan 表面失败: {:?}", e))?
            }
        };
        let surface_loader = Surface::new(&entry, &instance);

        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(|e| format!("枚举物理设备失败: {}", e))?
        };
        if physical_devices.is_empty() {
            return Err("没有找到支持 Vulkan 的 GPU".to_string());
        }

        let (physical_device, physical_device_properties) = physical_devices
            .iter()
            .filter_map(|&device| {
                let properties = unsafe { instance.get_physical_device_properties(device) };
                let surface_support = unsafe {
                    surface_loader
                        .get_physical_device_surface_support(device, 0, surface)
                        .unwrap_or(false)
                };
                if surface_support {
                    Some((device, properties))
                } else {
                    None
                }
            })
            .max_by_key(|&(_, props)| match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 2,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                _ => 0,
            })
            .ok_or_else(|| "没有找到合适的物理设备".to_string())?;

        let device_name = unsafe {
            CStr::from_ptr(physical_device_properties.device_name.as_ptr())
                .to_string_lossy()
                .to_string()
        };
        log::info!("选择物理设备: {}", device_name);

        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

        let graphics_queue_family_index = queue_families
            .iter()
            .position(|qf| qf.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| "没有找到图形队列族".to_string())?
            as u32;

        let present_queue_family_index = queue_families
            .iter()
            .enumerate()
            .find(|(i, _)| unsafe {
                surface_loader
                    .get_physical_device_surface_support(physical_device, *i as u32, surface)
                    .unwrap_or(false)
            })
            .map(|(i, _)| i as u32)
            .ok_or_else(|| "没有找到呈现队列族".to_string())?;

        let queue_priorities = [1.0_f32];
        let mut queue_indices = vec![graphics_queue_family_index];
        if present_queue_family_index != graphics_queue_family_index {
            queue_indices.push(present_queue_family_index);
        }
        queue_indices.sort();
        queue_indices.dedup();

        let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = queue_indices
            .iter()
            .map(|&index| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(index)
                    .queue_priorities(&queue_priorities)
            })
            .collect();

        let swapchain_ext_name = c"VK_KHR_swapchain";
        let device_extensions = [swapchain_ext_name.as_ptr()];
        let physical_device_features = vk::PhysicalDeviceFeatures::default();

        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&physical_device_features);

        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|e| format!("创建逻辑设备失败: {}", e))?
        };

        let graphics_queue = unsafe { device.get_device_queue(graphics_queue_family_index, 0) };
        let present_queue = unsafe { device.get_device_queue(present_queue_family_index, 0) };

        let (debug_utils_loader, debug_messenger) = match debug_utils {
            Some((loader, messenger)) => (Some(loader), Some(messenger)),
            None => (None, None),
        };

        let swapchain_loader = Swapchain::new(&instance, &device);

        Ok(Self {
            _entry: entry,
            instance,
            debug_utils: debug_utils_loader,
            debug_messenger,
            surface_loader,
            surface,
            physical_device,
            physical_device_properties,
            graphics_queue_family_index,
            present_queue_family_index,
            device,
            graphics_queue,
            present_queue,
            swapchain_loader,
            swapchain: vk::SwapchainKHR::null(),
            swapchain_images: Vec::new(),
            swapchain_format: vk::Format::UNDEFINED,
            swapchain_extent: vk::Extent2D::default(),
            swapchain_image_views: Vec::new(),
            render_pass: vk::RenderPass::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            framebuffers: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            image_available_semaphores: Vec::new(),
            render_finished_semaphores: Vec::new(),
            in_flight_fences: Vec::new(),
            current_frame: 0,
            max_frames_in_flight: 2,
            first_frame_done: false,
            vertex_buffer: vk::Buffer::null(),
            vertex_buffer_memory: vk::DeviceMemory::null(),
            // ---- 新增字段初始值 ----
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            descriptor_sets: Vec::new(),
            uniform_buffers: Vec::new(),
            uniform_buffers_memory: Vec::new(),
            uniform_mapped: Vec::new(),
            start_time: Instant::now(),
        })
    }

    fn init_swapchain(&mut self) -> Result<(), String> {
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
        let present_mode = present_modes
            .iter()
            .find(|&&m| m == vk::PresentModeKHR::MAILBOX)
            .copied()
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = if surface_capabilities.current_extent.width != u32::MAX {
            surface_capabilities.current_extent
        } else {
            vk::Extent2D { width: 1280, height: 720 }
        };

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

        let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(surface_capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        self.swapchain = unsafe {
            self.swapchain_loader
                .create_swapchain(&swapchain_create_info, None)
                .map_err(|e| format!("创建交换链失败: {}", e))?
        };
        self.swapchain_images = unsafe {
            self.swapchain_loader
                .get_swapchain_images(self.swapchain)
                .map_err(|e| format!("获取交换链图像失败: {}", e))?
        };
        self.swapchain_format = format.format;
        self.swapchain_extent = extent;

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
                        .expect("创建图像视图失败")
                }
            })
            .collect();

        log::info!(
            "交换链初始化完成: {}x{}, 格式: {:?}, 图像数: {}",
            extent.width, extent.height, format.format, image_count
        );
        Ok(())
    }

    fn init_render_pass(&mut self) -> Result<(), String> {
        let color_attachment = vk::AttachmentDescription::default()
            .format(self.swapchain_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        let color_attachment_refs = [color_attachment_ref];

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs);
        let subpasses = [subpass];
        let attachments = [color_attachment];

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
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

    // ============================================================
    // 新增：初始化 Descriptor（Uniform Buffer + 布局 + 池 + 分配）
    // ============================================================
    fn init_descriptors(&mut self) -> Result<(), String> {
        let max_frames = self.max_frames_in_flight;

        // ---- 1. 创建 Descriptor Set Layout ----
        // 描述：binding=0, 类型=UNIFORM_BUFFER, 阶段=VERTEX
        let ubo_layout_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let bindings = [ubo_layout_binding];

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&bindings);

        self.descriptor_set_layout = unsafe {
            self.device
                .create_descriptor_set_layout(&layout_info, None)
                .map_err(|e| format!("创建 Descriptor Set Layout 失败: {}", e))?
        };

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

        // ---- 3. 创建 Descriptor Pool ----
        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(max_frames as u32);
        let pool_sizes = [pool_size];

        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(max_frames as u32);

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
        }

        log::info!("Descriptor 初始化完成（{} 帧）", max_frames);
        Ok(())
    }

    fn init_pipeline(&mut self) -> Result<(), String> {
        let vs_spirv = load_spirv("assets/triangle.vert.spv")?;
        let fs_spirv = load_spirv("assets/triangle.frag.spv")?;
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
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(std::mem::size_of::<[f32; 2]>() as u32),
        ];

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
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::ALWAYS);

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

        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_state)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
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

        unsafe {
            self.device.destroy_shader_module(vs_module, None);
            self.device.destroy_shader_module(fs_module, None);
        }

        self.create_vertex_buffer()?;
        log::info!("图形管线创建完成");
        Ok(())
    }

    fn create_shader_module(&self, spirv: &[u32]) -> Result<vk::ShaderModule, String> {
        let create_info = vk::ShaderModuleCreateInfo::default().code(spirv);
        unsafe {
            self.device
                .create_shader_module(&create_info, None)
                .map_err(|e| format!("创建着色器模块失败: {}", e))
        }
    }

    fn create_vertex_buffer(&mut self) -> Result<(), String> {
        let buffer_size = std::mem::size_of_val(&VERTICES) as u64;

        let buffer_create_info = vk::BufferCreateInfo::default()
            .size(buffer_size)
            .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        self.vertex_buffer = unsafe {
            self.device
                .create_buffer(&buffer_create_info, None)
                .map_err(|e| format!("创建顶点缓冲失败: {}", e))?
        };

        let mem_requirements = unsafe {
            self.device.get_buffer_memory_requirements(self.vertex_buffer)
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
            .ok_or_else(|| "没有找到合适的内存类型".to_string())?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type);

        self.vertex_buffer_memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配顶点缓冲内存失败: {}", e))?
        };

        unsafe {
            self.device
                .bind_buffer_memory(self.vertex_buffer, self.vertex_buffer_memory, 0)
                .map_err(|e| format!("绑定缓冲内存失败: {}", e))?;
        }

        let data_ptr = unsafe {
            self.device
                .map_memory(
                    self.vertex_buffer_memory,
                    0,
                    buffer_size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("映射顶点缓冲内存失败: {}", e))?
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                VERTICES.as_ptr() as *const u8,
                data_ptr as *mut u8,
                buffer_size as usize,
            );
            self.device.unmap_memory(self.vertex_buffer_memory);
        }
        Ok(())
    }

    fn init_framebuffers(&mut self) -> Result<(), String> {
        self.framebuffers = self
            .swapchain_image_views
            .iter()
            .map(|&image_view| {
                let image_views = [image_view];
                let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&image_views)
                    .width(self.swapchain_extent.width)
                    .height(self.swapchain_extent.height)
                    .layers(1);
                unsafe {
                    self.device
                        .create_framebuffer(&framebuffer_create_info, None)
                        .expect("创建帧缓冲失败")
                }
            })
            .collect();
        Ok(())
    }

    fn init_command_buffers(&mut self) -> Result<(), String> {
        let pool_create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.graphics_queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        self.command_pool = unsafe {
            self.device
                .create_command_pool(&pool_create_info, None)
                .map_err(|e| format!("创建命令池失败: {}", e))?
        };

        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.framebuffers.len() as u32);

        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配命令缓冲失败: {}", e))?
        };

        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i)?;
        }
        Ok(())
    }

    fn record_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
    ) -> Result<(), String> {
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::SIMULTANEOUS_USE);

        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(|e| format!("开始命令缓冲失败: {}", e))?;
        }

        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.1, 0.1, 0.15, 1.0],
            },
        };
        let clear_values = [clear_color];

        let render_pass_begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[image_index])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swapchain_extent,
            })
            .clear_values(&clear_values);

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin_info,
                vk::SubpassContents::INLINE,
            );
        }

        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
        }

        // ---- 新增：绑定 Descriptor Set ----
        // 用 current_frame 对应的 descriptor_set
        // 注意：record 时 image_index 可能 != current_frame，
        // 但因为我们每帧写的是 uniform_buffers[current_frame]，
        // 这里绑定 current_frame 对应的 set 即可。
        // 简化处理：绑定 image_index % max_frames 对应的 set
        let ds_index = image_index % self.max_frames_in_flight;
        let descriptor_sets = [self.descriptor_sets[ds_index]];
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &descriptor_sets,
                &[],
            );
        }

        let vertex_buffers = [self.vertex_buffer];
        let offsets = [0u64];
        unsafe {
            self.device.cmd_bind_vertex_buffers(
                command_buffer,
                0,
                &vertex_buffers,
                &offsets,
            );
        }

        unsafe {
            self.device.cmd_draw(command_buffer, 3, 1, 0, 0);
        }

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
            self.device
                .end_command_buffer(command_buffer)
                .map_err(|e| format!("结束命令缓冲失败: {}", e))?;
        }
        Ok(())
    }

    fn init_sync_objects(&mut self) -> Result<(), String> {
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info =
            vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        for _ in 0..self.max_frames_in_flight {
            let image_available = unsafe {
                self.device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| format!("创建信号量失败: {}", e))?
            };
            let render_finished = unsafe {
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
            self.render_finished_semaphores.push(render_finished);
            self.in_flight_fences.push(fence);
        }
        Ok(())
    }

    // ============================================================
    // 渲染循环
    // ============================================================

    pub fn render(&mut self, view_proj: [[f32; 4]; 4]) -> Result<(), String> {
        let fence = self.in_flight_fences[self.current_frame];
        unsafe {
            self.device
                .wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|e| format!("等待围栏失败: {}", e))?;
        }

        let (image_index, suboptimal) = unsafe {
            self.swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    self.image_available_semaphores[self.current_frame],
                    vk::Fence::null(),
                )
                .map_err(|e| match e {
                    vk::Result::ERROR_OUT_OF_DATE_KHR => "交换链过期".to_string(),
                    vk::Result::ERROR_SURFACE_LOST_KHR => "表面丢失".to_string(),
                    _ => format!("获取交换链图像失败: {}", e),
                })?
        };

        if suboptimal {
            log::warn!("交换链 SUBOPTIMAL，重建...");
            return Err("交换链过期".to_string());
        }

        unsafe {
            self.device
                .reset_fences(&[fence])
                .map_err(|e| format!("重置围栏失败: {}", e))?;
        }

        // ---- 新增：每帧把相机矩阵写进 Uniform Buffer ----
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let model = glam::Mat4::from_rotation_y(elapsed * 0.8);
        let mvp = glam::Mat4::from_cols_array_2d(&view_proj) * model;
        let cam = CameraUniform {
            mvp: mvp.to_cols_array_2d(),
        };
        if let Some(&ptr) = self.uniform_mapped.get(self.current_frame) {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &cam as *const _ as *const u8,
                    ptr as *mut u8,
                    std::mem::size_of::<CameraUniform>(),
                );
            }
        }

        let wait_semaphores = [self.image_available_semaphores[self.current_frame]];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal_semaphores = [self.render_finished_semaphores[self.current_frame]];
        let cmd_buffers = [self.command_buffers[image_index as usize]];

        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&cmd_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("提交队列失败: {}", e))?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let present_result = unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        };

        if let Err(vk::Result::ERROR_OUT_OF_DATE_KHR) = present_result {
            log::warn!("呈现 OUT_OF_DATE，重建交换链...");
            return Err("交换链过期".to_string());
        }
        if let Ok(true) = present_result {
            log::warn!("呈现 SUBOPTIMAL，重建交换链...");
            return Err("交换链过期".to_string());
        }

        if !self.first_frame_done {
            self.first_frame_done = true;
            println!("=== RENDERER OK ===");
        }

        self.current_frame = (self.current_frame + 1) % self.max_frames_in_flight;
        Ok(())
    }

    pub fn wait_idle(&self) -> Result<(), String> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| format!("等待设备空闲失败: {}", e))
        }
    }

    #[allow(dead_code)]
    pub fn recreate_swapchain(&mut self) -> Result<(), String> {
        self.wait_idle()?;
        self.destroy_swapchain();
        self.init_swapchain()?;
        self.init_framebuffers()?;
        self.recreate_command_buffers()?;
        Ok(())
    }

    fn recreate_command_buffers(&mut self) -> Result<(), String> {
        unsafe {
            self.device
                .free_command_buffers(self.command_pool, &self.command_buffers);
        }
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(self.framebuffers.len() as u32);
        self.command_buffers = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("重新分配命令缓冲失败: {}", e))?
        };
        for (i, &command_buffer) in self.command_buffers.iter().enumerate() {
            self.record_command_buffer(command_buffer, i)?;
        }
        Ok(())
    }

    fn destroy_swapchain(&mut self) {
        for &framebuffer in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        self.framebuffers.clear();
        for &image_view in &self.swapchain_image_views {
            unsafe { self.device.destroy_image_view(image_view, None) };
        }
        self.swapchain_image_views.clear();
        if self.swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }
            self.swapchain = vk::SwapchainKHR::null();
        }
    }
}

// ============================================================
// Drop：释放所有资源
// ============================================================

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();

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

            // 释放管线
            if self.pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.pipeline, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device.destroy_pipeline_layout(self.pipeline_layout, None);
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

            // 释放图像视图
            for &image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(image_view, None);
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

// ============================================================
// 调试回调
// ============================================================

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    let severity = if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        "ERROR"
    } else if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        "WARNING"
    } else if message_severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        "INFO"
    } else {
        "VERBOSE"
    };

    let ty = if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL) {
        "General"
    } else if message_type.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION) {
        "Validation"
    } else {
        "Performance"
    };

    if let Some(data) = p_callback_data.as_ref() {
        let msg = if data.p_message.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(data.p_message) }
                .to_string_lossy()
                .to_string()
        };
        match severity {
            "ERROR" => log::error!("[Vulkan][{}] {}", ty, msg),
            "WARNING" => log::warn!("[Vulkan][{}] {}", ty, msg),
            _ => log::info!("[Vulkan][{}] {}", ty, msg),
        }
    }
    vk::FALSE
}