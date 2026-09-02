# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = "    pub fn pt_set_scene_markers(&mut self, markers: &[WorldMarker]) -> Result<(), String> {"
add = "    pub fn pt_set_scene_markers(&mut self, markers: &[WorldMarker]) -> Result<(), String> {\n        let _ = markers;\n        return Ok(()); // TEMP: 512-AS 崩溃二分\n        "
if anchor in s and 'TEMP: 512-AS' not in s:
    s = s.replace(anchor, add, 1)
    io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
    print('skip 512-AS inserted')
else:
    print('miss')
