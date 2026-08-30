# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""            if gops < best { best = gops; }
            if gops > worst { worst = gops; }""", """            println!("    [轮] dt={:.3}ms gops={:.1}G ops={:.3}M", dt * 1000.0, gops, ops / 1e6);
            if gops < best { best = gops; }
            if gops > worst { worst = gops; }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('diag print')
