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

    # 1 m = 532 px in this 2x crop (d=4.3m, vFOV 70, original 1600px tall)
    px_per_m = 532.0
    print(f"{'y':>6} {'frac':>5} {'span_px':>8} {'span_m':>7}  {'left':>5} {'right':>5}")
    prev = None
    for y in range(y0, y1 + 1, step):
        idx = np.where(red[y])[0]
        if idx.size == 0:
            continue
        lo, hi = int(idx[0]), int(idx[-1])
        span = hi - lo + 1
        frac = (y - y0) / max(1, (y1 - y0))
        mark = ""
        if prev is not None and abs(span - prev) > 40:
            mark = "  <== 突变"
        print(f"{y:6d} {frac:5.2f} {span:8d} {span / px_per_m:7.3f}  {lo:5d} {hi:5d}{mark}")
        prev = span
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
