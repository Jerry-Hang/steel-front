# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
old = """@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }"""
new = """@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0); // 终极直通
    if (input.fade <= 0.02) {
        discard;
    }"""
if old not in s:
    print('anchor missing')
else:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('fs-top direct installed')
