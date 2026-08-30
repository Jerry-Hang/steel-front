# -*- coding: utf-8 -*-
import io
comps = {
  'fp32': '''float acc = uintBitsToFloat(o[g]) * 0.001 + 0.5;
    uint n = 16384u;
    for (uint i = 0; i < n; i++) {
        float c = 0.99 + float((o[g] + i) & 7u) * 0.001;
        acc = fma(acc, c, 0.0001);
        if ((i & 7u) == 0u) { o[g] = floatBitsToUint(acc) ^ o[g]; }
    }
    o[g] = floatBitsToUint(acc);''',
  'fp16': '''float16_t acc = float16_t(uintBitsToFloat(o[g]) * 0.001 + 0.5);
    uint n = 16384u;
    for (uint i = 0; i < n; i++) {
        float16_t c = float16_t(0.99 + float((o[g] + i) & 7u) * 0.001);
        acc = acc * c + float16_t(0.0001);
        if ((i & 7u) == 0u) { o[g] = (uint(acc) ^ o[g]) & 0xFFFFu; }
    }
    o[g] = uint(acc) & 0xFFFFu;''',
  'fp8': '''uint acc = (o[g] & 0x7Fu) | 0x8000u;
    uint n = 16384u;
    for (uint i = 0; i < n; i++) {
        uint c = ((o[g] + i) & 7u) + 0xFBu;
        acc = (acc * c + 0x0Fu) & 0xFFFFu;
        if ((i & 7u) == 0u) { o[g] = acc ^ o[g]; }
    }
    o[g] = acc;''',
  'fp4': '''uint acc = (o[g] & 0x7u) | 0x88u;
    uint n = 16384u;
    for (uint i = 0; i < n; i++) {
        uint c = ((o[g] + i) & 7u) + 0xDu;
        acc = ((acc << 2) ^ (acc >> 1) + c) & 0xFFu;
        if ((i & 7u) == 0u) { o[g] = acc ^ o[g]; }
    }
    o[g] = acc;''',
}
exts = {'fp16': '#extension GL_EXT_shader_explicit_arithmetic_types_float16 : require\n', 'fp32': '', 'fp8': '', 'fp4': ''}
for name, body in comps.items():
    src = '#version 450\n' + exts[name] + 'layout(local_size_x = 256) in;\nlayout(std430, binding = 0) buffer OutB { uint o[]; };\nvoid main() {\n    uint g = gl_GlobalInvocationID.x;\n    ' + body + '\n}\n'
    io.open(r'shaders\\' + name + '.comp', 'w', encoding='utf-8', newline='\n').write(src)
print('closed-form broken glsl')
