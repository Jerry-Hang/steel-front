# 画质路线图（2026-10-21 起）

> 目标：从"顶点色 + 屏幕导数平面着色"提升到**接近现代引擎的可信画面**：
> 真实光照、PBR 材质/贴图、有细节的模型。按"先地基、后表面"排序，每个里程碑都要有
> **可量化的验收**（截图 + 像素差 + 帧时间），不是"看起来好点了"。

## 0. 现状基线（2026-10-21 实测，RTX 5060 / 2560×1600）

- 顶点格式 `stride=32`：`pos@0 color@12 uv@24`，**没有法线槽位** ⇒ 法线全部由
  `cross(dpdx, dpdy)` 屏幕导数重建 = **每三角形一个面法线（平面着色）**。
- 建筑/道具：GLB 顶点色（`export_materials="NONE"`），**没有贴图、没有 PBR 参数**；
  道具在 `weld_props.py` 里 UV 被写成常量（**UV 已不可用**）。
- 光照：方向光 + 2 点光 + 环境光，Blinn-Phong 高光；阴影贴图 2048²（静态/动态两张）工作正常
  （开/关阴影像素差 63%）；**没有色调映射**（交换链是 B8G8R8A8_SRGB，光照数值按 sRGB 直出）。
- 模型：城市建筑 6 个 GLB 模块 ≈1872 顶点/栋；树 ≈500 顶点；士兵/枪有独立 GLB。
  **Linux 开发机没有 Blender**（`D:\3D_Work` 在 Windows 侧）⇒ 资产管线必须走 Windows 或改程序化生成。
- 已知画面缺陷（近距实测）：地面区块共面条纹（z-fighting/acne 形态）、广场树坑穿模、
  素色天空、无 AO/接触阴影、材质全是纯色。

## M1 表面材质细节（**已落地 v1**，2026-10-21）

- `FRAGMENT_SHADER_WGSL`（mesh 与传统共用）在 `flat_flag>0.5` 路径里加**世界尺度**程序化材质：
  预制板缝（3.6m / 3.15m=FLOOR_H、3–4cm、暗 42%）、混凝土颗粒（7Hz 值噪声 ±8%）、
  竖向雨痕（1.7Hz × 离地包络、暗 24%）；按屏幕足迹 `deriv` 收敛，远处不闪。
- 门控：近竖直、低饱和、非绿（树/玻璃/彩色面不碰）。
- 验收：同机位近距像素差 0.69%（v1 弱参数）→ v1.1 上调后目视可辨（柱身环缝/竖缝）。

## M2 法线与 PBR 地基（下一步，最大单项收益）

1. 顶点格式加 `normal: [f32;3]`（stride 32→44），`gpu_layout.rs` 的编译期断言同步；
   两条管线（传统 + mesh）属性/location 同步；`build.rs` 顶点/片元 `VertexOutput` 加
   `normal` 并**用插值法线替换导数法线**（`safe_face_normal` 仅作退化回退）。
2. 所有几何生产者补齐法线：GLB（已有 `GVertex.normal`，转换时别丢）、程序化盒/柱/地面、
   实例 cube（marker；可用实例旋转矩阵变换面法线）、Mesh 模板（ICOSPHERE/SPHERE/BOX/CYL）。
3. 片元改 **GGX + 法线扰动**（先从顶点法线插值；后续可加程序化 normal 扰动，不需贴图）。
4. 验收：球面/树冠/枪模不再"棱角分明"；同机位像素差 + 帧时间 < +3%；
   `RV3D_VALIDATION=1` 下 VUID=0（需要先装验证层）。

## M3 真实贴图管线（PBR 材质）

1. 资产侧：GLB 重新导出 `COLOR_0` + `TEXCOORD_0` + `baseColorTexture`（Blender 在 Windows）；
   引擎 `assets.rs` 已有 GLB 贴图读取（枪械在用），扩展到道具/建筑并给 `PropSet` 加材质表。
2. 顶点/实例上传带上材质索引；片元按材质采样 albedo/roughness/metallic（先用灰度 PBR）。
3. 无资产回退：程序化三平面材质（M1 的升级版）+ 世界尺度 UV，保证 Android/无资产路径不空。
4. 验收：建筑立面近距截图有窗框/砖缝/污渍材质；`RV3D_PROC_TEX=0/1` A/B 可复现。

## M4 模型细节（建模）

- 资产侧（优先，Windows+Blender）：`build_city_kit.py` 增加窗洞倒角、露台、空调外机、
  女儿墙压顶、管道等真几何；LOD0/1/2 多档；顶点预算按 `RV3D_PROP_STATS=1` 实测控制。
- 引擎侧（不依赖 Blender）：程序化细节生成器（窗框/台阶/勒脚/管道）合并进 marker/prop 批次。
- 验收：近距立面三角形数/栋 ≥ 6k（LOD0），帧时间预算内；截图对比。

## M5 光照与后处理

- 线性光照 + ACES/AgX 色调映射（把现有 sRGB 直出改为线性管线，交换链继续 SRGB）；
- 天空渐变 + 太阳盘 + 云层（全屏 pass 或 HUD 前的一次绘制）；
- 屏幕空间 AO（深度重建法线后做 SSAO）；阴影分辨率/PCF 质量分档；
- 验收：画面直方图/对比度、阴影边缘样本；帧时间分档实测。

## M6 场景与风格

- 街道杂物、污渍贴花、地面 wetness、植被层次、远景雾/大气透视；
- 目标："第一眼像现代引擎做的战区街景"，用固定机位三张截图（街道/建筑近景/枪模检视）
  与 Unity/Unreal 参考图并列评审。

## 验收工具（已就绪）

- 固定机位截图：`RV3D_CAM=fly:x,y,z:yaw,pitch` + `RV3D_AUTOSTART=1`
  + `RV3D_SHOT_AT=秒`（Linux 输出 `/tmp/steel_front_<ts>.png`）。
- 像素差脚本、帧时间来自 perf 日志；验证层 VUID（需先安装 `vulkan-validationlayers`）。

## M2 进度（2026-10-21 夜，本次会话）

**已落地且已验证（编译 0 警告、821 测试通过、dGPU 冒烟 0 panic/0 lost）**：

1. 顶点格式 `stride 32 → 44`：`Vertex` 增加 `normal: [f32;3]`（`gpu_layout.rs`），
   管线属性 `location 3`（`pipelines.rs`），`renderer.rs` 的尺寸断言同步；
   模板几何（立方体/远档十字/地面/球/柱）补齐法线；阴影拷贝与其余字面量填零。
2. **道具法线生成**：`props::ensure_smooth_normals` —— 因为 `assets/props/*.glb`
   **全都没有 NORMAL 属性**（合并流程丢弃），加载期按位置做面积加权平均法线；
   `props`/`soldier` 上传时把 `[f32;11]` 里的 normal 真正写进顶点缓冲
   （实测 489144/489144 非零）。
3. 诊断探针：启动日志 `props: 顶点法线 N/M`，用于确认法线真的进了缓冲。

**未接线（今晚调试到一半，为避免留半成品已回退）**：

- 着色器侧：`vs_main` 读取 `@location(3)` 并把法线传给片元（`@location(8)`），
  片元 `fnrm = has_nrm ? normalize(input.normal) : 屏幕导数面法线`。
  回退原因：WSGL 里插入块时**多了一个 `}`** 导致 `expected global item ... found "let"`
  （naga 解析失败），`build.rs` 已 `git checkout` 回 HEAD（M1 材质版）。
- 续做步骤（清晰、低风险）：
  1. 在 `FRAGMENT_SHADER_WGSL`/`VERTEX_SHADER_WGSL` 的 `VertexOutput` 结构与 `vs_main`
     输入各加减一处（`@location(8) normal` / `@location(3) normal`），再跑 `cargo build`
     确认 build.rs 的 WGSL 解析通过（解析失败会直接 panic，位置精确）。
  2. 片元里在 `deriv` 已算好的位置插入 `has_nrm` 分支（**不要动括号层级**，
     建议整块替换而不是逐行插入，上次就是插入导致多括号）。
  3. 用 `RV3D_CAM` 固定机位 + 绿色诊断（`has_nrm` 时 `base=绿`）确认 attribute 到达片元，
     再换回真实法线；A/B 用像素差 + `RV3D_CAM` 同机位。
  4. 预期增益：树冠/车壳/曲面平滑着色；建筑棱边轻微倒角。mesh 路径（marker/NPC 盒子）
     因 `normal=0` 保持逐像素不变。

## M2 进度更新（2026-10-22：**法线 + GGX 已全线打通**）

上一轮未接线的着色器部分已完成并验证：

1. `VERTEX_SHADER_WGSL`：`@location(3) normal` 输入 → 世界空间变换（零向量保护）
   → `VertexOutput @location(8) normal`；`FRAGMENT`/`MESH` 两个 `VertexOutput` 同步加字段，
   mesh 路径恒零（保持旧面着色）。
2. 片元两处消费顶点法线：
   - `flat_flag > 0.5` 块里 `fnrm = has_nrm ? normalize(input.normal) : 屏幕导数`，
     `valid_nrm` 同步切到 1.0（玻璃菲涅耳不再被导数闸门误杀）；
   - **`apply_lighting` 的主光照法线**也改用插值法线（这是视觉真正变化的那一刀——
     之前只改了局部效果，主光照仍在用导数法线，所以差异很小）。
3. **GGX 替换 Blinn-Phong**：`pbr_specular`（Trowbridge-Reitz D + Smith-GGX 可见性，
   α 由旧 `shininess` 反解 α=sqrt(2/(s+2))，保持高光尺寸手感）。
4. 验证：
   - 绿色诊断（有法线涂绿）确认 attribute 全线到达（建筑/树/柱/地面大面积命中）；
   - 同机位 A/B（法线开/关）差异 **4.67%** 像素（曲面 facet 条纹消失）；
   - 与 M2 前对比（法线 + GGX）差异 **29.3% / 14.5%** 像素，平均差 6.2/3.2；
   - 新增 CPU 判据 `ensure_smooth_normals_fills_and_preserves`（单位法线/保留已有/坏索引不 panic）。
5. 剩余 M2 项：法线扰动（程序化 bump）、按材质 roughness（等 M3 的材质表）。

实测性能：dGPU 2560×1600 同场景 FPS 与 M2 前同档（33→41 波动为场景/NPC 差异，
perf 日志另测）；无 VUID（未装验证层，沿用既有口径）。

## 画质排查实录：广场"条纹板块"（2026-10-21，**已定案，非玩家可见缺陷**）

现象：多张调试截图里地面出现亮色板块 + 细条纹，关阴影/关贴图/关 marker 均无效，
只有 `RV3D_NO_PROPS=1` 消失。

定位过程：
1. 消融档 `RV3D_SHADOW_MODE=2/3`（光照直出）条纹仍在 ⇒ 不是光照/阴影；
2. `RV3D_NO_MARKERS=1` / `NO_TERRAIN_FIELD` / `NO_GROUND_TEX` 均无效；
3. 道具重叠探针发现**相机 (85,3,85) 距一栋 `building_tall` 中心仅 4m** —— 调试机位
   在楼体内部；看到的"板块"是**楼基板**，条纹是它与地面/另一栋重叠建筑基板的近距共面。
4. 在玩家正常位置（广场 `0,1.7,0`）复拍：**无条纹**。

结论：删除实验性的"建筑高度抖动"改动（未证实收益）；该现象是**调试机位 artifact**，
不是玩家路径缺陷。已登记诊断测试 `no_severe_prop_overlaps`（ignored）：城市布局确有
约 **190 对垂直相交+水平重叠**的道具（联排/街区互压），当前只影响"相机进楼"这种非法视角；
若将来要做室内/相机穿墙防护再修生成器。

保留的调试旋钮：`RV3D_SHADOW_EXTENT`（阴影覆盖半宽，默认 400 与历史一致；
远平面自动 = 2×extent，默认仍是 800）。

## M5 第一步：天空渐变 + 太阳光晕 + 雾色同源（2026-10-22 落地）

**做了什么**：
- 主 pass 最先画**天空全屏三角形**（`init_sky_pipeline`）：顶点着色器只用 `vertex_index` 生成
  覆盖 NDC 的大三角形；片元用 `push constant`（80B：相机 fwd/right/up + tan 半 FOV + aspect +
  太阳方向/强度 + 渲染分辨率）**逐像素重建世界方向**，再按 `dir.y` 做天顶→地平线渐变，
  叠加太阳圆盘 + 双指数大气光晕；8bit 输出加 ±1/255 抖动防色带。
- 管线**无顶点缓冲、无描述符集、不写深度**（世界几何随后覆盖），`void_mode`（检视）跳过。
- **雾色与地平线色单一真源**：`build.rs::SKY_ZENITH_XYZ / SKY_HORIZON_XYZ / SKY_BELOW_XYZ`
  → 构建期 `.replace()` 注入天空 WGSL 与片元 `FOG_TINT`；生成 `crate::shaders::FOG_TINT_RGB`
  等常量供判据。判据 `sky_and_fog_colors_share_one_source`（含"天顶必须比地平线暗"）。

**效果实测**（dGPU，2560×1600，固定机位）：
- 天空从"纯色 clear color"变成俯仰相关渐变；远处楼/树雾化到地平线的颜色与天空连续（无接缝）；
- 检视模式白底不变；HUD/小地图不受影响；帧率与改动前同档（同一场景 53 fps vs 38→54 波动为 NPC 差异）。

**下一步**：ACES/AgX 色调映射（需先把光照换算到线性域，风险较大，单独里程碑做）、
SSAO（从深度重建）、阴天/黄昏天空预设（3 组颜色常量 + 环境光联动）。

## 阴影分辨率实验（2026-10-22，**结论：4096 在本机不可用**）

- 现状 400m/2048² = **0.39m/纹素**，PCF 边缘块状感明显；试着把贴图提到 4096。
- 实测：**4096² D32 在 RTX 5060 + Wayland/PRIME 上首帧 device lost**（fence 5s 超时，
  日志无 VUID；2048/1024 均正常）⇒ 本机不能靠提高贴图分辨率换锐度。
- 保留旋钮 `RV3D_SHADOW_MAP`（**1024..=2048**，默认 2048 = 历史行为），用于低配/性能对照；
  由 `lighting::shadow_map_size()` 单点取用（分配/视口/着色器 `config.x` 同源，
  判据 `shadow_map_size_is_shared_by_allocation_and_uniform`）。
- 阴影锐度的后续方向：**缩小覆盖范围**（`RV3D_SHADOW_EXTENT`）或做级联，而不是堆分辨率。
