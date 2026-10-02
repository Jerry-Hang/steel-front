# raytrace-pt —— `PROGRESS.md` 主题分档

> 由 `docs/PROGRESS.md` 按主题切档而来（2026-10-02，v5）。
> **移动单位 = 一个完整事件**：事件标题（H1，或带事件标记的 H2）+ 其下所有
> 续写标题（`改动` / `取证` / `判定` / `症状与根因` / `为什么` …）整段连续搬运，
> 绝不从中间切开；H3/H4 本来就在正文内，未作为切点。
> 历史约定不变：**错版保留 + 追加更正**。
> 本档 4 个事件（含 7 个续写标题）、15.6 KB，日期 2026-09-12 .. 2026-09-19。
> 跨主题引用查 `docs/PROGRESS.md` 索引。

<!-- 原 PROGRESS.md 第 2419-2436 行 · 2026-09-12 · H2 · 1 个标题 -->
## ✅ 确认：地面 AO 烘焙的调用链完整（2026-09-12 第 98 轮）

第 96/97 轮我据 `procedural.rs` 的**模块注释**判定"⑦ 的烘焙已有一半"。本轮**从调用链验证**它确实接到了屏幕上：

```
renderer.rs:7128   let size = super::procedural::GROUND_TEXTURE_SIZE;
renderer.rs:7133   procedural::generate_city_ground_texture(size, &height_at)
                                              ^^^^^^^^^^ 地形高度采样器 = AO 烘焙的输入
renderer.rs:7543   procedural::GROUND_DETAIL_SIZE / generate_default_ground_detail_texture()  // 细节层
renderer.rs:7510   generate_default_marker_skin_texture()   // marker 混凝土皮肤
renderer.rs:7521   generate_default_npc_skin_texture()      // NPC 皮肤
```

**⇒ 链路完整**：`height_at`（`terrain_height`）→ `generate_city_ground_texture` 烘焙
AO/静态天光（高度场凹度遮蔽）→ 上传为地面纹理 → 片元按 world-space UV 采样。

**⇒ 第 97 轮写进入口摘要的"⑦ 烘焙已有一半"现在是从调用链验证过的结论，不只是从注释推断。**

<!-- 原 PROGRESS.md 第 1658-1720 行 · 2026-09-15 · H1 · 8 个标题 -->
# ✅ 未结案 #2 结案 —— PT 史上首次真正出图；#3 原结案被推翻并真修（2026-09-15）

## 前提：验证层当天上午才第一次能跑

- 当天上午修掉 mesh 着色器的 SPIR-V 布局问题（未结案 #9）之后，`RV3D_VALIDATION=1` **第一次真的可用**
  —— 此前它必然**灰屏**（它的失败曾被当成"已知限制"写进文档，见教训 36）。
- 于是"把 PT 打开"第一次产生了**指名道姓的验证层报文**，两个互相独立的真 bug 因此一次全暴露。

## Bug A：交换链 `image_usage` 缺 `VK_IMAGE_USAGE_TRANSFER_DST_BIT`

- PT 通路要把 `pt_img` **blit 进交换链图像**（先 barrier 到 `TRANSFER_DST_OPTIMAL`，再 `vkCmdBlitImage`），
  而 `image_usage` 里没有 `TRANSFER_DST`。
- 验证层：`VUID-vkCmdBlitImage-dstImage-00224` 与 `VUID-VkImageMemoryBarrier-oldLayout-01213`。
- 修法：加上 `TRANSFER_DST`（先查 `surface_capabilities.supported_usage_flags`，不支持则告警）。

## Bug B（真正的崩溃源）：`hud_framebuffers` 指向已销毁的 ImageView

- `hud_framebuffers` 只在 `init_hud_overlay()` 里建一次，取自当时的 `swapchain_image_views`；
  而 `destroy_swapchain()` 会销毁这些 image view，`recreate_swapchain()` **从不重建 HUD framebuffer**。
- **启动阶段光是 resize 事件就有 5 次交换链重建** ⇒ 它们指向的全是已销毁的 `VkImageView`。
- **它唯一的消费者恰恰是 PT 通路**（光栅通路画的是 `self.framebuffers`，那个是重建的）⇒
  症状精确地是「**光栅一切正常、一开 PT 就崩**」，而崩溃原因**与 PT 代码毫无关系**。
- 验证层：`vkCmdBeginRenderPass(): pCreateInfo->pAttachments[0] VkImageView ... is invalid` 与
  `VUID-VkRenderPassBeginInfo-framebuffer-parameter`。
- 修法：抽出 `recreate_hud_framebuffers()`（先销毁旧的、再按当前 views 重建），
  由 `recreate_swapchain()` 在 `init_swapchain()` 之后调用；`destroy_swapchain()` 与 `Drop` 也销毁它。

## 验收（**PT 打开状态下**）

- 跑到 `PT-BLAS` / `PT-TLAS` / `PT-RESIDENT (2560x1600, spp target 256)` / `PT-SCENE (1024 boxes)`，
  **渲出一张一眼就是路径追踪的图，约 75 fps，HUD 正确合成在上面** ⇒ `screenshots/pt_live_b.png`。
- **两族 VUID 全部消失**；`scripts/run_smoke_pm.ps1` 在 **PT 打开**下报 `RESULT: ALL-OK`
  （VUID=0、panics=0、kill 已登记、fps 76.5）。
- `cargo build --release` **0 警告**、`cargo test --release` **485 passed / 0 failed**。

## ⚠️ 遗留（不致命，PT 能出图）：布局记账还不干净

验证层还剩 3 条：`VUID-VkImageMemoryBarrier-oldLayout-01197`、
`VUID-vkCmdBeginRenderPass-initialLayout-00900`（HUD 的 render pass 声明 `initialLayout=PRESENT_SRC_KHR`，
而实际布局不是它）、`VUID-vkCmdDraw-renderPass-02684`（绑定的管线与当前 render pass 不兼容）。

## 未结案 #3 重开：原结案只查了"字段存在"，没查接线

- **原结案是错的**：当时只核对字段存在（`config.rs:25/27`）与 `main.rs` 在读它，**没看 parse 分支**。
- 真相：`load_from` 的 match **没有 `pt_enable` / `rt_enable` 两个 arm**，`save_to` **也从不写这两个键**
  ⇒ 两字段**只可能等于编译进去的默认值**，**配置文件与设置面板根本开不了 PT**
  （这也是 #2 那条"设 true 一启动即崩"无法从正常路径复现的原因）。
- 修法：补两个 arm + `parse_bool`（接受 `1/0` 与 `true/false`，**非法值保持默认、不 panic**）；
  `save_to` 现在两个键都写。
- 测试：新增 `pt_and_rt_enable_are_read_from_file`；并**加强**原有 `save_then_load_roundtrip` ——
  它原先对这两个字段**既不写也不读**，两边都取默认值，`assert_eq!` 照样通过，**正好把 bug 藏住**。
- **守卫验证过会红**：临时删掉 `load_from` 的两个 arm ⇒ **两条测试同时 FAIL**。
- 🔴 **教训**：**往返测试只对"非默认值"有区分度**；**"字段存在"≠"接线完成"——
  结案前要走完整条 写 → 读 → 用 的链路。**

## 📄 文档维护：`AGENTS.md` 压缩 + 教训 36

- 把几条已结案的未结案条目压成一行，以留在 **65,536 B 硬上限**之内（超限会**静默截断**注入视图）。
- 新增**教训 36**：「**「工具跑不起来」本身就是一条要修的缺陷**」——
  验证层因灰屏被写进文档当"已知限制"，此后**几周没人开过它**；
  根因修掉的当天第一次开起来，**立刻**报出两条一直存在的 VUID。


<!-- 原 PROGRESS.md 第 7183-7219 行 · 2026-09-19 · H2 · 1 个标题 -->
## 18. PT 专项第三段：道具进 BLAS + RT 死开关清除（2026-09-19）

用户指令「把 PT 道具喂进 BLAS，把 RT 踢掉」。两项均完成，509/509 测试，release 零警告，验证层探针（val2）无新增 VUID。

### 18.1 RT 踢除：本来就是个死开关

`rt_enable` 全仓检索后确认：只有 config 读写、UI 字段、game.rs current_config 三处**搬运**，渲染侧**零消费者**——RT 光栅化路径早在 PT 接管时已删空，只剩开关壳。删除范围：config.rs 字段/默认值/解析/保存、ui.rs 字段、game.rs current_config 行。旧配置文件里的 `rt_enable=` 行现在**静默忽略**（config 测试专门加了这条断言：读入不报错、不写回）。`pt_enable` 独立保留，仍默认关。

### 18.2 道具进 BLAS：零拷贝第二几何 + 设备本地属性表

BLAS 从单几何（盒）扩为双几何：geom0=盒（`PT_MAX_BOXES*24` 顶点缓冲，12 三角/盒），geom1=道具——**直接引用** `prop_vertex_buffer`/`prop_index_buffer`（零拷贝，仅 build 期读，无逐帧风险）。道具 872,032 三角 / 盒 21,480，`PT-SCENE: 盒 1790 + 道具三角 872032` 一次重建 ~100ms。

**关键教训一（pt3 灰顶棚事故，94.7% 像素灰）**：`rayQueryGetIntersectionPrimitiveIndexEXT` 返回的是**每几何局部索引**，不是全局编号。最初按全局边界 `hitPrim < pc.g.x` 分派，前 21,480 个道具三角命中盒材质表越界回落 vec3(0.5) 灰。正确判据是 `rayQueryGetIntersectionGeometryIndexEXT`（0=盒，1=道具）。

**关键教训二（pt3 帧率 126→1.5fps）**：着色器逐命中随机读 HOST_VISIBLE 的 VB/IB（`create_host_buffer` 是 HOST_VISIBLE|HOST_COHERENT，非 DEVICE_LOCAL）= 每命中一次 PCIe 随机读风暴。**永不**让着色器随机读 HOST_VISIBLE 缓冲。修复：`set_props` 上传期在 CPU 预烘焙**设备本地**属性表（binding 4，2×u32/三角：w0=量化面法线 `v*127+127` 三轴 u8，w1=顶点色均值 u8×3），着色器只读这一张表。1.5→18.2fps，灰 94.7%→0.0%。法线朝入射射线翻转（绕序无关，闭合壳体）。

**BLAS 生命周期**：道具几何准入条件 `prop_attr_tris*3 == prop_index_count`；`pt_prop_key=(vb_handle, attr_handle, index_count)` 变化 ⇒ 整体重建 PtAssets（wait_idle → build_pt_as → 重置 dset 绑定 0/2/4 → 场景重建 → 销毁旧资产 → 清累积帧）。帧序 `set_props → pt_set_scene_markers → render` 保证无悬垂窗口。任何一步失败 ⇒ 道具不进 BLAS，退回盒场景（宁缺勿错）。

### 18.3 顺带修掉的两个隐患

- **PT_MAX_BOXES 1024→2048**：城市 marker=1789 > 1024，#10 静默截断陷阱**第三次复发**；且 `markers.take(PT_MAX_BOXES-1)` 在告警比较**之前**截断，导致 `build_pt_as` 的 warn 闩锁永远不触发。截断前先比较并告警（测试固化）。
- **PT 地面反照率 沙色→沥青色 [0.115,0.120,0.128]**：§15 路面亮度 ×2.14 偏差的主因之一（盒场景地面用了沙色），与光栅沥青贴图对齐。

### 18.4 验证矩阵（全绿）

| 门 | 结果 |
|---|---|
| `cargo test` | 509/509（含 pack 第七 vec4、cap 告警前置、rt_enable 忽略、tint 通道等） |
| `cargo build --release` | 零警告 |
| `compile_pt.ps1` + spirv-val | OK（**glsl+spv 同 commit**，fbc6031 铁律） |
| pt4 探针 | fps 18.2，canopy 灰 0.0%，绿 342→1857 |
| val2 验证层 | 仅 5 条已知 #23 交换链误报；PT-SCENE 重建触发 2 次（初始+开局）符合设计 |
| 光栅 patrol（PT 关） | 12/12 视角全绿，black ≤0.12%，阴影 LOD 门守住——VB/IB usage 变更与属性表烘焙对光栅零回归 |
| 冒烟 A/B | PT 关：ALL-OK（1 击杀、104.3fps）；PT 开：VUID=0/panics=0、NPC 持续掉血、55.5fps——击杀数未达标系低帧率下 harness 鼠标注入收敛竞态，非游戏缺陷 |

**PT 开时 101→55fps 定性**：`pt_live_enabled` 下 PT 是**每帧计算路径**（非仅开视图才渲染），872k 三角 BLAS 的遍历成本使然（pt4 原生分辨率 18.2fps 同链证据）。冒烟门语义就此定案：**PT 开=稳定性门**（VUID/panic/掉血判定），**PT 关=玩法门**（击杀/patrol）。PT 仍默认关（`pt_enable=false`）：全景 1spp 是收敛前下限，默认开需用户拍板。

<!-- 原 PROGRESS.md 第 7220-7275 行 · 2026-09-19 · H2 · 1 个标题 -->
## 19. PT↔光栅标定对齐：参照帧走同一条曲线（2026-09-19 深夜）

§18 后复审 §15 分区偏差表（同机位 `fly:0,1.7,30:180,2`，bat5 光栅基线 vs pt5 PT 实时合成）：方向从 §15 的 **×2.14 过亮翻成 ×0.44-0.59 过暗**。道具遮挡+沥青地面收掉了过亮侧，暴露出剩下的是**结构性不可比**，不是光照差：

| 偏差源 | 光栅 | PT（旧） | 处置 |
|---|---|---|---|
| tone 曲线 | `1-exp(-x·1.55)`（build.rs apply_lighting） | ACES 拟合 | PT 改为同源指数压缩 |
| 全照太阳 | 1.0×sun×ndl | 两点抖动 `(lit1+lit2)×1.1` = **2.2×** | 归一 ×0.5（均值语义） |
| 曝光 | 无 | live ×0.2 | 标定值 **0.4**（见下） |
| 天空 | 清屏色 (0.24,0.36,0.60)（艺术） | 0.49-0.65 常数 | 重定义为**补光源**：余弦半球均值 ≈0.30，对齐 ambient (0.5,0.55,0.6)×0.55 |

**曝光 0.4 的推导**（不是试出来的）：光栅把反照率乘在 tone **外**（`alb×(1-exp(-1.55L))`），PT 物理正确在**内**（`tone(alb·L)`）——两模型对暗面差一个曲线位置。解 `tone(0.12×1.525×e)=0.12×tone(1.525)` 得 e≈0.40，此值下 albedo 0.1~0.8 全域互差 ≤15%。

**标定后实测**（pt6 vs bat5）：路面 ×2.14→**×1.07**，树冠 ×1.90→**×1.08**（且树在 PT 里真实存在了），楼体 R ×1.91→**×0.93**，楼体 L **×0.84**，天空 ×0.72（**刻意保留**——PT 天空是补光源不是显示天空，参照判表面不判天空）。

**已知边界（不调）**：楼体 L ×0.84 是盒面表对光栅立面程序化图案的材质近似差，用补光常数去补材质误差是错的杠杆。下一步正确方向是盒面表反照率按立面均值重采样（属材质工作，非光照）。

PT-VIEW 参考帧路径（exposure 0.5、PT_SUN_INTENSITY 1.5）语义不变，但曲线换源后其历史参照帧不可直接对比。测试 509/509，glsl+spv 同 commit。

### 19.1 跨机位泛化验证与雾裁决（同日晚些）

标定常数只在大道机位上定的，有单视角过拟合风险。换 sw06 东西街机位（`fly:82.5,1.7,0:90,4`，垂直于太阳方向，两侧立面受光不对称）重拍 bat7/pt7，通用 8×8 tile 比值探针（target/pt_tiles.py，跨机位可复用）：

- **近/中景（r4-r7，雾起点 70m 以内）：0.88-1.04** —— 标定泛化通过 ✓
- 远景上排 0.5-0.85 暗格：根因=光栅雾（build.rs `fog_amount` 70m 起、630m 满、上限 0.92、FOG_TINT 与清屏天空逐分量同源）。距离相关性与近格全绿互证；PT 地面盒 half=400m 排除"远射线漏地面"共因。
- **裁决：PT 保持无雾**——烘焙参照=入射光照，大气属运行时效果；偏差表今后按距离分段解读（<70m 判标定，>70m 判雾）。
- 接管线预研：立面图案**只作用 marker 分支**（build.rs:630 注释），GLB 道具顶点色两侧天然同源——改造点收敛到 `pt_albedo_of` 的 material=1（建筑 marker 盒平 tint → 图案均值系数），PropBin 无需加型号字段（除非代理表证明图案分型）。

### 19.2 环境教训：GPU 门与用户 ML 负载互斥

§19 稳定性冒烟连续 NO-WINDOW，根因**不是代码**：用户并行的 QLoRA E2E 评测（run_dialog_e2e_complex.py）占住显存，游戏连深度图都分配失败（光栅单起同样挂），而同一二进制 17 分钟前全绿。教训入册：**GPU 门（冒烟/patrol/双拍）开跑前查 `nvidia-smi memory.used`，>3GiB 即挂起轮询**；用户训练/评测任务绝对不碰。

### 19.3 提交前人工走查 + PT-VIEW 断层注记

评审管线不支持 commit-range 目标（parse-args 会误分类为 file），改为人工走查两笔提交全部风险面，无 Critical：日光有效系数守恒（0.44→0.40）；`props_changed` 分支 `pt_refresh_dset` 错误路径理论泄漏但调用点不可达；空道具地图早退 ⇒ prop_key 变化 ⇒ 盒-only BLAS 无悬垂；`max_vertex` 用实际计数非容量；法线量化值域 0..254 解码对称、退化三角形回退 [0,1,0]。

**PT-VIEW 断层点名**：太阳归一 ×1.1→×0.5 使 VIEW 路径（sun 1.5/exposure 0.5）直照面有效系数 1.65→0.75（**变暗 ≈2.2×**）。用 VIEW 历史帧对暗部时注意；若成为日常需求再把 `PT_SUN_INTENSITY` 提至 3.0 补偿（当前不动，无消费者）。

### 19.4 PT 高光镜像 + 同日 A/B 定案（2026-09-20）

§19 走查遗留的 bldg L ×0.84 改判：大道立面是 **GLB 楼**（PT/raster 顶点色同源），差的不是反照率，是**光栅有 Blinn-Phong 太阳高光而 PT 纯漫反射**。给 PT 直照块镜像光栅公式（`alb·sun·sh·(ndl + 0.4·spec)`，spec=pow(N·H,32)，阴影均值共用，反照率外乘与光栅同语义）。

**同日 A/B（关键方法论）**：跨天对比出现全图 ×1.28 均匀漂移（连不受高光影响的天空常数都从 115→136）——用 de2316b 旧 spv 同日重拍（pt8old）证明漂移与着色器无关（机器/驱动日态），**分区表基线必须同日拍摄**。同机位同日三帧对比（vs bat8）：

| 区域 | 旧 spv | 新 spv（高光） |
|---|---|---|
| bldg L | ×1.05 | **×1.00** ✓ |
| bldg R | ×1.16 | ×1.11 |
| road | ×1.30 | ×1.24 |
| canopy | ×1.31 | ×1.26 |
| sky | ×0.84 | ×0.81（刻意色差） |

**路面/树冠 +24% 裁决：保留，不压补光**——街谷互反光是 PT 的真实 GI 信号，光栅半球环境项本就模拟不出来；为对齐光栅而抹平它等于废掉参照价值。§19 的 ≤15% 目标修订为：**直照主导面 ≤10%，GI 富集面（街谷/树冠下）允许 +25% 物理超额**。补光常数注释同步修正（t² 混合余弦均值实算 0.292H+0.708Z≈(0.285,0.309,0.354)，非先前声称的 0.30）。

**教训 42：渲染数值对照永远同日跑；跨天的 ×N.NN 先怀疑机器日态，再怀疑代码。**

<!-- 以下为本档**切档后新增**的日志，不属于切档原文；verify_v6 会在此处截断逐字比对。 -->

## §33 结案（部分）：PT 显示增益与 SPP/win 解耦 —— 代码与守卫已落地，视觉验收仍等显存窗口（2026-10-02）

**缺陷**（推导，非猜测）：`pt_panorama.glsl` 里 `lum` 是每帧 SPP 个样本的**求和**，却直接进时域 EMA；
显示端再除以饱和于 `win` 的 `acc.a` ⇒ 稳态显示增益 = **SPP/win**：

| 状态 | SPP | win | 稳态 outc |
|---|---|---|---|
| 静止 | 16 | 64 | 0.25·L·e |
| 行走 | 64 | 1 | 64·L·e |

同一个像素在"走"与"站"之间摆 **256 倍**。

**修复**（两处必须成对）：

1. 入栈前归一 `lum *= pc.e.w / float(SPP);`，显示端 `vec3 outc = acc.rgb;`（去掉 `/ max(acc.a, 1.0)`）
   ⇒ 稳态 outc = L·e，与 SPP、win 无关。
2. 标定值 `PT_EXPOSURE_DEFAULT` 0.4 → **0.1**（÷4）。tone 曲线 `1-exp(-x·1.55)` 在两侧同形可约去，
   静止端 `f(0.25·L·0.4) = f(0.1·L)` 与新实现 `f(L·0.1)` **逐字相等** ⇒ §19 静态参照帧标定原样保持。

**为什么归一放"入栈前"而不是"显示端除以当帧 SPP"**：后者在停下的瞬间 `acc.rgb` 里还混着旧帧的
64·L·e，会先亮约 4 倍、再花 ~1 秒衰减回去（亮度脉冲）；前者在运动期 alpha=1、`acc.rgb` 本就等于
当帧均值，mix 的是同一个量 ⇒ 切换无脉冲。这一条是**偏离原计划**的地方，理由在此，不当作笔误。

**顺带扫出的第二处缺陷**：`ray_tracer.rs::pack()` 里的 `else { 0.4 }` 是旧标定值的**手抄副本**，
改常量后它就成了全仓唯一还认为标定是 0.4 的地方（当前不可达：config 与 `RV3D_PT_EXPOSURE` 都夹在
`PT_EXPOSURE_MIN = 0.05` 之上）。已改为引用 `PT_EXPOSURE_DEFAULT`。
另核 `spp_target`：它被打包进 `pc.f.z` 而着色器从不读 `f.z`（只读 f.x/f.y/f.w）—— 但 CPU 侧
`renderer.rs:11563` 用它决定何时停止累积，所以是**打包冗余**，不是"存在但未接线"那类缺陷。

**新守卫** `pt_display_gain_is_motion_independent_and_paired_with_exposure`：真的去读
`assets/rt/pt_panorama.glsl` 源，锁三件事 —— 入栈前必须 `/ float(SPP)`、显示端不得再除 `acc.a`、
`PT_EXPOSURE_DEFAULT == 0.4 × (着色器当下的静止 SPP/win)`。SPP/win 端点是**从着色器解析**出来的，
不是抄进测试的数字（抄数字就是我这周拆掉的那两条假守卫的形状）。两侧都做过**红注入**：
改 `PT_EXPOSURE_DEFAULT` → 红在 `ray_tracer.rs:341`；把 `lum *= pc.e.w / float(SPP)` 改回
`lum *= pc.e.w` → 红在 `ray_tracer.rs:316`。

**已验证**：`cargo build --release` 绿；`cargo test --release` **656 passed / 0 failed**；
CJK 门禁绿（1590 用点）；`scripts/compile_pt.ps1` + `spirv-val` 通过；
`pt_panorama.spv` SHA256 `838557e1…445946` → `4a7db466…0672c5`（24,660 → 24,604 B）。

**未验证（阻塞在显存准入，未放宽门禁）**：smoke / patrol sweep / VVL 轮 / 视觉三方 A/B。
本机显存今天反复被外部负载占到 7.8 GB / 8.1 GB，`cap_safe.ps1` 的 3200 MiB 准入门正确拒绝；
实测可用窗口只有约 2 分钟（14:16 那次 4753 MiB 空闲，14:18 就回到 7608 MiB 占用）。
为此把 A/B 做成一键可跑：`python -P target/ptab.py`（旧 spv 已存 `target/pt_old.spv`，
同一 exe 只换 `.spv`，曝光用 `RV3D_PT_EXPOSURE` 覆盖，不需要重编）：
A = old + 0.4（真·改前基线）、B = new + 0.4（应显著变亮，证明"改一个必须改另一个"）、
C = new + 0.1（必须与 A 同亮度）。判据 `|mean(C)-mean(A)|/mean(A) < 3%` 且 `mean(B)/mean(A) > 1.30`。

**⚠️ 本节随代码在分支 `pt-gain-fix`，未进 master**：上面四项门禁跑完之前，不把未验收的视觉改动推上主线。
A/B 与冒烟通过 → fast-forward 到 master；不通过 → revert 分支，退回"只保留守卫与手抄值修正"的形态。

**踩到的工具坑**：`target/struct.py`（早前探针）**遮蔽标准库 `struct`** —— 从 `target/` 里跑脚本时
`sys.path[0]` 就是该目录，于是 `import numpy` 连带 `pickle → struct` 全部炸掉，而且 `struct.py`
会在导入时执行并打印无关内容。⇒ 跑 `target/` 下的脚本一律 `python -P`。

**教训：手抄的标定值就是下一个缺陷。** 一个常数出现在两个地方，改动的当天它就已经是错的了；
今天不是靠测试抓到的，是靠"改完再扫一遍还有谁写着 0.4"扫到的。

## §50 挂账之「PT 取景 fov_y 与光栅同源」：查完判定**接线正确**，但抓到一处静默回退（2026-10-02）

`ray_tracer.rs:114` 声明「`camera.fov` = `perspective_rh` 的 fov_y，与光栅同源」。逐点核对：

- 光栅：`camera.rs:416` `Mat4::perspective_rh(self.fov, aspect, …)` —— glam 的这个参数**就是 fov_y**；
- 实机 PT：`main.rs:3068` `tan_half_fov: (self.camera.fov * 0.5).tan()` ⇒ **同一个 fov_y，声明成立** ✅
- 参照帧冒烟路径：`main.rs:3257` 写死 `(60°…)`。**这不是缺陷** —— 那条是
  `RV3D_PT_VIEW=1` 的**四盒玩具场景**独立冒烟（相机固定在 (0,1.7,6.5)、输出 256×256
  `screenshots/pt_ref.bmp`），60° 与它自己的场景自洽，**不是** §19 分区表用的城市参照帧。
  ⇒ 差点把一条正确代码"修"成 bug（今天第 5 次"读上下文才没动手"）。

**真问题在第三处 60°**：`pack()` 里 `if tan_half_fov > 1e-4 { … } else { 60° }` 是**静默回退**。
取景参数一旦哪天被传进 0，PT 照常出图、只是**整体尺度错**，而画面看起来"没问题"。
这与本仓已确立的原则直接冲突 —— `RV3D_GPU` 那条的注释写着
「匹配不到就报错退出并列出可用设备（**不静默回退** —— 否则"在核显上验过"这句是假的）」。
三个调用点（实机 / 玩具路径 / 单测）都不传 0 ⇒ 这条兜底是**专门用来掩盖未来接线错误的死分支**。

⇒ 加 `debug_assert!` 让它在开发构建里响（保留回退以免发布版崩）。
**两侧都验过**：debug 下合法输入 `tan = 0.5` ⇒ 2 条测试照常绿；
把单测改成 `0.0` ⇒ 立刻 panic 在 `ray_tracer.rs:144`；已还原。
⚠️ 诚实标注局限：`debug_assert` 在 `--release` 下**不生效**，而本仓门禁是 `cargo test --release`
⇒ 它只是开发构建的绊线，**不是**门禁级守卫。要成门禁得让 `pack()` 返回 `Result` 或改调用方契约，
那属于动 PT 接线，等显存窗口能跑图时一并做。

**门禁**：`cargo test --release` 659 passed / 0 failed；`cargo test`（debug，`ray_tracer` 过滤）2 passed。

