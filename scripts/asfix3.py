# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("let ext = crate::khr::acceleration_structure::Device::new(&self.instance, &self.device);", "let ext = ash::khr::acceleration_structure::Device::new(&self.instance, &self.device);")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('ash::khr')
