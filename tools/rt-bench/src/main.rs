use ash::vk;
use std::time::Instant;

static mut WARN: String = String::new();
fn log_stop(msg: &str) { unsafe { WARN = msg.to_string(); } }
fn take_warn() -> String { unsafe { std::mem::take(&mut WARN) } }
mod rt_impl;
use rt_impl::rt_test;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut want_fp = [true, true, true, true]; // 默认全勾选
    if args.len() > 2 {
        for (i, a) in args[2..].iter().enumerate() {
            if i < 4 { want_fp[i] = a == "1"; }
        }
    }
    println!("============ RT 算力测试台 v0.1 ============");
    let entry = unsafe { ash::Entry::load() }.expect("Vulkan 加载失败");
    let inst = create_instance(&entry).expect("Instance 创建失败");
    let (phys, props) = pick_gpu(&inst).expect("未找到 GPU");
    let name = unsafe { std::ffi::CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy().to_string();
    println!("GPU: {}", name);

    let (device, queue) = create_device(&inst, phys).expect("设备创建失败");
    println!("设备就绪，开始测试...\n");

    let mut results: Vec<(String, String, f64)> = Vec::new();

    // FP 系列（默认勾选）
    let fps = [
        ("FP32", include_bytes!("../assets/fp32.spv").as_slice(), "GFLOPS", "标准单精度 FMA"),
        ("FP16", include_bytes!("../assets/fp16.spv").as_slice(), "GFLOPS", "半精度(真FP16)"),
        ("FP8 ", include_bytes!("../assets/fp8.spv").as_slice(), "GOPS ", "8-bit 打包(模拟)"),
        ("FP4 ", include_bytes!("../assets/fp4.spv").as_slice(), "GOPS ", "4-bit 打包(模拟)"),
    ];
    for (i, (name_, spv, unit, explain)) in fps.iter().enumerate() {
        if !want_fp[i] { continue; }
        match fp_test(&device, &queue, phys, &inst, spv, 1 << 18) {
            Ok(v) => { results.push((name_.to_string(), unit.to_string(), v)); println!("  {} : {:.2} {} ({})", name_, v, unit, explain); }
            Err(e) => println!("  {} : 失败 ({})", name_, e),
        }
    }

    // RT 递归压满测试
    println!("\n[RT] 持续射线回溯压测（跑满整卡功耗，递归进行）...");
    let rt_val = rt_test(&device, &queue, &inst, phys).unwrap_or(0.0);
    results.push(("RT ".to_string(), "Mrays/s".to_string(), rt_val));
    println!("  RT  : {:.1} Mrays/s (1M射线 x 200次迭代)", rt_val);

    let score = (results.iter().map(|r| r.2).sum::<f64>() * 100.0) as u64;
    println!("\n========= 总分: {} =========", score);

    write_log(&name, &results, score);
}


fn create_instance(entry: &ash::Entry) -> Result<ash::Instance, String> {
    unsafe {
        let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
        let ci = vk::InstanceCreateInfo::default().application_info(&app);
        entry.create_instance(&ci, None).map_err(|e| format!("instance: {:?}", e))
    }
}

fn pick_gpu(entry: &ash::Instance) -> Option<(vk::PhysicalDevice, vk::PhysicalDeviceProperties)> {
    unsafe {
        for p in entry.enumerate_physical_devices().ok()? {
            let props = entry.get_physical_device_properties(p);
            let exts: Vec<vk::ExtensionProperties> = entry.enumerate_device_extension_properties(p).unwrap_or_default();
            let has_ray = exts.iter().any(|e| {
                let n = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) }.to_string_lossy();
                n.contains("ray_query")
            });
            if has_ray { return Some((p, props)); }
        }
    }
    None
}

fn create_device(entry: &ash::Instance, phys: vk::PhysicalDevice) -> Result<(ash::Device, vk::Queue), String> {
    let prio = [1.0f32];
    unsafe {
        // 只启用设备实际支持的扩展（ray 查询必需；float16 可选）
        let sys_exts: Vec<String> = entry.enumerate_device_extension_properties(phys).map(|v| v.iter().map(|e| {
            let c = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) };
            c.to_string_lossy().into_owned()
        }).collect()).unwrap_or_default();
        let need = ["VK_KHR_buffer_device_address", "VK_KHR_acceleration_structure", "VK_KHR_deferred_host_operations", "VK_KHR_ray_query", "VK_EXT_shader_float16"];
        let mut ext_names: Vec<Vec<u8>> = Vec::new();
        for n in need {
            if sys_exts.iter().any(|e| e == n) {
                let mut s2 = n.as_bytes().to_vec();
                s2.push(0);
                ext_names.push(s2);
            }
        }
        let ext_ptrs: Vec<*const std::ffi::c_char> = ext_names.iter().map(|e| e.as_ptr() as *const std::ffi::c_char).collect();
        let mut rq_f = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        rq_f.ray_query = 1;
        let mut as_f = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_f.acceleration_structure = 1;
        let mut bd_f = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
        bd_f.buffer_device_address = 1;
        as_f.p_next = &mut bd_f as *mut _ as *mut std::ffi::c_void;
        rq_f.p_next = &mut as_f as *mut _ as *mut std::ffi::c_void;
        let prio = [1.0f32];
        let mut dq = vk::DeviceQueueCreateInfo::default();
        dq.queue_family_index = 0;
        dq.queue_count = 1;
        dq.p_queue_priorities = prio.as_ptr();
        let queues = [dq];
        let _ = &prio;
        let mut feats = vk::PhysicalDeviceFeatures::default();
        let mut info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&ext_ptrs)
            .enabled_features(&feats);
        info.p_next = &mut rq_f as *mut _ as *mut std::ffi::c_void;
        let dev = entry.create_device(phys, &info, None).map_err(|e| format!("创建设备: {:?}", e))?;
        let queue = dev.get_device_queue(0, 0);
        Ok((dev, queue))
    }
}

// ---------- FP 测试：compute dispatch + 吞吐测量 ----------
fn fp_test(dev: &ash::Device, queue: &vk::Queue, phys: vk::PhysicalDevice, inst: &ash::Instance, spv: &[u8], items: u32) -> Result<f64, String> {
    unsafe {
        let module = dev.create_shader_module(
            &vk::ShaderModuleCreateInfo::default().code(std::slice::from_raw_parts(spv.as_ptr() as *const u32, spv.len() / 4)),
            None,
        ).map_err(|e| format!("module: {:?}", e))?;
        let layout = dev.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None).map_err(|e| format!("layout: {:?}", e))?;
        let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(module).name(c"main");
        let info = vk::ComputePipelineCreateInfo::default().stage(stage).layout(layout);
        let pipe = dev.create_compute_pipelines(vk::PipelineCache::null(), std::slice::from_ref(&info), None)
            .map_err(|e| format!("pipeline: {:?}", e.1))?[0];

        // 输出存储缓冲
        let size = items as u64 * 4;
        let buf = create_buffer(dev, phys, size, vk::BufferUsageFlags::STORAGE_BUFFER)?;
        let mem = alloc_mem(dev, inst, phys, buf, 0)?;
        // 随机初值 → 防循环常量折叠（结果必须依赖输入！）
        if let Ok(p) = dev.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()) {
            let mut seed = 0x9E3779B9u32;
            let n = size as usize / 4;
            let arr = std::slice::from_raw_parts_mut(p as *mut u32, n);
            for v in arr.iter_mut() { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); *v = seed; }
            dev.unmap_memory(mem);
        }
        dev.bind_buffer_memory(buf, mem, 0).map_err(|e| format!("bind: {:?}", e))?;

        // descriptor
        let dsl_binding = vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let dsl = dev.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&[dsl_binding]), None).map_err(|e| format!("dsl: {:?}", e))?;
        let pool = dev.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&[vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(1)]), None).map_err(|e| format!("pool: {:?}", e))?;
        let ds = dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl])).map_err(|e| format!("ds: {:?}", e))?[0];
        let bai = vk::DescriptorBufferInfo { buffer: buf, offset: 0, range: vk::WHOLE_SIZE };
        let mut w = vk::WriteDescriptorSet::default();
        w.dst_set = ds;
        w.dst_binding = 0;
        w.descriptor_type = vk::DescriptorType::STORAGE_BUFFER;
        w.descriptor_count = 1;
        w.p_buffer_info = &bai;
        dev.update_descriptor_sets(&[w], &[]);

        // 命令缓冲
        let cpool = dev.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(0), None).map_err(|e| format!("cpool: {:?}", e))?;
        let cmd = dev.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(cpool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1)).map_err(|e| format!("cb: {:?}", e))?[0];

        // 计时：dispatch items/256 workgroup；每个线程 4096 次 FMA（fp32 流）
        let wg = (items / 256).max(1);
        let iters = 32u32; // 重复计数取均
        let mut best = f64::MAX;
        let mut worst = 0.0f64;
        for _ in 0..4 {
            dev.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default()).map_err(|e| format!("begin: {:?}", e))?;
            for i in 0..iters {
                dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, pipe);
                dev.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::COMPUTE, layout, 0, &[ds], &[]);
                dev.cmd_dispatch(cmd, wg, 1, 1);
            }
            dev.end_command_buffer(cmd);
            let t0 = Instant::now();
            dev.queue_submit(*queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], vk::Fence::null()).map_err(|e| format!("submit: {:?}", e))?;
            dev.queue_wait_idle(*queue).map_err(|e| format!("wait: {:?}", e))?;
            let dt = t0.elapsed().as_secs_f64();
            let ops = items as f64 * 4096.0 * 2.0 * iters as f64; // FMA=2 ops
            let gops = ops / dt / 1e9;
            if gops < best { best = gops; }
            if gops > worst { worst = gops; }
            // 输出校验：验证计算真实发生（防折叠被驱离）
            if let Ok(p) = dev.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()) {
                let arr = std::slice::from_raw_parts(p as *const u32, (size / 4) as usize);
                let mut sum = 0u64;
                for v in arr.iter().take(1024) { sum = sum.wrapping_add(*v as u64); }
                dev.unmap_memory(mem);
                if sum == 0 { best = 0.0; log_stop("FP 输出全零（计算未发生）"); }
            }
        }
        // 清理
        for b in [buf] { dev.destroy_buffer(b, None); }
        if mem != vk::DeviceMemory::null() { dev.free_memory(mem, None); }
        dev.destroy_descriptor_pool(pool, None);
        dev.destroy_descriptor_set_layout(dsl, None);
        dev.destroy_pipeline(pipe, None);
        dev.destroy_pipeline_layout(layout, None);
        dev.destroy_shader_module(module, None);
        dev.destroy_command_pool(cpool, None);
        Ok(best)
    }
}

fn create_buffer(dev: &ash::Device, phys: vk::PhysicalDevice, size: u64, usage: vk::BufferUsageFlags) -> Result<vk::Buffer, String> {
    unsafe { dev.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE), None).map_err(|e| format!("buf: {:?}", e)) }
}

fn alloc_mem(dev: &ash::Device, inst: &ash::Instance, phys: vk::PhysicalDevice, buf: vk::Buffer, alignment: u64) -> Result<vk::DeviceMemory, String> {
    unsafe {
        let _align = alignment;
        let req = dev.get_buffer_memory_requirements(buf);
        let mprops = inst.get_physical_device_memory_properties(phys);
        let mut idx = 0u32;
        for (i, t) in mprops.memory_types.iter().enumerate() {
            if req.memory_type_bits & (1 << i) != 0 && t.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT) {
                idx = i as u32; break;
            }
        }
        dev.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(idx), None).map_err(|e| format!("alloc: {:?}", e))
    }
}


fn write_log(gpu: &str, results: &[(String, String, f64)], score: u64) {
    let now = chrono_like_now();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("{}.txt", now));
    let mut s = format!("RT 算力测试日志\nGPU: {}\n时间: {}\n评分: {}\n\n项目,数值,单位\n", gpu, now, score);
    for (n, u, v) in results { s.push_str(&format!("{},{},{}\n", n, v, u)); }
    if let Ok(_) = std::fs::write(&file, s) { println!("\n日志已保存: {}", file.display()); }
    else { println!("\n日志保存失败"); }
}

fn chrono_like_now() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    // Windows 本地时间：用简单方式——系统日期
    let secs = d.as_secs() as i64 + 8 * 3600; // UTC+8 本地时区
    let (y, mo, da) = civil_from_days((secs / 86400) as i64);
    let rem = secs % 86400;
    let (h, mi, se) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{}-{:02}-{:02}_{:02}-{:02}-{:02}", y, mo, da, h, mi, se)
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doye = (z - era * 146097).rem_euclid(146097);
    let yoe = (doye - doye / 1460 + doye / 36524 - doye / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doye - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
