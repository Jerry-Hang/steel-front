# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let rt_val = rt_test(&device, &queue, phys, &entry).unwrap_or(0.0);", "let rt_val = rt_test(&device, &queue, unsafe { device.get_physical_device_memory_properties(phys) }).unwrap_or(0.0);")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main call fixed')
