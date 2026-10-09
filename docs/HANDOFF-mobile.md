# 手机移植交接（Android / iOS）

> 写于 2026-10-09，基线 `6c787bf`（Linux 适配全部落地之后）。
> **本文的定位**：这不是"计划书"，是**交接**。所以它只写三类东西 ——
> **① 已经用命令验过的事实**、**② 必须由你拍板的政策问题**、**③ 我明确不知道、必须实测的东西**。
> 本仓的传统是「没有 lead 的不要瞎猜」（AGENTS.md 未结案清单前言），这里照办：
> **凡是没验过的，本文一律标「未验」并给验证方法**，不写成结论。

---

## 0. 一句话结论

**起点比想象的好**：代码库**已经能编 AArch64**（下面有命令与输出），
`ash-window` 0.13 **原生支持 Android 与 iOS 的 surface**，而平台相关代码**高度集中在 3 个文件**。

**真正的硬骨头不在 `cfg`，在四件"架构级"的事**（见 §4）：
**mesh shader 在手机上基本不存在（而它是本仓的主路径）**、**触摸输入与整套鼠标安全协议的前提相反**、
**事件循环的所有权反转**（Android 是系统调你，不是 `main` 跑起来）、**资源在 APK 里而不是磁盘上**。

**另有一件必须先拍板的事**（§1）：winit 在 Android 上**必然**引入 `android-activity` 这个新依赖。

---

## 1. 🔴 先拍板：这会不会破坏「不新增第三方依赖」？

`Cargo.toml` 现在 10 个依赖，而 **AGENTS.md 把「不新增第三方依赖」写成了硬约束**。
手机端的第一道冲突就在这里：

**winit 0.30.13 的 `Cargo.toml` 里，Android 支持是靠 feature 引可选依赖 `android-activity` 实现的**（已验，见下）：

```toml
android-game-activity   = ["android-activity/game-activity"]
android-native-activity = ["android-activity/native-activity"]
```

- 它**不是**我们手写进 `[dependencies]` 的，而是 `winit` 的**传递依赖** ——
  但它在构建图里，且是一个真实的第三方 crate。
- 本机 **没有** `cargo-ndk`、**没有** NDK（已验）。

**三个选项，各有代价**（你来定）：

| 选项 | 代价 |
|---|---|
| **A. 接受这个传递依赖** | 破一次例。理由可以写清楚：它是 **winit 官方指定的 Android 入口**，不是我们另找的轮子 |
| **B. 不用 winit 的 Android 后端**，自己写 `NativeActivity` + `ndk-sys` FFI | 零新依赖，但等于自己维护一套事件循环/生命周期适配层 —— 与我当初**拒绝移植 `launcher/`** 是同一类判断，但这次省不掉 |
| **C. Android 端只做"能跑起来"的验证分支**，不追求产品化 | 最小改动，但只是探路 |

> 我的建议是 **A**，且**明确记账**（在 AGENTS.md 铁律里加一行例外及其理由）。
> 理由：这条约束的**目的**是"别让依赖膨胀、别把可移植性卖出去"，
> 而 `android-activity` 恰恰是**为了可移植性**存在的官方适配层，不是相反。
> 但这是政策，**我不替你定**。

---

## 2. 已经验过的事实（都带命令，可复现）

### 2.1 代码库已经能编 AArch64 ✅

```bash
cargo check --target aarch64-unknown-linux-gnu     # 退出码 0
# → Compiling steel-front v0.1.0 … Finished `dev` profile in 2.83s
```

`aarch64-unknown-linux-gnu` 目标本机已装（`rustup target list --installed`）。
这正是**铁律 E 里写的交叉验证口径**，也就是说这一条**一直在守**，不是新做的。

⚠️ 注意口径：`cargo check` **不能**当 0 警告闸门（铁律 F：它会重放缓存诊断）。
上面这条只证明**能编过**，不证明 0 警告。要 0 警告得 `cargo build --release --target …`。

### 2.2 `ash-window` 0.13 原生支持两边的 surface ✅

`~/.cargo/registry/src/…/ash-window-0.13.*/src/lib.rs`（已读出原文）：

```rust
// Android
let surface_desc = vk::AndroidSurfaceCreateInfoKHR::default()
    .window(window.a_native_window.as_ptr());
let surface_fn = android_surface::Instance::new(entry, instance);
surface_fn.create_android_surface(&surface_desc, allocation_callbacks)

// 所需 instance 扩展它也替你算好了：
RawDisplayHandle::Android(_) => {
    const ANDROID_EXTS: [*const c_char; 2] =
        [surface::NAME.as_ptr(), android_surface::NAME.as_ptr()];
    &ANDROID_EXTS
}
// iOS/macOS 走 VK_EXT_metal_surface（=> MoltenVK），与铁律 E「不做原生 Metal 后端」一致
```

⇒ **surface 这一层不用我们写**。这与 Linux 侧的经验一致：
当初 `main.rs` 里后端选择那套逻辑改动很小，真正花时间的是**平台语义差异**。

### 2.3 平台相关代码高度集中 ✅

`#[cfg(...)]` 的实测分布：

| 目标 | 处数 | 分布 |
|---|---|---|
| `target_os = "windows"` | **42** | `audio_out.rs` **23** · `main.rs` **11** · `cpu.rs` **8** |
| `target_os = "linux"` | 22 | （同上三家） |
| `target_arch = "x86_64"` | **53** | `cpu.rs` 15 · `simd.rs` 13 · `renderer/*` … |
| `target_arch = "aarch64"` | **13** | `simd.rs` 4 · `instances.rs` 3 · `geometry.rs` 3 · `tests_gpu.rs` 3 |
| `android` / `ios` / `macos` / `unix` | **0** | —— 手机侧一行都还没有 |

⇒ **`cfg` 层面的主战场只有 3 个文件**（`audio_out.rs` / `main.rs` / `cpu.rs`）。
**好消息**：Linux 移植刚把 `audio_out.rs` 拆成「Windows waveOut / Linux ALSA」两套并列结构，
手机要加的是**第三套**（AAudio 或 OpenSL ES），结构已经在了。

---

## 3. 逐文件的移植面（结合 Linux 那一轮的经验）

> Linux 那一轮的教训（`docs/linux-native.md`）：**"同一个现象，两种成因"**是最容易误判的一类。
> 下面每一条都写清"**为什么**它挡路"，而不是只列个文件名。

### 3.1 `src/audio_out.rs`（23 处 windows cfg）—— **劳动量最大，但结构最好**

现状：`WaveOutSink`（Windows winmm）/ `AlsaSink`（Linux，运行时 `dlopen libasound.so.2`）。

手机侧：
- **Android**：`AAudio`（API 26+，推荐）或 `OpenSL ES`（更老）。两者都是 **`.so` + FFI**，
  与 Linux 的 `dlopen` 路子同构 —— **可以照抄 ALSA 那一套**（运行时加载、失败降级）。
- **iOS**：`AudioToolbox` / `AVAudioEngine`（要走 MoltenVK 那条的话是 ObjC 运行时）。
- **必须注意**：Linux 侧踩过的坑是「只试 `default` 会让一部分机器永远没声音，
  本机 `default` 解析到 dmix 并报 `unable to open slave`」⇒ 手机侧**同样不要只试一个设备名**。
- 降级路径必须仍然可用（`SilentSink`）—— 手机权限/后台策略会让音频拿不到。

### 3.2 `src/main.rs`（11 处）—— **不止是 cfg，是所有权模型**

这里要分开看：

- **光标捕获那套（铁律 C）在手机上整个前提不成立**。现在的一切都建立在
  「抓光标 ⇒ 相对位移 ⇒ 视角」上；手机是**触摸**：没有光标、没有悬停、没有右键。
  ⇒ 要设计的是**触屏操作方案**（虚拟摇杆 / 拖动转视角 / 陀螺仪），
  这是**设计工作**，不是移植工作。**别指望把 `sync_cursor` 改几个 cfg 就能用。**
- **`RV3D_BACKEND` 那套后端选择**要加第三、第四个分支（Android/iOS 没有 Wayland/X11 之争）。
- **`WindowEvent::Resized` 语义**：手机有**旋转屏幕**与**分屏/画中画**，
  交换链重建的触发频率与桌面完全不同 —— 而这条路径**我们刚在 Linux 上验过**
  （`resize_probe.sh` 20 步长跑，VUID=0），**手机侧要重跑同一套探针**（§6）。

### 3.3 `src/engine/cpu.rs`（8 处）—— 🔴 **这个文件是只读红线**

AGENTS.md 写明 `engine/cpu.rs` 是 **🔴 只读**（线程池 / 亲和 / 降频，用户红线）。
手机侧它**必然要动**，因为：

- `sched_setaffinity` 在 Android 上**存在但语义不同**：big.LITTLE + 热插拔核心，
  照抄桌面的"每对取最小 vCPU"会**把线程钉在小核上**。
- **不可写死 SMT 奇偶**（铁律 E 已经写了这条）—— 手机**多数没有 SMT**，这条要重新推。
- **动它之前必须先问用户**（这是他自己定的红线）。

### 3.4 `src/engine/simd.rs` + `renderer/{instances,geometry}.rs`

- `x86_64` 53 处 / `aarch64` 13 处 —— **NEON 路径已经存在**（`cull_spheres_neon`，
  铁律 E：4 实例/批、非 FMA、与标量**逐位一致**）。
- ⚠️ **但"存在"不等于"等价"**：铁律 E 说它与标量逐位一致，那是**当年验过的**。
  手机上是**新的 aarch64 实现（不同编译器/不同 CPU）**，`cargo test` 里的
  `simd_cull_microbench` / 逐位一致判据**必须在真机上重跑**。**未验。**

### 3.5 `engine/renderer/device.rs`（19 处 mesh 相关）

见 §4.1 —— 这是最大的架构问题。

---

## 4. 四件架构级的事（`cfg` 解决不了）

### 4.1 🔴 mesh shader 在手机上基本不存在 —— 而它是本仓的**主路径**

**AGENTS.md 铁律 A 原文**：「**主开发路径 = `VK_EXT_mesh_shader`**。
所有新渲染功能、性能优化、视觉迭代一律在 mesh 路径上做」，
而传统 `VERTEX + FRAGMENT` 管线是「**冻结维护**，只作缺扩展时的兼容回退，
**不再新增任何功能**」。

手机现实：**Android 的 Vulkan 驱动普遍不暴露 `VK_EXT_mesh_shader`**
（Apple 侧压根没有 Vulkan，走 MoltenVK，更没有）。
⇒ **手机上的主路径会变成那条"冻结、不再新增功能"的回退路径。**

**这意味着**（这条值得单独想清楚，别当成技术细节）：

1. 那 19 处 `mesh_enabled` 分支里，**回退路径才是手机上唯一跑得通的** ——
   它必须从"兼容回退"升级为**一等公民**，而它已经**很久没有新功能投入**了。
2. ⚠️ **"两条路径逐字节等价"这个假设要立刻验证**。
   历史记录里 `RV3D_MENU_GLASS` 那段就写着「**游戏内 HUD 那条路逐字节不变**」——
   那是**设计承诺**，但承诺是否在 65536 实例场 / 地形 LOD / 阴影两图这些大改之后**仍然成立**，
   **未验**。判据建议：同一机位、两条路径各截一张，`tools/png_diff.py` 比。
3. **好消息**：Linux 移植里 `d7b2444`（设备扩展缺一个不再让游戏起不来）
   和 `3ce98f3` 那一族改动，正好是这条路的**前置条件** ——
   没有它，手机上一开始就 `create_device` 失败。

### 4.2 🔴 事件循环的所有权反转

桌面上是：

```rust
let event_loop = EventLoop::new()?;
event_loop.run_app(&mut app)?;      // 我们的 main 拥有控制权
```

**Android 上是反的**：系统（`Activity`/`NativeActivity`）**回调你**，
`android_main` 由 `android-activity` 提供，且 **`Activity` 可以在任意时刻被销毁重建**
（旋转、内存压力、切后台）。

⇒ 需要回答的问题（**都是未验**）：
- `GameApp` 的构造/析构能不能经得起**多次创建**？（Vulkan device、swapchain、资源缓存）
- **切后台**时 Android 会要求释放 surface —— 现在 `device_lost` 是**粘性不可恢复**的
  （铁律 B），而手机上 **surface 丢失是常态、必须可恢复** —— 这两条**直接冲突**。
  **这是本仓最需要重新设计的一处。**
- Linux 侧我加过 `present_stall` 检测与降级；手机侧要加的是**恢复**路径。

### 4.3 资源在 APK 里，不是磁盘上

引擎现在按**相对路径**读 `assets/`（`package_release.sh` 头部专门记过这个坑）。
Android 上 APK 里的资源要走 **`AAssetManager`**（`asset://` 不在文件系统里），
`engine/assets.rs` / `props.rs` / `map.rs` / `shaders` 的加载路径**全部要过一层抽象**。

**Linux 那一轮的教训直接适用**：`SteelFront.sh` 开头那句 `cd` 到脚本真实目录，
就是为了解决"从别处启动找不到资产"。手机上是**同一类问题的更硬版本**。

**建议**：先在 Linux 上做一个**只读的抽象层**（`trait AssetSource`：`fs` 实现 + 内存实现），
用 `RV3D_ASSETS_FROM_MEMORY=1` 在桌面上把这条路**先跑通**——
这样手机侧接 `AAssetManager` 只是加第三个实现，**不必在真机上调试资源路径**。

### 4.4 触摸输入 = 铁律 C 的整套前提反转

铁律 C 是**用户 2026-09-03 明确要求**的鼠标安全协议，代价很高、理由很足：

> 引擎自己抓光标，抓取期间鼠标被锁 → 用户的机器会「像死机」。
> **按键用 `PostMessage`，不用 `SendInput`、不用 `SetForegroundWindow`。**

这条协议**在手机上完全不适用**（没有光标、没有前台/后台窗口之争），
但它背后那条**更普适的教训适用**：**别让游戏夺走用户对设备的控制**。
手机侧的对偶问题是：**别让游戏锁死返回手势 / 别让玩家退不出去 / 别在后台偷跑**。

⚠️ 参考 Linux 侧同类判断：`3ce98f3` 修的正是「Wayland `Locked` 抓取**假成功**」
—— 抓取失败**无法从返回值看出**。手机侧的"锁死返回键"很可能是**同一个形态**。

---

## 5. 其余必须过一遍的点（逐条给"为什么"）

| # | 点 | 为什么挡路 |
|---|---|---|
| 1 | **配置路径** `$HOME/.steel_front.cfg` | Android 没有 `$HOME` 给应用写；要用 app-private dir（`internalDataPath`）。`config.rs` 是「原子写 + 容错加载」，路径来源要参数化 |
| 2 | **日志** stderr → **logcat** | 现在 `env_logger` 写 stderr，手机上 stderr 没人看。**所有闸门都读 `logs/*.log.err`** ⇒ 需要一个 logcat→文件的落盘器，否则 §6 的判据全部失效 |
| 3 | **`RV3D_*` 环境变量** | 手机上没法 `RV3D_VALIDATION=1 ./game`。要么改用**配置文件/Intent extra/调试菜单**，要么让开发构建硬编码一组。**这是移植期最大的效率瓶颈** |
| 4 | **TBDR（瓦片式）GPU** | 手机 GPU 是 tile-based：现在那套「HUD 挪到 overlay pass + 交换链→320×200 blit 当模糊图」（铁律 B 磨砂玻璃）在 TBDR 上代价模型**完全不同**，甚至可能更便宜（on-chip）也可能更贵（多一次全屏）——**必须实测，别推理** |
| 5 | **内存** | 本机 12 GB，而手机 6–12 GB 且**与系统共享**。65536 实例场 + `PT_MAX_BOXES=2048` + 道具 2^21 顶点预算，**得重新测一遍**。⚠️ 铁律 F 那条「内存 12GB：一次只跑一个 cargo」在手机上变成**更紧**的约束 |
| 6 | **呈现模式** | `IMMEDIATE/FIFO/mailbox` 在 Android 上的可用集不同；且**手机没有"独显直连"**，铁律 B 里那一大段混合输出/跨卡拷贝的推理**整段不适用** |
| 7 | **构建** | `build.rs` 生成 WGSL→SPIR-V（naga）与 GLSL→SPIR-V（glslangValidator）。`build_spv_rt.rs` 需要**宿主机的 glslangValidator** —— 交叉编译时要确认它在 CI/开发机上存在（本机有） |
| 8 | **`cargo-ndk` / NDK** | 本机**都没有**（已验）。Android 构建必须先装这两样 |
| 9 | **AArch64 上的 `image` crate** | `image = 0.25` 是纯 Rust 还是带 SIMD 后端？**未验**。手机侧贴图解码的性能要实测 |
| 10 | **iOS** | 走 MoltenVK（铁律 E 已经定了「不做原生 Metal 后端」）⇒ **Vulkan 是第三方的**，`VK_EXT_mesh_shader` 更没有，且**不能依赖任何 Windows/Linux 的驱动行为**。本仓大量注释基于「NVIDIA 专有驱动实测」，**在 MoltenVK 上一条都不成立** |

---

## 6. 验收口径：手机侧要有自己的一套闸门（坑不互替）

**环境铁律**（AGENTS.md）已经写了「Windows / Linux 原生**并存，坑不互替**」。
手机要成为**第三条**，同样不能互相照抄。必需的闸门：

1. **`cargo check --target aarch64-linux-android`** —— 编译面（现在连目标都没装，已验）
2. **0 警告闸门**：只能是 `cargo build --release --target …`（铁律 F：`check` 不算）
3. **真机冒烟**：判据沿用**三态退出码**（0/1/2，教训 46），
   且判据必须**只认正证据** —— Linux 侧的教训是「`vuid==0` 在空日志上恒真」，
   手机侧**更容易**犯这个错（日志要经过 logcat 搬运，中间断了就"看起来没错误"）。
4. **交换链重建探针**：`scripts/resize_probe.sh` 那套判据（**必须证明缩放真的发生过**）
   在手机上对应**旋转屏幕 / 分屏 / 切后台回前台** —— 比桌面更重要，因为手机上是常态。
5. **VUID 判据**：⚠️ 与 Linux/Windows 侧**同步那张已知噪声表**
   （`docs/linux-native.md` §10 刚记了这条；`KNOWN_VUID` / `known_driver` 两侧必须逐条一致）。
   手机上会有**全新的**驱动噪声，**别直接照抄桌面的白名单**。
6. **A/A 噪声底**：`aa_probe` / `ab_pair` 那一套（教训 43/45）在手机上要**重新量**，
   而且手机有**热降频** —— 几分钟就会漂，噪声底可能远大于桌面的 0.2~5.5%。

---

## 7. 建议的第一步（具体、可执行、且不碰红线）

按"**先摸基线 → 再记录 → 再动手**"（用户习惯）：

1. **装目标**（不改仓库）：`rustup target add aarch64-linux-android`，
   再跑 `cargo check --target aarch64-linux-android`。
   ⇒ 这一步**不需要 NDK 也能暴露一批 `cfg` 问题**（链接器才需要 NDK）。
2. **拍板 §1 那个依赖问题**（这是唯一的阻塞项，且只有你能定）。
3. **在 Linux 上先做 §4.3 的资源抽象层**（`trait AssetSource` + 内存实现 + `RV3D_*` 开关）。
   ✅ **这一步完全在 Linux 上闭环**，且手机侧接 `AAssetManager` 时就不用碰真机。
4. **在 Linux 上先验 §4.1 的"两条渲染路径等价"**（mesh vs 传统，同机位 `png_diff`）。
   ✅ 同样在 Linux 上闭环，而且**这条本来就是欠账** ——
   它不只为手机，桌面上的回退路径也一直没人比过。
5. 上面两步做完，再谈 NDK / 真机 / `android-activity`。

> **第 3、4 步是这次交接的核心建议**：它们把"手机移植"里**能在 Linux 上做完的部分**
> 剥离出来了 —— 而 Linux 侧的工具链（`smoke_linux.sh` / `resize_probe.sh` / `ab_pair.sh` /
> `png_diff.py`）**现在都是通的**，可以直接用。

---

## 8. 交接清单（给下一个接手的人）

**先读**：`AGENTS.md`（铁律 + 未结案 + 教训）→ `docs/PROGRESS.md`（索引，别通读）
→ `docs/linux-native.md`（**最近一次平台移植的完整经验，最值得抄的是它的"踩坑"节**）。

**三条最容易踩的**：
1. 🔴 **`engine/cpu.rs` 是用户红线（只读）** —— 手机侧必然要动它，**动之前先问**。
2. 🔴 **别把桌面的驱动行为当普适真理** —— 本仓大量注释写着"实测"，
   但那些实测几乎全部在 **NVIDIA 独显 + 桌面合成器**上做的。手机是**另一套世界**。
3. 🔴 **闸门必须能红**（教训 46）：手机侧"看起来没报错"极可能只是**日志没搬到**。

**已验证的事实**（可直接引用，不必重做）：§2 的三条。
**明确未验**：§3.4 的 SIMD 逐位一致、§4.1 的两路径等价、§4.2 的 Activity 重建、
§5 的全部 10 条。
