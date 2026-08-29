# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
s = s.replace('println!("RQ_PARSE_OK")', 'println!("cargo:warning=RQ_PARSE_OK")')
s = s.replace('println!("RQ_PARSE_ERR: {}",', 'println!("cargo:warning=RQ_PARSE_ERR: {}",')
io.open('build.rs', 'w', encoding='utf-8', newline='').write(s)
print('warning-probe')
