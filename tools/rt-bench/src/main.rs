use ash::vk;

mod rt_impl;
use rt_impl::rt_test;

fn main() {
    println!("============ RT 鍏夌嚎杩借釜绠楀姏娴嬭瘯鍙?v0.2 ============");
    let args: Vec<String> = std::env::args().collect();
    let rays: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1 << 22);
    let iters: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let rounds: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);

    let entry = unsafe { ash::Entry::load() }.expect("Vulkan 鍔犺浇澶辫触");
    let inst = create_instance(&entry).expect("Instance 鍒涘缓澶辫触");
    let (phys, props) = pick_gpu(&inst).expect("鏈壘鍒?RT 鏄惧崱");
    let name = unsafe { std::ffi::CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy().to_string();
    println!("GPU: {}", name);
    let (device, queue) = create_device(&inst, phys).expect("璁惧鍒涘缓澶辫触");
    println!("妯℃嫙: {} 灏勭嚎 x {} 杩唬 x {} 杞?(姣忚疆閲嶅缓AS+娓呴浂hits+璇诲洖楠岃瘉)", rays, iters, rounds);

    if args.iter().any(|a| a == "--vram") {
        println!("== RT Memory Capacity Probe ==");
        match rt_impl::vram_probe(&device, &inst, phys) {
            Ok(()) => println!("probe done"),
            Err(e) => println!("probe fail: {e}"),
        }
        return;
    }
    match rt_test(&device, &queue, &inst, phys, rays, iters, rounds) {
        Ok(r) => {
            println!("");
            println!("========= 缁撴灉 =========");
            println!("宄板€? {:.1} Mrays/s ({:.2} G rays/s)", r.best_mrays, r.best_mrays / 1000.0);
            println!("涓綅: {:.1} Mrays/s", r.median_mrays);
            println!("鎬诲懡涓? {} (灏勭嚎閬嶅巻鐪熷疄鍙戠敓)", r.total_hits);
            println!("璇勫垎: {}", (r.best_mrays * 100.0) as u64);
            write_log(&name, rays, iters, rounds, &r);
        }
        Err(e) => println!("RT 娴嬭瘯澶辫触: {e}"),
    }
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
            for e in entry.enumerate_device_extension_properties(p).unwrap_or_default() {
                let n = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) }.to_string_lossy();
                if n.contains("ray_query") { return Some((p, props)); }
            }
        }
    }
    None
}
fn create_device(entry: &ash::Instance, phys: vk::PhysicalDevice) -> Result<(ash::Device, vk::Queue), String> {
    unsafe {
        let sys_exts: Vec<String> = entry.enumerate_device_extension_properties(phys).map(|v| v.iter().map(|e| {
            let c = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) };
            c.to_string_lossy().into_owned()
        }).collect()).unwrap_or_default();
        let need = ["VK_KHR_buffer_device_address", "VK_KHR_acceleration_structure", "VK_KHR_deferred_host_operations", "VK_KHR_ray_query"];
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
        let dev = entry.create_device(phys, &info, None).map_err(|e| format!("鍒涘缓璁惧: {:?}", e))?;
        let queue = dev.get_device_queue(0, 0);
        Ok((dev, queue))
    }
}
fn write_log(gpu: &str, rays: u32, iters: u32, rounds: u32, r: &rt_impl::RtResult) {
    let now = local_now();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("{}.txt", now));
    let mut s = format!("RT 鍏夌嚎杩借釜绠楀姏娴嬭瘯鏃ュ織\nGPU: {}\n鏃堕棿: {}\n\n閰嶇疆: {} 灏勭嚎 x {} 杩唬 x {} 杞甛n宄板€? {:.1} Mrays/s\n涓綅: {:.1} Mrays/s\n鎬诲懡涓? {}\n璇勫垎: {}\n", gpu, now, rays, iters, rounds, r.best_mrays, r.median_mrays, r.total_hits, (r.best_mrays * 100.0) as u64);
    for (i, v) in r.rounds_mrays.iter().enumerate() {
        s.push_str(&format!("Round{}: {:.1} Mrays/s\n", i + 1, v));
    }
    let _ = std::fs::write(&file, s);
    println!("鏃ュ織宸蹭繚瀛? {}", file.display());
}

fn local_now() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    let secs = d.as_secs() as i64 + 8 * 3600;
    let (y, mo, da) = civil_from_days(secs / 86400);
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
