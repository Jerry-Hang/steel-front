"""量士兵特写里"单条腿"的像素宽 —— 用来反解段尺寸表里 `scale` 的语义。

背景：段表注释写「圆柱 scale = (半径, 高, 半径)」。腿是圆柱 r=0.085 ⇒ 直径 0.17m。
相机 d=4.3m、vFOV 70 度、原图高 1600px ⇒ 原图 1m = 1600/(2*4.3*tan(35)) = 266 px。
脚本读的是 2x 裁剪图（1120x1640）⇒ 1m = 532 px。

判据：
  单腿宽约 90 px（2x 图）⇒ 圆柱是"全半径"语义 ⇒ 单位网格 ⇒ 盒子是"半宽" ⇒ 胸廓实宽 0.72m
  单腿宽约 45 px           ⇒ "全直径"     ⇒ 盒子是"全宽" ⇒ 胸廓实宽 0.36m

用法：python tools/measure_legs.py screenshots/soldier_check.png
"""

import sys

import numpy as np
from PIL import Image


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else "screenshots/soldier_check.png"
    im = Image.open(path).convert("RGB")
    a = np.asarray(im).astype(np.int16)
    h, w, _ = a.shape
    print(f"图 {w}x{h}  {path}")

    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    red = (r > 70) & (r > g + 15) & (r > b + 15)

    rows = np.where(red.any(axis=1))[0]
    if rows.size == 0:
        print("没有红色像素 —— 判据或图不对")
        return 1
    y0, y1 = int(rows[0]), int(rows[-1])
    print(f"红色行范围 y = {y0} .. {y1}（高 {y1 - y0 + 1} px）")

    print("\n逐行游程（下半部，每 20 行）：")
    widths: list[int] = []
    for y in range(y1 - (y1 - y0) // 3, y1 + 1, 20):
        row = red[y]
        idx = np.where(row)[0]
        if idx.size == 0:
            continue
        # 切成连续游程
        splits = np.where(np.diff(idx) > 1)[0]
        segs = np.split(idx, splits + 1)
        desc = "  ".join(f"{s[0]}..{s[-1]}({len(s)}px)" for s in segs if len(s) >= 3)
        print(f"  y={y:4d}: {desc}")
        for s in segs:
            if 20 <= len(s) <= 200:  # 腿这种细柱
                widths.append(len(s))

    if widths:
        arr = np.array(widths)
        print(f"\n细游程（20..200px）共 {arr.size} 个：")
        print(f"  中位 {int(np.median(arr))} px  最小 {arr.min()}  最大 {arr.max()}")
        print("  判据：约 90px ⇒ 盒=半宽（胸廓 0.72m）；约 45px ⇒ 盒=全宽（胸廓 0.36m）")
    else:
        print("\n没找到 20..200px 的细游程")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
