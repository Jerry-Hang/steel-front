# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('include_bytes!(concat!(env!("OUT_DIR"), "/fp32.spv")).as_slice()', 'include_bytes!("../assets/fp32.spv").as_slice()')
s = s.replace('include_bytes!(concat!(env!("OUT_DIR"), "/fp16.spv")).as_slice()', 'include_bytes!("../assets/fp16.spv").as_slice()')
s = s.replace('include_bytes!(concat!(env!("OUT_DIR"), "/fp8.spv")).as_slice()', 'include_bytes!("../assets/fp8.spv").as_slice()')
s = s.replace('include_bytes!(concat!(env!("OUT_DIR"), "/fp4.spv")).as_slice()', 'include_bytes!("../assets/fp4.spv").as_slice()')
s = s.replace('include_bytes!(std::concat!(std::env!("OUT_DIR"), "/rt.spv")).as_slice()', 'include_bytes!("../assets/rt.spv").as_slice()')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('include_bytes -> assets')
