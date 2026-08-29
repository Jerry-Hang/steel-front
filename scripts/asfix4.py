# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("std::ptr::copy_nonoverlapping(idx.as_ptr(), ip as *mut u8, n_idx * 4);", "std::ptr::copy_nonoverlapping(idx.as_ptr(), ip as *mut u32, n_idx);")
s = s.replace("geom.p_geometries = &[geo];", "geom.p_geometries = [&geo].as_ptr();")
# ext 方法名：get_acceleration_structure_build_sizes + device_ad...
s = s.replace("ext.get_acceleration_structure_build_sizes_khr(", "ext.get_acceleration_structure_build_sizes(")
s = s.replace("ext.get_acceleration_structure_device_address(", "ext.get_acceleration_structure_device_address(")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('fixed 3')
