"""把 tools/blender/prep_guns.py 的产出安装为引擎枪模资产。

引擎按 `assets/guns/{weapon_key}.glb` 查找（main.rs::load_gun_glb），找不到回退
ak12.glb。预处理脚本的输出名沿用 Sketchfab 源文件名，因此这一层映射是必需的，
而映射本身是**领域知识**，值得写成代码而不是靠人记：

  PP-9 是 Bizon 最初的设计代号，PP-19-01 才是 Vityaz —— 武器表里 `pp9` / `pp19`
  两个 key 就来自这两个编号，看文件名很容易装反。

`assets/guns_ext/` 是**可再生的中间产物**（重跑 prep_guns.py 约 2 分钟），所以它
不进版本库；只有安装后的 assets/guns/<key>.glb 进。这样二进制资产只有一份真相。

用法:
    python tools/install_guns.py            # 安装并报告
    python tools/install_guns.py --check    # 只校验，不写盘（CI/自查用）
"""
import hashlib
import json
import os
import shutil
import sys

SRC_DIR = "assets/guns_ext"
DST_DIR = "assets/guns"
REPORT = os.path.join("tools", "blender", "prep_guns_report.json")

# 源文件名（去 .glb）→ 引擎武器 key。key 必须存在于 engine/guns/mod.rs::gun_mesh_by_key。
KEY_MAP = {
    "as_val": "asval",
    "ash_12.7__assault_rifle_shak_12": "ash12",
    "komrad_12_saiga_12": "saiga12",
    "low-poly_mp-443_grach": "mp443",
    "low-poly_osv-96": "osv96",
    "low-poly_rpk-16": "rpk16",
    "low_poly_ak104": "ak104",
    "pkm": "pkm",
    "pkp": "pkp",
    "pp-19-01_vityaz": "pp19",
    "pp-19_bizon": "pp9",      # 见模块注释：Bizon 的原设计代号就是 PP-9
    "sv98": "sv98",
    "vss_vintorez": "vss",
    # osv-96 也带 dup_warn（Object_23/Object_35 顶点数 2658/2651 几乎相同）。这里仍然
    # 安装，依据是几何证据而不是感觉：其 pos_range 为 X ±0.031 / Y ±0.075 / Z ±0.5，
    # 若真存在两把相差 90° 的完整枪身，X 会与 Z 同量级，而它只有 Z 的 6% —— 所以那是
    # 一对镜像部件，不是重复装配。分身也目视核对过它的截图。
    # 对照：svd_63 没有这种反证，故进 SKIP。
}

# 明确**不安装**的源文件，以及原因。留在这里是为了下次有人看到 svd_63 躺在
# guns_ext 里时，不会以为是漏装了。
SKIP = {
    "svd_63_-_dragunov":
        "源文件是产品宣传图：含两把相差 90° 的完整枪身 + 独立瞄具，需先在 Blender "
        "里删掉一把再装（prep_guns_report.json 标了 dup_warn）。目标 key 应为 svd12。",
}


def load_dup_warnings():
    """从预处理报告里读 dup_warn，用来提示"看着干净其实可能有重复装配"的资产。"""
    if not os.path.exists(REPORT):
        return {}
    try:
        with open(REPORT, "r", encoding="utf-8") as fh:
            data = json.load(fh)
    except Exception as e:  # noqa: BLE001 - 报告缺失不该阻断安装
        print("warn: 读不到 %s（%s），跳过 dup 校验" % (REPORT, e))
        return {}
    out = {}
    for entry in data if isinstance(data, list) else data.get("guns", []):
        # 报告条目里源文件名的字段就叫 src（不是 source/name），且带 .glb 后缀
        stem = os.path.splitext(str(entry.get("src") or ""))[0]
        if stem and entry.get("dup_warn"):
            out[stem] = "; ".join(str(n) for n in entry.get("notes", [])
                                  if "DUPLICATED" in str(n).upper()) or "dup_warn"
    return out


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()[:12]


def main():
    check = "--check" in sys.argv
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    src = os.path.join(root, SRC_DIR)
    dst = os.path.join(root, DST_DIR)
    if not os.path.isdir(src):
        print("找不到 %s；先跑 prep_guns.py" % src)
        return 1

    dups = load_dup_warnings()
    installed = skipped = changed = 0
    problems = []

    for stem, key in sorted(KEY_MAP.items()):
        s = os.path.join(src, stem + ".glb")
        if not os.path.exists(s):
            problems.append("%s: 源文件缺失 %s" % (stem, s))
            continue
        # dup_warn 是预处理阶段测到的"疑似重复部件"。osv-96 这类只是对称部件对，
        # 所以只打印不阻断；真正要人工处理的那件走 SKIP 明确排除。
        if stem in dups:
            print("note: %s 带 dup_warn（%s）——人工确认过再决定是否用" % (stem, dups[stem]))
        d = os.path.join(dst, key + ".glb")
        same = os.path.exists(d) and digest(s) == digest(d)
        if not same:
            changed += 1
        if not check and not same:
            shutil.copy2(s, d)
        installed += 1
        print("%-34s -> assets/guns/%-9s.glb  %s"
              % (stem, key, "已一致" if same else ("待更新" if check else "已安装")))

    for stem, why in sorted(SKIP.items()):
        if os.path.exists(os.path.join(src, stem + ".glb")):
            skipped += 1
            print("SKIP %-30s %s" % (stem, why))

    # 防漏装：guns_ext 里出现了映射表和 SKIP 都没覆盖的文件，必须报出来
    mapped = set(KEY_MAP) | set(SKIP)
    for f in sorted(os.listdir(src)):
        if not f.endswith(".glb"):
            continue
        if os.path.splitext(f)[0] not in mapped:
            problems.append("未映射的资产 %s（既不在 KEY_MAP 也不在 SKIP）" % f)

    print("-" * 72)
    print("%s: 安装 %d，跳过 %d，需更新 %d"
          % ("CHECK" if check else "INSTALL", installed, skipped, changed))
    for p in problems:
        print("!! " + p)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
