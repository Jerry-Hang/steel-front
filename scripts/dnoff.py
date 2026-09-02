# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 字段
s = s.replace("    pub pt_move_flag: std::cell::Cell<bool>,", "    pub pt_move_flag: std::cell::Cell<bool>,\n    pub pt_dn_enabled: bool,")
s = s.replace("            pt_move_flag: std::cell::Cell::new(false),", "            pt_move_flag: std::cell::Cell::new(false),\n            pt_dn_enabled: false, // 2026-09-02: 降噪默认关闭（修复白屏后 enable！）")
# dispatch + blit 加开关
s = s.replace("if self.dn_pipeline != vk::Pipeline::null() {", "if self.dn_pipeline != vk::Pipeline::null() && self.pt_dn_enabled {")
s = s.replace("if self.dn_pipeline != vk::Pipeline::null() { self.dn_img } else { self.pt_img }", "if self.dn_pipeline != vk::Pipeline::null() && self.pt_dn_enabled { self.dn_img } else { self.pt_img }")
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('dn off default')
