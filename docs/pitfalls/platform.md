# 平台与驱动 —— 误区与陷阱（16 条）

> Windows / Linux / Wayland / 驱动 / 验证层的真实行为
>
> 返回 [`docs/pitfalls.md`](../pitfalls.md)（总索引与全局综述）。
> 每条格式：**症状 → 根因 → 教训 → 类别**。根因里保留了「当时以为是什么」——那是最值钱的部分。

## 平1. 驱动会往我们传进去的 create info 里写：关掉验证层照样回写，验证层照被改过的结构体报 VUID

- **症状**：整局 gameplay + 验证层跑出 VUID=22（VICTORY 5/5 波、panics=0），报的是 `VkImageViewCreateInfo-usage-02275` 与 `VkSwapchainCreateInfoKHR-imageFormat-01778`，都指向交换链 `image_usage` 里的 STORAGE，而源码只申请 `COLOR_ATTACHMENT | TRANSFER_SRC [| TRANSFER_DST]`。（a18011d, 2026-10-08）
- **根因**：埋点逐点打印出的地面真相是——我们算出来的 usage 没有 STORAGE，链式 setter/硬赋值之后结构体字段也没有，但 `create_swapchain()` 返回**之后**结构体字段变成 `… | STORAGE`；关掉验证层重跑同一探针**照样回写** ⇒ 是驱动往我们传进去的 const 结构体里写，验证层只是照着被改过的结构体报 VUID。**引擎的用法合法，这不是引擎 bug**（当时先被当成引擎用法问题排查）。
- **教训**：传给驱动的 create info 一律传副本（回写只污染副本），诊断只打印"我们申请的那份"（打印结构体字段会得到被驱动改过的值，日志就不准了）；"日志与自身输入矛盾"的 VUID 先别赌一个说不清的改动，按"层侧/驱动侧误报"记档并给自证证据（此处：代码从不设置 flags、`create_swapchain` 全仓只有一个调用点、5 次创建对 5 条报文）。
- **来源模块**：渲染管线

## 平2. 越界索引不报 VUID、只让整台设备消失；截断的 GLB 直接 panic

- **症状**：变异模糊（3000 条）第一次跑就红在两种形态上——把末尾某个索引改成 0xFFFFFFFF，`parse_glb` **照样返回 Ok**；截断文件触发 `range end index 872 out of range for slice of length 725`（**不是 Err，是 panic**）。
- **根因**：`read_acc` 把 5125 读成 f32 后直接 `as u32` 拼进 indices，从不校验范围——索引越界在 GPU 上是**顶点抓取越界**：不报 VUID、不 panic，只是整台设备消失（与 `gun_glb_indices_all_in_range` 同一条失效模式，但那条只覆盖入库的枪，而解析器面对的是任意资产：损坏文件、下载来的模型）。panic 那处是 JSON 块切片直接拿文件头里的 `json_len`，而同一函数的 BIN 块切片本来就有 `.min(bytes.len())`，**唯独 JSON 块漏了**。
- **教训**：解析不可信输入的硬不变式要逐个别校验（`0 <= idx < 本 primitive 顶点数`），并给模糊测试写"至少 50 条必须被接受"的自检，否则"没 panic"什么都证明不了；所有切片长度用 `checked_add().filter(<= len)`，别信文件头里自报的长度。
- **来源模块**：城市与几何

## 平3. 光标被钉成 1×1：一半是"自认为有焦点"，一半是 Confined+HIDDEN 的组合

- **症状**：非前台启动后光标被 ClipCursor 钉成 1×1、整台机器像死机（用户 2026-09-03 报的"鼠标死锁"）；另一路是窗口有焦点但视角纹丝不动，实机 `GetClipCursor` 量到 `853,533-854,534`（1×1），修复后为 `0,0-1707,1067`。
- **根因**：两个成因。① `sync_cursor` 的 `want` 以 `self.focused` 起头，而该字段初值写死 `true`，winit 只在收到 `WM_SETFOCUS` 时才投 `Focused(true)`，窗口被别的程序占着前台时两个事件都收不到 ⇒ 本进程一直"自认为有焦点"就去抓光标；② winit Windows 后端对 `Confined + HIDDEN` 会把指针钉在窗口中心 1×1（`window_state.rs::refresh_os_cursor`），而绝对位置路径的视角全靠 `CursorMoved` 增量算 ⇒ `dx` 恒 0，视角纹丝不动。
- **教训**：`focused` 这类"状态镜像"字段的初值必须取保守侧（没确认为真就不认）；**判据 = 直接量 `GetClipCursor` 矩形** —— 1×1 就是本仓鼠标失效的通用指纹，别去问框架的焦点标志；`Confined` 与 `HIDDEN` 不要同时设。
- **来源模块**：输入与交互

## 平4. 平台的指针协议不完整时"静默崩在原生层"——没有 panic 日志

- **症状**：WSLg（Wayland/Weston）下捕获后光标不隐藏、视角不动；右键拖动会在原生层直接崩，**没有任何 panic 日志**；同一构建在 Xvfb 的 x11 后端复现不出。
- **根因**：WSLg 的指针约束/相对指针协议实现不完整，winit 走 Wayland 后端时这些调用进不到有效实现。
- **教训**："日志里没有崩溃"不等于"没崩" —— 只用"有没有 panic"当崩溃判据会漏掉原生层退出，判据要加上"进程还在不在 + 退出码"（配套的硬要求：stderr 必须落盘到 `logs/*.log.err`，否则只能盲猜）；绕开办法是显式选后端（winit 0.30 已移除 `WINIT_UNIX_BACKEND`，改用 `with_x11()`），且只在用户没显式指定时才覆盖。
- **来源模块**：输入与交互

## 平5. 控制台子系统多出一个窗口抢前台 ⇒ 光标永不抓取；而 stderr 随控制台丢失 ⇒ 只能盲猜

- **症状**：用户反复报告"键盘能用、鼠标转不了视角"；排查时日志里什么都没有。
- **根因**：release 用 console 子系统会额外创建一个可见控制台窗口，与游戏窗口争前台，而 `sync_cursor` 要求**本进程是前台**才抓光标（链路上任何一环失败都只是"不抓"，不报错）；同时游戏 stderr 没落盘，随控制台一起消失，于是鼠标失效与卡死只能靠猜。
- **教训**：玩家输入链路依赖"谁是前台"这种外部状态时，在启动器里消除竞争者（release 改 windows 子系统、调试构建保留控制台，实测枚举确认 `ConsoleWindowClass` 消失）；**任何"只能盲猜"的排查都要先当缺陷修** —— 把 stderr 落到 `logs/play_latest.log.err`。
- **来源模块**：输入与交互

## 平6. 窗口尺寸写死了别人机器的 DPI 补偿 ⇒ 分辨率与日志一起说谎

- **症状**：Linux（scale=1.0）配置 2560x1600 实得 1706x1066，而日志照样打印"窗口创建成功: 2560x1600"；Wayland 下交换链尺寸一起错（`currentExtent` 未定义）；另一路是 Wayland 没有"主显示器"概念，默认分辨率取主显示器直接失败。
- **根因**：`LogicalSize::new(w / 1.5, h / 1.5)` 里的 1.5 是 Windows 那台机器 `scale_factor=1.5` 的硬编码补偿，而设置面板改分辨率走 `PhysicalSize` ⇒ 同一个"分辨率"有两套语义；`RESOLUTIONS` 与配置项本来就是物理像素。
- **教训**：逻辑像素↔物理像素的换算只允许出现在框架 API 边界，且同一个量只能有一种语义（这里一律 `PhysicalSize`）；夹取逻辑抽成纯函数并给四条断言（不缩放 / 超屏等比缩 / 拿不到显示器如实请求不猜 / 0 尺寸退下限，实测把 `/1.5` 加回去会红）；**日志必须打印最终请求值而不是意图值**，否则它自己就是下一个骗人的判据。
- **来源模块**：输入与交互

## 平7. waveOut 输出层的三条静默路径：丢样本不说话、WAVEHDR 地址被搬家、Close 失败后释放回调上下文

- **症状**："音频断续"用户听得出来、**日志里一个字都没有**；"退出/关声时偶发崩溃"依赖驱动时序，平时看不见。
- **根因**：① 丢样本有两条路，只有"单块装不下"（帧率 < 23fps）有告警，"**4 块全满**（队列 ≈170ms）"完全没有；② `waveOutPrepareHeader` 先在栈上临时量上做、随后 `buffers.push` 搬家 ⇒ prepare 的地址与 `waveOutWrite` 用的地址不是同一个（`WAVEHDR.reserved` 还是"应用不得改"的字段）；③ `Drop` 里 `waveOutClose` 的返回值被丢掉，失败（`WAVERR_STILLPLAYING`）⇒ 设备仍开着、回调线程随时进来解引用 `Arc::as_ptr` 给的裸指针并 lock 即将释放的 Mutex ⇒ use-after-free。
- **教训**：每条丢数据的路径都要先问"它丢东西时会说什么"（一次性纯闩 + 队列深度/丢弃样本数，且丢弃本身是有意的）；WinAPI 结构必须"**先收齐、容量一次给足**再逐个 prepare"；`Drop` 里每个返回码都要判，失败路径宁可**刻意泄漏**几十 KB 也不 close-then-free；无法单测的缺陷（要真声卡）要如实记账，并写明判据 = 代码顺序本身。
- **来源模块**：游戏逻辑

## 平8. ALSA 只试 default：一部分机器永远没声音，而且是静默降级

- **症状**：Linux 真机没声音，日志只有一行 `audio: ALSA 打开失败（snd_pcm_open(default) 失败 rc=-2），静默降级为无声`（`pcm_dmix.c:1000 unable to open slave`）。
- **根因**：本机（Arch + PipeWire）的系统 ALSA 配置只有 `50-pipewire.conf`、缺 `99-pipewire-default.conf`——只有后者会把 `!default` 指到 pipewire，于是 `default` 落到 dmix 并打不开 slave。实测 `aplay -D default` 失败、`aplay -D pipewire` 退出码 0 能出声；编译期、单测、null 插件**全都测不出来**。
- **教训**：不能要求每个用户的 ALSA 配置都完整——候选链 `default → pipewire → sysdefault`，并把**成功的那个名字打进日志**（`audio: ALSA PCM 设备 = pipewire`），否则下次再遇到"没声音"又要从头推一遍；诚实边界：听感未验证（一次"缓冲已满，本帧 324 个样本被丢弃"无法区分启动瞬态与持续丢帧）。
- **来源模块**：游戏逻辑

## 平9. `Instant::now() - 3600s` 下溢：同一份代码在开机几天的机器上一路绿

- **症状**：重启后跑全量测试，`net::tests::reset_connection_clears_session_scoped_state` panic 在 `std time.rs:445`「overflow when subtracting duration from instant」——**昨晚（机器已开机多日）同一份代码全绿**。
- **根因**：`reset_connection` 里 `self.last_join_at = Instant::now() - Duration::from_secs(3600);` —— `Instant` 的原点在 Windows 上是**开机时刻**，机器启动不足 1 小时时这次减法直接下溢 panic，属"上线首小时才炸"的缺陷；同文件另一处早就用了 `checked_sub`（注释还写着「进程启动不足 1 小时时无下溢」），**只有这一处漏了**。
- **教训**：凡 `now ± Duration` 一律 `checked_sub`/`checked_add` + 显式回退（这里退化成"现在"，只把立刻重试推迟一个 interval，无副作用），改完 `rg` 全仓扫一遍（判据 = 全仓只剩这一处 `Instant::now() - Duration`）；**"昨晚还绿"不是证据**——机器 uptime 是隐藏变量，时间基类缺陷要按"最坏环境"验，不是按"我这台机器当前状态"验。
- **来源模块**：联网与命令

## 平10. 独显 + 默认 IMMEDIATE 静默挂掉 GPU，harness 对着死进程空跑 900 秒

- **症状**：独显（RTX 5060）+ defense_line + 默认 IMMEDIATE，游戏打完 `game: run started (wave 1)` 就在**第一个 Playing 帧**死掉——日志里既没有 fps 行，也没有 panic / VUID / `has been lost`，harness 于是对着一个已死的进程空跑 900 秒（0 发 0 杀）。
- **根因**：呈现模式用错（引擎默认 IMMEDIATE 是为基准最稳设的，玩家路径 `SteelFront.bat` 本来就是 mailbox）从而触发 TDR；**证据在游戏日志之外**——Windows 应用程序日志同一时段有 4 条 LiveKernelEvent，**P1 = 141**（VIDEO_ENGINE_TIMEOUT_DETECTED）。
- **教训**：harness 的跑法必须与玩家路径一致（独显长跑用 mailbox，`perf_run.ps1` 保持 IMMEDIATE）；"日志里什么都没有"本身就是一种症状——去查系统事件日志（TDR 的 LiveKernelEvent P1=141），别只盯游戏日志；驱动器要有活性判据（日志/进程长时间无变化即中止），别把 900 秒空跑当成一次有效的失败实验结果。
- **来源模块**：工具与闸门

## 平11. release 用 console 子系统会多出一个窗口抢前台 ⇒ 光标永不抓取

- **症状**：用户反复报告"键盘能用、鼠标转不了"——引擎抓光标要求本进程是前台，而 console 子系统额外创建一个可见控制台窗口与游戏窗口争前台 ⇒ `sync_cursor` 的判据永不成立、光标永不抓取。
- **根因**：不是代码逻辑错，是链接子系统的副作用：`console` 子系统在建进程时多带一个控制台窗口（实测枚举窗口确认改成 `windows` 子系统后 `ConsoleWindowClass` 已消失）。
- **教训**：平台级"看不见的第二个窗口/焦点竞争者"要先用窗口枚举确认，别在 `sync_cursor` 逻辑里空推；release 用 `windows` 子系统、调试构建保留控制台（否则日志无处可看）。
- **来源模块**：工具与闸门

## 平12. Windows CPU 拓扑按固定 48 字节跨度解析，8 个物理核只认出 2 个，整局只有 2 个逻辑核在跑

- **症状**：scene_pool / ai_pool 各只建了 1 个 worker（应各 8）⇒ 整局只有 2 个逻辑核在跑；修复后 8+8 线程，冒烟 fps 106.9 → 128.7（+20.4%）。
- **根因**：GetLogicalProcessorInformationEx 是变长条目，而旧代码用固定 48 字节跨度遍历（该 InfoEx 结构本身没有 Size 字段）⇒ 条目一多就整体错位，8 个物理核只解出 2 个；同一文件第二处：walk 只保证条目 sz >= 8 且不越过缓冲末尾，调用方紧接着按 ProcessorRel / CacheRel（偏移 8）解引用，截断/畸形条目就是读越界 UB（属「没触发过的 UB」，不是正在出错）。
- **教训**：变长结构一律按各自的 Size 遍历，并且按 T 解引用前先判长度（entry_fits::<T>(sz) = sz >= 8 + size_of::<T>()，只有头部、差 1 字节都要拒绝，刚好够长才放行）；验收看解析出的物理核/线程数与真实拓扑对照，别只看有没有报错 —— 解析错位是完全静默的。
- **来源模块**：平台与杂项

## 平13. release 的 console 子系统多开一个控制台窗口抢前台，光标永不抓取（鼠标转不了视角）

- **症状**：用户反复报告「键盘能用、鼠标转不了视角」—— 光标永远抓不住。
- **根因**：release 用 console 子系统，会额外创建一个可见控制台窗口与游戏窗口争前台，而 sync_cursor 要求本进程是前台才抓光标 ⇒ 抓取条件永远不成立；实测枚举窗口类确认改成 windows 子系统后 ConsoleWindowClass 已消失。
- **教训**：Windows 发布构建用 windows 子系统（调试构建保留控制台）；输入/光标这类依赖「前台」的功能，验收要枚举真实窗口确认没有额外窗口抢前台，而不是只看自己的窗口能不能收到消息。
- **来源模块**：平台与杂项

## 平14. 窗口请求尺寸写死 DPI 1.5，非 Windows 分辨率整体失真，而日志照样打印「创建成功 2560x1600」

- **症状**：scale=1.0 的 Linux 上配置 2560x1600 实得 1706x1066；设置面板里改分辨率走的是 PhysicalSize ⇒ 同一个分辨率有两套语义；Wayland 下窗口尺寸直接决定交换链尺寸，错的是整条渲染尺寸链；而尺寸被夹小了，日志照样打印「窗口创建成功: 2560x1600」。
- **根因**：with_inner_size 用的是 LogicalSize::new(w / 1.5, h / 1.5)，那个 1.5 是 Windows 那台机器 scale_factor=1.5 的硬编码补偿，换个平台必然错；日志按请求值打印而不是实际值，于是日志与事实不符。
- **教训**：RESOLUTIONS 与配置项本来就是物理像素 ⇒ 一律 PhysicalSize、不留任何魔法缩放系数（抽成纯函数 window_physical_request(w, h, monitor)：不缩放、超屏等比缩、拿不到显示器如实请求不猜、0 尺寸退到下限）；判据 window_request_is_physical_and_clamped 用四条断言分别钉住一个会写错的方向，并实测把 /1.5 加回去它会红。
- **来源模块**：平台与杂项

## 平15. Wayland 的 currentExtent 设计上未定义，Windows 上永远走不到的兜底 1280x720 漂到 Linux 才暴露

- **症状**：Wayland 下交换链恒为 1280x720、不跟随窗口，Resized 重建走同一段代码 ⇒ 永远不跟随；后果是三重错位（交换链恒 720p、投影用窗口尺寸、HUD 排版也用窗口尺寸 ⇒ 画面比例错乱 + HUD 按窗口排版却画进 720p 视口）；同期 PT 上屏 blit 把目标范围写死 2560x1600，换个窗口尺寸就 VUID-vkCmdBlitImage-dstOffset-00248 并把设备打掉。
- **根因**：VkSurfaceCapabilitiesKHR::currentExtent 在 Wayland 下设计上就未定义（Mesa 填 {UINT32_MAX, UINT32_MAX}），而 Win32/X11 下恒等于窗口尺寸 ⇒ 那条兜底分支在 Windows 上永远走不到，兜底值一路漂到 Linux 才暴露；PT blit 那条同理：默认窗口恰好就是 2560x1600，于是此前每轮 PT 验证都躲过去了。
- **教训**：currentExtent 有定义时原样返回、未定义时才用 window.inner_size()（window_extent 由 new() 播种、Resized 更新），三条路都夹进 min/max；main.rs 的 Resized 必须先 set_window_extent 再 recreate_swapchain（顺序反了会静默重建出旧尺寸）；凡硬编码像素范围都要有「非默认配置」的判据（blit_regions_never_hardcode_pixel_extents：非原点 Offset3D 不许是纯字面量）。
- **来源模块**：平台与杂项

## 平16. 设备扩展「缺一个就整个游戏起不来」：可选被当必需，特性结构还无条件挂进 pNext

- **症状**：设备缺任一光追扩展时不是降级，而是 create_device 失败、游戏整个起不来 —— 而 PT 本来就是默认关的，根本不值得为它挡住启动；同一段 device-create 日志还是 warn 级每局都打，即使缺失为空，真正的缺失会被这条噪声淹掉。
- **根因**：VK_EXT_mesh_shader 可用时无条件请求 5 个光追扩展，enumerate_device_extension_properties 的结果只在事后用来打一行 warn；第二处更隐蔽：PhysicalDeviceRayQueryFeaturesKHR / AccelerationStructureFeaturesKHR / BufferDeviceAddressFeaturesKHR 三个特性结构无条件挂进 pNext，即使对应扩展没启用 —— 那本身就是无效用法。
- **教训**：required 逐个按支持情况启用、缺的如实报出但不阻止其它（不把「可选」当「必需」）；光追组全有或全无 —— 只启用一半时特性链与后续代码路径都假设齐全，半套是未定义行为，比整组不用更危险；判据 device_extensions_degrade_instead_of_failing 要能实测变红（把「部分启用」改回去必须失败）。
- **来源模块**：平台与杂项
