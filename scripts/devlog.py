# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = """        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|e| format!("创建逻辑设备失败: {}", e))?
        };"""
new = """        {
            let mut names = Vec::new();
            let exts = unsafe { instance.enumerate_device_extension_properties(physical_device).unwrap_or_default() };
            for e in &exts {
                let n = std::ffi::CStr::from_ptr(e.extension_name.as_ptr()).to_string_lossy().into_owned();
                names.push(n);
            }
            let want: Vec<String> = unsafe {
                use std::ffi::CStr;
                device_extensions.iter().map(|p| CStr::from_ptr(*p).to_string_lossy().into_owned()).collect()
            };
            log::warn!("device-create: 请求={:?} 缺失={:?}", want, want.iter().filter(|w| !names.contains(*w)).collect::<Vec<_>>());
        }
""" + anchor
if anchor in s:
    s = s.replace(anchor, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('debug log added')
else:
    print('anchor missing')
