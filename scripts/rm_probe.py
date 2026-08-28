# -*- coding: utf-8 -*-
import io, re
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
s = re.sub(r'^.*eprintln!\("gdi stage.*$\n', '', s, flags=re.M)
s = s.replace('let row0 = std::slice::from_raw_parts(locked.data, (w as usize).min(8) * 4);\n            eprintln!("gdi stage 6b first px: {:02x} {:02x} {:02x} {:02x}", row0[0], row0[1], row0[2], row0[3]);\n', '')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('probes removed:', 'stage' not in s)
