# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("        let mut info = vk::DeviceCreateInfo::default()\n            .queue_create_infos(&queues)\n            .enabled_extension_names(&ext_ptrs)\n            .enabled_features(&vk::PhysicalDeviceFeatures::default());", "        let mut feats = vk::PhysicalDeviceFeatures::default();\n        let mut info = vk::DeviceCreateInfo::default()\n            .queue_create_infos(&queues)\n            .enabled_extension_names(&ext_ptrs)\n            .enabled_features(&feats);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('feats fixed')
