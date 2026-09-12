"""打印士兵特写的"剪影宽度随高度"曲线 —— 改尺寸**之前**必须先跑这个。

存在理由（2026-09-12 第 119/120 轮）：第 115/117 两轮直接改了胸廓/背心尺寸，
但量的是"整幅图里最宽的那一行"，**那一行未必由被改的段主导** ⇒ 两次都白做。
**⇒ 先看清"目标段在画面上的哪一段高度"和"它贡献多宽"，再动手。**

用法：python tools/measure_silhouette.py screenshots/soldier_check.png [step]
"""

import sys

import numpy as np
from PIL import Image


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else "screenshots/soldier_check.png"
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 20
    im = Image.open(path).convert("RGB")
    a = np.asarray(im).astype(np.int16)
    h, w, _ = a.shape
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    red = (r > 70) & (r > g + 15) & (r > b + 15)

    rows = np.where(red.any(axis=1))[0]
    if rows.size == 0:
        print("no red pixels")
        return 1
    y0, y1 = int(rows[0]), int(rows[-1])
    print(f"{path}  {w}x{h}  red rows y={y0}..{y1}")

    # ⚠️ 2026-09-12 第 135 轮修：原先把 px_per_m 写死 532（= 2x 裁剪）。
    #    喂 1x 裁剪时**静默给出小一倍的 span_m** —— 这属于"测量工具测的不是你以为的东西"。
    #    ⇒ 改成由**图宽**推出缩放比，并在不是已知两种宽度时报错。
    #    基准：原图 2560 宽 ⇒ 1m = 266 px（d=4.3m、vFOV 70 度、原图高 1600）。
    px_per_m_1x = 266.0
    if w == 1120:
        zoom = 2.0  # 2x 裁剪（如 soldier_check.png）
    elif w == 900:
        zoom = 1.0  # 1x 裁剪（如 soldier_gXX_wide.png）
    else:
        print(f"!! 未知的裁剪宽度 {w}（期望 1120=2x 或 900=1x）—— 拒绝给出 span_m，避免静默算错")
        zoom = None
    if zoom is not None:
        px_per_m = px_per_m_1x * zoom
        print(f"zoom={zoom:g}x  ⇒ 1 m = {px_per_m:.0f} px")
    else:
        px_per_m = 0.0
    print(f"{'y':>6} {'frac':>5} {'span_px':>8} {'span_m':>7}  {'left':>5} {'right':>5}")
    prev = None
    for y in range(y0, y1 + 1, step):
        idx = np.where(red[y])[0]
        if idx.size == 0:
            continue
        # ⚠️ 2026-09-12 第 135 轮修：必须取**最大连续游程**，不能取"最左到最右"。
        #    画面里可能有别的红色物体（远处士兵/红色道具），最左到最右会把跨度撑大 ——
        #    第 120 轮实测过：810px 里只有 560px 是士兵，其余 4px 是杂点，却让结论差了 45%。
        splits = np.where(np.diff(idx) > 1)[0]
        segs = np.split(idx, splits + 1)
        big = max(segs, key=len)
        lo, hi = int(big[0]), int(big[-1])
        span = len(big)
        frac = (y - y0) / max(1, (y1 - y0))
        mark = ""
        if prev is not None and abs(span - prev) > 40:
            mark = "  <== 突变"
        print(f"{y:6d} {frac:5.2f} {span:8d} {span / px_per_m:7.3f}  {lo:5d} {hi:5d}{mark}")
        prev = span
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
