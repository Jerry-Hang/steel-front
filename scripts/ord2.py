# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
old = '.args([glsl_path.to_str().unwrap(), "-V", "-o", out_spv.to_str().unwrap()]).status()'
new = '.args(["-V", glsl_path.to_str().unwrap(), "-o", out_spv.to_str().unwrap()]).status()'
assert old in s
s = s.replace(old, new, 1)
io.open('build.rs', 'w', encoding='utf-8', newline='\\n').write(s)
print('order fixed')
