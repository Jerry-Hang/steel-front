# -*- coding: utf-8 -*-
import io
lines = io.open('src/main.rs', encoding='utf-8').read().split('\n')
# 删除 1083-1087 的误插（"let fp_gun_pre = {" 到 "};"）
del_start = next((i for i, l in enumerate(lines) if 'let fp_gun_pre = {' in l), None)
if del_start is not None:
    # 找该块结束（{" 到 "}\n"）——块结构：{ ... } 后
    # 以 "        }; " 结束行
    end = None
    depth = 0
    started = False
    for i in range(del_start, len(lines)):
        if 'let fp_gun_pre = {' in lines[i]:
            started = True
            depth = 1
        elif started:
            depth += lines[i].count('{') - lines[i].count('}')
            if depth <= 0:
                end = i
                break
    if end:
        del lines[del_start:end+1]
        print('removed misplaced block', del_start, end)
# 插到正确的 render 的 if let Some(renderer)（第二个 = 第 1135 当前行）前
idx = None
count = 0
for i, l in enumerate(lines):
    if 'if let Some(renderer) = &mut self.renderer {' in l:
        count += 1
        if count == 2:
            idx = i
            break
if idx:
    block = [
        '        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前）',
        '        let fp_gun_pre = {',
        '            let show = self.inspect_weapon.is_some()',
        '                || (self.game.state() == GameState::Playing',
        '                    && self.camera.mode == CameraMode::FirstPerson);',
        '            if show { self.fp_gun_matrix() } else { glam::Mat4::IDENTITY }',
        '        };',
        '',
    ]
    lines[idx:idx] = block
    print('inserted before line', idx)
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write('\n'.join(lines))
