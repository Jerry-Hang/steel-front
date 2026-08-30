# -*- coding: utf-8 -*-
import io
comps = {
  'fp32': '''float acc = uintBitsToFloat(o[g]) * 0.001 + 0.5;
    for (uint i = 0; i < 4096; i++) {
        if ((i & 15u) == 0u) { acc = fma(acc, uintBitsToFloat(o[g]) * 1e-5, 0.9999); }
        acc = fma(acc, 0.9999, 0.0001);
    }
    o[g] = floatBitsToUint(acc);''',
  'fp16': '''float16_t acc = float16_t(uintBitsToFloat(o[g]) * 0.001 + 0.5);
    for (uint i = 0; i < 4096; i++) {
        if ((i & 15u) == 0u) { acc = acc * float16_t(uintBitsToFloat(o[g]) * 1e-5) + float16_t(0.9999); }
        acc = acc * float16_t(0.9999) + float16_t(0.0001);
    }
    o[g] = uint(acc) & 0xFFFFu;''',
  'fp8': '''uint acc = (o[g] & 0x7Fu) | 0x8000u;
    for (uint i = 0; i < 4096; i++) {
        if ((i & 15u) == 0u) { acc = (acc + (o[g] & 0xFFu)) & 0xFFFFu; }
        acc = (acc * 0xFBu + 0x0Fu) & 0xFFFFu;
    }
    o[g] = acc;''',
  'fp4': '''uint acc = (o[g] & 0x7u) | 0x88u;
    for (uint i = 0; i < 4096; i++) {
        if ((i & 15u) == 0u) { acc = (acc + (o[g] & 0xFu)) & 0xFFu; }
        acc = ((acc << 2) ^ (acc >> 1) + 0xDu) & 0xFFu;
    }
    o[g] = acc;''',
}
exts = {'fp16': '#extension GL_EXT_shader_explicit_arithmetic_types_float16 : require\n', 'fp32': '', 'fp8': '', 'fp4': ''}
for name, body in comps.items():
    src = '#version 450\n' + exts[name] + 'layout(local_size_x = 256) in;\nlayout(std430, binding = 0) buffer OutB { uint o[]; };\nvoid main() {\n    uint g = gl_GlobalInvocationID.x;\n    ' + body + '\n}\n'
    io.open(r'shaders\\' + name + '.comp', 'w', encoding='utf-8', newline='\n').write(src)
print('glsl written')
