# -*- coding: utf-8 -*-
import io
s = io.open('Cargo.toml', encoding='utf-8').read()
s = s.replace('naga = { version = "30", features = ["wgsl-in", "spv-out"] }', 'naga = { version = "30", features = ["wgsl-in", "spv-out", "spv-in"] }')
io.open('Cargo.toml', 'w', encoding='utf-8', newline='').write(s)
print('spv-in added')
