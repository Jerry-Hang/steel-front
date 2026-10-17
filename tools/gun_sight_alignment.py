"""机械瞄准线检查：准星顶面 vs 表尺顶面（程序化枪模，24+ 把）。

    python tools/gun_sight_alignment.py            # 报告 + 与基线比对
    python tools/gun_sight_alignment.py --quiet    # 只报新增/恶化

为什么需要它
------------
2026-10-17 外部复核指出：多把枪的**准星顶面远低于表尺顶面**（差 3~10cm），
铁瞄的前后觇孔根本不在一条线上。这个问题**用现成判据抓不到** ——
`weapon_data.rs` 的判据只看包围盒尺寸与朝向，而 `GunMesh` 的部件是
**匿名 `(变换, 网格, 颜色)` 元组，没有「这是准星」的标签**。

本工具的取巧处：**源码注释就是标签**（`// 准星（深钢）` 后面那行就是准星），
所以它解析的是 `.rs` 文本而不是编译产物。同理 `// 表尺` / `// 照门`。

为什么是「基线」而不是「必须为 0」
----------------------------------
直接抬准星对齐表尺会做出**漂浮准星**：实测 `m249` 的准星在 `z=0.56`，
那里只有半径 0.018 的枪管（顶面 `y=0.063`），而表尺顶面在 `y=0.180`
⇒ 把准星组抬上去，它的底面会**悬空 8cm**。
正确修法是**同时补一段导气箍/增高座**（建模改动，会改轮廓），不在「挪坐标」的范围内。
⇒ 因此本工具记录**已知违规的基线值**，只在 (a) 出现新的违规武器、(b) 已违规的差值**变大** 时报失败。
这样新加的枪不会比现状更差，而修好一把后把基线调小即可。

判据
----
`|准星顶面 - 表尺顶面| <= 0.015 m`（15mm 内视为共线）。
顶面 = 部件中心 `y` + 该部件在 Y 方向的半高（箱体取 `h/2`；绕 Z/Y 旋转的圆柱取半径）。
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
GUNS = REPO / "src" / "engine" / "guns"
TOLERANCE = 0.015

# 已知违规基线（2026-10-17 实测）。修好一把就把这里调小；**新枪不许出现在这里**。
BASELINE = {
    ("antimaterial.rs", "m82a1"): 0.0635,
    ("antimaterial.rs", "osv96"): 0.087,
    ("dmr.rs", "mk14p"): 0.089,
    ("dmr.rs", "svd12"): 0.055,
    ("hmg.rs", "m2a1"): 0.097,
    ("hmg.rs", "rope12"): 0.091,
    ("lmg_blue.rs", "m240l"): 0.084,
    ("lmg_blue.rs", "m249"): 0.08,
    ("lmg_red.rs", "rpk16"): 0.0425,
    ("shotgun.rs", "aa12"): 0.0945,
    ("shotgun.rs", "m1014"): 0.065,
    ("shotgun.rs", "saiga12"): 0.059,
    ("smg_blue.rs", "mp5sd"): 0.027,
    ("smg_blue.rs", "mp7"): 0.047,
    ("smg_blue.rs", "mpx"): 0.04,
    ("smg_red.rs", "pp9"): 0.017,
}

BOX = re.compile(r"beveled_box\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)")
CYL = re.compile(r"cylinder\(\s*([\d.]+)\s*,\s*([\d.]+)")
FRU = re.compile(r"frustum\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)")
TOR = re.compile(r"torus_arc\(\s*([\d.]+)\s*,\s*([\d.]+)")
T = re.compile(r"\bt\(\s*(-?[\d.]+)\s*,\s*(-?[\d.]+)\s*,\s*(-?[\d.]+)\s*\)")
ROTATED = re.compile(r"\*\s*(?:rz|ry|rx)\(|\*\s*Mat4::from_rotation_[xyz]")


def y_half(line):
    """部件在 Y 方向的半高（足够判顶面，不必精确到倒角）。"""
    m = BOX.search(line)
    if m:
        return float(m.group(2)) / 2.0
    m = CYL.search(line)
    if m:
        return float(m.group(1)) if ROTATED.search(line) else float(m.group(2)) / 2.0
    m = FRU.search(line)
    if m:
        return float(m.group(3)) / 2.0
    m = TOR.search(line)
    if m:
        return float(m.group(1))
    return None


def scan():
    """-> {(file, gun): {front: top, rear: top}}（两遍：注释可能在准星前后任意一侧）"""
    out = {}
    for path in sorted(GUNS.glob("*.rs")):
        if path.name == "mod.rs":
            continue
        gun = None
        label = None
        for line in path.read_text(encoding="utf-8").splitlines():
            s = line.strip()
            m = re.match(r"pub fn (\w+)\(\)", s)
            if m:
                gun = m.group(1)
                label = None
                continue
            if s.startswith("//"):
                label = s.lstrip("/ ").strip()
                continue
            if not (gun and s.startswith("(")):
                continue
            mt = T.search(s)
            if not mt:
                continue
            half = y_half(s)
            if half is None:
                continue
            top = float(mt.group(2)) + half
            if label and "准星" in label:
                cur = out.setdefault((path.name, gun), {})
                cur["front"] = max(cur.get("front", -9.9), top)
            if label and ("表尺" in label or "照门" in label):
                cur = out.setdefault((path.name, gun), {})
                cur["rear"] = max(cur.get("rear", -9.9), top)
    return out


def main():
    quiet = "--quiet" in sys.argv
    data = scan()
    if len(data) < 20:
        print(f"FAIL: 只解析到 {len(data)} 把枪，解析器大概坏了（教训 27：没测到 != 测到 0）")
        return 2
    rows = []
    for (fn, gun), d in sorted(data.items()):
        if "front" not in d or "rear" not in d:
            continue
        gap = d["front"] - d["rear"]
        if abs(gap) > TOLERANCE:
            rows.append((fn, gun, d["front"], d["rear"], abs(gap)))
    if not quiet:
        print(f"解析 {len(data)} 把枪，其中 {len(rows)} 把准星/表尺不共线（> {TOLERANCE} m）：")
        print(f"{'文件':<20}{'武器':<12}{'准星顶':>8}{'表尺顶':>8}{'差':>8}{'基线':>8}")
        print("-" * 68)
        for fn, gun, f, r, g in rows:
            base = BASELINE.get((fn, gun))
            tag = "  NEW!" if base is None else ("" if g <= base + 1e-4 else "  恶化!")
            print(f"{fn:<20}{gun:<12}{f:>8.3f}{r:>8.3f}{g:>8.3f}"
                  f"{(base if base is not None else 0):>8.3f}{tag}")
    bad = []
    for fn, gun, f, r, g in rows:
        base = BASELINE.get((fn, gun))
        if base is None:
            bad.append(f"  {fn}:{gun} 是新违规（{g:.3f} m），要么修好要么加进基线并说明理由")
        elif g > base + 1e-4:
            bad.append(f"  {fn}:{gun} 从基线 {base:.3f} 恶化到 {g:.3f} m")
    if bad:
        print("\nFAIL: 出现新的/恶化的不共线")
        for b in bad:
            print(b)
        return 1
    print(f"\nOK: 没有新的违规（已知 {len(rows)} 把在基线内）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
