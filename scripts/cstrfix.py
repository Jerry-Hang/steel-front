# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""            let khr_rt: [&'static str; 2] = [
                "VK_KHR_acceleration_structure",
                "VK_KHR_ray_query",
            ];
            for ext in khr_rt.iter() {
                device_extensions.push(ext.as_ptr() as *const std::ffi::c_char);
            }""", """            device_extensions.push(c"VK_KHR_acceleration_structure".as_ptr());
            device_extensions.push(c"VK_KHR_ray_query".as_ptr());""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('C-string literals used')
