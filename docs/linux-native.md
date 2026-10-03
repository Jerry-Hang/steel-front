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

`smoke` 与 `package` 两个模式在 Linux 侧**都已实现**（分别转交 `scripts/smoke_linux.sh`
与 `scripts/package_release.sh`，退出码原样带回）。三态约定贯穿到底：
不许把「没跑成」写成成功，也不许指向一个不存在的脚本。

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
+ `VUID=0 panics=0`。

#### ✅ 听感已从「未验证」结案（2026-10-03，客观测量）

此前一直挂着「听感没量到」。现在用**录 sink monitor + 逐 50ms RMS** 量了，
并且**引擎计数与录音两条独立证据互相印证**：

| 证据 | 结果 |
|---|---|
| **稳态段静音窗口**（t=4.5–30s，50ms 窗） | **0/534** ⇒ 没有任何 ≥0.2s 的连续静音 |
| 中位电平 | **−30.4 dBFS**（有声、不削顶） |
| 引擎「缓冲已满」告警 | 整轮 30s **只出现 1 次**，在 ALSA 打开后 1 秒 |
| ALSA `xrun/underrun/EPIPE/ESTRPIPE/recover` | **0 次** |
| `audio_us`（混音耗时） | 中位 **30µs** / 最大 86µs |

⇒ **开局那次丢弃是「队列从空到满」的一次性瞬态，不是持续欠载。**
分段看得很清楚：`t=0–2.0`（游戏未启动）40/40 静音且是 −180 dBFS 的**真数字静音**；
`t=2.0–4.5`（引擎启动）12/50 静音；`t=4.5` 之后**一个静音窗口都没有**。

复现方法（`parec` 录 sink monitor，30s 后逐窗算 RMS；注意**先录再启动游戏**，
否则会把引擎启动窗口误判成丢帧 —— 我第一次就差点这么读）：

```bash
SINK=$(pactl list short sinks | awk '{print $2}' | head -1)
parec -d "$SINK.monitor" --file-format=wav /tmp/audio_cap.wav &
sleep 2 && RV3D_AUTOSTART=1 RV3D_AUTOFIRE=1 ./target/release/steel-front &
sleep 30; pkill -x steel-front
```

若真出现断续，调 `queue_latency_us` 的队列长度即可（现为 4×2048 帧 ≈ 170ms）。

---

## 6. 脆弱点（改一行就断，且报错点离原因很远）

- **`Cargo.toml` 的 winit feature**：`winit = { version = "0.30", features = ["rwh_06"] }`
  **没有** `default-features = false` —— X11/Wayland 两个后端都来自 winit 的**默认 feature**。
  谁加上 `default-features = false`，`main.rs` 的 `EventLoopBuilderExtX11` 与整个 Linux
  构建立刻断。**别加**，或者加了就把 feature 显式写全。
- ✅ **`main.rs` 顶部的 blanket `allow(dead_code)` 已删除**（2026-10-03）。
  它 2026-09-26 的理由是「非 Windows **只是只编译不运行**的交叉验证目标」——
  **那个前提现在不成立了**：Linux 已是原生支持、而且要真的跑起来的平台
  （就是这份文档），于是它会在 Linux 上**藏住真正的死代码**。

  拆掉后实测浮出 **4 条，全在 `audio_out.rs`**（一次 `cargo build --release` 就够，
  判据只看编译器、不做文本匹配）：
  | 死代码 | 为什么没被接线 | 修法 |
  |---|---|---|
  | `WaveOutSink` 的 3 个字段/`new` | 它只在 Windows 上被构造（`DefaultSink` 在 Linux 是 `AlsaSink`）；原来**只有字段**带 cfg，结构体与 impl 在 Linux 上也编 ⇒ 整条链没人用 | 整个 `WaveOutSink` 及其 impl 加 `#[cfg(target_os = "windows")]` |
  | `submit_plan` / `warn_submit_truncation_once` | 只被 `WaveOutSink::submit` 调用（"单块装不下"那个场景）；Linux 侧的对应场景由 `queue_full`/`first_starved_drop` 覆盖 | 纯函数用 `#[cfg(any(target_os = "windows", test))]` —— 生产只有 Windows 用得到，但纯函数判据不该丢掉 Linux 覆盖 |

  aarch64 上还会多出 **2 条**（x86_64 上看不见）：`cpu::forced_simd_path` 与
  `simd::warn_forced_simd_unsupported` —— 三个生产调用点**全在
  `#[cfg(target_arch = "x86_64")]` 里**（aarch64 走 NEON，没有"强制选路"这回事）。
  同样按平台门控，`warn_*` 那条保留 `test` 分支（判据
  `forced_simd_warning_is_latched_to_once` 直接调它）。

  ⇒ 现在**三个目标各自都是真的 0 警告**（原生 Linux `build --release` /
  msvc 交叉 / aarch64 交叉），不再是"靠 allow 压出来的 0"。
  ⚠️ 改 `cpu.rs` 只加了那一行 `cfg`，**没有碰任何 CPU 亲和逻辑**（该文件标着只读，
  红线针对的是亲和/拓扑）。
- ✅ **`queue_present` 已补上耗时判据与降级**（2026-10-03，commit `7029677`）。
  `vkQueuePresentKHR` 的签名里**没有超时参数**，加不了真正的上界 ⇒ 判据只能退化到耗时：
  单次 ≥ `PRESENT_STALL_US`(1s) 记一次，连续 `PRESENT_STALL_FALLBACK`(3) 次就按与 acquire
  **完全相同**的方式降级为 MAILBOX 并重建交换链。1s 有实测支撑：本机 `present_us`
  中位 81µs / 最大 160µs ⇒ **6000 倍余量**，不会把「某帧慢了一下」误判成卡死。
  判据 `present_stall_classifies_and_clears`（含"正常帧必须清零"—— 漏了它，几次偶发
  长卡顿会累积成"连续三次"从而误降级）。
  ✅ **已真机验证：这个冻结场景在本机不复现**（2026-10-03）。
  做法 —— `RV3D_PRESENT_MODE=fifo` 启动后用 **KWin 脚本**把窗口最小化
  （`qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript <js>` +
  `...start`，脚本里 `w.minimized = true`），再逐秒观察 20s：

  | 观测 | 结果 |
  |---|---|
  | 最小化是否真的执行 | ✅ `kwin_wayland: minimized: Steel Front - Vulkan` |
  | 进程 | **20 秒全程存活**（CPU 从 32% 缓降到 16%） |
  | `present_us` 最大值 | **172µs**（阈值 1s，差 5800 倍） |
  | 卡顿检测触发次数 | **0** |

  ⚠️ **这个负结果只有验过"最小化真的生效"才算数** —— 否则就是一次什么都没做的空跑，
  而空跑同样会报"进程存活"。所以上面同时留了两条证据：KWin 自己的 print，
  以及一个**独立查询脚本**（`KWINQUERY ... minimized=`）确认脚本 API 确实匹配到了
  `cls=steel-front` 那个窗口。（教训 27：先确认你的测量工具测的是你以为的东西。）

  ⇒ 所以 `present_stall` 那套在本机**是纯防御性的**：场景不复现，但别的合成器/驱动组合上
  仍可能出现（Wayland 下 FIFO 等 frame callback 是规范允许的行为），保留它成本极低。
- ✅ **RT 设备扩展已改为按真实能力筛选**（2026-10-03，commit `d7b2444`）。
  原来在 `VK_EXT_mesh_shader` 可用时**无条件**请求 5 个光追扩展，缺任一就是
  `create_device` 失败 = **游戏起不来**（而 PT 是**默认关**的，根本不值得为它挡住启动）。
  现在光追组**全有或全无**：缺一个就整组不启用 —— 只启用一半时，特性链与后续代码路径
  都假设它们齐全，**半套是未定义行为，比整组不用更危险**。
  同一段还有第二处一并修了：`RayQueryFeaturesKHR` / `AccelerationStructureFeaturesKHR` /
  `BufferDeviceAddressFeaturesKHR` 三个特性结构原来**无条件挂进 pNext**，即使对应扩展
  没启用（本身就是无效用法）。
  判据 `device_extensions_degrade_instead_of_failing`（已实测会红）。
  `RV3D_GPU=igpu` 选 610M 时取决于 RADV 是否暴露 RT —— 现在缺了只是没有 RT，不是起不来。
- ✅ **致命启动错误不再以退出码 0 结束**（2026-10-03，commit `9d398ff`）。
  实测踩到：从 TTY/自动化 shell 跑（`XDG_SESSION_TYPE=tty`，缺 `WAYLAND_DISPLAY`/`DISPLAY`）
  时引擎报「创建事件循环失败」然后 `return` ⇒ **退出码 0**，`perf_run.sh` 只看到
  「游戏提前退出（code 0）」、当成正常结束。同一处的 `run_app` 出错路径还会继续打出
  「程序正常退出」。判据 `fatal_startup_paths_never_exit_zero`（源码扫描型，已实测会红）。
  `smoke_linux.sh` / `perf_run.sh` 另加图形会话预检：`/run/user/$UID/wayland-0` 存在就
  自动补 `WAYLAND_DISPLAY=wayland-0`，否则**明确退 2**（没跑成）。

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
| **性能尺子** | ✅ `scripts/perf_run.sh`（三态 0/1/2 均已实测；`-Log` 可离线复核已有日志，见 §11） |
| 真机跑通一局（起窗 / 分辨率 / CJK / VUID / **音频**） | ✅ 全部实测；音频听感见 §5.4（录音 + 引擎计数双向印证） |
| MUX 独显直连 | 用户选择暂不切（§7.3） |
| `package`（打包） | ✅ `scripts/package_release.sh`（`./SteelFront.sh package`，见 §12） |
| `launcher/`（Win32 原生 GUI 启动器） | **不在移植范围**（`#![cfg(windows)]`，整 crate） |
| `queue_present` 上界 / RT 扩展过滤 / 致命错误退出码 | ✅ 均已修（见 §6）；⚠️ present 那条的**最小化场景仍未真机验过** |
| blanket `allow(dead_code)`（`main.rs`） | **未清**（清它要按编译器判据逐条过，别用文本匹配） |
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

---

## 11. Linux 性能尺子（`scripts/perf_run.sh`）

```bash
scripts/perf_run.sh                       # 默认 60s，压力模式 128/方
scripts/perf_run.sh -Secs 30 -NoShadow    # 阴影成本 A/B
scripts/perf_run.sh -Cam "0,0:0,0"        # 固定机位，可复现取景
scripts/perf_run.sh -CullDiag             # 剔除的 CPU 成本
scripts/perf_run.sh -Extra "RV3D_NO_PROPS=1,RV3D_PROC_TEX=0"
scripts/perf_run.sh -Log logs/perf_20261003_103217.log   # 只复核已有日志，不启动游戏
```

**为什么是另写而不是移植 `playtest_perf.py`**：那份是 **X11 专有**的（libX11/XTest 注入、
XImage 截屏、`/proc` 解析），移植它不是改路径而是重写。本脚本改为复用仓库里**已经实测过**
的两样东西 —— 引擎每秒写的 `logs/perf_<stamp>.log`，以及 `pkill -x`。
⇒ **不需要输入注入、不需要截屏**，于是它天然不碰鼠标。

### 三态退出码（教训 46），**三条都实测过**

| 码 | 含义 | 实测 |
|---|---|---|
| 0 | 产出稳态统计（`t >= 3s` 窗口里 ≥3 个样本） | 24 样本 / 稳态 22 |
| 1 | 没有 perf 日志，或样本 <5 行 | 移走 exe 后复现 |
| 2 | 统计打出来了，但稳态窗口 <3 样本 ⇒ **这些数不能当 A/B 的一条臂** | 合成日志复现（见下） |

⚠️ **态 2 靠真实时长几乎落不进去**：引擎每秒一行 ⇒ `NROWS=N` 必然推出 `STEADY_N=N-2`，
所以加了 **`-Log <路径>`**（只分析已有日志、不启动游戏）—— 它同时让这条分支
**可确定性验证**：合成 5 行且 `t` 最大 2.9 ⇒ 实测 `exit 2`；同样 5 行但 `t` 到 5.0 ⇒ 实测 `exit 0`。

### 与 Windows 侧的差异（**不要互相照抄**）

- Windows 版在 `finally` 里调 `release_input.ps1` 解 `ClipCursor`；**Linux 不需要** ——
  游戏在后台跑，永远拿不到焦点（`capture_wanted` 恒 false），且额外设了
  `RV3D_NO_CAPTURE=1`，把"不夺指针"变成**代码级保证**而不是"碰巧没夺"。
- **只许 `pkill -x`**（精确进程名），**绝不许 `-f`** —— 本仓库目录名就叫 `steel-front`。
- 认领本轮 perf 日志时**必须排除本脚本自己的 `logs/perf_run.log`**，否则会分析错文件、
  却把锅甩给这一轮跑动（`perf_run.ps1` 里记录了同一个坑）。
- ⚠️ **不设 `RV3D_PRESENT_MODE`**：与 Windows 版一致，量的是引擎默认（IMMEDIATE）。

### 图形会话预检

从 **TTY/自动化 shell** 里跑时，`WAYLAND_DISPLAY`/`DISPLAY` **不在**（实测
`XDG_SESSION_TYPE=tty`，只有 `XDG_RUNTIME_DIR`），引擎会报
`neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set`。
两个脚本现在都会：`/run/user/$UID/wayland-0` 存在就自动补 `WAYLAND_DISPLAY=wayland-0`，
否则**明确退 2**（没跑成）。配合 `fix(main)` 的退出码修复（原来这种情况退 0）。

### 噪声底（**别把小于它的差当结论**）

`perf_run.ps1` 头部记着：同一二进制连跑两次曾差 2.8%，而其中大部分是 fps 列本身的假象
（它曾记"某一帧的 1/dt"，而 `frame_us` 记的是**另一帧**的耗时 ⇒ 同二进制能"差 48%"）。
改用 `perf_log.rs::window_fps` 后稳定到 ~0.2%。
⇒ **先量当前二进制与参数的 A/A 底噪，低于它一律写"没测到"**；单次一对不是证据（教训 24/45）。

---

## 12. Linux 打包（`./SteelFront.sh package`）

```bash
./SteelFront.sh package                    # 构建 + 打包，tag = 当前时间
./SteelFront.sh package -SkipBuild         # 用现有 exe
./SteelFront.sh package -Tag rc1
```

产出 `dist/steel-front-<tag>/`（可运行目录）与 `dist/steel-front-<tag>.tar.gz`。

### 与 Windows 侧的三处差异（**不要互相照抄**）

| | Windows（`package_release.ps1`） | Linux |
|---|---|---|
| 压缩格式 | `.zip`（Compress-Archive） | **`.tar.gz`** —— tar/gzip 是 Linux 必备，而 **zip 本机根本没装**；收包方换了，格式就该换 |
| 启动器 | 双击 exe（系统把 CWD 设成 exe 目录） | **多装一个 `run.sh`**（见下） |
| exe 名 | `steel-front.exe` | `steel-front` |

🔴 **`run.sh` 不是装饰**：引擎**所有资产都是相对 CWD 的路径**，而 Linux 没有"双击自动
设 CWD"这个默认动作。做过反证 —— 从别的目录直接跑 exe：

```
props: 未载入（读取 assets/props 失败: No such file or directory）
渲染器初始化失败: 打开着色器文件失败 'assets/mesh.spv': No such file or directory
```

⇒ 起不来。用 `run.sh`（只做 `cd` 到自己所在目录 + `exec`）则正常起来。
（而且**上面那次失败当时退出码是 0** —— 这条假绿灯已由 `fix(main)` 修掉。）

### 自检与最强验收

- 照抄了「**装出来跑不起来的包，比没有包更糟**」：缺任一必需资产
  （exe / `mesh.spv` / `triangle.vert.spv` / `triangle.frag.spv` / `maps/index.toml` / `run.sh`）
  就**拒绝出包并删掉半成品**，绝不产出一个"看着成功"的坏包。
- **最强验收 = 解到干净目录真的跑一次**：`tar -xzf` 到 `/tmp`，
  **从别的 CWD** 用 `run.sh` 启动 ⇒ 找到 GPU、交换链 2560x1543 初始化完成。
- 另外逐字节 `diff -r assets <包内 assets>` 一致（56 个 glb：props 24 / guns 16 /
  guns_ext 15 / soldier 1）。
- ⚠️ 报告里「模型 N 个 glb」刻意数**整包**：照抄 Windows 只数 `assets/props` 会写 24，
  而包里其实有 56 个 —— 那是个会让人误判「资产漏拷了」的假数字。

退出码：0 = 打成 / 1 = 跑了但失败 / 2 = 没跑成。
