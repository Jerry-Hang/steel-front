# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()

old_gun = '''    // 枪模专用 identity 槽（renderer.rs GUN_INSTANCE_INDEX = 65536+1+64+3072+64）：
    // 走 marker 纯色路径（flat=1），避免被地面纹理 0.75 混合 → 枪模隐形（2026-08-16）
    if (instance_index == 69697u) {
        output.flat_flag = 1.0;
        output.fade = 1.0;
    }'''
new_gun = '''    // 枪模专用 identity 槽（renderer.rs GUN_INSTANCE_INDEX = 65536+1+64+3072+64）：
    // flat=3 = baked 顶点光照直出路径（2026-08-22：marker 改走实时光照后，枪模保持烘焙）
    if (instance_index == 69697u) {
        output.flat_flag = 3.0;
        output.fade = 1.0;
    }'''
assert old_gun in s
s = s.replace(old_gun, new_gun, 1)

old_fs_start = '''@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }
    // 自发光（爆炸闪光等，顶点阶段 fade=2.0 标记）：直出顶点色 × tint，不受光照影响
    if (input.fade > 1.0) {
        return vec4<f32>(input.color, 1.0);
    }
    // marker/NPC 材质编码路径：flat_flag=1（marker）/ 2（NPC），按顶点 UV 采样
    // 程序化皮肤纹理（立方体每面 UV 铺满 0..1，与 procedural.rs 皮肤纹理逐面对齐）。
    // RV3D_SKIN_TEX=1（light_data.flags.z）启用；缺省 0 保持纯色路径（冒烟基线不变）。
    if (input.flat_flag > 0.5) {
        if (light_data.flags.z >= 0.5) {
            if (input.flat_flag > 1.5) {
                // NPC 士兵：迷彩军服纹理 × 阵营 tint（权重 0.65，阵营识别保留）
                let texel = textureSample(npc_skin_tex, texture_sampler, input.uv);
                let colored = mix(input.color, texel.rgb, 0.65) * input.fade;
                return vec4<f32>(colored, 1.0);
            } else {
                // marker 障碍：军备木板墙纹理 × 障碍 tint（权重 0.8，材质可辨识）
                let texel = textureSample(marker_skin_tex, texture_sampler, input.uv);
                let colored = mix(input.color, texel.rgb, 0.8) * input.fade;
                return vec4<f32>(colored, 1.0);
            }
        }
        return vec4<f32>(input.color * input.fade, 1.0);
    }
    // 世界空间 UV：地面/地形用 world_pos.xz 映射到全图 [0,1]（覆盖 2*256 米），
    // 与 procedural.rs 烘焙纹理严格对齐（marker/NPC/自发光已走 flat_flag 纯色路径，不采样）。
    let world_uv = (input.world_pos.xz + vec2<f32>(256.0, 256.0)) / 512.0;
    let texel = textureSample(texture_sampled, texture_sampler, world_uv);
    // 地面/地形：纹理占主导（0.75），顶点 tint 色仅轻微着色，保证材质配色可辨识
    let mixed = mix(input.color, texel.rgb, 0.75) * input.fade;

    // 默认关闭：light UBO 全零（flags.x = 0）时保持原「纹理+顶点颜色 50% 混合」渲染
    if (light_data.flags.x < 0.5) {
        return vec4<f32>(mixed, 1.0);
    }

    // 法线：顶点数据无法线，用世界坐标的屏幕导数求面法线，并朝向相机（双面凸体近似）
    let view_dir = normalize(input.view_dir);
    var normal = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
    if (dot(normal, view_dir) < 0.0) {
        normal = -normal;
    }

    // 阴影因子（方向光）：shadow map 3x3 PCF 深度比较 + bias。
    // 阴影图由 depth-only pass 渲染：顶点输出经 ADJUST_COORDINATE_SPACE Y 翻转，
    // 光空间 clip.y → 帧缓冲行 y_img=(1-clip.y)/2*H，采样 V 必须同式镜像
    // （uv.y=1-(clip.y*0.5+0.5)，否则阴影整体前后颠倒）；glam ortho_rh 的 clip.z∈[0,1]
    // 经 viewport 映射到深度 [0.5,1]，比较前同样映射 frag_depth = clip.z*0.5+0.5。
    var shadow_factor = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        let sp = light_data.shadow.light_view_proj * vec4<f32>(input.world_pos, 1.0);
        let shadow_uv = vec2<f32>(sp.x * 0.5 + 0.5, 1.0 - (sp.y * 0.5 + 0.5));
        let frag_depth = sp.z * 0.5 + 0.5;
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            let texel = 1.0 / light_data.shadow.config.x;
            var occluded = 0.0;
            for (var dy = -1; dy <= 1; dy = dy + 1) {
                for (var dx = -1; dx <= 1; dx = dx + 1) {
                    let d = textureSample(shadow_map, shadow_sampler,
                        shadow_uv + vec2<f32>(f32(dx), f32(dy)) * texel);
                    if (frag_depth - light_data.shadow.bias.x > d) {
                        occluded = occluded + 1.0;
                    }
                }
            }
            shadow_factor = occluded / 9.0;
        }
    }

    // Blinn-Phong 光照：环境光 + 方向光（乘阴影因子）+ 点光源（最多 4 个）
    let shininess = 32.0;
    var radiance = light_data.ambient.rgb * light_data.ambient.w;
    radiance = radiance + evaluate_directional(light_data.directional, normal, view_dir, shininess) * (1.0 - shadow_factor);
    for (var i = 0u; i < 4u; i = i + 1u) {
        radiance = radiance + evaluate_point(light_data.points[i], input.world_pos, normal, view_dir, shininess);
    }
    let lit = mixed * min(radiance, vec3<f32>(1.0));
    return vec4<f32>(lit, 1.0);
}
'''

new_fs = '''// 光照应用（地面与 marker/NPC 共用，2026-08-22）：屏幕导数法线 + 3x3 PCF 阴影 + 方向/点光。
// 障碍/建筑此前走“纯色直出”无面光照 → 一律同色剪影，纸片感；现在与地面同光源后，
// 顶面/迎光面/背光面自然分层，建筑/树/集装箱有立体明暗。
fn apply_lighting(input: VertexOutput, color: vec3<f32>) -> vec3<f32> {
    if (light_data.flags.x < 0.5) {
        return color;
    }
    let view_dir = normalize(input.view_dir);
    var normal = normalize(cross(dpdx(input.world_pos), dpdy(input.world_pos)));
    if (dot(normal, view_dir) < 0.0) {
        normal = -normal;
    }
    var shadow_factor = 0.0;
    if (light_data.flags.y >= 0.5 && light_data.shadow.bias.z >= 0.5) {
        let sp = light_data.shadow.light_view_proj * vec4<f32>(input.world_pos, 1.0);
        let shadow_uv = vec2<f32>(sp.x * 0.5 + 0.5, 1.0 - (sp.y * 0.5 + 0.5));
        let frag_depth = sp.z * 0.5 + 0.5;
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0
            && sp.z >= 0.0 && sp.z <= 1.0) {
            let texel = 1.0 / light_data.shadow.config.x;
            var occluded = 0.0;
            for (var dy = -1; dy <= 1; dy = dy + 1) {
                for (var dx = -1; dx <= 1; dx = dx + 1) {
                    let d = textureSample(shadow_map, shadow_sampler,
                        shadow_uv + vec2<f32>(f32(dx), f32(dy)) * texel);
                    if (frag_depth - light_data.shadow.bias.x > d) {
                        occluded = occluded + 1.0;
                    }
                }
            }
            shadow_factor = occluded / 9.0;
        }
    }
    let shininess = 32.0;
    var radiance = light_data.ambient.rgb * light_data.ambient.w;
    radiance = radiance + evaluate_directional(light_data.directional, normal, view_dir, shininess) * (1.0 - shadow_factor);
    for (var i = 0u; i < 4u; i = i + 1u) {
        radiance = radiance + evaluate_point(light_data.points[i], input.world_pos, normal, view_dir, shininess);
    }
    return color * min(radiance, vec3<f32>(1.0));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.fade <= 0.02) {
        discard;
    }
    // 自发光（爆炸闪光等，顶点阶段 fade=2.0 标记）：直出顶点色 × tint，不受光照影响
    if (input.fade > 1.0) {
        return vec4<f32>(input.color, 1.0);
    }
    // 枪模（flat=3）：顶点色已含烘焙光照，直出（不再被实时光照二次处理）
    if (input.flat_flag > 2.5) {
        return vec4<f32>(input.color * input.fade, 1.0);
    }
    // marker/NPC 材质编码路径：flat_flag=1（marker）/ 2（NPC），按顶点 UV 采样
    // 程序化皮肤纹理（立方体每面 UV 铺满 0..1，与 procedural.rs 皮肤纹理逐面对齐）。
    // RV3D_SKIN_TEX=1（light_data.flags.z）启用；缺省 0 保持纯色路径（冒烟基线不变）。
    if (input.flat_flag > 0.5) {
        var base: vec3<f32> = input.color;
        if (light_data.flags.z >= 0.5) {
            if (input.flat_flag > 1.5) {
                // NPC 士兵：迷彩军服纹理 × 阵营 tint（权重 0.65，阵营识别保留）
                let texel = textureSample(npc_skin_tex, texture_sampler, input.uv);
                base = mix(input.color, texel.rgb, 0.65);
            } else {
                // marker 障碍：军备木板墙纹理 × 障碍 tint（权重 0.8，材质可辨识）
                let texel = textureSample(marker_skin_tex, texture_sampler, input.uv);
                base = mix(input.color, texel.rgb, 0.8);
            }
        }
        // 障碍/NPC：应用 Blinn-Phong（同地面路径）——消除纯色剪影的纸片感
        let lit = apply_lighting(input, base);
        return vec4<f32>(lit * input.fade, 1.0);
    }
    // 世界空间 UV：地面/地形用 world_pos.xz 映射到全图 [0,1]（覆盖 2*256 米），
    // 与 procedural.rs 烘焙纹理严格对齐（marker/NPC/自发光已走 flat_flag 纯色路径，不采样）。
    let world_uv = (input.world_pos.xz + vec2<f32>(256.0, 256.0)) / 512.0;
    let texel = textureSample(texture_sampled, texture_sampler, world_uv);
    // 地面/地形：纹理占主导（0.75） + Blinn-Phong（默认关闭时保持旧混合渲染）
    let mixed = mix(input.color, texel.rgb, 0.75);
    if (light_data.flags.x < 0.5) {
        return vec4<f32>(mixed * input.fade, 1.0);
    }
    let lit = apply_lighting(input, mixed);
    return vec4<f32>(lit * input.fade, 1.0);
}
'''
assert old_fs_start in s
s = s.replace(old_fs_start, new_fs, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('shader splice ok')
