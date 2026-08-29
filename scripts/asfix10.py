# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("""                matrix: [
                    [1.0f32, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],""", """                matrix: [
                    [1.0f32, 0.0f32, 0.0f32, 0.0f32],
                    [0.0f32, 1.0f32, 0.0f32, 0.0f32],
                    [0.0f32, 0.0f32, 1.0f32, 0.0f32],
                ],""")
s = s.replace("vk::Packed24_8::new(0u32, 0xFFu32)", "vk::Packed24_8::new(0u32, 0xFFu8)")
s = s.replace("vk::Packed24_8::new(0u32, 0u32)", "vk::Packed24_8::new(0u32, 0u8)")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('fix matrix+packed')
