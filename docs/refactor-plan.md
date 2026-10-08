# 拆分计划：`renderer.rs` / `game.rs` 模块化（2026-09-28 起）

> 目标 —— 单文件从 **1.6 万行**降到 **800–1200 行**：新功能只动一个文件，
> 出 bug 时排查面从"两万行"缩到一个模块，`renderer.rs` 不再"整文件禁止重写"。

## 铁律（每一步都适用）

1. **纯移动**：一步之内不夹带任何逻辑改动。判据 = `tools/refactor_move_check.py`：
   从 `git HEAD` 取出原文块，**逐字节**在目标文件里找；允许的偏差（如 `include_str!` 相对路径）
   必须用**命名预设**显式列出并打印出来，不许静默通过。退出码 **0 = 全一致 / 1 = 有编辑 / 2 = 没跑成**。
2. **闸门全绿**才提交：`cargo test --release` 全过（并核对渲染器过滤计数）、
   `cargo build --release` **0 警告**（`cargo check` 不算）、`python tools/cjk_cover_check.py` OK。
3. **一步一个 commit**，commit 里写清"从哪搬到哪 + 判据输出"。
4. 里程碑（动到 pipeline / swapchain / 描述符 / 渲染循环）**额外跑冒烟**
   （`scripts/run_smoke_pm.ps1`，判据 `vuid==0 and panics==0 and killed>=1`）。

## 为什么 Rust 允许这样拆

- 一个类型的 `impl` 可以分散在**同一 crate 的多个模块**里。
- **私有字段对"定义它的模块及其后代"可见** ⇒ 子模块里的 `impl Renderer { … }` 照旧能读写
  `Renderer` 的私有字段；子模块开头写 `use super::*;` 取上一层的名字。
- ⇒ 形态：`engine/renderer.rs` 保留为**模块根**（结构体定义、构造、对外 API、常量），
  其余放 `engine/renderer/*.rs`；`game.rs` 同理。

## 已完成的第 0 步（测试搬出去）

| 新文件 | 行数 | 内容 |
|---|---|---|
| `src/engine/renderer/tests_geom.rs` | 901 | marker 尺寸 / 实例槽位 / 地形 LOD+高度 / 画质档 / 水平面绕序 / 剔除分段 |
| `src/engine/renderer/tests_gpu.rs` | 852 | 截图像素序 / SIMD 剔除 / NPC 视觉 / workgroup 布局 |
| `src/engine/renderer/tests_vk.rs` | 1138 | PT 配置 / PT 道具属性 / Vulkan 失败路径 / present 结果 / 物理设备选择 |

`renderer.rs` 16264 → **13398** 行。验证：**16/16 块逐字节一致**；
`cargo test --release` **660 passed / 0 failed**，其中 `renderer` 过滤 **71 passed**（与搬前 `#[test]` 数一致）；
`cargo build --release` **0 警告**；CJK 闸门 OK。

## 后续顺序（每步一个 commit，最大块优先）

> **进度（2026-09-28）**：`renderer.rs` 已从 **16501 → 10710 行**。已落地：
> `tests_geom.rs`(901) / `tests_gpu.rs`(852) / `tests_vk.rs`(1242) / `geometry.rs`(816) /
> `instances.rs`(963) / `parts.rs`(1085)。工具：`tools/refactor_extract.py`（spec 驱动搬运）
> + `tools/refactor_move_check.py`（逐字节判据）。

1. **renderer 的小类型与纯函数**：`QualityPreset`(665) / `WorldMarker`(711) / `TerrainLod`(82) /
   地形高度与噪声函数 → `renderer/quality.rs`、`renderer/world_marker.rs`、`renderer/terrain.rs`。
2. **`impl Renderer` 分片**（当前 11205 行）：PT(≈1600) / 场景上传(≈1200) /
   `record_command_buffer`(1106) / 纹理(≈1000) / init 族(≈1600) / 阴影(≈700) / HUD(≈500) /
   截图回读(173) / present 与设备生命周期（含 `Drop` 461 行）。
3. **`impl Game` 分片**（当前 5437 行）：`drain_collisions`(529) / `update_ai`+`step_npc`(613) /
   `update_projectiles`(198) / `fire_shot`(129) / 爆炸与弹道 / 压力模式与 survive /
   网络桥接 / HUD 状态 / 类型段（`Stance`/`FireMode`/`AiTierParams`/`MapObstacle`/`LevelMap`/
   `MissionObjective`/`ImpactMark` 601）。
4. **`game.rs` 的 3604 行测试单模块** → `game/tests_*.rs` 若干（按 `#[test]` 分组整体搬运）。

## 踩过的坑（工具与脚本已固化）

1. **块的边界要按「文档注释归下一个块」推导**。按 `#[cfg(test)]` 切会把 `///` 注释留在上一块尾部，
   变成悬空文档注释（`expected item after doc comment`）。
   ⇒ 搬运脚本导出 `logs/split_ranges.txt`，判据用 `--ranges-from` 消费它（**单一定义**，两处不再各推一份行号）。
2. **自扫源码的测试对目录敏感**：`include_str!("renderer.rs")`、`include_str!("../../build.rs")`
   搬一层目录即失效。好消息是它们**响亮地失败**（不是静默扫错文件）。
   ⇒ 判据里用 `--normalize-preset include-str-renderer` / `include-str-build-rs` 显式记账。
   🔴 **后续把 `impl Renderer` 拆成多文件后，这些自扫测试的扫描范围必须同步扩到整个 `renderer/` 子树**，
   否则会"看起来还在扫，其实只扫了根文件" —— 判据静默失效（教训 46 同形）。
3. **判据的 `--normalize` 取值含 ASCII 双引号 ⇒ 过 PowerShell 会被剥掉**（匹配数变 0，且没有任何报错）
   ⇒ 增设命名预设，命令行里不出现引号。
4. **推送前先 `fetch`**：本机 Windows 会话与 Linux 会话同时推同一仓库，
   基于旧 `renderer.rs` 行号的搬运会在 rebase 时冲突 ⇒ **搬运脚本必须可重放**
   （结构驱动、不手打行号），冲突后重跑即可。
5. **Rust 的「方法私有性」≠「字段私有性」**（第一条切缝当场踩到，43 条编译错误）：
   私有**字段**对定义模块的后代可见 ⇒ 子模块里的 `impl` 照旧能读写 `Renderer` 的字段；
   但私有**方法**只在"定义它的模块及其后代"可见 ⇒ 父模块与 `renderer::tests_*` 都看不见
   子模块 impl 里的私有 `fn`。所以搬走的方法统一加宽为 `pub(crate)`，并在范围表里以 `widen=*`
   记账（加宽后编译器还会报 `private_interfaces`，要求把签名里的私有类型一并提到 `pub(crate)`：
   已按它的要求做了 `Vertex` / `TerrainLod` / `InstanceData`）。
6. **搬方法必须套 `impl` 外壳**：搬出来的是方法，裸 `fn` 带 `self` 是语法错 ⇒ 目标文件写
   `impl Renderer { … }`（外壳是脚手架，不计入搬运字节）。
7. 🔴 **自扫源码的判据必须跟着扩到整个子树**（第二条切缝当场红了 1 条）：
   `upload_buffers_are_created_before_the_old_ones_are_destroyed` 扫 `renderer.rs` 找 `pub fn set_props(`，
   而那段已搬进 `parts.rs`。修法是新增 `renderer/tests_support.rs::renderer_production_sources()`：
   **运行时枚举** `renderer.rs` + 全部非 `tests_*.rs` 子模块（跟着拆分自动变大，不需要谁记得改），
   并用根文件的 `mod` 声明做**交叉校验**（声明了却没被扫到 ⇒ 直接红）、加字节下限与
   `impl Renderer` 存在性下限 ⇒ **fail-closed**，避免退化成"扫了个空还说 OK"（教训 46）。
   同时把判据里的 `pub fn NAME(` 改成 `fn NAME(`（加宽后前者匹配不到，会误红）。
