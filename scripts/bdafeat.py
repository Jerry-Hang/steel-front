# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        let mut as_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_features.acceleration_structure = vk::TRUE;
        as_features.buffer_device_address = vk::TRUE;""", """        let mut as_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        as_features.acceleration_structure = vk::TRUE;
        let mut bda_features = vk::PhysicalDeviceBufferDeviceAddressFeaturesKHR::default();
        bda_features.buffer_device_address = vk::TRUE;""")
s = s.replace("""        let device_create_info = device_create_info
            .push_next(&mut as_features)
            .push_next(&mut rq_features);""", """        let device_create_info = device_create_info
            .push_next(&mut as_features)
            .push_next(&mut bda_features)
            .push_next(&mut rq_features);""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('bda separated')
