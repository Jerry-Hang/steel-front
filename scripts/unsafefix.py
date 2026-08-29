# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""            let mut names = Vec::new();
            let exts = unsafe { instance.enumerate_device_extension_properties(physical_device).unwrap_or_default() };
            for e in &exts {
                let n = std::ffi::CStr::from_ptr(e.extension_name.as_ptr()).to_string_lossy().into_owned();
                names.push(n);
            }""", """            let mut names = Vec::new();
            let exts = unsafe { instance.enumerate_device_extension_properties(physical_device).unwrap_or_default() };
            for e in &exts {
                let n = unsafe { std::ffi::CStr::from_ptr(e.extension_name.as_ptr()) }.to_string_lossy().into_owned();
                names.push(n);
            }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('unsafe fixed')
