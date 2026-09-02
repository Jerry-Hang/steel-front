# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
s = s.replace("{ let mut bb = spv_rt_bench::rt_bench_spv(); let mut by = Vec::with_capacity(bb.len()*4);", "{ let bb = spv_rt_bench::rt_bench_spv(); let mut by = Vec::with_capacity(bb.len()*4);")
io.open('build.rs', 'w', encoding='utf-8', newline='\n').write(s)
print('mut removed')
p2 = 'src/engine/ray_tracer.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("            i += 8;\n        }};", "            i += 8;\n            let _ = i;\n        }};")
io.open(p2, 'w', encoding='utf-8', newline='\n').write(s2)
print('i consumed in macro');
