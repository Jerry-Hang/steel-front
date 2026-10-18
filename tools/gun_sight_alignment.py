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
    ("antimaterial.rs", "osv96"): 0.090,
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
    ("assault_red.rs", "ak12m"): 0.034,
}

# 🔴 **「已知不可判定」基线**（2026-10-18 实测，同形于 BASELINE 的棘轮逻辑）：
# 这些枪**没法判**，不是"合格"，而是注释里没有可用的语义标签（或只有一侧）。
# 记录在此 ⇒ **新增的不可判定会 FAIL**，历史遗留不会让门禁永远红。
# 修法：给源码补一行 `// 准星` / `// 表尺`，然后从本表删掉。
ONE_SIDE_KNOWN = {
    ("smg_blue.rs", "p90"),   # 只有 `// 内置瞄具`（既非准星也非表尺）
    ("smg_red.rs", "vss"),
    ("smg_red.rs", "asval"),
}
UNLABELED_KNOWN = {
    ("pistols.rs", "mp443"),
    ("pistols.rs", "rsh12"),
    ("pistols.rs", "m18"),
    ("pistols.rs", "mk23"),
    ("sniper.rs", "sv98"),
    ("sniper.rs", "m2010"),
    ("sniper.rs", "mrad"),
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
        # 🔴 `frustum` 乘了 `* rz()`（= 绕 X 轴 90°）时，**沿 Z 展开的 height 变成了 Y 方向**
        # ⇒ Y 半高是半径 `max(r0, r1)`，不是 `height / 2`（2026-10-18 复核指出）。
        # 反向验证：`ak12m` 的准星护翼 `frustum(0.004, 0.006, 0.052)` 乘了 `* rz()`，
        # 旧写法算 0.026（真实半高 0.006，大 4 倍）。
        if ROTATED.search(line):
            return max(float(m.group(1)), float(m.group(2)))
        return float(m.group(3)) / 2.0
    m = TOR.search(line)
    if m:
        # 环半径 + 管半径（原来只取环半径 ⇒ 顶面算少了一个 tube_r）
        return float(m.group(1)) + float(m.group(2))
    return None


def scan():
    """解析每把枪的准星/表尺顶面。

    -> {(file, gun): {"front": top, "rear": top, "ambiguous": bool, "unlabeled": int}}

    🔴 2026-10-17 修掉三个解析洞（外部复核指出，实测确认）：
    1. **`label` 消费完一行必须清零** —— 原来不清零，一个 `// 准星` 之后**所有无注释部件**
       都会被当成准星取 max；
    2. **合写注释 `// 准星 / 表尺` 不能同时赋给 front 和 rear** —— 那样 `front == rear`
       恒成立，哪怕把表尺改到 `y = -99` 脚本也报 0 差（**一个永远绿的判据**）；
    3. **英文注释漏检** —— `// front sight post` / `// rear sight` 原来完全不认。
    另外 `torus_arc` 的半高要加**管半径**（原来只取了环半径）。
    """
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
            half = y_half(s) if mt else None
            if mt and half is not None:
                top = float(mt.group(2)) + half
                cur = out.setdefault(
                    (path.name, gun),
                    {"front": None, "rear": None, "ambiguous": False, "unlabeled": 0},
                )
                is_front = bool(label) and ("准星" in label or "front sight" in label.lower())
                is_rear = bool(label) and (
                    "表尺" in label or "照门" in label or "rear sight" in label.lower()
                )
                if is_front and is_rear:
                    # 合写注释：无法判定哪一个才是，**整把枪标记为 ambiguous 并跳过比较**
                    cur["ambiguous"] = True
                elif is_front:
                    cur["front"] = top if cur["front"] is None else max(cur["front"], top)
                elif is_rear:
                    cur["rear"] = top if cur["rear"] is None else max(cur["rear"], top)
                else:
                    cur["unlabeled"] += 1
            # 🔴 消费完一行部件，标签必须清零
            label = None
    return out


def main():
    quiet = "--quiet" in sys.argv
    data = scan()
    if len(data) < 20:
        print(f"FAIL: 只解析到 {len(data)} 把枪，解析器大概坏了（教训 27：没测到 != 测到 0）")
        return 2
    both = {k: v for k, v in data.items()
            if v["front"] is not None and v["rear"] is not None and not v["ambiguous"]}
    ambiguous = [k for k, v in data.items() if v["ambiguous"]]
    unlabeled = [k for k, v in data.items() if v["front"] is None and v["rear"] is None
                 and not v["ambiguous"]]
    one_side = [k for k, v in data.items()
                if (v["front"] is None) != (v["rear"] is None)]
    # 🔴 覆盖率必须报出来并且守住下限：解析器悄悄退化时（比如标签规则改坏），
    # 光看「没有新违规」会误以为没事。
    print(f"解析 {len(data)} 把枪：可判定 {len(both)} · 合写注释跳过 {len(ambiguous)} ·"
          f" 无标签 {len(unlabeled)} · 只有单侧 {len(one_side)}")
    if len(both) < 20:
        print(f"FAIL: 可判定的只有 {len(both)} 把（< 20）—— 解析器覆盖退化了")
        return 2
    rows = []
    for (fn, gun), d in sorted(both.items()):
        gap = d["front"] - d["rear"]
        if abs(gap) > TOLERANCE:
            rows.append((fn, gun, d["front"], d["rear"], abs(gap)))
    if not quiet:
        print(f"其中 {len(rows)} 把准星/表尺不共线（> {TOLERANCE} m）：")
        print(f"{'文件':<20}{'武器':<12}{'准星顶':>8}{'表尺顶':>8}{'差':>8}{'基线':>8}")
        print("-" * 68)
        for fn, gun, f, r, g in rows:
            base = BASELINE.get((fn, gun))
            tag = "  NEW!" if base is None else ("" if g <= base + 1e-4 else "  恶化!")
            print(f"{fn:<20}{gun:<12}{f:>8.3f}{r:>8.3f}{g:>8.3f}"
                  f"{(base if base is not None else 0.0):>8.3f}{tag}")
        if unlabeled:
            print(f"⚠️ 无法判定（无标签，需人工看）：{' '.join(g for _, g in unlabeled)}")
        if ambiguous:
            print(f"⚠️ 合写注释跳过（改写注释才能判）：{' '.join(g for _, g in ambiguous)}")
    bad = []
    # 🔴 **基线里的枪必须仍然可判定**（2026-10-18 复核指出）：
    # 否则谁把某把枪的 `// 表尺` 注释删掉，它就从 `both` 掉进 `one_side`，
    # 而 `bad` 只查 `both` ⇒ **那把手枪会静默脱管、脚本照报 OK**。
    for (fn, gun) in BASELINE:
        if (fn, gun) not in both:
            bad.append(f"  {fn}:{gun} 在基线里但**已无法判定**（标签被删/改名？）")
    # 🔴 单侧标签 = 标签退化，必须报错而不是只打印一行（把"测不到"当"测到 0"是教训 46 的同形）
    for fn, gun in one_side:
        if (fn, gun) in ONE_SIDE_KNOWN:
            continue
        d = data[(fn, gun)]
        side = "准星" if d["front"] is not None else "表尺"
        bad.append(f"  {fn}:{gun} 只有{side}标签、另一侧缺失 ⇒ 无法比较（注释退化？）")
    for fn, gun in unlabeled:
        if (fn, gun) in UNLABELED_KNOWN:
            continue
        bad.append(f"  {fn}:{gun} 完全没有准星/表尺标签 ⇒ 无法判定（新枪？补注释）")
    for fn, gun, f, r, g in rows:
        base = BASELINE.get((fn, gun))
        if base is None:
            bad.append(f"  {fn}:{gun} 是新违规（{g:.3f} m）")
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
