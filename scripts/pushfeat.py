# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
old = """        let device_create_info = if mesh_shader_available {
            device_create_info.push_next(&mut mesh_features)
        } else {
            device_create_info
        };"""
new = """        let device_create_info = if mesh_shader_available {
            device_create_info.push_next(&mut mesh_features)
        } else {
            device_create_info
        };
        // RT 特性链（Ext 启用 ≠ Feature 启用；rayQuery/accelStructure 必须显式 true）
        let device_create_info = device_create_info
            .push_next(&mut as_features)
            .push_next(&mut rq_features);"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('RT features pushed')
else:
    print('anch missing')
