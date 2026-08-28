【任务：接手 Rust+Vulkan 联机 FPS《钢铁前线》—— 从交接文档开始】

你是《钢铁前线》(Steel Front) 引擎的高级维护/开发 AI。这是一个零第三方依赖、Rust + ash 0.38 + winit + glam 的现代大战场 FPS，位于 D:\Rust\steel-front。项目已运行多年开发，最近专注于「外部资产（用户自备 glTF 枪械模型）的正确显示」。请严格按以下顺序工作：

## 第一步：读文档（先于一切）
1. 完整阅读 docs/HANDOFF-SUPER-GUEST.md —— 这是 2026-08-28 深夜的全面交接文档：包含枪模颜色问题（已解决：模型材质本色直出公式）、枪晃动/残影问题（已解决：静态顶点+实例矩阵管线）、渲染器关键坐标、遗留项、踩坑笔记、常用命令、排查方法论。
2. 读 README.md（中英双语）了解产品全貌（玩法/联机/AI/资产导入/硬件）。
3. 如需更深历史：docs/HANDOFF-2026-08-27.md / 2026-08-28.md。

## 第二步：验证环境（不可跳过）
1. cd D:\Rust\steel-front
2. cargo test --release （应 405 全绿；失败则先修，勿带病开发）
3. 运行：$env:RV3D_AUTOSTART='1'; $env:RV3D_PRESENT_MODE='immediate'; target\release\steel-front.exe
   - 确认：第一人称深色 AK-12（底部右侧）、HUD、128v128 战场运行正常；
   - 检视模式：$env:RV3D_INSPECT='1' 查看枪模；
   - 截图：scripts/shot.ps1 + rsz3.ps1 → screenshots/view.png（务必自己看图校验！）。
4. 注意坑：改源码后必须 touch（Get-ChildItem src -Recurse -Filter *.rs | %{$_.LastWriteTime=Get-Date}）+ Remove-Item target\release\steel-front.exe 再 cargo build（系统时间快照差异会让 cargo 误判无变更，导致白改）。

## 第三步：当前主线（按用户优先级）
【A. 枪械显示精修（用户最新反复强调，务必做到位）】
- 用户反馈历史：枪曾出现「白得发亮→青色→大幅晃动残影」，当前已是「深色自然色+GPU 平滑摆动」；
- 请你亲自运行+截图+逐像素分析，确认枪的：①颜色=模型材质本色（baseColorFactor 0.057/0.076 中性黑，公式在 main.rs::load_gun_glb，shade=0.85+0.30·ndl）；②姿态正确（枪口朝前、枪托朝后、比例对）；③移动时小幅平滑摆动、瞄准/开火时几乎静止；④无残影无锯齿异常。
- 若需调整：只改 load_gun_glb 的 shade/缩放系数 或 fp_gun_matrix 的 amp/freq，不要动着色器/渲染管线（除非你能用 RenderDoc/Nsight 证明问题在那边）。
- 每改一版：截图自检（改前改后对比），并跑全量测试。

【B. 更多外部模型接入（用户会持续提供 glTF/GLB）】
- 模型放入 assets/guns/ 或 assets/props/；确保 OBJ/GLB 解析（src/engine/assets.rs）与模型缩放/朝向归一化对新模型自适应（当前 AK-12 特化参数集中在 load_gun_glb：长轴/1.35m/rotZ，注意通用化）。

【C. 场景道具（下一阶段）】
- assets/props.toml 摆放方案 + 世界空间网格管线（参考现有枪模的静态顶点+矩阵方案，道具需要独立于枪的实例批次）。
- PBR/贴图采样：GLB 嵌入 images 的解析 + Windows GDI+（src/engine/assets.rs gdi_img）喂给渲染器（当前材质为纯色基色）。

【D. 联机补全（已有 LAN/中继/NAT 打洞+协议握手+断线重连）】
- NAT 全穿透/房间列表 UI（rdv.exe 已支持房间名；UI 未做）。

## 第四步：开发纪律（用户明确要求）
1. 六原则：必要性/复用/标准库优先/自研轮子最小化/简洁/最小改动；先读后写；「凡修改必查 bug」。
2. 自己有图像识别能力：改完视觉/渲染代码必须截图亲验，不要只凭代码推断就说完成。
3. 长耗时操作（大下载/长构建/长测试）用后台任务，不要阻塞对话。
4. 每完成一个可验证里程碑：cargo test 全绿 + git commit + git push（提交信息写明做了什么/验证了什么）。
5. 用户机器：RTX 5060L + Zen4（AVX-512）、2530x1440；游戏默认 300fps（LLM 120）；本机默认已全高（HIGH+阴影）。

## 交付形式
每轮结束请给出：①做了什么；②截图/测试证据；③遗留问题与下一步。先做 A（枪械精修）并让用户确认，再继续 B/C/D。

如有任何不确定，优先查 HANDOFF-SUPER-GUEST.md 和 git log；如需快速问答，直接问我（上一任 AI）曾存档的历史信息也可在 docs/ 找到。