# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# 1) 走原始 ak12.glb（材质本色 0.057/0.076）
s = s.replace("""        let path = if std::path::Path::new("assets/guns/ak12_baked.glb").exists() {
            "assets/guns/ak12_baked.glb"
        } else {
            "assets/guns/ak12.glb"
        };""", """        // 2026-08-28 终局：使用原始模型材质本色（baseColorFactor 0.057/0.076 中性黑）
        let path = "assets/guns/ak12.glb";""")
# 2) 转换：×1.0 + 忠实现光 shade = 0.85 + 0.30×ndl（不再提亮/压暗——模型本色直出）
s = s.replace("""                        let ndl = n.dot(light).max(0.0);
                        // 烘焙版：顶点色已含基色+AO（直接使用，只加微光对比）；
                        // 原始版：材质基色 ×5.5 × 光照
                        let raw = [v[8], v[9], v[10]];
                        let c = if path.ends_with("baked") {
                            // 烘焙色已含 AO（模型本色 0.08-0.11 纯黑金属）：直出微光（2026-08-28 色调修正）
                            let shade = 0.35 + 0.20 * ndl;
                            [
                                (raw[0] * 0.15 * shade).min(1.0),
                                (raw[1] * 0.15 * shade).min(1.0),
                                (raw[2] * 0.15 * shade).min(1.0),
                            ]
                        } else {
                            let shade = 0.30 + 0.20 * ndl;
                            [
                                (raw[0] * 0.15 * shade).min(1.0),
                                (raw[1] * 0.15 * shade).min(1.0),
                                (raw[2] * 0.15 * shade).min(1.0),
                            ]
                        };""", """                        // 终局：模型材质本色直出（×1.0 + 忠实现光 0.85+0.30·ndl）
                        let ndl = n.dot(light).max(0.0);
                        let raw = [v[8], v[9], v[10]];
                        let shade = 0.85 + 0.30 * ndl;
                        let c = [
                            (raw[0] * shade).min(1.0),
                            (raw[1] * shade).min(1.0),
                            (raw[2] * shade).min(1.0),
                        ];""")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('final natural path:', '0.85 + 0.30 * ndl' in s and 'ak12.glb"' in s)
