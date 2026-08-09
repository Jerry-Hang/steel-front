//! GPU 硬件能力探测：在 WSLg/dzn（Vulkan-on-D3D12 转译）视角下判定
//! 光追（RT Core）、Tensor Core/协作矩阵、DLSS 私有扩展的可用性，
//! 供"是否迁移环境 / 如何打通光追与 DLSS 路径"决策。
//!
//! 判定逻辑：
//! - 枚举 device 扩展，按 光追 / 协作矩阵 / DLSS 私有 / 其它 分类；
//! - `vkGetPhysicalDeviceFeatures2`/`Properties2` 查 RT 特性与管线属性；
//! - 决定性测试：尝试创建启用光追扩展与特性的探测逻辑设备；
//! - CUDA 直通检查（/usr/lib/wsl/lib），判断 Tensor Core 可编程访问路径。

use ash::vk;
use std::ffi::CStr;

/// 扩展名是否在列表中存在（前缀/包含匹配，便于归类）
fn has_ext(exts: &[String], pat: &str) -> bool {
    exts.iter().any(|n| n.contains(pat))
}

/// 打印一组扩展的存在状态（每个一行）
fn log_ext_group(exts: &[String], patterns: &[&str]) {
    for p in patterns {
        log::info!("gpu-caps:   {} = {}", p, has_ext(exts, p));
    }
}

/// 把三个 RT 特性结构体串成 pNext 链并返回链头指针（可变引用参数避免
/// 局部变量"赋值后未读"误报；链头由调用方挂到 DeviceCreateInfo::p_next）
fn rt_features_chain(
    rt: &mut vk::PhysicalDeviceRayTracingPipelineFeaturesKHR,
    as_: &mut vk::PhysicalDeviceAccelerationStructureFeaturesKHR,
    bda: &mut vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR,
) -> *const std::ffi::c_void {
    as_.p_next = bda as *mut _ as *mut std::ffi::c_void;
    rt.p_next = as_ as *mut _ as *mut std::ffi::c_void;
    rt as *const _ as *const std::ffi::c_void
}

/// 探测主入口（renderer 初始化时调用一次）
pub fn log_gpu_hardware_caps(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device_name: &str,
) {
    log::info!("gpu-caps: 设备 {} 硬件能力探测（WSLg/dzn 视角）", device_name);

    // 1. device 扩展枚举
    let ext_names: Vec<String> = unsafe {
        instance
            .enumerate_device_extension_properties(physical_device)
            .unwrap_or_default()
            .iter()
            .map(|e| CStr::from_ptr(e.extension_name.as_ptr()).to_string_lossy().into_owned())
            .collect()
    };
    log::info!("gpu-caps: 可用 device 扩展 {} 个", ext_names.len());

    // 2. 分类打印
    log::info!("gpu-caps: -- 光追扩展 --");
    log_ext_group(
        &ext_names,
        &[
            "VK_KHR_ray_tracing_pipeline",
            "VK_KHR_acceleration_structure",
            "VK_KHR_ray_query",
            "VK_KHR_deferred_host_operations",
            "VK_KHR_buffer_device_address",
            "VK_KHR_ray_tracing_position_fetch",
            "VK_NV_ray_tracing",
        ],
    );
    log::info!("gpu-caps: -- Tensor Core / 协作矩阵 --");
    log_ext_group(
        &ext_names,
        &[
            "VK_KHR_cooperative_matrix",
            "VK_NV_cooperative_matrix",
            "VK_NV_compute_shader_derivatives",
        ],
    );
    log::info!("gpu-caps: -- DLSS / NVIDIA 私有 --");
    log_ext_group(
        &ext_names,
        &[
            "VK_NVX_image_view_handle",
            "VK_NVX_binary_import",
            "VK_NV_cuda_kernel",
            "VK_NV_external_memory",
        ],
    );
    log::info!("gpu-caps: -- 其它新特性 --");
    log_ext_group(
        &ext_names,
        &["VK_EXT_mesh_shader", "VK_KHR_shader_object", "VK_KHR_dynamic_rendering"],
    );

    // 3. RT 特性与管线属性（dzn 未启用扩展时特性可能报 false，以探测 device 为准）
    let mut rq_f = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
    let mut bda_f = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
    let mut as_f = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
    let mut rt_f = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default();
    bda_f.p_next = &mut rq_f as *mut _ as *mut std::ffi::c_void;
    as_f.p_next = &mut bda_f as *mut _ as *mut std::ffi::c_void;
    rt_f.p_next = &mut as_f as *mut _ as *mut std::ffi::c_void;
    let mut f2 = vk::PhysicalDeviceFeatures2::default();
    f2.p_next = &mut rt_f as *mut _ as *mut std::ffi::c_void;
    unsafe {
        instance.get_physical_device_features2(physical_device, &mut f2);
    }
    let mut rt_props = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
    let mut p2 = vk::PhysicalDeviceProperties2::default();
    p2.p_next = &mut rt_props as *mut _ as *mut std::ffi::c_void;
    unsafe {
        instance.get_physical_device_properties2(physical_device, &mut p2);
    }
    log::info!(
        "gpu-caps: RT 特性 ray_tracing_pipeline={} acceleration_structure={} buffer_device_address={} ray_query={}",
        rt_f.ray_tracing_pipeline > 0,
        as_f.acceleration_structure > 0,
        bda_f.buffer_device_address > 0,
        rq_f.ray_query > 0
    );
    log::info!(
        "gpu-caps: RT 管线属性 shaderGroupHandleSize={}B maxRecursionDepth={} maxRayDispatchInvocationCount={}",
        rt_props.shader_group_handle_size,
        rt_props.max_ray_recursion_depth,
        rt_props.max_ray_dispatch_invocation_count
    );

    // 4. 决定性测试：创建启用 RT 扩展与特性的探测 device
    let rt_ok = has_ext(&ext_names, "VK_KHR_ray_tracing_pipeline")
        && has_ext(&ext_names, "VK_KHR_acceleration_structure")
        && has_ext(&ext_names, "VK_KHR_deferred_host_operations")
        && has_ext(&ext_names, "VK_KHR_buffer_device_address");
    if rt_ok {
        let qfps = unsafe {
            instance.get_physical_device_queue_family_properties(physical_device)
        };
        let gfx_idx = qfps
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .map(|i| i as u32);
        if let Some(gfx_idx) = gfx_idx {
            let priorities = [1.0f32];
            let qi = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(gfx_idx)
                .queue_priorities(&priorities);
            let names = [
                c"VK_KHR_ray_tracing_pipeline".as_ptr(),
                c"VK_KHR_acceleration_structure".as_ptr(),
                c"VK_KHR_deferred_host_operations".as_ptr(),
                c"VK_KHR_buffer_device_address".as_ptr(),
            ];
            let mut rt_f = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default()
                .ray_tracing_pipeline(true);
            let mut as_f = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
                .acceleration_structure(true);
            let mut bda_f = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default()
                .buffer_device_address(true);
            let mut dci = vk::DeviceCreateInfo::default()
                .queue_create_infos(std::slice::from_ref(&qi))
                .enabled_extension_names(&names);
            dci.p_next = rt_features_chain(&mut rt_f, &mut as_f, &mut bda_f);
            match unsafe { instance.create_device(physical_device, &dci, None) } {
                Ok(probe) => {
                    log::info!(
                        "gpu-caps: 【结论】光追探测 device 创建成功 → RT Core 路径可用（dzn→DXR）"
                    );
                    unsafe {
                        probe.destroy_device(None);
                    }
                }
                Err(e) => {
                    log::info!("gpu-caps: 【结论】光追探测 device 创建失败（{}）", e);
                }
            }
        } else {
            log::info!("gpu-caps: 【结论】无图形队列族，跳过探测 device 测试");
        }
    } else {
        log::info!("gpu-caps: 【结论】RT 扩展不全，光追管线不可用");
    }

    // 5. CUDA 直通（Tensor Core 可编程访问路径，独立于 Vulkan）
    let cuda_libs = [
        "/usr/lib/wsl/lib/libcuda.so.1",
        "/usr/lib/wsl/lib/libcuda.so",
        "/usr/lib/wsl/lib/libnvcuda.so",
    ];
    let cuda_ok = cuda_libs.iter().any(|p| std::path::Path::new(p).exists());
    log::info!(
        "gpu-caps: CUDA 直通（Tensor Core 可编程访问）={}（libcuda in /usr/lib/wsl/lib）",
        cuda_ok
    );
    let dlss_vulkan_ok = has_ext(&ext_names, "VK_NVX_image_view_handle")
        || has_ext(&ext_names, "VK_NV_cuda_kernel");
    log::info!(
        "gpu-caps: 【结论】Vulkan-DLSS（NVIDIA 私有扩展）={}；{}",
        dlss_vulkan_ok,
        if dlss_vulkan_ok {
            "可直接接 DLSS SDK"
        } else if cuda_ok {
            "不可用；但 CUDA 直通可用 Tensor Core，未来可自研超分/降噪"
        } else {
            "不可用且无 CUDA 直通"
        }
    );
}
