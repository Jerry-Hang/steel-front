# 交接文档（超级外援版）— 2026-08-28 深夜

> 本文档供接手 AI（Kimi K3 / Claude 5 级）直接阅读：包含项目当前状态、
> 枪模颜色/晃动两大问题的完整技术真相、关键代码位置、排查方法论、遗留事项。
> 前提：先跑 cargo test --release（404 全绿基线），再按本文逐项核验。

## 0. 项目速览
- 零依赖 Rust+Vulkan FPS：ash 0.38 / winit / glam；src/engine/ 模块化；build.rs 生成 OUT_DIR/shaders.rs（内嵌 SPIR-V）；assets/ 下有工具模型。
- 目标平台：Windows x86_64；RTX 5060L + Zen4（AVX-512 可用）；默认 300fps 上限（LLM 120）。
- 玩家应知：枪械模型 = 用户自备 glTF（Sketchfab AK-12），用户会持续提供新模型；引擎需要正确显示外部资产。

## 1. 当前 Git 状态（HEAD = d909348）
- d909348：枪晃动残影根治——静态顶点管线（详见三）
- f078ddf：枪模颜色修复完成——模型材质本色直出（详见二）
- 7e6b54d：内嵌 SPIR-V 切换 + 色调调查；其余（AI 战斗/联机/NAT 等）见 README

## 2. 枪模颜色问题（已解决，f078ddf）
最终公式：显示颜色 = 模型材质 baseColorFactor x (0.85 + 0.30·dot(法线, 光)) —— 乘 1.0，无提亮/压暗。
- 路径：assets/guns/ak12.glb（原始 Sketchfab：两材质 baseColorFactor = 0.057/0.076 中性黑）
- 代码：main.rs::load_gun_glb（parse → 归一化：bbox 中心/1.35m 长轴/rotZ 对齐 → 材质色×忠实现光）
- 验证：第一人称 + 检视模式均显示标准暗色 AK-12（用户已认可此为目标）

### 历史坑（防复发）
1. 外置 SPV 陈旧：renderer.rs init_pipeline 曾从 assets/triangle.vert.spv 加载——该文件并非 build.rs 生成（build.rs 只写 OUT_DIR/shaders.rs！），长期不同步（着色器里 color 通道失效被 UV 顶替）→ 已改为 crate::shaders::VS_SPIRV/FS_SPIRV（7e6b54d 模块）。**此为所有「数据对但渲染错」类 bug 的头号嫌疑**。
2. 提亮系数失控：历史曾用 raw×5.5×光照（0.057 拉成亮灰）；烘焙版 Blender vertex_color_dirt 基色曾写 (0.12,0.13,0.15) 产生青色。最终 = 原始 GLB 材质本色直出。
3. scripts/blender_bake.py：Blender 无头烘焙（AO 顶点色）——当前未用（用原始模型）；后续需细节可复用（基色已设 0.055 中性）。

### 验证工具（可复用）
- 顶点色链：renderer.rs 的 gun 写入首色 / gun-POSTWRITE 日志（验证 CPU-RAM 值）
- 着色器真值：vs 临时 output.color 编码实验（红注入 / step 三档 / 常量灰）

## 3. 枪晃动/残影（已解决，d909348）
根因：每帧 CPU 把枪 63283 顶点全部重新变换（first_person_gun_mesh 每帧 map fp_gun_matrix → 3MB 计算）——走路时帧时间被挤压 + 顶点与移动帧错位 → 残影/重影 + 高频抖动观感。

修复架构（当前）：
加载时：顶点一次性静态化（load_gun_glb 归一化后不再动）；
每帧：main.rs 预计算 fp_gun_pre = self.fp_gun_matrix()（必须在 if let Some(renderer) = &mut self.renderer 之前！）
     → renderer.set_first_person_gun_model(fp_gun_pre)（写入实例槽 75841 的 model 矩阵：view_inv × anchor(bob/后坐) × scale × rot）
     → 顶点缓冲不变，GPU 用实例矩阵变换 —— 零 CPU 顶点计算。
- 新 API：renderer.rs::set_first_person_gun_model(&mut self, m: glam::Mat4)（枪槽唯一写者；InstanceData{model:[f32;16],tint}，直接 copy model 16 floats）。
- bob 语义（fp_gun_matrix）：移动（Game::player_speed() 新访问器 > 0.6）才摆；双相 x(7.5Hz,0.009)+y(0.8 相位,0.9x)；开镜阻尼约 1.5%；开火后 0.25s 阻尼 15%；后坐脉冲 y/z（0.3s 二次衰减）。
- 注意：fp_gun_pre 若位置/朝向异常 → 检查其计算时机与 load_gun_glb 归一化一致性。

## 4. 渲染器关键坐标（排查必读）
- 顶点结构：renderer.rs Vertex{pos@0, color@12, uv@24}（32B）；attrs loc0/1/2 = pos/color/uv（offset_of!）
- 主管线：init_pipeline（vs/fs 内嵌 shaders 模块）；自发光槽位区间修复（>= EMISSIVE_BASE && < +64；枪槽 75841 = EMISSIVE_BASE+64 一槽，勿当自发光）
- 实例槽：GUN_INSTANCE_INDEX = 65536+1+1024+3072x3+64 = 75841；EMISSIVE_SLOT_BASE = 75777；NPC 三区；marker 区 65537..

## 5. 已知遗留/开放项（如实）
1. 枪模颜色：这版自然色=目标；剩余微调只改 load_gun_glb 的 shade = 0.85 + 0.30·ndl 系数，不动着色器。
2. bob 手感微调：幅度 0.009 / 频率 7.5Hz（用户若觉得重/轻，改 fp_gun_matrix 的 amp/freq）。
3. 下一阶段主线：场景道具导入（assets/props.toml + 世界空间网格管线）、PBR/贴图采样（GLB 嵌入 images）、更多用户模型即插即用。
4. 联机：LAN/中继已完成（README）；NAT 全穿透/房间列表 UI 待续。

## 6. 环境与常用命令
- 构建/测试：cargo build --release ; cargo test --release
- 运行（本机满配默认）：$env:RV3D_AUTOSTART='1'; $env:RV3D_PRESENT_MODE='immediate'; target\release\steel-front.exe
- 检视/压力/联机：RV3D_INSPECT=1 / RV3D_STRESS_AI=1 / RV3D_NET=server|client + RV3D_NET_ADDR
- 截图：scripts/shot.ps1 + rsz3.ps1 → screenshots/view.png
- 构建静默失败陷阱：改源码后务必 touch（Get-ChildItem src -Recurse -Filter *.rs | %{$_.LastWriteTime=Get-Date}）+ Remove-Item exe 再 build（系统时间快照差异会致 cargo 认为无变更）
- Blender：D:\3D_Work\blender\blender-5.2.1-windows-x64\blender.exe（无头 -b --python scripts/blender_bake.py）
- 性能：RV3D_FPS=9999 无上限；nvidia-smi 采样；128v128 实测 311fps@99%GPU

## 7. 给 Kimi K3 的第一建议
1. 读 README.md（双语）了解产品全貌；
2. 跑基线测试 + 一次游戏（看枪）确认物理状态；
3. 用户核心痛点排序（最新反馈优先）：枪模显示效果精修 → 更多外部模型接入 → 道具/场景 → 联机补全；
4. 如需更多细节，问「枪模颜色/晃动历史」（本文即索引）。