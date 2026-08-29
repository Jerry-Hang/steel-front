# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("use vk::KhrAccelerationStructure as _;", "use vk::KhrAccelerationStructure as _;\n        use vk::AccelerationStructureKhr as _;")
s = s.replace("            .index_stride(4)\n", "")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('fix1')
