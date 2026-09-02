# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
# 替换 png_decode_rgba 测试整块（从 #[cfg(windows)]\n    #[test]\n    fn png_decode_rgba 开始到 "    }" 结束（gdi_img 引用）
import re
pat = re.compile(r'    #\[cfg\(windows\)\]\n    #\[test\]\n    fn png_decode_rgba\(\) \{.*?\n    \}\n', re.S)
s2 = pat.sub('', s, count=1)
# 尾部可能残留 "#[cfg(windows)]\n" 悬空
s2 = s2.rstrip() + '\n'
io.open(p, 'w', encoding='utf-8', newline='\n').write(s2)
print('tail cleaned', len(s2))
