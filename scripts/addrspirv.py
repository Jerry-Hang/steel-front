# -*- coding: utf-8 -*-
import io
s = io.open('Cargo.toml', encoding='utf-8').read()
if 'rspirv' not in s:
    s = s.replace('naga = { version = "30", features = ["wgsl-in", "spv-out", "spv-in"] }', 'naga = { version = "30", features = ["wgsl-in", "spv-out", "spv-in"] }\nrspirv = "0.11"')
    io.open('Cargo.toml', 'w', encoding='utf-8', newline='').write(s)
    print('rspirv added')
