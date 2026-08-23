
# -*- coding: utf-8 -*-
import io
p = 'src/engine/procedural.rs'
s = io.open(p, encoding='utf-8').read()
start = s.index('/// 障碍物（marker）皮肤基色：军备木板墙。')
end_marker = '/// 士兵（NPC）皮肤基色：四色迷彩军服。'
end = s.index(end_marker)
new_fn = r'''/// 障碍物（marker）皮肤：中性灰混凝土砌块墙。
/// 浅灰底 + 砌块横排错缝（砂浆缝）+ 骨料噪点 + 水渍/风化暗斑（确定性纯函数）。
/// 设计（2026-08-22）：纹理只供「表面细节/凹凸感」，颜色由障碍 tint 主导
/// （shader mix 权重 0.45）→ 墙=混凝土、树=绿色细节、集装箱=彩色细节共用此皮肤。
fn marker_skin(u: f32, v: f32, seed: u32) -> [f32; 3] {
    // 砌块横排错缝：4 行 × 4 列（UV 0..1 内；上行与下行错半块）
    let rows = 4.0f32;
    let vv = v * rows;
    let row = vv.floor().min(rows - 1.0) as i32;
    let off = if row % 2 == 0 { 0.0 } else { 0.5 };
    let fu = (u * 4.0 + off).fract();

    // 砂浆缝：块边缘 0.06 宽暗缝（水平缝 + 垂直缝）
    let u_edge = fu.min(1.0 - fu);
    let v_fr = vv.fract();
    let v_edge = v_fr.min(1.0 - v_fr);
    let seam = 1.0 - smoothstep(0.02, 0.09, u_edge.min(v_edge));

    // 每块混凝土明度抖动 + 骨料颗粒（双频噪点）
    let tone = 0.88 + unit_from_hash(hash2(row, (u * 8.0) as i32, seed.wrapping_add(40))) * 0.24;
    let grain = value_noise(u * 22.0, v * 22.0, 1.0, seed.wrapping_add(41)) * 0.5
        + value_noise(u * 90.0, v * 90.0, 1.0, seed.wrapping_add(42)) * 0.5;
    let base: [f32; 3] = [0.50, 0.50, 0.52];
    let mut c = [
        base[0] * tone * (1.0 + grain * 0.22),
        base[1] * tone * (1.0 + grain * 0.20),
        base[2] * tone * (1.0 + grain * 0.18),
    ];

    // 水渍/风化暗斑（竖向条纹 + 斑点）
    let stain = value_noise(u * 6.0, v * 14.0, 1.0, seed.wrapping_add(43));
    if stain > 0.55 {
        let m = smoothstep(0.55, 0.92, stain) * 0.35;
        c = lerp3(c, [0.26, 0.27, 0.29], m);
    }
    // 砂浆缝压暗
    c = lerp3(c, [0.28, 0.29, 0.31], seam * 0.7);
    // 最终细噪（防色带）
    let dither = value_noise(u * 128.0, v * 128.0, 1.0, seed.wrapping_add(44)) * 0.035;
    [c[0] + dither, c[1] + dither, c[2] + dither]
}

'''
s = s[:start] + new_fn + s[end:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('marker_skin replaced')
