# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("    let secs = d.as_secs() as i64;", "    let secs = d.as_secs() as i64 + 8 * 3600; // UTC+8 本地时区")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('tz fixed')
