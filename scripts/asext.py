# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("        use vk::KhrAccelerationStructure as _;\n        use vk::AccelerationStructureKhr as _;", "        let ext = vk::khr::acceleration_structure::Device::new(&self.instance, &self.device);")
s = s.replace("self.device.get_acceleration_structure_build_sizes_khr(", "ext.get_acceleration_structure_build_sizes_khr(")
s = s.replace("self.device.create_acceleration_structure_khr(", "ext.create_acceleration_structure(")
s = s.replace("self.device.get_acceleration_structure_device_address_khr(", "ext.get_acceleration_structure_device_address(")
s = s.replace("self.device.get_buffer_device_address(", "self.device.get_buffer_device_address(")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('ext-device pattern')
