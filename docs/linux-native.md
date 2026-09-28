# Linux 原生开发与运行（Arch + KDE Wayland）

> 建立于 **2026-09-28**，分支 `feature/linux-port`。
>
> **本文只写仍然生效的结论。** 被推翻的直接删掉，不留「错误 + 更正」两段。
> `AGENTS.md` 里那条「开发/验证 = Windows 原生，WSL2 材料全部作废」**已改为双平台并存**：
> Windows 路径与本文描述的 Linux 路径**各自保留自己的坑**，互不替代。
> 2026-08-15 迁出 WSL2 的那条结论**仍然成立** —— 本文描述的是**原生 Linux**，不是 WSLg。

---

## 1. 本机（与 AGENTS.md 同一台机器，双系统）

| | Windows 侧 | Linux 侧 |
|---|---|---|
| 机型 | ASUS TUF FA608PM，BIOS FA608PM.309 | 同 |
| CPU | AMD Ryzen 9 8940HX（16C/32T） | 同（`amd-pstate-epp`，EPP=performance 时全核 4.27 GHz / 92 °C 满载实测） |
| 独显 | RTX 5060 Laptop（`10de:2d59`） | 同，`renderD128`，`nvidia-utils 615.71.09` |
| 核显 | AMD 610M / Raphael（`1002:164e`） | 同，`renderD129` |
| 面板 | 挂 610M（`card2-eDP-2 = connected`，`card1-eDP-1 = disconnected`） | **同构**，所以跨卡拷贝那条链**一模一样存在** |
| 合成器 | dwm | **KWin Wayland**（Plasma 6），`XDG_SESSION_TYPE=wayland`，XWayland 在 `:0` |

🔴 **混合输出不是 Windows 独有的问题。** 面板在核显上、游戏在独显上渲染 ⇒
每帧跨适配器拷贝 + 合成器在核显上合成。Windows 侧实测 Copy 23–26% / dwm 3D 22–44%；
Linux 侧机制相同（数字待测，别照抄 Windows 的）。

---

## 2. 环境搭建（一次性）

```bash
# 工具链：rustc 1.96.1（与 Windows 侧同版本，用 rustup 而不是 pacman 的 rust，
# 因为 pacman 的 rust 与 rustup 冲突，且版本会随滚动发布漂走）
rustup toolchain install 1.96.1 --profile default && rustup default 1.96.1

# 构建期依赖：.cargo/config.toml 已经写了 mold 作为链接器（x86_64-unknown-linux-gnu），
# 缺它会在链接期报错，而不是在配置期 —— 报错点离原因很远。
sudo pacman -S --needed mold

# 着色器工具（scripts/compile_pt.sh 用；缺 spirv-val 时它会 fail-closed exit 2）
sudo pacman -S --needed glslang spirv-tools

# 音视频/窗口：winit 的 X11/Wayland 后端来自它的默认 feature，
# Cargo.lock 里已有 wayland-client / x11-dl / x11rb ⇒ 当前无需额外操作。
# 但**不要**给 winit 加 default-features = false（见 §6 脆弱点）。
```

**首次构建**：`cargo build --release`（约 60 s，24 个依赖）。
`.cargo/config.toml` 的 `jobs = 32` 与本机 32 线程一致。

---

## 3. 提交闸门（Linux 上原本是**失效**的）

`.githooks/pre-commit` 与 `pre-push` 在 git index 里的模式曾是 **100644**，
而 **Linux 上 git 会静默跳过不可执行的钩子**（只在 stderr 给一句提示就照常提交）。
Windows 上这条从来不成立（Git for Windows 一律经 `sh` 执行），所以它漂到 Linux 没人发现
⇒ `tools/commit_guard.py` 那道白名单 + 密钥闸门等于没生效。

```bash
# 一次性安装（记录 base hooksPath / 设 core.hooksPath / 验权限位 / 自检扫描）
scripts/install_git_hooks.sh
# 还原
scripts/install_git_hooks.sh --uninstall
```

判据：本脚本用 `test -x` **真验可执行位**，不是只看文件在不在（否则装完还是假绿灯）。

> 铁律 G 那条「`.ps1` 的每一行都不许以非 ASCII 字节结尾」**在 Linux 上不适用**
> （它修的是 PS 5.1 按 ANSI/GBK 读无 BOM 脚本）；**但不要删** —— Windows 侧还在用。
> `.sh` **没有**这个约束（bash 按 UTF-8 读），判据 `powershell_scripts_never_end_a_line_with_a_non_ascii_byte`
> 按扩展名过滤，不会误伤 `.sh`。

---

## 4. 运行

```bash
./SteelFront.sh              # 构建 + 启动（玩家路径）
./SteelFront.sh fast         # 不构建，直接跑现有 exe
./SteelFront.sh diag         # 带 RV3D_AI_PROF=1 RV3D_PROP_STATS=1
```

`SteelFront.sh` 是 `SteelFront.bat` 的逐条复刻，三件事在 Linux 上**没有任何替代品**：

1. **`cd` 到仓库根** —— 引擎全部资产是 CWD 相对路径（`assets/mesh.spv`、`assets/props`、
   `assets/guns/*.glb`、`assets/maps/*.toml`），从别处启动直接「渲染器初始化失败」退出。
2. **`RV3D_PRESENT_MODE=mailbox`** —— 引擎默认 `IMMEDIATE`（压测口径）。
3. **`RV3D_BG_FPS=20`** —— 引擎默认 0 = 失焦后照样全速渲染，会把整个桌面拖住。

只在这三个变量**未设置**时补默认值，不覆盖用户的值。退出码 0=成功 / 1=失败 / **2=没跑成**。

⚠️ **`smoke` / `package` 两个模式在 Linux 侧尚未实现，会 exit 2**（不是 0）——
三态约定：不许把「没跑成」写成成功，也不许指向一个不存在的脚本。

---

## 5. 平台差异：**同一个现象，两种成因**（最容易误判的一节）

### 5.1 光标捕获 —— Linux 上 **X11/XWayland 才是可验证的那条路**

| | `set_cursor_grab(Locked)` | 绝对位置路径（`set_cursor_position`） | 结论 |
|---|---|---|---|
| **X11 / XWayland** | **恒 `Err(NotSupported)`** | 走 `XWarpPointer`，**真的生效** | ✅ 自洽：退到 `Confined` + 回中 + `CursorMoved` 增量 |
| **Wayland** | 返回 `Ok`，但**不保证生效** | 只在已 `Locked` 时才成功 | ⚠️ `Confined` 下指针撞到窗口边就再也转不动 |

Wayland 的 `Ok` 为什么不可信（两个独立原因，都在 winit 0.30 侧）：

1. `set_cursor_grab_inner` 的 `Locked` 分支走 `apply_on_pointer`，它**只遍历已经
   `wl_pointer::enter` 过的指针** —— 指针还没进窗口时它**什么都没做，也返回 `Ok`**。
2. winit **完全忽略合成器的确认事件**：`ZwpLockedPointerV1` 的 Dispatch **函数体是空的**
   ⇒ compositor 拒绝锁（或激活后又 `unlocked`）时，应用层**无法感知**。

而 Wayland 下 `DeviceEvent::MouseMotion` 的唯一来源是 `zwp_relative_pointer_v1`，
它**只在 `lock_pointer` 里一起创建** ⇒ **没锁 = 零 raw 事件 = 视角彻底不动**，
症状与「鼠标坏了」完全一样。

**⇒ 可用手段：**

```bash
RV3D_BACKEND=x11 ./SteelFront.sh       # 强制 XWayland（XInput2 raw motion 完整）
RV3D_BACKEND=wayland ./SteelFront.sh   # 不强制（默认；有 WAYLAND_DISPLAY 就是 Wayland）
```

`RV3D_BACKEND` 是 **2026-09-28 新增**的：此前「强制 X11」写死在 `is_wsl` 分支里
（判据是 `/proc/version` 含 `microsoft`），原生 Linux **没有任何手段**换后端 ——
winit 0.30 已删除 `WINIT_UNIX_BACKEND`（v0.29 changelog）。

**引擎会自己报警**：锁定后 1.5 s 观察窗内一个相对增量都没收到 ⇒ 先**补抓一次**
（指针此时多半已 enter），仍然为零则 error 级提示改用 `RV3D_BACKEND=x11`。
判据 = `lock_observation_needs_evidence_not_just_ok` + `cam:` 日志里的 `evt` 计数。

### 5.2 窗口 / 交换链尺寸 —— Wayland 的 `currentExtent` **是未定义的**

`VkSurfaceCapabilitiesKHR::currentExtent` 在 Wayland 下由 Mesa 填
`{UINT32_MAX, UINT32_MAX}`（`wsi_common_wayland.c::wsi_wl_surface_get_capabilities`），
而 Win32 / X11 下它**恒等于窗口尺寸** ⇒ 旧代码那条「否则用 1280x720」的兜底分支
**在 Windows 上永远走不到**，于是它在 Linux 上把整条渲染尺寸链钉死在 720p，且不报错。

现在：`currentExtent` 未定义时用**窗口物理尺寸**（`Renderer::window_extent`，
由 `new()` 播种、`Resized` 更新 —— 顺序必须是**先 `set_window_extent` 再
`recreate_swapchain`**，反了就是静默重建出旧尺寸）。
判据 = `swapchain_extent_follows_the_window_when_current_extent_is_undefined`，
真机看 `swapchain diag:` 那行。

同一类缺陷还有一处：`with_inner_size` 曾用 `LogicalSize::new(w / 1.5, h / 1.5)` ——
那个 `1.5` 是 Windows 那台 `scale_factor=1.5` 的硬编码补偿。现在一律 `PhysicalSize`，
判据 = `window_request_is_physical_and_clamped`。

### 5.3 中文渲染 —— 曾经在 Linux 上**全是 `?`**

`font_cjk` 早在 2026-09-14 就换成了**仓内预烘焙点阵**（纯二分查找，docstring 写明
「跨平台无依赖」），但 `ui.rs::glyph_cjk` 还留着 GDI 时代的平台分叉：非 Windows 无条件
`return None` ⇒ Linux 上每个汉字都渲染成 `?`，**不 panic、不报错、日志里一个字都没有**。
`ui.rs::is_cjk` 同样只覆盖 11 个区间里的 2 个 ⇒ 全角标点/假名被当成半角，
`text_width` 少算一半、居中与对齐整片偏移。

现已合并为单一判据。判据 = `cjk_glyphs_are_not_platform_gated`
+ `cjk_width_ranges_match_the_glyph_table`。

🔴 **改任何 `src/**/*.rs` 里的中文（含注释）之后，先跑这个再编译**：

```bash
python3 tools/cjk_cover_check.py     # 1 秒，列出缺哪个字、在哪个文件
```

闸门 `source_cjk_codepoints_all_have_glyphs` **重扫 `src/`，注释里的字也算**，
而**源字体未入库 ⇒ 字模表无法重建** ⇒ 唯一出路是**改写文案去用已有的字**。
（本次移植在 `ui.rs` / `main.rs` / `audio_out.rs` 上一共踩了 4 次，共 18 个字。）

### 5.4 音频 —— Linux 已接 ALSA（`src/audio_out.rs`）

Windows 走 `winmm` 的 waveOut 直接 FFI；Linux 走 **`dlopen("libasound.so.2")` + 7 个 dlsym**，
**刻意不硬链接 libasound**（没装 ALSA 的机器仍能编译运行，失败一律降级为静默并 warn 出错误码原文）。

🔴 **设备名要试三个，只试 `default` 会让一部分机器永远没声音**（2026-09-28 真机跑出来才知道）：
本机（Arch + PipeWire）`default` 会解析到 **dmix** 并报
`snd_pcm_dmix_open: unable to open slave` ⇒ `snd_pcm_open` 返回 `-ENOENT`。
根因在**系统侧**：缺 `99-pipewire-default.conf`（那一条才会把 `!default` 指到 pipewire），
只有 `50-pipewire.conf`。实测 `aplay -D default` 失败、`aplay -D pipewire` **退出码 0 能出声**。
⇒ 引擎按 `default` → `pipewire` → `sysdefault` 依次试，**并把成功的那个名字打进日志**
（看 `audio: ALSA PCM 设备 = ...`）。

**判据（真机实测）**：`audio: ALSA 打开成功 48000Hz/2ch（队列目标 170666µs ≈ 4×2048 帧）`
+ `VUID=0 panics=0`。⚠️ **尚未验证**：听感（本机没有能产生背压的设备可量部分写比例）。
启动时出现过一次「缓冲已满，本帧 324 个样本被丢弃」的一次性告警 —— 是否只是启动瞬态、
听感会不会断续**没有量到**，别当成已验证。若真断续，调 `queue_latency_us` 的队列长度即可。

---

## 6. 脆弱点（改一行就断，且报错点离原因很远）

- **`Cargo.toml` 的 winit feature**：`winit = { version = "0.30", features = ["rwh_06"] }`
  **没有** `default-features = false` —— X11/Wayland 两个后端都来自 winit 的**默认 feature**。
  谁加上 `default-features = false`，`main.rs` 的 `EventLoopBuilderExtX11` 与整个 Linux
  构建立刻断。**别加**，或者加了就把 feature 显式写全。
- **`main.rs` 顶部的 `#![cfg_attr(not(windows), allow(dead_code))]`**：它当初的理由之一是
  「CJK 字形表在非 Windows 明确回退成 `None`」—— 而那个回退**已证明是失效代码**
  （见 §5.3）。⇒ 这条 blanket `allow` 的正当性**已经被削弱**，它会把真正的
  dead code 一起藏住。**尚未清理**（清它要按编译器判据逐条过，别用文本匹配）。
- **`queue_present` 没有上界**：本仓「所有 Vulkan 等待必须有上界」只覆盖了 acquire 与
  fence。Wayland 下 FIFO 在没有 `wp_fifo_v1` 的合成器上**就是在 present 里阻塞等 frame
  callback**，而不可见的 surface 收不到 frame callback ⇒ 最小化/切 workspace 可能冻结主循环。
  **尚未修**。真机第一步：最小化 30 s 看进程是否还活着。
- **RT 设备扩展随 `VK_EXT_mesh_shader` 无条件启用**：缺任一（`buffer_device_address` /
  `deferred_host_operations` / `acceleration_structure` / `ray_query` /
  `ray_tracing_pipeline`）时 `create_device` **直接失败 = 游戏起不来**，
  而代码其实算出了缺失集合却只打 warn。`RV3D_GPU=igpu` 选 610M 时取决于 RADV 是否暴露 RT。
  **尚未修**。

---

## 7. 性能释放（Linux 没有奥创中心，这是**真问题**）

### 7.1 已经做的（安全、可持久）

```bash
sudo pacman -S --needed asusctl rog-control-center   # Arch 官方 extra 仓库，不用 AUR
sudo systemctl enable --now nvidia-powerd            # Dynamic Boost 守护进程
```

`asusd` 的默认配置本来就是对的，**之前只是没装、没运行**，于是档位无人应用、
机器一直停在固件默认：

```
platform_profile_on_ac:      Performance   ← 插电自动性能模式
platform_profile_linked_epp: true          ← EPP 跟随 profile
profile_performance_epp:     Performance
platform_profile_on_battery: Quiet         ← 电池模式自动降频
disable_nvidia_powerd_on_battery: true
```

`asusd` 由 udev 规则开机自启（`.service` 是 `static`，不是 `enabled`，别据此判断没装）。
`nvidia-powerd` 的 preset 是 **disabled** ⇒ 必须显式 enable，否则 Dynamic Boost 不工作。

**CPU 侧实测（32 线程满载 12 s）**：全核 **4.27 GHz**、Tctl 92 °C、单核可到 5.29 GHz
⇒ **CPU 本身是正常的**，"R9 变 R3" 的现象不在 CPU 频率上。

### 7.2 做不到的（诚实结论，别再去试）

- 🔴 **GPU 基础 TGP 在这台机器上改不了**：
  `nv_tgp` / `nv_dynamic_boost` / `nv_temp_target` / `ppt_pl1_spl` / `ppt_pl2_sppt` /
  `ppt_pl3_fppt` **全部 `unavailable`**（`asusctl armoury list`）。
  内核启动时那句 `asus_armoury: No matching power limits found for this system` 就是这个意思 ——
  **FA608PM 的固件不暴露这些 WMI 功能**，不是驱动版本问题。
  `nvidia-smi -pl 100` 也明确报 `Changing power management limit is not supported for this GPU`。
  ⇒ **55 W 基础墙（`nv_base_tgp`）在 Linux 上抬不动**；能拿到的上限来自
  `nvidia-powerd` 的 Dynamic Boost（`max_limit = 115 W`）。
- Windows 侧 AGENTS.md 记的「解锁 111.92 W」是**奥创中心手动模式**的产物，Linux 无对应物。

### 7.3 还没做的：MUX 切独显直连（**收益最大，但有风险**）

```bash
asusctl armoury get gpu_mux_mode        # 1 = 混合（当前）/ 0 = 独显直连
asusctl armoury set gpu_mux_mode 0      # 需要重启；本机 gpu_mux_mode 是可用的
```

切到 0 后面板直连 5060，**跨卡拷贝与核显合成整条消失**（§1 那个瓶颈）。

🔴 **风险与前提**（截至 2026-09-28 用户选择**先不切**）：

- 这是**固件(EC)级**设置，写进去就持久，**只看得到屏幕就没法用命令改回来**
  （需要 SSH 或 U 盘救援）。
- 兜底条件**已确认具备**：`nvidia_drm.modeset=Y`、`nvidia_drm.fbdev=Y`
  （⇒ 有 `/dev/fb0` + `fbcon`，即使 KWin 起不来也有文本控制台）。
- **切之前建议先启用 SSH**，让黑屏时还有一条路改回 `gpu_mux_mode 1`。

**判据（切之前/之后各测一次，别只测一边）：**
`RV3D_GPU=dgpu` 跑同一张图同一时长，比 `perf_run` 的稳态 fps。
⚠️ 性能数字的纪律见 AGENTS.md 教训 43/45：**先量 A/A 底噪，n ≥ 5 对，报中位差 + 符号一致数**，
小于底噪就写「没测到」。

---

## 8. Linux 侧尚未实现 / 尚未验证（诚实清单）

| 项 | 状态 |
|---|---|
| **Linux 冒烟门** | ✅ `scripts/smoke_linux.sh`（`./SteelFront.sh smoke`，见 §10） |
| `scripts/perf_run.sh` | **未实现**（`perf_run.ps1` 只需换进程管理与路径，是最容易移植的一个） |
| 真机跑通一局（起窗 / 分辨率 / CJK / VUID / 音频） | ✅ 见 §5 各条的实测数字 |
| MUX 独显直连 | 用户选择暂不切（§7.3） |
| `package`（打包） | **未实现**（`SteelFront.sh package` → exit 2） |
| `launcher/`（Win32 原生 GUI 启动器） | **不在移植范围**（`#![cfg(windows)]`，整 crate） |
| `queue_present` 上界 / RT 扩展过滤 / blanket `allow(dead_code)` | 见 §6 |
| Wayland 下 `IMMEDIATE` 支持面 | 实测 NVIDIA Wayland **支持**（`present_mode: IMMEDIATE`）；`SteelFront.sh` 仍用 mailbox |

---

## 10. Linux 冒烟门（`./SteelFront.sh smoke`）

```bash
./SteelFront.sh smoke                 # 默认档：确定性闸门
./SteelFront.sh smoke -Secs 90        # 跑久一点
scripts/smoke_linux.sh -RequireKill   # 严格档：额外要求 killed>=1（见下）
python3 scripts/smoke_linux.py --self-check    # 闸门自检（14 个用例，含 4 个 exit 2 分支）
```

**与 Windows 侧的关系**：判据口径**同源**（`vuid==0 and panics==0`），但**驱动方式完全不同**，
这是平台决定的，不是偷懒：

- **Windows** 靠 `PostMessage` 把按键投进目标窗口队列（因为抢前台会让 winit 不发 `Focused`，
  光标永不抓取 —— 用户 2026-09-03 报的「鼠标死锁」就是这一类）。
- **Linux 没有 PostMessage**，而 XTEST 是全局注入、**会抢焦点并把指针锁进游戏窗口**，
  正好违反那条协议的本意 ⇒ 改为**零输入驱动**：`RV3D_AUTOSTART=1` +
  `RV3D_DIAG_NPC_FRONT=1`（把 npcs[0] 摆到相机正前方 20m）+ `RV3D_AUTOFIRE=1`，
  一根手指都不用碰，并强制 **`RV3D_NO_CAPTURE=1`** 让「不夺指针」成为**代码级保证**。

### 🔴 两档判据，因为"击杀"那一档在本机是**不确定的**

| 档 | 判据 | 稳定性（实测） |
|---|---|---|
| 默认 | `vuid==0 && panics==0` **且真的进过 Playing** | **8/8** —— 确定性 |
| `-RequireKill` | 再加 `killed>=1` | **7/8 ≈ 88%** —— **不确定** |

失败的那次是 `score 增量 0`：**零输入路径没有闭环瞄准**，`RV3D_AUTOFIRE` 每帧开火会累积
后坐力与散布，`RV3D_DIAG_NPC_FRONT` 只把目标摆在正前方、**不保证命中** ⇒
它靠的是"在 255 个目标里蒙中几个"。实测击杀数分布：45s → 3/2/2/2/1；60s → **0**/3。

⇒ **默认档不要求击杀**。理由（AGENTS.md 教训 26）：**会喊狼来了的闸门会训练人不再当回事**，
那种假警报与漏报一样有害。`-RequireKill` 仍然提供（与 Windows 同口径），
但调用方必须知道那个 88% 的数字。

⚠️ 顺带记下一个被推翻的旧结论：Windows 侧脚本设 `RV3D_STRESS_AI=0`（波次模式）并注明
「压力模式下玩家无敌、killed 不可达」。**在 Linux 零输入路径上这是反的**：
波次模式只有 6 个敌人且玩家会在 ~27s 被打死（`game: player down ... (GameOver: gameplay frozen)`），
实测 **0/2**；压力模式（255 敌人 + 玩家无敌）才是 **3/3**。所以本脚本默认
`RV3D_STRESS_AI=1` —— **不要照抄 Windows 那个值**。

### 三态（教训 46）

| 退出码 | 含义 |
|---|---|
| 0 | 通过 |
| 1 | 失败（有 VUID / 有 panic / 开了 `-RequireKill` 时 0 杀） |
| 2 | **没跑成** —— exe 不存在 / 日志空 / **从没进过 Playing** / 只有 1 个 score 采样点 |

最后三条是**硬条件**：`vuid==0` 在一份**空日志**上同样是 0，不挡住就又是一个恒真判据
（Windows 侧 `survive_pm` 就这么骗过一整轮）。

### 尚未具备的能力（诚实记账）

- **没有闭环瞄准** ⇒ 打不出稳定击杀。要补的话，参照 `gameplay_smoke_pm.py` 的 `aim`：
  从日志里读出活着的敌人坐标 → 算需要的 yaw/pitch 增量 → 转视角 → 再开火。
- **没有画面取证**（截图/差分）。Windows 的 `cap_safe.ps1` 靠 `PrintWindow` 不前置窗口；
  Linux 可用引擎自带 F12（非 Windows 落 `/tmp`）或 X11 `XGetImage`，都还没做。

---

## 9. 与 Windows 侧的分工（不要互相照抄）

| | Windows | Linux |
|---|---|---|
| 输入注入 | `PostMessage` + VK 码（**不抢前台**） | XTEST 会抢焦点；引擎侧用 `RV3D_NO_CAPTURE=1` 保证不抓光标 |
| 截图 | `PrintWindow`（不前置窗口） | 引擎自带 F12（非 Windows 写 `/tmp`） |
| 强制 X11 | 不适用 | `RV3D_BACKEND=x11` |
| 玩家入口 | `SteelFront.bat`（`start /b` 异步） | `SteelFront.sh`（前台，回传退出码） |
| 性能旋钮 | 奥创中心 | `asusctl` + `nvidia-powerd` |
