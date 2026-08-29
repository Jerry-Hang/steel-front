# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
old = """        let mut count = 0u32;
        unsafe {
            ext.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &geom,
                &[32],
                &mut count,
            );
        }"""
new = """        let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            ext.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &geom,
                &[32],
                &mut size_info,
            );
        }
        let count = size_info.acceleration_structure_size;"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('size_info fixed')
else:
    print('anchor missing')
