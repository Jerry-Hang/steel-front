# 生成式贴图（本地 qwen-image-2.1-turbo）

> 2026-11-27 起：材质贴图用**本地出图模型**生成，不用数学公式硬算。

## 环境

- 服务：ComfyUI，`~/Desktop/ComfyUI.desktop` → `~/ai/bin/comfy-launch.sh`（端口 **8189**）
- 手动起（不开浏览器）：
  ```bash
  cd ~/ai/ComfyUI && setsid nohup ./venv/bin/python main.py \
      --listen 127.0.0.1 --port 8189 --reserve-vram 0.4 > /tmp/comfy_start.log 2>&1 &
  ```
- 模型（在 D 盘，经 `extra_model_paths.yaml` 挂进来）：
  - UNet：`qwen-image-2.1-turbo-Q4_K_M.gguf`
  - CLIP：`qwen3vl_8b_int8_convrot.safetensors`（type `qwen_image`）
  - VAE：`qwen_image_2.1_vae_bf16.safetensors`
- 采样：**8 步**、`cfg 1.0`、`euler` / `simple`、`denoise 1.0`
- 分辨率上限 **430 万像素** ⇒ 2048×2048 = 4.19 MP 是稳妥上限

实测耗时（本机 RTX 5060 Laptop 8GB + 30GB RAM，Q4_K_M 重卸载）：
1024² **36 s** / 2048² **111 s**（8 步）。

## 出图

```bash
python3 /tmp/comfy_gen.py <前缀> <宽> <高> "<提示词>" [seed] [steps]
# 打印生成的绝对路径（在 ~/ai/ComfyUI/output/）
```

提示词要点（材质类）：写明 **seamless tileable**、**orthographic top-down/flat front**、
**no shadows / no perspective / even diffuse lighting**、**photogrammetry albedo**，
再补材质细节词；写得越具体越像真材质。

## 接缝（必须做）

生成模型出的是"照片"，四条边对不上：实测 2048² 沥青左右接缝差 **23.6**，
而它自身像素标准差只有 20.4 —— **接缝和纹理本身一样大**，直接平铺就是网格状缝。

```bash
python3 tools/make_seamless.py in.png out.png [--size N]
```
做法：2×2 镜像拼贴后取正中心 ⇒ **逐字节可平铺**（实测接缝降到 0.0）。
代价是镜像对称：噪声类（沥青/混凝土/泥地/碎石）看不出；**有明确方向性的图案**
（砖的错缝、文字、栏杆）会看出来，那类应按砖块周期直接画。

## 已接入引擎的贴图

`src/engine/renderer/textures.rs` 里两个加载器**优先**读生成版，缺文件或
`RV3D_GEN_TEX_OFF=1` 时回退旧资产：

| 引擎插槽 | 生成版文件 | 旧资产 |
|---|---|---|
| 墙体皮肤（binding 7） | `assets/textures/marker_skin_bricks_generated.png`（1024²） | `marker_skin_bricks.png`（CC0 ambientCG） |
| 地面细节（binding 9） | `assets/textures/ground_detail_asphalt_generated.png`（512²） | `ground_detail_3way.png` / `_asphalt.png` |

另备 `marker_skin_concrete_generated.png`（无缝混凝土，1024²）供替换/对照。

## ⚠️ 已知：墙面"绒状花纹"**不是贴图造成的**

2026-11-27 排查（同机位 `RV3D_CAM='fly:-96,3,64:200,-6'`，640×400）：

| 对照 | 与基线差异 | 结论 |
|---|---|---|
| `RV3D_GEN_TEX_OFF=1`（砖→ambientCG） | 27.1% 像素 | 贴图确实换了 |
| `RV3D_SKIN_TEX=0` | **0.73%** | 皮肤贴图几乎不影响那片墙 |
| `RV3D_NO_PROP_ATLAS=1` | 48.8% | 图集也不是那块 |
| `RV3D_PROC_TEX=0` | 28.0%（灰度差仅 1.7） | 程序化贴图也不是 |
| **三者全关** | 花纹**仍在** | ⇒ 来自**片元里的程序化表面细节**（`window_dark` / 混凝土皮肤 / `is_canopy` 值噪声等），与贴图无关 |

⇒ **换贴图治不了那片墙**。下一步要动的是 `build.rs` 里 `flat_flag==2` 那条
marker 表面路径（世界尺度噪声 + 逐层窗带），不是贴图资产。

## 墙面"绒状/斜向点链"排查记录（2026-11-27，未结案）

同机位 `RV3D_CAM='fly:-96,3,64:200,-6'`，640×400，无 HUD。
指标 = 墙面区域 `(60,40)-(300,200)` 的**拉普拉斯平均绝对值**（高频能量，越小越干净）。

| 试验 | 墙面 hf | 判定 |
|---|---|---|
| 基线 | **19.70** | — |
| 换生成的砖皮（vs ambientCG） | 19.70 | 皮肤**不是**来源 |
| 皮肤换纯灰 1024² | **19.57（与上一行逐位相同）** | 该墙**完全不采样皮肤** |
| `RV3D_SKIN_TEX=0` | 19.70（整图仅 0.73% 差） | 不是皮肤路径 |
| `RV3D_NO_PROP_ATLAS=1` | 整图 48.8% 差，花纹仍在 | 不是道具图集 |
| `RV3D_PROC_TEX=0` | hf≈19.7（灰度差仅 1.7） | 不是程序化贴图 |
| `RV3D_QUALITY=0`（无 SSAO） | 20.14 | 不是 SSAO/SSCS |
| `RV3D_NO_SHADOW=1` | 27.79（更脏） | 不是阴影 PCF |
| `grain` 加距离淡出（2~7cm 半波长收敛） | 19.57 | **grain 不是主因**（只降 0.13） |
| 参照：天空 / 路面 / 草地 | 2.20 / 1.32 / 11.90 | 只在几何上，不是全局后处理 |

8× 放大的结构：**单像素暗点连成斜向点链，周期约 3px** —— 典型的高频走样或有序抖动，
而不是噪声贴图（贴图放大会有纹理内容）。

**尚未排查**（下一步按此顺序）：
1. `sf_hash21` / `vnoise2` 的其它调用点（`flat_flag 0.5~2.5` 两块用了 0.55~0.75 周期/米，
   理论上不该产生 3px 周期；但 `window_dark` 的**盒式平均**在缩小时可能产生摩尔纹）；
2. `build.rs:2256` 的 `fract(sin(lane_x * 12.9898))` —— 一维逐像素哈希；
3. marker 的**程序化皮肤回退**（`procedural.rs::marker_skin`，`rows=4`/`u*4.0`）在
   `RV3D_SKIN_TEX=0` 时才启用，需要单独关它再测一次。

## 已落到代码的修正

`build.rs` 的立面程序化块里，`grain`（7 周期/米 ⇒ 波长 14cm）原来**没有任何距离收敛**，
现在按**半波长**收敛（一个像素盖过 7cm 就归零）。这是原理性修正（Nyquist），
保留；但它不是那片墙的主要原因（实测只降 0.13）。
