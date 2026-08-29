# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('''            let khr_rt: [&'static str; 3] = [
                "VK_KHR_acceleration_structure",
                "VK_KHR_ray_query",
                "VK_KHR_ray_tracing_pipeline",
            ];''', '''            let khr_rt: [&'static str; 2] = [
                "VK_KHR_acceleration_structure",
                "VK_KHR_ray_query",
            ];''')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('two ext only')
