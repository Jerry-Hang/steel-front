# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
old = """                        let ndl = n.dot(light).max(0.0);
                        // 烘焙版：顶点色已含基色+AO（直接使用，只加微光对比）；
                        // 原始版：材质基色 ×5.5 × 光照
                        let raw = [v[8], v[9], v[10]];
                        let c = if path.ends_with("baked") {
                            // 烘焙色已含 AO（模型本色 0.08-0.11 纯黑金属）：直出微光（2026-08-28 色调修正）
                            let shade = 0.35 + 0.20 * ndl;
                            [
                                (raw[0] * shade).min(1.0),
                                (raw[1] * shade).min(1.0),
                                (raw[2] * shade).min(1.0),
                            ]
                        } else {
                            let shade = 0.30 + 0.20 * ndl;
                            [
                                (raw[0] * 0.15 * shade).min(1.0),
                                (raw[1] * 5.5 * shade).min(1.0),
                                (raw[2] * 5.5 * shade).min(1.0),
                            ]
                        };"""
new = """                        // 终局（2026-08-28）：模型材质本色直出——baseColor × 忠实现光（×1.0，无提亮/压暗）
                        let ndl = n.dot(light).max(0.0);
                        let raw = [v[8], v[9], v[10]];
                        let shade = 0.85 + 0.30 * ndl;
                        let c = [
                            (raw[0] * shade).min(1.0),
                            (raw[1] * shade).min(1.0),
                            (raw[2] * shade).min(1.0),
                        ];"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
    print('REPLACED')
else:
    print('anchor still missing')
