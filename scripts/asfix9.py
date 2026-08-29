# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("instance_custom_index_and_mask: vk::Packed24_8::new(0, 0xFF),", "instance_custom_index_and_mask: vk::Packed24_8::new(0, 0xFF),")
s = s.replace("acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_address: blas_addr },", "acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: blas_addr },")
# Packed24_8::new 参数可能 (u32, u32)——改显式
s = s.replace("vk::Packed24_8::new(0, 0xFF)", "vk::Packed24_8::new(0u32, 0xFFu32)")
s = s.replace("vk::Packed24_8::new(0, 0)", "vk::Packed24_8::new(0u32, 0u32)")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('fixed ref+packed')
