# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
old = """            let khr_rt: [&str; 4] = [
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
new = """            let khr_rt: [&'static str; 4] = [
                "VK_KHR_acceleration_structure",
                "VK_KHR_ray_query",
                "VK_KHR_deferred_host_operations",
                "VK_KHR_ray_tracing_pipeline",
            ];
            for ext in khr_rt.iter() {
                device_extensions.push(ext.as_ptr());
            }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('fixed')
