# 判据欠账台账（未验证的"反向验证"声称）

> 2026-11-26 建立。目的：**让每一条欠账都有可查的解释**，而不是留空白。
> 口径：`tools/judge_claims_verified.txt` 里记了名字 = 真跑过注入并确认变红；
> 本文列出**没记**的那些，并写明原因分类。

## 分类

| 类 | 含义 | 处理 |
|---|---|---|
| **A 需新增代码** | 声称描述的"注入"等于**写一段新行为**（不是改现有代码） | 判据是**回归护栏**，留着；接受"不可注入" |
| **B 退化输入不可判别** | 参数取到退化值时两种实现数学等价 ⇒ 注入后仍通过 | 已改声称 + 由另一条可判别判据承接 |
| **C 需构造复杂场景** | 需要多对象/多帧/跨模块编排才压得到 | 成本高于收益，留作后续 |
| **D 需运行时环境** | 要起 GPU/窗口/网络才能观测 | 本仓已有 e2e 脚本覆盖，不进单测 |

## 明细

| 判据 | 类 | 说明 |
|---|---|---|
| `scale_factor_doubles_per_handle_length` | **B** | `GIZMO_LEN == 1.0` ⇒ `delta/1.0 ≡ delta`，两种写法同值。已由 `scale_factor_is_inverse_in_handle_length_not_just_delta`（handle_len=2.0）承接 |
| `glb_skin_bind_pose_roundtrips_without_moving_vertices` | **B** | 绑定姿势下 `w·ib == I`，换序仍是单位阵。已由 `matrices_multiply_world_before_inverse_bind`（非单位 world）承接 |
| `obstacles_are_identical_with_and_without_assets` | **A** | 生成器**当前完全不读资产尺寸**（`set` 只用于日志）⇒ 声称的"改回按实测尺寸"= 新增代码。判据防的是未来改动 |
| `destructible_cover_blocks_the_blast_that_destroys_it` | **C** | 需要"障碍 AoE 与 NPC 冲击波"的**顺序**对照，得搭多刚体场景 |
| `gizmo_mouse_drag_moves_along_the_picked_axis` | **C** | 需替换 `pick_axis_at_screen` 为固定轴并保持类型/借用成立，注入面比收益大 |
| `ai_roundtrip_matches_every_field_of_every_entity` | **C** | 需要"序列化 → 反序列化 → 逐字段比对"的完整往返夹具 |
| `ai_command_chain_spawn_drag_save_load_roundtrips` | **C** | 同上，且要串起 spawn/drag/save/load 四条命令 |

## 已完成（数字）

- 起始 48 / 174（2026-11-26 早）
- 当前 **183 / 189**
- 另外**撤掉/改写**了 6 条不可兑现或结构恒真的声称
  （`ui_text_has_no_unrenderable_chars` 覆盖面、`anim_cubic_*` dt=1、`weather_channel` 自指扫描、
  `street_clutter` 的 blocked 守卫、`one_input_packet` 分工说明、`http_response_is_bounded` 的 take）
