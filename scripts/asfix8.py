# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
old = """        let mut instance = vk::AccelerationStructureInstanceKHR::default();
        instance.transform.matrix = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        instance.instance_custom_index = 0;
        instance.mask = 0xFF;
        instance.instance_shader_binding_table_record_offset = 0;
        instance.flags = vk::GeometryInstanceFlagsKHR::TRIANGLE_CULL_DISABLE;
        let mut instance_vk = instance;
        instance_vk.acceleration_structure_reference = vk::DeviceOrHostAddressConstKHR { device_address: blas_addr };"""
new = """        let mut instance = vk::AccelerationStructureInstanceKHR {
            transform: vk::TransformMatrixKHR {
                matrix: [
                    [1.0f32, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            },
            instance_custom_index_and_mask: vk::Packed24_8::new(0, 0xFF),
            instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(0, 0),
            acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_address: blas_addr },
        };
        let instance_vk = &mut instance;"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('instance fixed')
else:
    print('anchor missing')
