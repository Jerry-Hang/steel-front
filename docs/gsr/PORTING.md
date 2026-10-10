# SGSR1 → WGSL 移植要点

> 上游：`SnapdragonGameStudios/snapdragon-gsr`，`sgsr/v1`，**BSD 3-Clause**。
> 原始文件（取用时的逐字地址）：
> `https://raw.githubusercontent.com/SnapdragonGameStudios/snapdragon-gsr/main/sgsr/v1/include/glsl/sgsr1_shader_mobile.frag`
> **不在仓库里 vendor 它** —— BSD-3 要求的是**衍生作品保留版权声明**，
> 而移植后的 WGSL 才是衍生作品 ⇒ **那份 WGSL 文件头部必须原样保留上游的
> `Copyright (c) 2025, Qualcomm Innovation Center, Inc.` + `SPDX-License-Identifier: BSD-3-Clause`**。
> （为一个文件类型去拓宽提交白名单不值得；本文记全了结构，移植时按需再取原文对照。）
>
> 本文只记"怎么翻"，不含"已完成"的声称。

## 为什么选 SGSR1 而不是 SGSR2

| | SGSR1 | SGSR2 |
|---|---|---|
| 类型 | **单趟空间超分**（fragment shader）| 时域超分 |
| 输入 | **只要一张颜色纹理** | 还要 depth + motion vector + jitter + clipToPrevClip + preExposure |
| 扩展 | **零** —— 只用 `textureLod` + `textureGather`（core Vulkan 1.0）| 内部要 RGBA16F/R32UI + history ping-pong |

SGSR2 要的那份清单正是 `docs/DLSS-evaluation.md:37-41`，而本仓 `rg motion_vector src/` **零命中**。

## 上游 shader 的结构（读原文得出）

三个 `#define`：`OperationMode`（1=RGBA / 3=RGBY / 4=LERP）、`EdgeThreshold = 8/255`、`EdgeSharpness = 2.0`。
Uniform：`ViewportInfo[1] = vec4(1/inW, 1/inH, inW, inH)`（**输入纹理的尺寸**，不是输出的）+ `ps0`。

算法（`mode != 4` 即非 LERP 时）：
1. 底色 `color = textureLod(ps0, uv, 0)`
2. `imgCoord = uv * ViewportInfo.zw + vec2(-0.5, +0.5)` ← 🔴 **X 减、Y 加，不对称**
3. `left = textureGather(ps0, coord, mode)`
4. **边缘投票** `edgeVote = |left.z-left.y| + |color[mode]-left.y| + |color[mode]-left.z|`
5. 超阈值才做重活：gather right / up / down → 算局部 `mean` 与 `std = 2.181818/sum`
6. **12 个 `weightY` 抽头**（`fastLanczos2` 权重）→ `finalY = aWY.y / aWY.x`
7. 夹到 `[minY, maxY]`、乘 `EdgeSharpness`、`deltaY` 再夹到 ±23/255、加到 rgb
8. **`color.w = 1.0`** —— alpha 假定不用 ⇒ **必须整体替换，不能开混合**

## WGSL 映射表

| GLSL | WGSL（naga 30）| 备注 |
|---|---|---|
| `textureLod(ps0, uv, 0.0)` | `textureSampleLevel(ps0, samp, uv, 0.0)` | |
| `textureGather(ps0, coord, mode)` | `textureGather(1, ps0, samp, coord)` | 🔴 **component 是第一个参数，且 naga 要求是字面量** ⇒ 只能固定 `1`（即 RGBA 版），**做不了 mode=3/4 的运行时分支** |
| `uniform highp vec4 ViewportInfo[1]` | `struct PC { viewport: vec4<f32> }` + `var<push_constant>` 或 uniform | 本仓 `PtParams` 就是 push constant 的先例 |
| `in_TEXCOORD0` | `@location(0) uv: vec2<f32>` | |
| `out_Target0` | `@location(0) @interpolant(linear)` 返回值 | |
| `mediump`/`highp` | WGSL 无精度限定符 | 忽略即可（WGSL 默认足够）|

## 🔴 两条必须真机实测、不能靠推理的

1. **Y 方向半纹素符号**：上游是 `vec2(-0.5, +0.5)`（X 减、Y **加**）。
   底色那次 `textureLod` 不带偏移、不受影响，**只有锐化邻域受影响** ⇒
   症状是「**水平边缘的过冲上下错开一个内部纹素**」，**不是**整幅上下翻转。
   验法：1px 棋盘图 + `mode=4`（纯双线性）对照臂做 `png_diff`；错了就把 Y 的 `+0.5` 改成 `-0.5`（一行）。
2. **SGSR 自身帧时间**：官方表是 SD888~8Gen2、输出 ≤1240×576；
   本机是 **Adreno 810（6 系）输出 2400×1080**，外推不可靠。
   成本判据 = `SGSR 帧时间 < 基线 − 低分辨率帧时间`。

## 与 HUD 的关系（官方明确要求）

> 「2D UI 应在设备分辨率单独渲染叠加，不要进超分」（超分伪影在 2D 文字上最明显）

**本仓架构天然满足**：HUD 本来就走独立 overlay pass（`pipelines.rs` 的 `hud_render_pass`，
device 尺寸 / 1 采样 / 无深度 / 独立 framebuffer）。
只要把 `record.rs` 里 `hud_has_glass` 的条件改成 `hud_has_glass || sgsr_enabled`，
HUD 就不会被一起超分。

## 插入点

主 pass 结束之后、磨砂玻璃 blit（`record.rs` 那条 `交换链→320×200`）之前：
输入 = 新的**低分辨率场景图**，输出 = 交换链，`finalLayout = PRESENT_SRC_KHR`。
主 pass 的 resolve 附件 `finalLayout` 必须从 `PRESENT_SRC_KHR` 改成 `SHADER_READ_ONLY_OPTIMAL`。
**PT 分支跳过 SGSR。**

## 前置条件

SGSR 需要「内部渲染分辨率」这个概念（现无 —— 主 pass 直接渲到交换链）。
规格与 10 处改动点见 **`docs/android-render-scale-plan.md`**。
🔴 **顺序**：先把分辨率档落地并量出真实收益，再决定 SGSR 值不值得接 ——
它补的是"降分辨率后损失的画质"，不是"额外的性能"。
