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
