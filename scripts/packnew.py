# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/renderer.rs', encoding='utf-8').read()
# 用 bitfield 构造块替代（避开 impl 位置问题）——直接赋值 packed 通过 Packed24_8::new（按 doc 签名）
s = s.replace("vk::Packed24_8::new(0, 0xFF)", "vk::Packed24_8::new(0, 0xFF)")
io.open('src/engine/renderer.rs', 'w', encoding='utf-8', newline='').write(s)
print('ok (keep as is)')
