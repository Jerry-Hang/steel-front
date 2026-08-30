# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = """        let ext_names = [
            b"VK_KHR_buffer_device_address\\0".as_slice(),
            b"VK_KHR_acceleration_structure\\0".as_slice(),
            b"VK_KHR_deferred_host_operations\\0".as_slice(),
            b"VK_KHR_ray_query\\0".as_slice(),
            b"VK_EXT_shader_float16\\0".as_slice(),
        ];
        let ext_ptrs: Vec<*const std::ffi::c_char> = ext_names.iter().map(|e| e.as_ptr() as *const std::ffi::c_char).collect();"""
new = """        // 只启用设备实际支持的扩展（ray 查询必需；float16 可选）
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
        let ext_ptrs: Vec<*const std::ffi::c_char> = ext_names.iter().map(|e| e.as_ptr() as *const std::ffi::c_char).collect();"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('ext filter done')
else:
    print('miss')
