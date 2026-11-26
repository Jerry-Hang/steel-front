# 判据欠账台账（未验证的"反向验证"声称）

> 2026-11-26 建立，同日收口。口径：`tools/judge_claims_verified.txt` 里记了名字 =
> **真跑过注入并确认变红**（`tools/injections.txt` 是可复跑清单）。
> 本文列出**没记**的那些，并写明原因 —— 目的是每条欠账都有可查解释。

## 最终数字

| 项 | 值 |
|---|---|
| 起始 | 48 / 174（28%） |
| 最终 | **187 / 189（99%）** |
| 新增验证 | **139 条** |
| 剩余 | **2 条**（都是"退化输入不可判别"） |
| 撤掉/改写不可兑现或恒真的声称 | 8 条 |
| 期间抓到的真缺陷/恒真断言 | 13 条 |

## 剩余 2 条（类 B：退化输入不可判别）

| 判据 | 为什么压不掉 | 由谁承接 |
|---|---|---|
| `scale_factor_doubles_per_handle_length` | `GIZMO_LEN == 1.0` ⇒ `delta/1.0 ≡ delta`，两种写法**数学同值** | `scale_factor_is_inverse_in_handle_length_not_just_delta`（handle_len = 2.0） |
| `glb_skin_bind_pose_roundtrips_without_moving_vertices` | 绑定姿势下 `w·ib == I`，互逆矩阵换序仍是单位阵 | `matrices_multiply_world_before_inverse_bind`（非单位 world） |

这两条**保留**（它们压的是更强的整体性质：翻倍语义、绑定姿势顶点不动），
只是"声称的反向验证"做不到 —— 已在源码注释里改成**覆盖范围说明**并指向承接判据。

## 曾经的 A / C 类：全部清空

原以为"需新增代码"或"需复杂场景"的 6 条，逐个试过之后**都能注入**：

| 判据 | 实际注入 |
|---|---|
| `obstacles_are_identical_with_and_without_assets` | 生成器**当前不读资产尺寸**（`set` 只用于日志）⇒ 声称的注入等于新增代码；判据是**防未来回归的护栏**（唯一真正的 A 类，已随声称改写并入覆盖说明） |
| `destructible_cover_blocks_the_blast_that_destroys_it` | `damage_obstacle(i, …)` 改成空操作 ⇒ 红 |
| `gizmo_mouse_drag_moves_along_the_picked_axis` | 拖动入口换成固定 `Axis::X` ⇒ 红 |
| `ai_roundtrip_matches_every_field_of_every_entity` | `apply_to` 不搬网格 ⇒ 红 |
| `ai_command_chain_spawn_drag_save_load_roundtrips` | `ctx.selection = Some(id)` 去掉（all）⇒ 红 |
| `engine_layer_does_not_grow_game_dependencies` | 往 renderer 子模块**追加** `use crate::engine::game::Npc` ⇒ 红 |

🔴 **教训**：这 6 条一开始被我按"看起来复杂"归进 C 类，实际难点几乎都是
**"找对那一行"**（缩进 + 唯一性），不是"造场景"。
⇒ **分类要按"我实际试过没有"来定，不是按印象。**

## 期间撤掉/改写的 8 条声称（假账清单）

| 判据 | 问题 |
|---|---|
| `ui_text_has_no_unrenderable_chars` | 只覆盖少数映射（`×` 改成无字模字符仍通过）⇒ 已补 `fallback_chain_never_points_at_an_unrenderable_char` |
| `anim_cubic_matches_hand_computed_hermite` | `times=[0,1]` ⇒ dt=1，漏乘 dt 不可判别 ⇒ 补 `cubic_dt_matters_when_the_segment_is_not_unit_length` |
| `weather_channel_is_declared_on_both_sides` | **自指扫描**（扫自己所在文件找自己的字符串）⇒ 结构性恒真，改成 `offset_of!` |
| `street_clutter_is_deterministic_and_clear_of_buildings` | `blocked` 守卫的落点几何压不到 ⇒ 抽出 `placement_clear_of_footprints` 单独钉住 |
| `one_input_packet_per_frame_is_the_speed_baseline` | 不是反向验证，是两条判据的**分工说明** |
| `http_response_is_bounded` | 去掉 `.take` 不红（长度检查才是语义守卫，take 是资源保护） |
| `hardcoded_gameplay_keys_are_reserved` | "任一键"不成立，只有清单里的键才压得住 |
| `blit_degenerate_inputs_return_zero_not_panic` | release 下 `u32` 下溢 wrap 被 `pixel()` 边界检查兜住 ⇒ 守卫只在 debug 可验 |

## 判据本身被修好的 13 处

`GIZMO_LEN` 退化、`dt=1` 退化、绑定姿势退化、`weather` 自指、`cell_assignment` 缺分布断言、
`feature_bit` 缺数值锁、`editor_panel_draws_a_row` 用整块面板 quad 数当代理、
`street_clutter` 守卫不可观测、`ui_text` 覆盖面不足、`inject_check` 把编译失败记成 RED、
注入打到注释（3 次）、`anim` 的 `for ch in channels` 多处只命中一处、`hardcoded_keys` 清单误读。
