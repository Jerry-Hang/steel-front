"""整幅 PNG 差分（第一道筛子）。

用法: python scripts/png_diff.py A.png B.png

输出: 尺寸、有差异像素数/占比、只在差异像素上的平均通道差、最大通道差。

判据（教训 28）：整幅 diff 的数值**不能当改善幅度**用，它只回答"两次运行画的是不是同一幅"。
所以它必须配合同一时刻的配对（同一次运行的 `_a`/`_a`），否则差异里混着 NPC 走动等时间噪声。
"""
import sys

from PIL import Image, ImageChops


def main() -> int:
    a_path, b_path = sys.argv[1], sys.argv[2]
    a = Image.open(a_path).convert("RGB")
    b = Image.open(b_path).convert("RGB")
    if a.size != b.size:
        print(f"SIZE MISMATCH: {a.size} vs {b.size}")
        return 1
    diff = ImageChops.difference(a, b)
    # 三通道差的绝对值和（0..765）
    gray = diff.convert("L")
    hist = gray.histogram()
    total = a.size[0] * a.size[1]
    diff_px = total - hist[0]
    weighted = sum(i * n for i, n in enumerate(hist))
    max_ch = max(diff.getextrema(), key=lambda t: t[1])[1]
    avg = (weighted / diff_px) if diff_px else 0.0
    print(f"A={a_path.split('/')[-1].split(chr(92))[-1]}  B={b_path.split('/')[-1].split(chr(92))[-1]}")
    print(
        f"size={a.size[0]}x{a.size[1]}  diff_px={diff_px}/{total}  "
        f"diff_pct={100.0 * diff_px / total:.3f}%  avg_gray_on_diff={avg:.2f}  max_channel={max_ch}"
    )
    # 差异的包围盒 —— **没有它，几百个差异像素既可能是"引擎坏了"也可能是"HUD 上的 FPS 数字变了"**。
    # 定位到具体区域之后，才谈得上判断这是噪声还是真缺陷（教训 27：先确认工具测的是你以为的东西）。
    if diff_px:
        box = gray.point(lambda v: 255 if v else 0).getbbox()
        print(f"diff_bbox={box}  (x0,y0,x1,y1)  区域={box[2] - box[0]}x{box[3] - box[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
