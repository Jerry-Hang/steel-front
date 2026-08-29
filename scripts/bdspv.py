# -*- coding: utf-8 -*-
import io
s = io.open('Cargo.toml', encoding='utf-8').read()
s = s.replace('naga = { version = "30", features = ["wgsl-in", "spv-out"] }\n#', 'naga = { version = "30", features = ["wgsl-in", "spv-out", "spv-in"] }\n#')
# 若有多个同样行（deps 也有）——只改第一处（build）
s = s.replace('[build-dependencies]\n# 编译 WGSL 着色器为 SPIR-V（构建时）\nnaga = { version = "30", features = ["wgsl-in", "spv-out"] }', '[build-dependencies]\n# 编译 WGSL 着色器为 SPIR-V（构建时）\nnaga = { version = "30", features = ["wgsl-in", "spv-out", "spv-in"] }')
io.open('Cargo.toml', 'w', encoding='utf-8', newline='').write(s)
print('build-deps spv-in ok')
