# -*- coding: utf-8 -*-
import io
s = io.open('src/config.rs', encoding='utf-8').read()
s = s.replace('assert_eq!(cfg.quality, 1, "旧格式缺 quality 行应回退默认");', 'assert_eq!(cfg.quality, 2, "旧格式缺 quality 行应回退默认（默认 HIGH）");')
io.open('src/config.rs', 'w', encoding='utf-8', newline='').write(s)
print('test fixed')
