# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# ① 字段
s = s.replace("    pub pt_move_base_fwd: std::cell::Cell<[f32; 3]>,", "    pub pt_move_base_fwd: std::cell::Cell<[f32; 3]>,\n    pub pt_move_flag: std::cell::Cell<bool>,")
s = s.replace("            pt_move_base_fwd: std::cell::Cell::new([0.0; 3]),", "            pt_move_base_fwd: std::cell::Cell::new([0.0; 3]),\n            pt_move_flag: std::cell::Cell::new(false),")
# ② 运动计算设 flag
old = """                            if d > 0.03 { 1.0 } else { (d / 0.03).min(1.0) }"""
new = """                            if d > 0.03 { self.pt_move_flag.set(true); 1.0 } else { self.pt_move_flag.set(d > 0.012); (d / 0.03).min(1.0) }"""
if old in s:
    s = s.replace(old, new, 1)
    print('move flag wired')
# ③ 降噪 dispatch 加 if（move_flag 才跑）；blit 源跟随
old2 = """                if self.dn_pipeline != vk::Pipeline::null() {"""
new2 = """                if self.dn_pipeline != vk::Pipeline::null() && self.pt_move_flag.get() {"""
if old2 in s:
    s = s.replace(old2, new2, 1)
    print('dn on move only')
# ④ blit 源：move_flag
old3 = "self.device.cmd_blit_image(command_buffer, if self.dn_pipeline != vk::Pipeline::null() { self.dn_img } else { self.pt_img }, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);"
new3 = "self.device.cmd_blit_image(command_buffer, if self.dn_pipeline != vk::Pipeline::null() && self.pt_move_flag.get() { self.dn_img } else { self.pt_img }, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);"
if old3 in s:
    s = s.replace(old3, new3, 1)
    print('blit follows flag')
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
