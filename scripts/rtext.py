# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
old = "            device_extensions.push(mesh_shader_ext_name.as_ptr());"
new = """            device_extensions.push(mesh_shader_ext_name.as_ptr());
            // 2026-08-29 路径追踪基准：启用光线追踪核心扩展（ray_query 计算侧；AS 构建）
            let khr_rt: [&str; 4] = [
                "VK_KHR_acceleration_structure",
                "VK_KHR_ray_query",
                "VK_KHR_deferred_host_operations",
                "VK_KHR_ray_tracing_pipeline",
            ];
            for ext in khr_rt.iter() {
                let ptr = std::ffi::CString::new(*ext).unwrap();
                let name: &'static str = Box::leak(ptr.into_boxed_str());
                device_extensions.push(name.as_ptr());
            }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('RT extensions enabled')
else:
    print('anchor missing')
