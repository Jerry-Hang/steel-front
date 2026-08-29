# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
s = s.replace("""                matrix: [
                    [1.0f32, 0.0f32, 0.0f32, 0.0f32],
                    [0.0f32, 1.0f32, 0.0f32, 0.0f32],
                    [0.0f32, 0.0f32, 1.0f32, 0.0f32],
                ],""", """                matrix: [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],""")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('flat matrix')
