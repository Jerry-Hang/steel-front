# steel-front Android 移植进度（2026-10-09）

> 本次会话（Operit 手机端 agent，经 SSH 操作 Arch 笔记本）的产出记录。
> 仓库：`/home/jerry/Work/steel-front`（Arch，jerry@jerry-archlinux）

## 已完成的提交

| 提交 | 内容 |
|---|---|
| `a95927e` | `chore(android)`：为 Android 目标启用 winit `android-game-activity`，记账 `android-activity` 传递依赖例外（AGENTS.md 铁律） |
| `517f19e` | `feat(assets)`：抽出只读资产来源抽象层（`trait AssetSource` + `FsSource`） |
| `fd0b5eb` | `feat(render)`：加 `RV3D_NO_MESH=1` 开关（强制走传统顶点管线，A/B 取证） |
| `2798794` | `feat(assets)`：内存资产来源 `MemSource` + `RV3D_ASSETS_FROM_MEMORY=1` |
| `661d155` | `feat(android)`：`android_main` 入口 + 主循环跨平台提取 |
| `d09ffdb` | `docs(android)`：移植进度记录 |
| `15f7c75` | `fix(android)`：Android 目标 0 警告（cfg 收窄 waveOut/ALSA 专属项） |

## 环境（本机已装好，免 sudo）

- **NDK r29**：`~/Android/android-ndk-r29`（2.4G，手动下载解压）
- **cargo-ndk**：v4.1.2（`cargo install`）
- **编译命令**（裸 `cargo check` 会因 cc-rs 找不到 `aarch64-linux-android-clang++` 失败）：
  ```bash
  export PATH=$HOME/.cargo/bin:$PATH
  export ANDROID_NDK_HOME=$HOME/Android/android-ndk-r29
  export ANDROID_NDK_ROOT=$HOME/Android/android-ndk-r29
  cd ~/Work/steel-front && cargo ndk -t arm64-v8a check
  ```

## 关键发现

### 1. 渲染 A/B 取证必须加 `RV3D_PROC_TEX=0`
- 程序化地面纹理开启时，**同一路径跑两遍的 A/A 差异高达 99.97%**（噪声淹没信号）。
- 关掉程序化纹理后 A/A = 0.000%（完全确定）。
- 已查：纹理**内容**是确定的（三次跑 fnv 校验和一致 `47969fcf94dd7532`），
  非确定性在它的 **GPU 使用路径**（`create_sampled_image` 的 mip 链生成/采样）。**根因未定**。

### 2. mesh vs 传统两路径**并非逐字节等价**
确定性基线（`RV3D_PROC_TEX=0`，同机位 `street_fight fly:-30,3,-40:45,0`）下：
| 阈值 | 差异像素 |
|---|---|
| >4 | 6.055% |
| >8 | 3.909% |
| >16 | 2.990% |
| >32 | **2.361%** |

A/A 对照全 0。⇒ HANDOFF-mobile §4.1 的"两条路径逐字节等价"设计承诺**已不成立**。
手机只能走传统路径 ⇒ 画面会与主路径不同。

## 取证环境（Linux 原生，KDE Wayland）

```bash
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 \
RV3D_AUTOSTART=1 RV3D_MAP=street_fight \
RV3D_CAM="fly:-30,3,-40:45,0" RV3D_SHOT_AT=5 RV3D_FORCE_UNFOCUSED=1 \
RV3D_PROC_TEX=0 [RV3D_NO_MESH=1] [RV3D_ASSETS_FROM_MEMORY=1] \
timeout 15 ./target/release/steel-front
```
- 截图落 `/tmp/steel_front_<unix秒>.png`（Linux）
- 比对：`python3 scripts/png_diff.py A.png B.png`

## 下一步（未做）

- [ ] Android 打包成 `.so`（jniLibs）需要把 crate 改成 **cdylib**（当前是纯 bin crate）
- [ ] 生命周期：`onPause`/`onResume`/`onDestroy`；surface 丢失恢复（与铁律 B 的 `device_lost` 粘性冲突）
- [ ] `audio_out.rs` 的 Android 路径（AAudio）——**Android 目标已 0 警告**（`15f7c75` 用 cfg 收窄），但音频仍走静默回退，真机需实装 AAudio
- [ ] `AAssetManager` 实现（接 `AssetSource` 的第三个实现）——需新增 `ndk` 直接依赖（又一次破"10 依赖"铁律，需记账），且只能在真机验证
- [ ] 触摸输入（铁律 C 前提反转）、质量档、ASTC 纹理
- [ ] 渲染非确定性的根因（mip 链 / 采样路径）
