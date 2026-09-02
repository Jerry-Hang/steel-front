# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
old = """                }
                    log::info!("PT-RESIDENT init: {e}");
                }
            }"""
new = """                }
            }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/main.rs', 'w', encoding='utf-8', newline='\n').write(s)
    print('dup removed')
else:
    print('miss dup')
