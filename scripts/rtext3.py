# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("                device_extensions.push(ext.as_ptr());", "                device_extensions.push(ext.as_ptr() as *const std::ffi::c_char);")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('cast fixed')
