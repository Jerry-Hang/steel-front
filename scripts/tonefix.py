# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# 烘焙版 shade 0.55+0.25 → 0.35+0.20（更深）；原始版 ×5.5 → ×1.2
old1 = """                            // 烘焙色已含 AO（模型本色 0.08-0.11 纯黑金属）：直出微光
                            let shade = 0.55 + 0.25 * ndl;"""
new1 = """                            // 烘焙色已含 AO（模型本色 0.08-0.11 纯黑金属）：直出微光（2026-08-28 色调修正）
                            let shade = 0.35 + 0.20 * ndl;"""
old2 = """                            let shade = 0.30 + 0.92 * ndl;
                            [
                                (raw[0] * 5.5 * shade).min(1.0),"""
new2 = """                            let shade = 0.35 + 0.20 * ndl;
                            [
                                (raw[0] * 1.2 * shade).min(1.0),"""
old3 = """                            (raw[1] * 5.5 * shade).min(1.0),
                            (raw[2] * 5.5 * shade).min(1.0),"""
new3 = """                            (raw[1] * 1.2 * shade).min(1.0),
                            (raw[2] * 1.2 * shade).min(1.0),"""
if old1 in s: s = s.replace(old1, new1, 1)
if old2 in s: s = s.replace(old2, new2, 1)
if old3 in s: s = s.replace(old3, new3, 1)
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('tone fixed:', old1 not in s and old2 not in s)
