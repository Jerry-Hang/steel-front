# -*- coding: utf-8 -*-
import io, re
s = io.open('build.rs', encoding='utf-8').read()
s = re.sub(r'/\* 【临时探针】.*?^\s*\}\n\n?', '', s, flags=re.S | re.M)
s = re.sub(r'    match naga::front::wgsl::parse_str\(RQ_PROBE\).*?\n', '', s, flags=re.S)
io.open('build.rs', 'w', encoding='utf-8', newline='').write(s)
print('clean', 'RQ_PROBE' not in s)
