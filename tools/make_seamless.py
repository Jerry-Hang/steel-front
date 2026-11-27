#!/usr/bin/env python3
"""把一张照片式贴图变成**可平铺**（无接缝）的贴图，确定性、零依赖（除 Pillow/numpy）。

## 为什么需要它

用生成模型出的是"照片"，四条边对不上 —— 实测一张 2048² 沥青的左右接缝差 23.6，
而它自身像素标准差只有 20.4（**接缝和纹理本身一样大**）。直接平铺会看到网格状缝。

## 做法：镜像拼贴 + 取中心

把原图按 2×2 镜像拼成 2W×2H，再从**正中心**裁回 W×H。
镜像拼贴在四条边天然连续（每对相邻边是同一列的镜像 ⇒ 差值恒为 0），
所以裁出来的任何 W×H 窗口都**逐字节可平铺**。

代价：纹理里出现镜像对称（同一图案左右/上下各一份）。对噪声类材质
（沥青、混凝土、泥地、木板、碎石）几乎看不出来；对**有明确方向性**的
图案（砖的错缝、文字、栏杆）会看出来 —— 那类应该直接按砖块周期画，
而不是靠这个工具。

用法：
    python3 tools/make_seamless.py <in.png> <out.png> [--size N]
"""
import sys

import numpy as np
from PIL import Image


def mirror_mosaic(a: np.ndarray) -> np.ndarray:
    """2×2 镜像拼贴：[[a, fliplr(a)], [flipud(a), flipud(fliplr(a))]]。"""
    top = np.concatenate([a, a[:, ::-1]], axis=1)
    bottom = np.flipud(top)
    return np.concatenate([top, bottom], axis=0)


def make_seamless(a: np.ndarray, out_size: int | None = None) -> np.ndarray:
    h, w = a.shape[:2]
    mosaic = mirror_mosaic(a)
    size = out_size or min(w, h)
    y0 = (mosaic.shape[0] - size) // 2
    x0 = (mosaic.shape[1] - size) // 2
    return mosaic[y0 : y0 + size, x0 : x0 + size]


def seam_gap(a: np.ndarray) -> tuple[float, float]:
    """左右 / 上下边缘的平均绝对差（0 = 完美可平铺）。"""
    return (
        float(np.abs(a[:, 0].astype(np.float32) - a[:, -1].astype(np.float32)).mean()),
        float(np.abs(a[0, :].astype(np.float32) - a[-1, :].astype(np.float32)).mean()),
    )


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    src, dst = sys.argv[1], sys.argv[2]
    size = None
    if "--size" in sys.argv:
        size = int(sys.argv[sys.argv.index("--size") + 1])
    img = Image.open(src).convert("RGB")
    a = np.asarray(img)
    before = seam_gap(a)
    out = make_seamless(a, size)
    after = seam_gap(out)
    Image.fromarray(out).save(dst)
    print(f"{src} → {dst}  {out.shape[1]}x{out.shape[0]}")
    print(f"接缝：改前 {before[0]:.1f}/{before[1]:.1f} → 改后 {after[0]:.1f}/{after[1]:.1f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
