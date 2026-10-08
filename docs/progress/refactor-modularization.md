# refactor-modularization —— 由 `PROGRESS.md` 拆出的专题日志

> 主题：把 `engine/renderer.rs`（16 501 行）与 `engine/game.rs`（10 715 行）拆成 800–1200 行的
> 模块化子文件。做法与判据见 [`docs/refactor-plan.md`](../refactor-plan.md)；
> 工具 = `tools/refactor_extract.py`（搬运）+ `tools/refactor_move_check.py`（逐字节判据）。
> 本文只记**结果与代价**，不重复计划里的规矩。

## 2026-09-28 拆分完成：两个根文件都落进 800–1200 行（`c041e96` … `gpu_layout` 一批）

**读数**：`renderer.rs` 16 501 → **1 197 行**（−93%）；`game.rs` 10 715 → **1 117 行**（−90%）。
新增 19 个 renderer 子模块 + 13 个 game 子模块，除测试文件外全部 ≤ 1 200 行。

| 侧 | 子模块（行数） |
|---|---|
| renderer | geometry 816 / instances 963 / parts 1085 / record 1105 / pt_assets 740 / pt_render 789 / textures 1002 / shadow 863 / pipelines 996 / descriptors 544 / device 922 / swapchain 511 / frame 898 / drop 416 / helpers 637 / gpu_layout 215 |
| game | session 1173 / weapons 620 / npc_ai 739 / projectiles 585 / waves 483 / net 397 / player 250 / collisions 56 / types 146 / ai_util 581 / map_util 256 / diag 103 |

**每一步的判据**（不是"看着搬完了"）：`refactor_move_check.py` 从 `git HEAD` 取出原文块，
逐字节在目标文件里找；允许的偏差必须用命名预设显式列出并打印。逐字节一致的块数：
16（测试）/ 17（geometry）/ 19（instances）/ 8（parts）/ 1（record）/ 8+4（PT）/ 5（textures）/
5（shadow）/ 10（pipelines）/ 4（descriptors）/ 9（device）/ 12（swapchain）/ 13（frame）/ 1（drop）/
8（game types）/ 13（net）/ 25（session）/ 33（weapons）/ 11（npc_ai）/ 14（projectiles）/
11（waves）/ 3（collisions）/ 24（player）/ 44（helpers）/ 29（gpu_layout）/ 17+11+11（game 自由函数）。

**闸门**：每一步都跑 `cargo test --release`（664 passed / 0 failed，全程未变）+ `cargo build --release`
0 警告 + CJK 闸门；里程碑（动过 pipeline/framebuffer、顶点布局）另跑真机冒烟两次，均
`VUID=0 panics=0` **ALL-OK**（fps 182.5 / 171.9）。

### 为什么必须"逐字节 + 显式偏差"

拆分期间**从来没有**因为搬错一段逻辑而红过测试 —— 红过的全是**边界与可见性**问题，
而它们全都表现为编译错误或警告（见下）。如果当时用"看一眼 diff"来验收，
这些错误会以"看起来搬完了"的形态混进去，然后在下一次真实改动里爆出来。

### 工具在这一轮被"打"出来的十个真缺陷（每一个都当场修掉并留档）

1. 🔴 **删除顺序**：先删脚手架、再按原始行号删搬运块 ⇒ 行号已漂移、删错地方。改成一次降序扫描。
2. 🔴 **陈旧范围表**：自检失败退出后范围表还是上一轮的旧行号，判据读旧数据仍报"逐字节一致"。
   现在开跑先删输出文件 —— 失败就没有证据文件。
3. **impl 收尾括号**：`impl` 的最后一个方法范围会吃掉 impl 的 `}` ⇒ 编译期 unclosed delimiter。
   mk_spec 改按花括号配平求隐含收尾，搬运侧兜底"块尾顶格 `}` 留给外层"。
4. **文档注释与 `fn` 之间夹空行**：原规则遇空行即停 ⇒ 注释留在源文件变悬空 doc（编译期报错），
   且被搬走的方法**静默丢文档**。现在允许跨空行 + 悬空 doc 体检。
5. **`super::` 层级**：`super::procedural::…` / `super::lighting::…` 下沉一层后失效 ⇒ 在根模块加
   `pub(crate) use …;` 再导出（搬走的字节保持原样，判据不需要新归一化规则）。
6. 🔴 **方法可见性 ≠ 字段可见性**：子模块 impl 里的私有 `fn`，父模块与测试模块都看不见
   （43 条 E0599）⇒ 统一加宽 `pub(crate)` 并记账 `widen=*`。
7. **trait impl 不能带可见性限定符**（E0449）⇒ 加宽时跳过 `impl Trait for Type`。
8. 🔴 **搬方法必须套 impl 外壳**：只给容器参数、漏了外壳时方法落成模块级裸函数（21 条 E0599）
   ⇒ 工具 fail-closed 拒绝这种搬运。
9. **模块名冲突**：新模块叫 `util` 撞上 `use ash::{…, util, …}`（E0255 + 把 `util::read_spv` 解析带偏）
   ⇒ 改名 helpers。
10. 🔴 **自由函数的范围会吃到文件尾**：边界正则不认带属性的声明（`#[cfg(test)] mod tests;`）⇒
    把测试模块声明一起删掉（E0583）。现在属性行、`// =====` 分隔行都算顶层边界。

### 结构与可见性的"账"（下次要动这些文件时先看）

- 根文件保留**类型与常量定义**（子模块是它的后代，看得见根里的私有项），逻辑与 `impl` 搬到子模块；
  子模块里的方法/自由函数统一 `pub(crate)`，根模块用 `pub(crate) use <mod>::*;` 再导出，
  外部路径（`main.rs`、`game.rs` 引用 `renderer::terrain_height_at` 等）因此逐字不变。
- 反过来**不行**：父模块看不见子模块的私有项 ⇒ 结构体一旦搬进子模块，被外部构造时
  其字段必须 `pub(crate)`（`gpu_layout.rs` 的 `Vertex` / `InstanceData` 就是这么加的，13 个字段）。
- 🔴 **自扫源码的判据必须跟着扩到整个子树**：`upload_buffers_are_created_before_the_old_ones_are_destroyed`
  搬完第 3 步就红了（它扫 `renderer.rs` 找 `pub fn set_props(`，而那段已搬进 parts.rs）。
  修法是 `renderer/tests_support.rs::renderer_production_sources()`：运行时枚举整个子树 +
  用根文件 `mod` 声明交叉校验 + 字节/`impl Renderer` 下限，**fail-closed**（教训 46 同形）。
  同类判据 7 处调用点全部改走该入口。

### 还没做的（下一轮）

- （已做）两个大测试文件：`game/tests.rs` 3,604 → `tests/{ai,combat,session}.rs` = **1182/1182/1092 行**
  + 168 行的索引与共享 helper；`renderer/tests_vk.rs` 1,248 → 1,248 − 354 = **897 + 359 行**（`tests_vk_side.rs`）。
- （已做）`game/session.rs` 1,302 → **1,136 行**（`view.rs` 173 行：HUD 与光照访问器）。

## 2026-09-28 收尾：**两个模块树 40 个文件全部 ≤ 1,200 行**

终审（`python -c` 遍历两棵树）：**40 个文件，超过 1200 行的 = 0**；最大 `renderer.rs` **1199**、
最小 `collisions.rs` **63**；两棵树合计 27,679 行。终局闸门：`cargo test --release` **664 passed /
0 failed**、`cargo build --release` **0 警告**、CJK 闸门 OK、真机冒烟 **`VUID=0 panics=0 fps=183.3`
ALL-OK**。

### 收尾阶段又踩到的四个坑（都是"工具语义"而不是"搬错代码"）

11. 🔴 **测试文件里的共享 helper 不能跟着搬**：`game/tests.rs` 里 118 个 `#[test]` 之外还有 2 个被多组
    复用的 helper（`explode_on` / `npc_at`）—— 按「所有 fn」分组会让另外两组找不到它（实测 23 条
    E0425）。改法：只搬 `#[test]` 项，helper 留在父文件（**父模块的私有项对子模块可见**）。
12. 🔴 **驱动器崩溃留下的陈旧 spec 会被下游照用**：分组器报错退出后，上一轮那份（含 helper 的）spec
    还在盘上，搬运工具照用 ⇒ 分组器现在开跑先删自己的输出（与搬运工具同款防线：失败就没有证据文件）。
13. 🔴 **一次搬运横跨两个文件时，回滚必须整对回滚**：平衡测试时我先从 `ai.rs` 挪 2 个测试进
    `session.rs`，随后为重做 combat 那一半而 `git checkout -- combat.rs session.rs` ——
    **session.rs 被回滚、ai.rs 的"移出"留在工作区** ⇒ 测试从 **664 掉到 662**。
    **抓住它的是"测试总数守恒"这条判据**（不是人眼）。修法：从 HEAD 恢复 `ai.rs` 后按原 spec 重放。
14. **模块声明归属**：容器本身是子模块时，新建的兄弟模块要声明在**父模块文件**里（把
    `tests_vk_side` 声明进 `tests_vk.rs` ⇒ rustc 去找 `renderer/tests_vk/tests_vk_side.rs`，E0583）；
    而**往已存在的模块文件追加代码时不该再声明一次**（实测给 `ai.rs`/`combat.rs` 各插了一条
    `mod session;`）。两者都已进工具：`--declare-in FILE` + "只声明本次新建的模块"。

