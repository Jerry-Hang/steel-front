"""数值巡检（AGENTS 教训 28 的执行工具）：多机位拍摄 + 逐帧色彩统计 + 纯黑判据。

为什么存在：图像通道会整批回放旧帧（2026-09-17/18/19 连续三晚复现），"看截图"在
这些夜晚不可用；而色彩统计不依赖通道。当晚就是靠它揪出树冠下的 NaN 黑带
（PROGRESS 2026-09-19 §7）。

用法:
  python tools/patrol.py sweep                    # 预设 12 机位逐个拍摄（走 cap_safe，强杀兜底）
  python tools/patrol.py stats a.png [b.png ...]  # 每帧 mean/过曝/纯黑/异常色占比
  python tools/patrol.py gate <png> [limit]       # 纯黑占比 < limit（默认 0.5%），超了 exit 1

判据（教训 27）：改动前后各跑一次 stats；gate 只对"纯黑/纯白"类成像事故敏感，
构图类缺陷仍需换视角配对（教训 3）+ 人眼或数值差分（scripts/png_diff.py）。
"""
import os
import subprocess
import sys

from PIL import Image

# 预设机位：双街向、四广场、柱廊仰视、NPC 环、树阵正下方（NaN 黑带的案发机位）、高空俯视、
# 哨卡院（cp1 穿模案发位）、商铺骑楼（棚带/牛腿）
CAMERAS = [
    ("sw01", "0,1.7,2:180,4"),
    ("sw02", "27.5,1.7,12:180,-25"),
    ("sw03", "-27.5,1.7,-14:0,10"),
    ("sw04", "-27.5,1.7,18.5:180,7"),
    ("sw05", "27.5,1.7,38:0,7"),
    ("sw06", "82.5,1.7,0:90,4"),
    ("sw07", "0,1.7,-70:180,2"),
    ("sw08", "55,1.7,55:225,0"),
    ("sw09", "27.5,1.7,9:0,-30"),
    ("sw10", "0,45,0:180,55"),
    ("sw11", "-137.5,1.7,-6:0,4"),
    ("sw12", "82.5,1.7,-10:0,-8"),
]


def _stats(path: str, step: int = 4):
    img = Image.open(path).convert("RGB")
    data = img.tobytes()  # 行主序 RGB 三元组，无 getdata 弃用警告
    n_total = len(data) // 3
    sr = sg = sb = 0
    blown = black = red = blue = white = 0
    n = 0
    for i in range(0, n_total, step):
        o = i * 3
        r, g, b = data[o], data[o + 1], data[o + 2]
        sr += r
        sg += g
        sb += b
        n += 1
        if r > 245 and g > 245 and b > 245:
            white += 1
        if r > 200 and g < 80 and b < 80:
            red += 1
        if b > 170 and r < 90:
            blue += 1
        if max(r, g, b) < 10:
            black += 1
        if min(r, g, b) > 235 and abs(r - b) < 12:
            blown += 1
    return {
        "mean": (sr // n, sg // n, sb // n),
        "blown": 100.0 * blown / n,
        "black": 100.0 * black / n,
        "red": 100.0 * red / n,
        "blue": 100.0 * blue / n,
        "white": 100.0 * white / n,
    }


def cmd_stats(paths):
    print("%-44s %14s %7s %7s %7s %7s %7s" % ("frame", "mean", "blown%", "black%", "red%", "blue%", "white%"))
    bad = False
    for p in paths:
        try:
            s = _stats(p)
        except OSError as e:
            print("%-44s ERROR %s" % (p, e))
            bad = True
            continue
        print("%-44s (%3d,%3d,%3d)  %6.2f%%  %6.2f%%  %6.2f%%  %6.2f%%  %6.2f%%%s" % (
            p, s["mean"][0], s["mean"][1], s["mean"][2],
            s["blown"], s["black"], s["red"], s["blue"], s["white"],
            "  <== BLACK" if s["black"] >= 0.5 else ""))
        if s["black"] >= 0.5:
            bad = True
    return 1 if bad else 0


def cmd_gate(path, limit=0.5):
    s = _stats(path)
    ok = s["black"] < limit
    print("%s black=%.2f%% limit=%.2f%% %s" % (path, s["black"], limit, "PASS" if ok else "FAIL"))
    return 0 if ok else 1


def cmd_sweep():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    env = dict(os.environ)
    env["RV3D_NO_NPC_CULL"] = "1"
    ps = os.path.join(root, "scripts", "cap_safe.ps1")
    for tag, cam in CAMERAS:
        env["RV3D_CAM"] = "fly:" + cam
        r = subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "$env:RV3D_CAM=%r; $env:RV3D_NO_NPC_CULL='1'; & %r -Tag %s -WarmupSec 6 -HoldSec 1 -Keys @(82) -AfterKeysSec 2"
             % (env["RV3D_CAM"], ps, tag)],
            capture_output=True, text=True, errors="replace", cwd=root)
        print(tag, cam, "ok" if "RESULT: ok" in (r.stdout or "") else "FAIL")
    return cmd_stats([os.path.join(root, "screenshots", "%s_b.png" % t) for t, _ in CAMERAS])


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    cmd = sys.argv[1]
    if cmd == "sweep":
        return cmd_sweep()
    if cmd == "stats":
        return cmd_stats(sys.argv[2:])
    if cmd == "gate":
        return cmd_gate(sys.argv[2], float(sys.argv[3]) if len(sys.argv) > 3 else 0.5)
    print("unknown subcommand:", cmd)
    return 2


if __name__ == "__main__":
    sys.exit(main())
