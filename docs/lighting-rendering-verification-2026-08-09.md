# 光照渲染验证报告（2026-08-09，只读）

验证对象：Steel Front（Rust + Vulkan/WSLg-dzn，零第三方游戏依赖）当前光照渲染的真实形态。
本文档只读分析，未修改任何 `.rs/.wgsl/.toml`，未运行 cargo，未读 `target/`、`Cargo.lock`、`*.spv`。

## 结论摘要

- 当前是**单 pass 前向光栅化 + 片元级 Blinn-Phong**：1 方向光 + 2 点光 + 环境光每帧上传并启用，但**阴影、法线贴图、PBR、HDR/tonemap 全都没有**。
- 法线不是顶点法线，而是片元里用 `dpdx/dpdy(world_pos)` 求的**平面法线（等效 flat shading）**，并双面翻转朝向相机。
- 光照是**乘性染色**：先「顶点色(tint) 与贴图 50% 混合」，再乘 `min(radiance, 1)`；NPC 纯色路径（`flat_flag`）**完全跳过光照**，是刻意的可读性设计。

## 1. 现状：实际是什么

### 1.1 管线形态

- 单 pass 前向：无 shadow pass、无 G-Buffer、无后处理；主 pipeline 一个 descriptor set（binding 0=view/proj UBO，1=贴图，2=实例 storage，3=采样器，4=光照 UBO）——`renderer.rs:1268-1320`。
- 光照数据每帧由 `main.rs:342 renderer.set_lights(&self.game.light_uniform())` 上传，`renderer.rs:4446-4453` 写入帧槽 UBO；所以**运行时光照是启用的**（`flags.x=1`）。全零 UBO 只是测试/向后兼容的"光照关闭"路径（`build.rs:192-194`，`lighting.rs:216-222` 空光源时保持全零）。

### 1.2 实际光源配置（`game.rs:1060-1081 light_uniform()`）

| 光源 | 参数 | 备注 |
|---|---|---|
| 方向光 sun | 表面→光源方向 `(-0.4,0.9,-0.3).normalize()`，颜色 `(1.0,0.95,0.85)`，强度 1.2 | 暖白日光 |
| 点光 A | 位置 `(0,6,0)`，颜色 `(0.9,0.6,0.4)`，强度 1.5 | 暖橙 |
| 点光 B | 位置 `(-24,5,-16)`，颜色 `(0.4,0.7,1.0)`，强度 1.0 | 冷蓝 |
| 环境光 | 颜色 `(0.5,0.55,0.6)` × 强度 0.35 | 冷灰天光 |
| 阴影 | `None`（`flags.y=0`） | 关闭 |

点光衰减默认 `c=1, l=0.09, q=0.032, range=0`（无限距离）；UBO 共 4 个点光槽位（`MAX_POINT_LIGHTS=4`，`lighting.rs:19`），当前用 2 个。

### 1.3 着色计算位置与模型

- 全部光照计算在 **片元着色器** `FRAGMENT_SHADER_WGSL`（内嵌于 `build.rs:76-224`），主函数 `fs_main` 在 `build.rs:179-224`；Blinn-Phong 辅助函数 `bp_diffuse/bp_specular/evaluate_directional/evaluate_point/shadow_test` 在 `build.rs:125-177`。build.rs 行号不受工作区并发改动影响。
- 模型：`radiance = ambient + Σ direction/point × (diffuse + 0.4×spec)`，`shininess=32`（`build.rs:217-222`）；最终 `lit = mixed × min(radiance, 1)`（`build.rs:223`），radiance 被 clamp 到 1，**无 HDR、无 tonemap**；交换链为 `B8G8R8A8_SRGB`（`renderer.rs:947-953`），输出即 sRGB。
- 法线：片元内 `normalize(cross(dpdx(world_pos), dpdy(world_pos)))`，朝向相机翻转（双面凸体近似）（`build.rs:197-203`）。**顶点数据没有法线**（`Vertex` 只有 pos/color/uv，`renderer.rs:49-54`；attribute 只有 location 0/1/2，`renderer.rs:1525-1552`），所以虽然是逐片元执行，实际等效于逐三角形平面着色。
- CPU 侧参考实现与 UBO 布局镜像在 `lighting.rs`（`blinn_phong_*`/`point_attenuation`/`shadow_view_proj`，`lighting.rs:262-353`；布局校验 `lighting.rs:203-207`，352 字节）。

### 1.4 阴影现状

- 阴影是**占位实现**：`shadow_test` 用硬编码深度 `1.0` 比较（`build.rs:212`，远平面外=无遮挡），场景里**没有任何 shadow map 贴图绑定**（renderer 无 shadow 资源）；阴影数学/常量骨架在 `lighting.rs:307-345`（`SHADOW_MAP_SIZE=2048`、`D32_SFLOAT`、光空间正交 view-proj），开启需接真实深度 pass。

## 2. 光照 × tint / flat_flag / 纹理混合路径

着色器执行顺序（`build.rs:179-224`）：

1. `fade ≤ 0.02` → discard（LOD 淡出，`build.rs:180-181`）。
2. **纯色路径**：`flat_flag > 0.5`（实例槽位 ≥ `NPC_INSTANCE_BASE=65601`，顶点着色器置 1，`build.rs:56-59`）→ 直接输出 `input.color × fade`，**跳过贴图和全部光照**（`build.rs:185-186`）。
3. **纹理混合路径**：`mixed = mix(顶点色×tint, texel, 0.5) × fade`（`build.rs:188-189`）。
4. 光照启用时：`lit = mixed × min(radiance, 1)`（`build.rs:217-223`）。

各类实例走哪条路：

| 对象 | tint 来源 | 路径 |
|---|---|---|
| 地形网格（identity 槽 65536，tint 白 `renderer.rs:2511-2512`，顶点色白 `create_terrain_lods`） | 白 | 纹理混合 + 光照 |
| 256×256 实例场（tint `0.7` 灰，`renderer.rs:2497`） | 灰 | 纹理混合 + 光照 |
| 世界障碍 marker（墙红/块橙/栅栏蓝灰，`main.rs:350-356`） | 障碍色 | 纹理混合 + 光照 |
| NPC 7 段积木人（红 `(0.95,0.12,0.08)` / 蓝 `(0.08,0.35,0.98)`，`main.rs:366-375`） | 阵营色 | **纯色，无光照无贴图** |

光照与 tint 的交互：顶点色已全部白化（`renderer.rs:66-94, 110-118`），所以 `color = 顶点色 × inst.tint` 实际就是 tint；光照整体乘在混合色上，**不改变色相方向，只压暗/提亮**（且 clamp 后易"泛白"）。NPC 纯色路径不走光照，保证红/蓝阵营色在灰地中清晰可辨（`build.rs:183-186` 注释明示）。

## 3. 对"真实光照"的支持度与改动点

已有：方向光/点光/环境光 UBO、Blinn-Phong、双面导数法线、阴影数学骨架（无实际阴影）。

缺失（要加的话动哪里）：

- **逐片元平滑法线 / 法线贴图**：`renderer.rs:49-54` 的 `Vertex` 加 `normal`（法线贴图还需 tangent，或由 uv+world 导数重建 TBN）；`renderer.rs:1525-1552` 加 attribute；`build.rs` `vs_main` 传世界空间法线，`fs_main` 用插值法线替换 `dpdx/dpdy`（`build.rs:197-203`）。
- **PBR**：`build.rs:125-177` 把 Blinn-Phong 换成 GGX/Smith BRDF，新增 material UBO（albedo/metalness/roughness，新 binding 或并入现有 UBO）；IBL/天空盒需新贴图 + sampler（仿 `renderer.rs:3631-3885` 纹理加载与 `init_descriptors`）；`min(radiance,1)`（`build.rs:223`）换成 HDR + tonemap。
- **阴影**：新增深度 pass（场景渲到 2048² `D32_SFLOAT`，常量已备 `lighting.rs:29-32`），descriptor 加 shadow map 采样 binding，`build.rs:206-212` 的占位比较换成 `textureSampleCompare`/PCF；`game.rs:1077` 传 `Some(&ShadowConfig)`。
- 注意：WGSL 在 `build.rs` 内嵌，**改后必须重新构建**（`build.rs:270-309` 编译写入 `assets/*.spv`），且 descriptor layout/UBO 布局改动要同步 `lighting.rs` 与 `renderer.rs:1268-1320, 1484-1497`。

以上全部是传统光栅特性，dzn 转译层可跑，不依赖 RT/DLSS；对"零第三方依赖 + 12GB 内存 + 纯 bin crate"约束友好（shadow map/PBR 都只需已有 API）。

## 4. gpu_caps 结论对光照路线的意义

探测结论（`gpu_caps.rs:37-207`，启动日志 `gpu-caps:` 前缀）：

- WSLg/dzn 下 **RT 全系不可用**：`VK_KHR_ray_tracing_pipeline / acceleration_structure / ray_query / deferred_host_operations` 均 false（`gpu_caps.rs:53-66` 枚举、`gpu_caps.rs:110-115` 特性、`gpu_caps.rs:126-166` 决定性探测 device 因扩展不全而跳过）；协作矩阵/DLSS 私有扩展（`VK_KHR_cooperative_matrix`、`VK_NV_cuda_kernel` 等）false。
- 可用：`VK_KHR_buffer_device_address`、`VK_KHR_dynamic_rendering`（`gpu_caps.rs:67-72`；AGENTS.md 实测记录）。
- CUDA 直通存在：`/usr/lib/wsl/lib/libcuda.so`（`gpu_caps.rs:176-191`）。

意义：

- **实时全局光照 / RT 阴影 / 路径追踪在 WSLg 的 Vulkan 路径不可行**（dzn 未实现 DXR 映射）；要光追/DLSS 需迁移 Windows 原生 Vulkan（NVIDIA 驱动全支持），或走 CUDA 直通自研（Tensor Core 可编程，可做超分/降噪）。
- 当前及近期光照路线（阴影映射、法线贴图、逐片元 Blinn-Phong/PBR）是**纯光栅特性，dzn 完全支持**，不构成迁移理由；gpu_caps 的 RT 探测只影响"未来光追方案"，不影响现有光照管线落地。

## 5. 引用源文件位置

- 着色器（WGSL 内嵌）：`build.rs:76-224`（vs 主函数 36-75，fs 主函数 179-224，光照函数 125-177，UBO 122 行）
- CPU 光照模块：`src/engine/lighting.rs:1-353`（布局镜像 130-246，Blinn-Phong 数学 262-305，阴影数学 307-345）
- 渲染器（内容锚点 + 验证时行号）：`src/engine/renderer.rs:49-54`（`Vertex` 无法线）、`init_descriptors`（descriptor 布局，光照 UBO binding(4)）、`set_lights`（当前 1857 行）、`create_terrain_lods`（地形网格，顶点色白）、`create_instance_buffer`（实例槽位/tint，当前 2476-2520）、`init_texture`（贴图，当前 3637+）、`record_command_buffer`（draw，含 NPC 段，当前 4095-4330）、「光照 Uniform：写入…」段（每帧上传，当前 4571-4578）。renderer.rs 行号在验证期间因工作区存在**并发未提交改动**（mip 链/各向异性 WIP）发生漂移，以上按最后一次核对的行号标注
- 场景光照配置：`src/engine/game.rs:1060-1081`
- 每帧接入与 tint：`src/main.rs:342`（set_lights）、`350-375`（marker/NPC 阵营 tint）
- GPU 能力探测：`src/engine/gpu_caps.rs:37-207`

> 附注（工作区状态）：验证过程中 `src/engine/renderer.rs` 出现非本次验证产生的未提交改动（各向异性过滤 + mip 链 WIP，约 +70 行），与光照模型无关，本文档未纳入评估，也未做任何修改；光照结论基于提交态（HEAD）语义与 build.rs/lighting.rs/game.rs 稳定代码。
