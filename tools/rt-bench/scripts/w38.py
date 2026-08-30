# -*- coding: utf-8 -*-
import io
# fp16.comp 需要扩展行（body 里没有）——把 #extension 直接放 fp16
s16 = io.open(r'shaders\fp16.comp', encoding='utf-8').read()
s16 = s16.replace('#version 450\n#if defined(EXT16)', '#version 450\n#extension GL_EXT_shader_explicit_arithmetic_types_float16 : require')
s16 = s16.replace('#endif\nlayout(local_size_x = 256)', '#endif\nlayout(local_size_x = 256)')
io.open(r'shaders\fp16.comp', 'w', encoding='utf-8', newline='\n').write(s16)
print('fp16 ext ok')
