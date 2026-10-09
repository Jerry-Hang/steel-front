// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    pub(crate) fn new(window: &Window) -> Result<Self, String> {
        let mut renderer = Self::init_instance(window)?;
        renderer.init_swapchain()?;
        renderer.init_render_pass()?;
        renderer.init_command_pool()?;
        renderer.create_instance_buffer()?;
        renderer.init_msaa_resources()?;
        renderer.init_depth_resources()?;
        renderer.init_descriptors()?;       // ← 新增
        // 🧊 磨砂玻璃的背景图 / 采样器 / 描述符集：**必须早于 `init_hud`**（HUD 的
        // pipeline layout 引用 `hud_glass_set_layout`）。
        renderer.init_menu_blur()?;
        renderer.init_hud_glass_set()?;
        renderer.init_pipeline()?;
        renderer.init_mesh_pipeline()?;
        renderer.init_hud()?;
        renderer.init_framebuffers()?;
        renderer.init_hud_overlay()?;
        renderer.init_texture()?;
        renderer.init_shadow_resources()?;
        renderer.init_shadow_pipeline()?;
        renderer.update_texture_descriptor_sets()?;
        renderer.init_command_buffers()?;
        renderer.init_sync_objects()?;
        Ok(renderer)
    }

    // ============================================================
    // 初始化步骤
    // ============================================================
    pub(crate) fn init_instance(window: &Window) -> Result<Self, String> {
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
        let mut required_extensions: Vec<RawCString> = window_extensions
            .iter()
            .map(|&p| p as RawCString)
            .collect();
        required_extensions.push(c"VK_EXT_debug_utils".as_ptr() as RawCString);
        let ext_names = required_extensions.as_slice();

        let layer_names = [c"VK_LAYER_KHRONOS_validation"];
        let layers: Vec<RawCString> = layer_names
            .iter()
            .map(|l| l.as_ptr() as RawCString)
            .collect();

        let layer_properties = unsafe {
            entry
                .enumerate_instance_layer_properties()
                .map_err(|e| format!("无法枚举实例层属性: {}", e))?
        };
        let has_validation = std::env::var("RV3D_VALIDATION").map(|v| v == "1").unwrap_or(false)
            && layer_properties.iter().any(|prop| {
                let name = unsafe { CStr::from_ptr(prop.layer_name.as_ptr()) };
                name.to_bytes_with_nul() == b"VK_LAYER_KHRONOS_validation\0"
            });
        if has_validation {
            log::info!("RV3D_VALIDATION=1 且验证层可用，已启用");
        } else {
            log::warn!("RV3D_VALIDATION 未设置或验证层不可用，将不使用验证层（驱动宽松行为）");
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
                    .map_err(|e| format!("创建调试报告器失败: {e}"))?
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

        // 🔴 2026-09-23：设备选择从"写死优先独显"改为**可指定**（`RV3D_GPU`）。
        // 起因：验证时 dGPU 可能被别的任务占着（用户在跑 AI），而本机是**双 GPU 笔记本**
        // （RTX 5060 Laptop + AMD Radeon 集显）—— 没有这个开关就只能跑在独显上。
        // 默认仍是"有窗口表面的设备里优先独显"⇒ 不设 `RV3D_GPU` 时行为与从前逐字一致。
        let candidates: Vec<(vk::PhysicalDeviceType, String, vk::PhysicalDevice)> = physical_devices
            .iter()
            .filter_map(|&device| {
                let properties = unsafe { instance.get_physical_device_properties(device) };
                let surface_support = unsafe {
                    surface_loader
                        .get_physical_device_surface_support(device, 0, surface)
                        .unwrap_or(false)
                };
                if surface_support {
                    let name = unsafe {
                        CStr::from_ptr(properties.device_name.as_ptr())
                            .to_string_lossy()
                            .to_string()
                    };
                    Some((properties.device_type, name, device))
                } else {
                    None
                }
            })
            .collect();
        if candidates.is_empty() {
            return Err("没有找到支持本窗口表面的物理设备".to_string());
        }
        let gpu_pref = parse_gpu_preference(std::env::var("RV3D_GPU").ok().as_deref());
        let pick_list: Vec<(vk::PhysicalDeviceType, String)> =
            candidates.iter().map(|(t, n, _)| (*t, n.clone())).collect();
        let picked = pick_physical_device(&pick_list, &gpu_pref).ok_or_else(|| {
            // ⚠️ 匹配不到时**报错退出**，不静默回退到独显 —— 否则"以为在核显上验的"会是假的
            let available: Vec<&str> = candidates.iter().map(|(_, n, _)| n.as_str()).collect();
            format!(
                "RV3D_GPU={:?} 没有匹配到任何设备（可选：{}；也可用 igpu/dgpu）",
                gpu_pref,
                available.join(" / ")
            )
        })?;
        let (physical_device_type, picked_name, physical_device) = candidates[picked].clone();
        let physical_device_properties =
            unsafe { instance.get_physical_device_properties(physical_device) };
        let device_name = picked_name;
        log::info!(
            "选择物理设备: {}（{:?}；RV3D_GPU={:?}）",
            device_name,
            physical_device_type,
            gpu_pref
        );
        // GPU 硬件能力探测：光追/Tensor Core/DLSS 可用性判定（仅日志，不影响初始化）
        crate::engine::gpu_caps::log_gpu_hardware_caps(&instance, physical_device, &device_name);

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
        let mesh_shader_ext_name = c"VK_EXT_mesh_shader";
        // 设备**实际支持**的扩展名（只枚举一次；下面 mesh 与光追两组都基于它判定）。
        // 旧代码把这次枚举关在 mesh 那个块里，于是光追那组只能"先无条件请求、事后打日志"——
        // 那正是"缺扩展就起不来"的来源。
        let device_ext_names: Vec<String> = unsafe {
            instance
                .enumerate_device_extension_properties(physical_device)
                .unwrap_or_default()
                .iter()
                .map(|e| {
                    CStr::from_ptr(e.extension_name.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        };
        // ---- 可选网格着色器路径：检测 VK_EXT_mesh_shader（仿 gpu_caps.rs 枚举模式）。
        //      设备没有 VK_EXT_mesh_shader 时 mesh_enabled=false，设备创建与旧代码逐字节一致。
        //      （历史上这条是在 WSLg/dzn 上实测出来的缺失；那段环境已作废，但回退路径照旧有效。）
        //      支持时：扩展加入 enabled_extension_names，并把
        //      PhysicalDeviceMeshShaderFeaturesEXT(mesh_shader=true) 挂到 pNext 链
        //      （task_shader 不启用：本设计为纯 mesh 阶段，无 task 阶段）。
        let mesh_shader_available = {
            if device_ext_names.iter().any(|n| n == "VK_EXT_mesh_shader") {
                let mut mesh_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
                let mut f2 = vk::PhysicalDeviceFeatures2::default();
                f2.p_next = &mut mesh_features as *mut _ as *mut std::ffi::c_void;
                unsafe {
                    instance.get_physical_device_features2(physical_device, &mut f2);
                }
                if mesh_features.mesh_shader == vk::TRUE {
                    log::info!("VK_EXT_mesh_shader 可用：启用可选网格着色器渲染路径");
                    true
                } else {
                    log::warn!(
                        "VK_EXT_mesh_shader 扩展存在但 meshShader 特性不可用，回退传统顶点管线"
                    );
                    false
                }
            } else {
                log::info!("VK_EXT_mesh_shader 不可用：使用传统顶点渲染路径");
                false
            }
        };

        // A/B 取证开关（2026-10-09）：强制走传统顶点管线，用于验证两条路径等价性。
        let mesh_shader_available = if matches!(std::env::var("RV3D_NO_MESH").as_deref(), Ok("1")) {
            log::warn!("RV3D_NO_MESH=1：强制关闭网格着色器，走传统顶点管线（A/B 取证）");
            false
        } else {
            mesh_shader_available
        };

        // 光追扩展组：**全有或全无**（理由见 `pick_device_extensions` 的文档）。
        // 只有真的全齐、且 mesh 路径可用时才启用 —— PT 是默认关的可选功能，
        // 缺扩展的正确后果是"本局没有 RT"，不是"游戏起不来"。
        const RT_DEVICE_EXTENSIONS: [&str; 5] = [
            "VK_KHR_buffer_device_address",
            "VK_KHR_deferred_host_operations",
            "VK_KHR_acceleration_structure",
            "VK_KHR_ray_query",
            "VK_KHR_ray_tracing_pipeline",
        ];
        let (rt_enabled, rt_missing) =
            pick_device_extensions(&device_ext_names, &[], &RT_DEVICE_EXTENSIONS);
        let rt_available = mesh_shader_available && rt_missing.is_empty();
        if rt_missing.is_empty() {
            log::info!("device-create: 光追扩展全齐，启用 {:?}", rt_enabled);
        } else {
            log::warn!(
                "device-create: 光追扩展缺 {:?} ⇒ **整组不启用**（半套是未定义行为）；\
                 本局无 RT/PT，其余渲染路径不受影响",
                rt_missing
            );
        }

        // 设备创建：按**实际支持**逐个启用（mesh 可用时追加 mesh；光追组全齐才追加）。
        let mut device_extensions: Vec<RawCString> = vec![swapchain_ext_name.as_ptr()];
        if mesh_shader_available {
            device_extensions.push(mesh_shader_ext_name.as_ptr());
        }
        if rt_available {
            // 2026-08-29 路径追踪基准：启用光线追踪核心扩展（ray_query 计算侧；AS 构建）
            device_extensions.push(c"VK_KHR_buffer_device_address".as_ptr());
            device_extensions.push(c"VK_KHR_deferred_host_operations".as_ptr());
            device_extensions.push(c"VK_KHR_acceleration_structure".as_ptr());
            device_extensions.push(c"VK_KHR_ray_query".as_ptr());
            device_extensions.push(c"VK_KHR_ray_tracing_pipeline".as_ptr());
        }
        let supported_features =
            unsafe { instance.get_physical_device_features(physical_device) };
        let mut physical_device_features = vk::PhysicalDeviceFeatures::default();
        physical_device_features.sampler_anisotropy = supported_features.sampler_anisotropy;

        let mut mesh_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default().mesh_shader(true);
        // 2026-08-29：RT 特性链（rayQuery + accelerationStructure features——扩展启用 ≠ 特性启用！）
        let mut rq_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        rq_features.ray_query = vk::TRUE;
        let mut as_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_features.acceleration_structure = vk::TRUE;
        let mut bda_features = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
        bda_features.buffer_device_address = vk::TRUE;
        // 链到 mesh 特性（若无 mesh 则直接挂在 device_create_info.pNext）
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&physical_device_features);
        let device_create_info = if mesh_shader_available {
            device_create_info.push_next(&mut mesh_features)
        } else {
            device_create_info
        };
        // RT 特性链（Ext 启用 ≠ Feature 启用；rayQuery/accelStructure 必须显式 true）。
        // 🔴 **必须与扩展启用同步**：只启用扩展不启用特性 = 功能不可用；
        // 而**扩展没启用却把特性结构挂进 pNext 本身就是无效用法** ——
        // 旧代码无条件挂这三个，是这次一并修掉的第二处。
        let device_create_info = if rt_available {
            device_create_info
                .push_next(&mut as_features)
                .push_next(&mut bda_features)
                .push_next(&mut rq_features)
        } else {
            device_create_info
        };

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
        let mesh_shader_loader = if mesh_shader_available {
            Some(MeshShaderDevice::new(&instance, &device))
        } else {
            None
        };

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
            // 播种窗口物理尺寸（见字段文档：Wayland 的 currentExtent 未定义时，
            // 这是交换链尺寸唯一的真实来源）。此刻窗口可能还没收到首个 configure，
            // 尺寸为 0 ⇒ `swapchain_extent_choice` 会退到兜底值，
            // 随后 `Resized` 会更新本字段并重建。
            window_extent: {
                let s = window.inner_size();
                vk::Extent2D {
                    width: s.width,
                    height: s.height,
                }
            },
            swapchain_image_views: Vec::new(),
            render_pass: vk::RenderPass::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            gun_pipeline: vk::Pipeline::null(),
            // 2026-09-05：**恢复网格着色器为唯一主渲染路径**（AGENTS.md 渲染技术路线铁律）。
            // 此前它是硬编码 false，起因是 2026-09-02 的「mesh 路径地面全黑」A/B 结论；但根因
            // 已查明是 `binding 9` 未绑定导致的乘零，而两条管线**共用同一个片元着色器**
            // （mesh 管线在 init_mesh_pipeline 里从磁盘读 assets/triangle.frag.spv），所以那
            // 从来不是 mesh 着色器的 bug，而是被误记到它头上的一个 FS 侧缺陷。binding 9 已修，
            // 止血补丁在此摘除。
            // ⚠ 顶点管线（init_pipeline / VERTEX_SHADER_WGSL）自此**冻结**：只作为缺
            //   VK_EXT_mesh_shader 时（WSLg / dzn）的兼容回退存在，不再接受功能开发，也不与
            //   mesh 路径做双份维护——新特性一律只写 mesh 路径。
            mesh_enabled: mesh_shader_available,
            device_name,
            mesh_shader: mesh_shader_loader,
            mesh_pipeline: vk::Pipeline::null(),
            mesh_pipeline_layout: vk::PipelineLayout::null(),
            mesh_max_wg_x: 1,
            void_mode: false,
            framebuffers: Vec::new(),
            // MSAA：RV3D_MSAA=1/2/4/8（默认 4x；0/1 = 关闭）
            msaa_samples: match std::env::var("RV3D_MSAA") {
                Ok(v) => match v.trim().parse::<u32>() {
                    Ok(2) => vk::SampleCountFlags::TYPE_2,
                    Ok(4) => vk::SampleCountFlags::TYPE_4,
                    Ok(8) => vk::SampleCountFlags::TYPE_8,
                    _ => vk::SampleCountFlags::TYPE_1,
                },
                Err(_) => vk::SampleCountFlags::TYPE_4,
            },
            msaa_images: Vec::new(),
            msaa_image_memory: Vec::new(),
            msaa_image_views: Vec::new(),
            command_pool: vk::CommandPool::null(),
            command_buffers: Vec::new(),
            image_available_semaphores: Vec::new(),
            render_finished_semaphores: Vec::new(),
            in_flight_fences: Vec::new(),
            acquire_timeouts: 0,
            fence_timeouts: 0,
            gpu_stalled: false,
            swapchain_broken: false,
            // 阴影拆两张（静态图 + 动态图，见 shadow_dyn_image）：静态图偶尔重画、动态图按
            // `shadow_every` 的节奏重画。默认 **2**（与拆分前整图的节奏一致），于是
            // `RV3D_NO_SHADOW_SPLIT=1` 就是逐帧等价的对照组；=1 可让 NPC 影子更实时。
            shadow_every: std::env::var("RV3D_SHADOW_EVERY")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|v| (1..=8).contains(v))
                .unwrap_or(2),
            shadow_split: std::env::var("RV3D_NO_SHADOW_SPLIT").is_err(),
            shadow_static_every: std::env::var("RV3D_SHADOW_STATIC_EVERY")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| (1..=600).contains(v))
                .unwrap_or(30),
            shadow_static_frame: true, // 首帧必须画（此时静态图里还没有任何内容）
            shadow_frame: true, // 首帧（以及启动时那批 dummy 录制）必须画
            frame_seq: 0,
            device_lost: false,
            last_recreate_attempt: Instant::now(),
            present_mode_override: None,
            current_frame: 0,
            max_frames_in_flight: 2,
            last_frame_us: 0,
            last_cull_us: 0,
            vertex_buffer: vk::Buffer::null(),
            vertex_buffer_memory: vk::DeviceMemory::null(),
            index_buffer: vk::Buffer::null(),
            index_buffer_memory: vk::DeviceMemory::null(),
            far_vertex_buffer: vk::Buffer::null(),
            far_vertex_buffer_memory: vk::DeviceMemory::null(),
            far_index_buffer: vk::Buffer::null(),
            far_index_buffer_memory: vk::DeviceMemory::null(),
            ground_vertex_buffer: vk::Buffer::null(),
            ground_vertex_buffer_memory: vk::DeviceMemory::null(),
            ground_index_buffer: vk::Buffer::null(),
            ground_index_buffer_memory: vk::DeviceMemory::null(),
            sphere_vertex_buffer: vk::Buffer::null(),
            sphere_vertex_buffer_memory: vk::DeviceMemory::null(),
            sphere_index_buffer: vk::Buffer::null(),
            sphere_index_buffer_memory: vk::DeviceMemory::null(),
            sphere_index_count: 0,
            cylinder_vertex_buffer: vk::Buffer::null(),
            cylinder_vertex_buffer_memory: vk::DeviceMemory::null(),
            cylinder_index_buffer: vk::Buffer::null(),
            cylinder_index_buffer_memory: vk::DeviceMemory::null(),
            cylinder_index_count: 0,
            terrain_lods: Vec::new(),
            instance_buffers: Vec::new(),
            instance_buffers_memory: Vec::new(),
            instance_mapped: Vec::new(),
            instances: Vec::new(),
            culled_scratch: Vec::new(),
            seg_near_counts: Vec::new(),
            seg_far_counts: Vec::new(),
            instance_radii: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_x: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_y: Vec::with_capacity(INSTANCE_COUNT as usize),
            instance_center_z: Vec::with_capacity(INSTANCE_COUNT as usize),
            markers: Vec::new(),
            last_marker_near: 0,
            last_marker_far: 0,
            npc_box_parts: Vec::new(),
        npc_cyl_parts: Vec::new(),
        npc_sph_parts: Vec::new(),
            last_npc_box_near: 0,
            last_npc_box_far: 0,
            last_npc_cyl_near: 0,
            last_npc_cyl_far: 0,
            last_npc_sph_near: 0,
            last_npc_sph_far: 0,
            emissive_markers: Vec::new(),
            last_emissive_near: 0,
            last_emissive_far: 0,
            last_perf_log: Instant::now(),
            frame_count: 0,
            perf_window_start: Instant::now(),
            stage_wait_fence_us: 0,
            stage_acquire_us: 0,
            stage_terrain_us: 0,
            stage_record_us: 0,
            stage_submit_us: 0,
            stage_present_us: 0,
            present_stall_frames: 0,
            depth_images: Vec::new(),
            depth_images_memory: Vec::new(),
            depth_image_views: Vec::new(),
            shadow_image: vk::Image::null(),
            shadow_image_memory: vk::DeviceMemory::null(),
            shadow_image_view: vk::ImageView::null(),
            shadow_sampler: vk::Sampler::null(),
            menu_blur_image: vk::Image::null(),
            menu_blur_memory: vk::DeviceMemory::null(),
            menu_blur_view: vk::ImageView::null(),
            menu_blur_sampler: vk::Sampler::null(),
            hud_glass_set_layout: vk::DescriptorSetLayout::null(),
            hud_glass_pool: vk::DescriptorPool::null(),
            hud_glass_set: vk::DescriptorSet::null(),
            hud_has_glass: false,
            menu_glass_enabled: match std::env::var("RV3D_MENU_GLASS") {
                Ok(v) => !(v == "0" || v.eq_ignore_ascii_case("false")),
                Err(_) => true,
            },
            shadow_render_pass: vk::RenderPass::null(),
            shadow_framebuffer: vk::Framebuffer::null(),
            shadow_dyn_image: vk::Image::null(),
            shadow_dyn_image_memory: vk::DeviceMemory::null(),
            shadow_dyn_image_view: vk::ImageView::null(),
            shadow_dyn_framebuffer: vk::Framebuffer::null(),
            shadow_pipeline_layout: vk::PipelineLayout::null(),
            shadow_pipeline: vk::Pipeline::null(),
            shadow_ubo_buffers: Vec::new(),
            shadow_ubo_memory: Vec::new(),
            shadow_ubo_mapped: Vec::new(),
            shadow_descriptor_set_layout: vk::DescriptorSetLayout::null(),
            shadow_descriptor_sets: Vec::new(),
            // ---- 新增字段初始值 ----
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            descriptor_sets: Vec::new(),
            uniform_buffers: Vec::new(),
            uniform_buffers_memory: Vec::new(),
            uniform_mapped: Vec::new(),
            light_uniform_buffers: Vec::new(),
            light_uniform_buffers_memory: Vec::new(),
            light_uniform_mapped: Vec::new(),
            texture_image: vk::Image::null(),
            texture_image_memory: vk::DeviceMemory::null(),
            texture_image_view: vk::ImageView::null(),
            texture_sampler: vk::Sampler::null(),
            skin_marker_image: vk::Image::null(),
            skin_marker_memory: vk::DeviceMemory::null(),
            skin_marker_image_view: vk::ImageView::null(),
            skin_npc_image: vk::Image::null(),
            skin_npc_memory: vk::DeviceMemory::null(),
            skin_npc_image_view: vk::ImageView::null(),
            ground_detail_image: vk::Image::null(),
            ground_detail_memory: vk::DeviceMemory::null(),
            ground_detail_image_view: vk::ImageView::null(),
            // 2026-08-22：默认启用（RV3D_SKIN_TEX=0 关闭纯色回退）——障碍需要表面细节
            skin_tex_enabled: std::env::var("RV3D_SKIN_TEX").as_deref() != Ok("0"),
            texture_anisotropy_enabled: physical_device_features.sampler_anisotropy != 0,
            hud_pipeline: vk::Pipeline::null(),
            hud_overlay_pipeline: vk::Pipeline::null(),
            hud_pipeline_layout: vk::PipelineLayout::null(),
            hud_vertex_buffer: vk::Buffer::null(),
            hud_vertex_buffer_memory: vk::DeviceMemory::null(),
            hud_mapped: std::ptr::null_mut(),
            hud_vertex_count: 0,
            hud_render_pass: vk::RenderPass::null(),
            hud_framebuffers: Vec::new(),
            hud_capacity_quads: 4096,
            gun_vertex_buffer: vk::Buffer::null(),
            gun_vertex_buffer_memory: vk::DeviceMemory::null(),
            gun_index_buffer: vk::Buffer::null(),
            gun_index_buffer_memory: vk::DeviceMemory::null(),
            gun_mapped: std::ptr::null_mut(),
            gun_vertex_count: 0,
            gun_index_count: 0,
            gun_buffer_capacity_verts: 0,
            gun_buffer_capacity_idx: 0,
            soldier_vertex_buffer: vk::Buffer::null(),
            soldier_vertex_buffer_memory: vk::DeviceMemory::null(),
            soldier_index_buffer: vk::Buffer::null(),
            soldier_index_buffer_memory: vk::DeviceMemory::null(),
            soldier_vertex_count: 0,
            soldier_index_count: 0,
            soldier_parts: Vec::new(),
            npc_cap_warned: false,
            pt_box_cap_warned: false,
            wait_idle_warned: false,
            soldier_drawn: 0,
            prop_vertex_buffer: vk::Buffer::null(),
            prop_vertex_memory: vk::DeviceMemory::null(),
            prop_index_buffer: vk::Buffer::null(),
            prop_index_memory: vk::DeviceMemory::null(),
            prop_mapped: std::ptr::null_mut(),
            prop_vertex_count: 0,
            prop_index_count: 0,
            prop_capacity_verts: 0,
            prop_capacity_idx: 0,
            prop_bins: Vec::new(),
            prop_sh_vertex_buffer: vk::Buffer::null(),
            prop_sh_vertex_memory: vk::DeviceMemory::null(),
            prop_sh_index_buffer: vk::Buffer::null(),
            prop_sh_index_memory: vk::DeviceMemory::null(),
            prop_sh_index_count: 0,
            prop_sh_bins: Vec::new(),
            shadow_lod: std::env::var("RV3D_SHADOW_LOD").as_deref() != Ok("0"),
            frame_frustum: [[0.0f32; 4]; 6],
            frame_cam_pos: glam::Vec3::ZERO,
            last_near_count: 0,
            last_far_count: 0,
            last_terrain_lod_name: "high",
            light_data: LightUniform::default(),
            quality: QualityPreset::DEFAULT,
            pt_live_enabled: false,
            pt_resident: None,
            pt_params: crate::engine::ray_tracer::PtParams::default(),
            pt_box_count: 0,
            pt_scene_sig: 0,
            pt_prop_key: (0, 0, 0),
            prop_attr_buf: vk::Buffer::null(),
            prop_attr_mem: vk::DeviceMemory::null(),
            prop_attr_tris: 0,
            pt_img: vk::Image::null(),
            pt_img_mem: vk::DeviceMemory::null(),
            pt_view: vk::ImageView::null(),
            pt_pipeline: vk::Pipeline::null(),
            pt_layout: vk::PipelineLayout::null(),
            pt_setl: vk::DescriptorSetLayout::null(),
            pt_pool: vk::DescriptorPool::null(),
            pt_dset: vk::DescriptorSet::null(),
            pt_module: vk::ShaderModule::null(),
            pt_acc: vk::Image::null(),
            pt_acc_mem: vk::DeviceMemory::null(),
            pt_acc_view: vk::ImageView::null(),
            pt_frame: std::cell::Cell::new(0),
            pt_spp_target: 256,
            pt_reset: std::cell::Cell::new(true),
            pt_view_sig: std::cell::Cell::new(0),
            pt_size: (64, 64),
            pt_move_base_cam: std::cell::Cell::new([0.0; 3]),
            pt_move_base_fwd: std::cell::Cell::new([0.0; 3]),

            screenshot_request: None,
            screenshot_buffers: Vec::new(),
            screenshot_buffers_memory: Vec::new(),
            screenshot_fences: Vec::new(),
        })
    }
    pub(crate) fn create_shader_module(&self, spirv: &[u32]) -> Result<vk::ShaderModule, String> {
        let create_info = vk::ShaderModuleCreateInfo::default().code(spirv);
        unsafe {
            self.device
                .create_shader_module(&create_info, None)
                .map_err(|e| format!("创建着色器模块失败: {}", e))
        }
    }
    /// 选择内存类型：prefer_device_local=true 优先 DEVICE_LOCAL（否则回退任意可用）；
    /// 否则要求 HOST_VISIBLE | HOST_COHERENT
    pub(crate) fn pick_memory_type(
        &self,
        requirements: vk::MemoryRequirements,
        prefer_device_local: bool,
    ) -> Result<u32, String> {
        let mem_properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let find = |flags: vk::MemoryPropertyFlags| {
            mem_properties
                .memory_types
                .iter()
                .enumerate()
                .find(|(i, mem_type)| {
                    let type_mask = 1 << i;
                    (requirements.memory_type_bits & type_mask) != 0
                        && mem_type.property_flags.contains(flags)
                })
                .map(|(i, _)| i as u32)
        };
        if prefer_device_local {
            find(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| find(vk::MemoryPropertyFlags::empty()))
                .ok_or_else(|| "没有找到合适的内存类型（Device Local）".to_string())
        } else {
            find(
                vk::MemoryPropertyFlags::HOST_VISIBLE
                    | vk::MemoryPropertyFlags::HOST_COHERENT,
            )
            .ok_or_else(|| "没有找到合适的内存类型（Host Buffer）".to_string())
        }
    }
    /// 创建 buffer 并分配 HOST_VISIBLE | HOST_COHERENT 内存
    pub(crate) fn create_host_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        size: u64,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let buffer_create_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe {
            self.device
                .create_buffer(&buffer_create_info, None)
                .map_err(|e| format!("创建缓冲失败: {}", e))?
        };
        let mem_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = self.pick_memory_type(mem_requirements, false)?;
        let mut alloc_flags = vk::MemoryAllocateFlagsInfo::default();
        alloc_flags.flags = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type)
            .push_next(&mut alloc_flags);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配缓冲内存失败: {}", e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| format!("绑定缓冲内存失败: {}", e))?;
        }
        Ok((buffer, memory))
    }
    /// 创建 DEVICE_LOCAL 静态缓冲（一次性：staging 上传后即释放）。
    /// 用于地形等一次性数据，避免 GPU 每帧从 host 内存读顶点/索引。
    pub(crate) fn create_device_local_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        data: &[u8],
        label: &str,
    ) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
        let size = data.len() as u64;

        // 1. staging buffer（HOST_VISIBLE | HOST_COHERENT，TRANSFER_SRC）
        let staging_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe {
            self.device
                .create_buffer(&staging_info, None)
                .map_err(|e| format!("创建 {} staging buffer 失败: {}", label, e))?
        };
        let staging_reqs = unsafe { self.device.get_buffer_memory_requirements(staging_buffer) };
        let staging_type = self.pick_memory_type(staging_reqs, false)?;
        let mut st_flags = vk::MemoryAllocateFlagsInfo::default();
        st_flags.flags = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let staging_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(staging_reqs.size)
            .memory_type_index(staging_type)
            .push_next(&mut st_flags);
        let staging_memory = unsafe {
            self.device
                .allocate_memory(&staging_alloc, None)
                .map_err(|e| format!("分配 {} staging 内存失败: {}", label, e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(staging_buffer, staging_memory, 0)
                .map_err(|e| format!("绑定 {} staging buffer 失败: {}", label, e))?;
            let ptr = self
                .device
                .map_memory(staging_memory, 0, size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射 {} staging 内存失败: {}", label, e))?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            self.device.unmap_memory(staging_memory);
        }

        // 2. 目标 buffer（DEVICE_LOCAL 优先，usage | TRANSFER_DST）
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage | vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe {
            self.device
                .create_buffer(&buffer_info, None)
                .map_err(|e| format!("创建 {} buffer 失败: {}", label, e))?
        };
        let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let memory_type = self.pick_memory_type(mem_reqs, true)?;
        let mut fin_flags = vk::MemoryAllocateFlagsInfo::default();
        fin_flags.flags = if (usage | vk::BufferUsageFlags::TRANSFER_DST).contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
            vk::MemoryAllocateFlags::DEVICE_ADDRESS
        } else {
            vk::MemoryAllocateFlags::empty()
        };
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_reqs.size)
            .memory_type_index(memory_type)
            .push_next(&mut fin_flags);
        let memory = unsafe {
            self.device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| format!("分配 {} 内存失败: {}", label, e))?
        };
        unsafe {
            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| format!("绑定 {} buffer 失败: {}", label, e))?;
        }

        // 3. staging → 目标 一次性拷贝
        self.run_single_time_commands(|cmd| {
            let region = vk::BufferCopy::default().size(size);
            unsafe {
                self.device.cmd_copy_buffer(cmd, staging_buffer, buffer, &[region]);
            }
        })?;

        // 4. 释放 staging
        unsafe {
            self.device.free_memory(staging_memory, None);
            self.device.destroy_buffer(staging_buffer, None);
        }
        Ok((buffer, memory))
    }
    /// 等 GPU 空闲 + **失败留痕**（`wait_idle_failure_message`）。
    ///
    /// 全仓 6 处"等空闲再销毁/重建在飞资源"统一走这里：原先每处都是
    /// `let _ = self.device.device_wait_idle();` —— 等待失败与等待成功在日志里无法区分。
    /// 判据 = `device_wait_idle_errors_are_never_silently_dropped`（源码扫描，改回 `let _ =` 即红）。
    pub(crate) unsafe fn wait_idle_checked(&mut self) {
        let err = match self.device.device_wait_idle() {
            Ok(()) => return,
            Err(e) => e,
        };
        if let Some(msg) = wait_idle_failure_message(err, self.wait_idle_warned) {
            self.wait_idle_warned = true;
            log::warn!("{msg}");
        }
    }
    /// 提交一次性命令（用于纹理布局转换、数据拷贝等）
    pub(crate) fn run_single_time_commands(&self, f: impl FnOnce(vk::CommandBuffer)) -> Result<(), String> {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd_buffer = unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| format!("分配一次性命令缓冲失败: {}", e))?
        }[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device
                .begin_command_buffer(cmd_buffer, &begin_info)
                .map_err(|e| format!("开始一次性命令缓冲失败: {}", e))?;
        }

        f(cmd_buffer);

        unsafe {
            self.device
                .end_command_buffer(cmd_buffer)
                .map_err(|e| format!("结束一次性命令缓冲失败: {}", e))?;
        }

        let cmd_buffers = [cmd_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], vk::Fence::null())
                .map_err(|e| format!("提交一次性命令失败: {}", e))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|e| format!("等待一次性命令失败: {}", e))?;
            self.device.free_command_buffers(self.command_pool, &[cmd_buffer]);
        }
        Ok(())
    }
    pub(crate) fn wait_idle(&self) -> Result<(), String> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| format!("等待设备空闲失败: {}", e))
        }
    }
}
