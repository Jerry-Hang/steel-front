# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = "    /// 每帧上传 NPC 士兵段到实例 buffer 的 NPC_SLOT_BASE 之后区域，"
if anchor in s:
    s = s.replace(anchor, """    /// 2026-08-28：第一人称枪的实例模型矩阵 per-frame（bob/后坐走矩阵，顶点静态）
    /// 枪槽 75841 的唯一写者：顶点缓冲 = 视空间静态（仅首次上传），矩阵每帧更新
    pub fn set_first_person_gun_model(&mut self, m: glam::Mat4) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return,
        };
        let stride = std::mem::size_of::<InstanceData>();
        unsafe {
            let p = slot.add(GUN_INSTANCE_INDEX as usize * stride);
            // InstanceData { model: [f32; 16], tint: [f32; 4] }
            let model = m.to_cols_array();
            std::ptr::copy_nonoverlapping(model.as_ptr(), p as *mut f32, 16);
        }
    }

""" + anchor, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('setter added')
else:
    print('anchor missing')
