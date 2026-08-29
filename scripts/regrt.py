# -*- coding: utf-8 -*-
import io
s = io.open('src/engine/mod.rs', encoding='utf-8').read()
if 'pub mod ray_tracer' not in s:
    s = s.replace("pub mod renderer;", "pub mod ray_tracer;\npub mod renderer;", 1)
    io.open('src/engine/mod.rs', 'w', encoding='utf-8', newline='').write(s)
    print('ray_tracer module registered')
