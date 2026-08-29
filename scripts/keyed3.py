# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
s = s.replace("""        let gkey = self.game.active_weapon_key().to_string();
        let entry = self.gun_glbs.entry(gkey).or_insert_with(|| Self::load_gun_glb(&gkey));""",
"""        let gkey = self.game.active_weapon_key().to_string();
        let load = |k: &String| Self::load_gun_glb(k);
        let entry = self.gun_glbs.entry(gkey.clone()).or_insert_with(|| load(&gkey));""")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('borrow fixed')
