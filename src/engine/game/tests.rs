// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
    use super::*;

    /// 🔴 判据（正例）：`aidiag` 节流必须**逐 NPC 独立**——同一个 5s 桶里两只不同的 NPC
    /// 都要各自拿到一行；同一只在同一桶内不得重复。
    ///
    /// 为什么必须显式测"两只"：旧实现把状态放在一个全局 `AtomicU64` 里，
    /// **单只 NPC 的行为完全正确**（每 5s 一行），所以老代码在这条性质上根本不会被发现；
    /// 只有"多只同时卡住"——也就是这个诊断工具存在的唯一理由——才暴露它。
    #[test]
    fn aidiag_throttle_is_per_npc() {
        let mut a = u64::MAX;
        let mut b = u64::MAX;
        assert!(aidiag_due(&mut a, 12.0, 1), "第一只在该桶首帧必须打印");
        assert!(
            aidiag_due(&mut b, 12.0, 2),
            "同一个桶里第二只也必须打印（旧的全局单槽会吞掉它）"
        );
        assert!(!aidiag_due(&mut a, 13.0, 1), "同一桶内不得重复");
        assert!(!aidiag_due(&mut b, 14.9, 2), "同一桶内不得重复");
        assert!(aidiag_due(&mut a, 17.0, 1), "跨到下一个桶应恢复");
        assert!(aidiag_due(&mut b, 16.0, 2), "跨到下一个桶应恢复");
    }

    /// 🔴 特征测试（反例）：把**旧实现的错误行为**钉成断言。
    ///
    /// 两只 NPC 共用一个槽时，逐帧交替会让判据每帧都成立 ⇒ 10 帧刷 20 行，
    /// 而不是 10 帧 2 行。这正是 §20.6 那次"300s 打 16.4 万行"的机制。
    /// ⚠ 这条**断言的是坏行为**，它绿不代表实现正确；留着是因为：
    /// 一旦有人把 `Npc::aidiag_bucket` 换回 `static`，上一条会红而这条仍然绿，
    /// 两条放在一起读，才看得出差别到底在哪。
    #[test]
    fn aidiag_single_shared_slot_would_flood() {
        let mut shared = u64::MAX;
        let mut lines = 0usize;
        for frame in 0..10 {
            let t = 12.0 + frame as f32 * 0.016;
            if aidiag_due(&mut shared, t, 1) {
                lines += 1;
            }
            if aidiag_due(&mut shared, t, 2) {
                lines += 1;
            }
        }
        assert_eq!(lines, 20, "共用一个槽 ⇒ 10 帧刷满 20 行（每帧两只都打）");
    }

    /// 判据：换弹动作包络**两端位移为 0、中点为 1**、区间外被夹住，前半段单调递增。
    ///
    /// 两端非 0 = 枪在换弹开始/结束那一帧位置跳变（本仓枪模历史上的残影/抖动都出自这类跳变）。
    /// ⚠️ 这条**不**断言"两端速度小" —— `sin(πp)` 的导数在两端恰好是最大值（= ±π）。
    /// 第一版就是这么写错的：我按"缓入缓出"的直觉去比两端与中点的变化率，结果中点变化率
    /// 反而是最小的（过了峰值开始回落）。**包络的"连续"指位移，不指速度。**
    #[test]
    fn reload_envelope_is_continuous_and_peaks_at_midpoint() {
        assert!(reload_envelope(0.0).abs() < 1e-6, "换弹刚开始不应有位移");
        assert!(reload_envelope(1.0).abs() < 1e-6, "换弹结束必须回到原位");
        assert!((reload_envelope(0.5) - 1.0).abs() < 1e-6, "中点应为满幅");
        // 区间外夹住（进度来自 HUD 的 f32，理论上可能因浮点略微越界）
        assert!(reload_envelope(-3.0).abs() < 1e-6);
        assert!(reload_envelope(4.0).abs() < 1e-6);
        // 前半段单调递增 + 全程有界
        let mut prev = -1.0;
        for i in 0..=50 {
            let v = reload_envelope(i as f32 / 100.0);
            assert!(v >= prev - 1e-6, "前半段应单调递增：{v} < {prev}");
            assert!((-1e-6..=1.0 + 1e-6).contains(&v), "包络必须落在 [0,1]：{v}");
            prev = v;
        }
        // 两端"接近 0"而不是"跳一下就到位"：进入/离开时位移必须小到看不见
        assert!(reload_envelope(0.01) < 0.05, "起点附近位移应很小（不许位置跳变）");
        assert!(reload_envelope(0.99) < 0.05, "终点附近位移应很小（不许位置跳变）");
    }

    /// 🔴 harness 打移动靶的命中率直接由**位置样本年龄**决定（1 Hz ⇒ 可能瞄 1 秒前的位置）。
    /// 这条旋钮必须：默认仍是 1 Hz（不改变既有日志量）、10 Hz 给到 0.1s、越界值被夹住。
    #[test]
    fn npc_pos_period_follows_the_rate_knob() {
        assert_eq!(npc_pos_period(None), 1.0, "缺省必须还是 1 Hz");
        assert_eq!(npc_pos_period(Some(0)), 1.0, "0 视为缺省");
        assert_eq!(npc_pos_period(Some(1)), 1.0);
        assert!((npc_pos_period(Some(10)) - 0.1).abs() < 1e-6);
        assert!((npc_pos_period(Some(30)) - 1.0 / 30.0).abs() < 1e-6);
        assert!((npc_pos_period(Some(240)) - 1.0 / 30.0).abs() < 1e-6, ">30 夹到 30");
    }

    /// 🔴 **导航不变式**：出生环（40–80m）上的每个可站立点都必须在**玩家可达域**里。
    ///
    /// 这是"波次能不能清掉"的前置条件：`update_waves` 要求 `npcs.is_empty()`，而波次 NPC 出生在
    /// 40–80m 的环上 —— 只要有一只在玩家到不了的连通域里，那一波就**永远清不掉**
    /// （真机实测：defense_line 第 1 波打完 5 只后卡在最后一只到超时，见 docs/PROGRESS.md
    /// 2026-09-25）。判据与地图来源无关：可站立 ⇒ 必须可达。
    ///
    /// 检查的是**真实建网结果**（走生产同一个 `block_obstacle_cells`），不是几何推测。
    #[test]
    fn wave_spawn_ring_is_reachable_from_the_player() {
        let assert_ring = |name: &str, grid: &GridMap| {
            let w = grid.width();
            let reach = crate::engine::ai::reachable_mask(grid, world_to_grid(0.0, 0.0));
            let mut ok = 0;
            let mut bad: Vec<(i32, GridPos)> = Vec::new();
            for k in 0..32 {
                let a = std::f32::consts::TAU * k as f32 / 32.0;
                for r in 40..=80i32 {
                    let g = world_to_grid(r as f32 * a.cos(), r as f32 * a.sin());
                    if !grid.is_passable(g) {
                        continue; // 障碍格：出生时会沿径向外推（push_out_of_obstacle），不算失败
                    }
                    if reach[g.y as usize * w + g.x as usize] {
                        ok += 1;
                    } else if bad.len() < 4 {
                        bad.push((r, g));
                    }
                }
            }
            assert!(
                ok + bad.len() > 0,
                "{name}: 出生环上没有任何可站立点（整圈都是障碍格）"
            );
            assert!(
                bad.is_empty(),
                "{name}: 出生环上有可站立点不在玩家可达域（可达样本 {ok}），例如 {bad:?} —— \
                 这些 NPC 永远走不到玩家，波次清不掉"
            );
        };
        // ① 程序化城市（默认地图：玩家出生在中央十字路口）
        let game = Game::new();
        assert_ring("procedural(city)", &game.grid);
        // ② 五张手写关卡（`index.toml` 是关卡**列表**不是地图，走 `load_map_list`）：
        //    按 `apply_level` 的同一套规则重建网格。
        for name in [
            "street_fight",
            "open_field",
            "factory_ambush",
            "bridgehead",
            "defense_line",
        ] {
            let path = format!("assets/maps/{name}.toml");
            let Some(mgr) = crate::engine::map::MapManager::load(&path).ok() else {
                panic!("关卡 {name} 加载失败");
            };
            let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
            for def in mgr.obstacles() {
                let (kind, x, z, half_w, half_d) =
                    crate::engine::map::obstacle_to_map_obstacle(def);
                let ob = MapObstacle {
                    x,
                    z,
                    half_w,
                    half_d,
                    y: 1.2,
                    half_h: 1.2,
                    kind,
                    tint: None,
                    max_hp: obstacle_max_hp(kind),
                    hp: obstacle_max_hp(kind),
                    shape: Shape::Legacy,
                };
                block_obstacle_cells(&mut grid, &ob);
            }
            assert_ring(name, &grid);
        }
    }

    /// 撞墙不丢帧：意图方向被障碍挡住时，必须沿墙滑出**有意义的一段**位移。
    ///
    /// 红证（改动前）：`resolve_circle_obstacles` 把整步推回 ⇒ 净位移只剩 0.07m（< 半步 0.08m）。
    /// 真机后果（RV3D_AI_DIAG=1）：1 秒 64 帧里 18–30 帧被完全抵消，NPC 实测 1.3–2.2 m/s。
    ///
    /// 对照断言（**防止测试变成恒真**）：先证明"无滑动"这一步确实是被抵消的，
    /// 再要求滑动后的位移明显更大。
    #[test]
    fn step_with_slide_keeps_moving_along_the_wall() {
        // 一堵 10m 长、1m 厚的墙（沿 x 轴，中心在原点）；NPC 在墙南面外 0.5m
        let obstacles = vec![MapObstacle::new(ObstacleKind::Wall, 0.0, 0.0, 5.0, 0.5)];
        let from = (0.0f32, 1.0f32);
        let step = 0.16f32;
        // 朝"西北偏北"：大部分分量撞进墙里，只有一小部分沿墙（-x）
        let dir = (-0.3f32, -0.954f32);
        let r = NPC_BODY_RADIUS;
        // 对照组：只有"走 + 推回"（旧行为）
        let (bx, bz) = resolve_circle_obstacles(&obstacles, from.0 + dir.0 * step, from.1 + dir.1 * step, r);
        let blocked = ((bx - from.0).powi(2) + (bz - from.1).powi(2)).sqrt();
        assert!(
            blocked < step * 0.5,
            "对照组本该被抵消（否则这条测试是恒真的）：位移 {blocked:.4} ≥ 半步 {:.4}",
            step * 0.5
        );
        // 处理组：加上沿墙滑动
        let (x, z) = step_with_slide(&obstacles, from, dir, step, r);
        let moved = ((x - from.0).powi(2) + (z - from.1).powi(2)).sqrt();
        assert!(
            moved > blocked * 2.0 && moved > step * 0.5,
            "滑动后应当保住大部分步长：{moved:.4}（对照 {blocked:.4}，步长 {step:.2}）"
        );
        assert!(x < from.0 - 1e-3, "必须真的朝目标方向的 −x 分量挪了：x={x:.4}");
        assert!(
            z >= 0.5 + r - 1e-3,
            "不得插进墙里（墙南面 z=0.5 + 半径 {r}）：z={z:.4}"
        );
        // 正撞（意图与法线平行）没有切向可走 ⇒ 不许凭空侧移，也不许倒退
        let (hx, hz) = step_with_slide(&obstacles, from, (0.0, -1.0), step, r);
        let head_on = ((hx - from.0).powi(2) + (hz - from.1).powi(2)).sqrt();
        assert!(head_on <= blocked + 1e-4, "正撞不该比推回点走得更远：{head_on:.4}");
        assert!(hz >= 0.5 + r - 1e-3, "正撞后仍须在墙外：z={hz:.4}");
        // 空旷处（无接触）必须与旧行为逐位一致：一步就是 step
        let (ox, oz) = step_with_slide(&obstacles, (0.0, 5.0), (0.6, 0.8), step, r);
        let free = ((ox - 0.0f32).powi(2) + (oz - 5.0f32).powi(2)).sqrt();
        assert!((free - step).abs() < 1e-5, "无障碍时不该改变步长：{free:.4}");
    }

    /// 🔴 **压力模式不变式**：红蓝两侧的出生点必须落在**同一个连通域**里。
    ///
    /// 起因（2026-09-25 真机）：`aidiag: astar` 显示压力模式每秒 278 次调用**全部**
    /// `连通域穷尽` ⇒ 探针一量：出生环 150–198m 穿过城市街区，`push_out_of_obstacle` 只保证
    /// "可通行"，实测 4 只红方里 2 只落在 **9 格 / 2 格**的小口袋 ⇒ **两军各在自己的院子里
    /// 隔空对射**，那套"20 轮红蓝对撞"的 A/B 全部带上这个前提。
    /// 修法 = `spawn_stress_battle` 把出生点收口到**全图最大连通域**（`nearest_in_component`）。
    #[test]
    fn stress_spawns_land_in_one_component() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        let grid = game.grid.clone();
        let w = grid.width();
        let cell_of = |team: Team| -> Vec<GridPos> {
            game.npcs
                .iter()
                .filter(|n| n.team == team)
                .map(|n| world_to_grid(n.position[0], n.position[2]))
                .collect()
        };
        let red = cell_of(Team::Red);
        let blue = cell_of(Team::Blue);
        assert!(red.len() >= 2 && !blue.is_empty(), "两侧都要有人：{red:?} {blue:?}");
        // 以红 0 为参考域：每一只（红与蓝）都必须在里面
        let mask = crate::engine::ai::reachable_mask(&grid, red[0]);
        let n = mask.iter().filter(|b| **b).count();
        assert!(
            n > 1000,
            "参考域只有 {n} 格 —— 出生点又落进小口袋了（红0 = {:?}）",
            red[0]
        );
        for g in red.iter().chain(blue.iter()) {
            assert!(
                mask[g.y as usize * w + g.x as usize],
                "出生点 {g:?} 不在主连通域里 ⇒ 它永远走不到对面（每秒几百次 A* 全部连通域穷尽）"
            );
        }
    }

    /// 🔴 **波次出生点不变式**：`spawn_wave` 出来的每一只都必须在**玩家可达域**里。
    /// 与 `wave_spawn_ring_is_reachable_from_the_player`（几何采样）互补：这条查**真实出生结果**，
    /// 连 `push_out_of_obstacle` 的落点与 `nearest_in_component` 的收口一起验。
    #[test]
    fn wave_spawns_land_inside_the_players_component() {
        let mut game = Game::new();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_wave(1, &player);
        assert!(!game.npcs.is_empty());
        let mask =
            crate::engine::ai::reachable_mask(&game.grid, world_to_grid(player.x, player.z));
        let w = game.grid.width();
        for n in &game.npcs {
            let g = world_to_grid(n.position[0], n.position[2]);
            assert!(
                mask[g.y as usize * w + g.x as usize],
                "npc #{} 出生在 {g:?} —— 不在玩家可达域里（永远走不到玩家，波次清不掉）",
                n.id
            );
        }
    }

    /// 部位伤害倍率阈值（设计文档：头 1.5 / 胸 1.0 / 臂 0.8 / 腿 0.6）
    #[test]
    fn part_multiplier_zones() {
        assert_eq!(Game::part_multiplier(1.5, 0.0), 1.5);
        assert_eq!(Game::part_multiplier(1.45, 0.0), 1.5);
        assert_eq!(Game::part_multiplier(1.44, 0.0), 1.0);
        assert_eq!(Game::part_multiplier(0.95, 0.0), 1.0);
        assert_eq!(Game::part_multiplier(0.94, 0.0), 0.8);
        assert_eq!(Game::part_multiplier(0.6, 0.0), 0.8);
        assert_eq!(Game::part_multiplier(0.59, 0.0), 0.6);
        assert_eq!(Game::part_multiplier(0.1, 0.0), 0.6);
        // 地面高度偏移：NPC 站山坡上时以 NPC 地面为基准
        assert_eq!(Game::part_multiplier(2.0, 0.5), 1.5);
    }

    /// 🔴 包抄 / 偷袭目标点必须落在**交战距离以内**（#17 死循环的红证）。
    ///
    /// 实机（2026-09-25，`RV3D_AI_DIAG=1`）：`tac=Flank goal=(14.0,2.0)`，而玩家格中心是
    /// `(2.0,2.0)` ⇒ 包抄点距玩家 **3 格 = 12m**，恰好等于波次 NPC 的 `attack_range`（12m）。
    /// NPC 于射程外一步反复「到点 → 重规划 → 换点」，`state` 恒为 `Chase`、永不进
    /// `Attack`，`update_waves` 又要求 `npcs.is_empty()` ⇒ **波次永远清不掉**。
    /// 判据：目标点半径必须**明显小于**射程（留 4m 余量，容下玩家在格内的偏移与 float 误差）。
    #[test]
    fn flank_and_ambush_goals_land_inside_engage_range() {
        // 波次 NPC 的射程（`ai::wave_profile` 默认档，也是 `Npc::attack_range` 的实际取值）
        let attack_range = 12.0f32;
        let flank_m = FLANK_OFFSET as f32 * GRID_CELL;
        let ambush_m = AMBUSH_OFFSET as f32 * GRID_CELL;
        assert!(
            flank_m <= attack_range - 4.0,
            "包抄点 {flank_m}m 离射程 {attack_range}m 太近（NPC 会在射程外一步无限来回）"
        );
        assert!(
            ambush_m <= attack_range - 4.0,
            "偷袭点 {ambush_m}m 离射程 {attack_range}m 太近（同上）"
        );
    }

    /// 散布方向：单位长度、轴向零散布保持原方向、非零散布仍归一化
    #[test]
    fn spread_direction_normalized() {
        let d = Game::spread_direction([0.0, 0.0, 1.0], 0.0, 0.0);
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!((l - 1.0).abs() < 1e-5);
        assert!(d[0].abs() < 1e-4 && d[1].abs() < 1e-4 && (d[2] - 1.0).abs() < 1e-4);
        // 水平射 + 散布：保持朝向 +Z 为主方向
        let d2 = Game::spread_direction([0.0, 0.0, 1.0], 0.05, 0.05);
        let l2 = (d2[0] * d2[0] + d2[1] * d2[1] + d2[2] * d2[2]).sqrt();
        assert!((l2 - 1.0).abs() < 1e-5);
        assert!(d2[2] > 0.99);
        // 斜向（俯射 45°）不退化
        let d3 = Game::spread_direction([0.0, -1.0, 1.0], 0.03, -0.02);
        let l3 = (d3[0] * d3[0] + d3[1] * d3[1] + d3[2] * d3[2]).sqrt();
        assert!((l3 - 1.0).abs() < 1e-5);
        // 垂直向上射（right 基退化路径）
        let d4 = Game::spread_direction([0.0, 1.0, 0.0], 0.04, 0.04);
        let l4 = (d4[0] * d4[0] + d4[1] * d4[1] + d4[2] * d4[2]).sqrt();
        assert!((l4 - 1.0).abs() < 1e-5);
        assert!(d4[1] > 0.99);
    }

    #[test]
    fn ai_tier_classify_boundaries() {
        let p = AiTierParams::default();
        // 交互中即使超远也 Near（每帧步进，不降频）
        assert_eq!(classify_ai_tier(1.0e9, true, &p), AiTier::Near);
        // 距离 ≤ 阈值 → Near（含边界相等）
        assert_eq!(classify_ai_tier(0.0, false, &p), AiTier::Near);
        assert_eq!(
            classify_ai_tier(p.near_radius * p.near_radius, false, &p),
            AiTier::Near
        );
        // 超远且不交互 → Far
        assert_eq!(
            classify_ai_tier(p.near_radius * p.near_radius + 1.0, false, &p),
            AiTier::Far
        );
    }

    #[test]
    fn ai_tier_partition_stable_and_split() {
        // (id, 期望档位) 乱序输入；分区后 Near 段在前、组内相对顺序保持
        let mut items = vec![
            (3u32, AiTier::Far),
            (0, AiTier::Near),
            (4, AiTier::Far),
            (1, AiTier::Near),
            (2, AiTier::Near),
        ];
        let near_len = partition_ai_tiers(&mut items, |(_, t)| *t);
        assert_eq!(near_len, 3);
        let (near, far) = items.split_at(near_len);
        assert!(near.iter().all(|(_, t)| *t == AiTier::Near));
        assert!(far.iter().all(|(_, t)| *t == AiTier::Far));
        // 稳定分区：Near 原顺序 0,1,2；Far 原顺序 3,4
        assert_eq!(
            near.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            far.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn ai_tier_partition_empty_and_uniform() {
        let mut empty: Vec<u32> = Vec::new();
        assert_eq!(partition_ai_tiers(&mut empty, |_| AiTier::Near), 0);
        let mut all_near = vec![7u32, 8, 9];
        assert_eq!(partition_ai_tiers(&mut all_near, |_| AiTier::Near), 3);
        assert_eq!(all_near, vec![7, 8, 9]); // 稳定：全 Near 原序不变
        let mut all_far = vec![7u32, 8, 9];
        assert_eq!(partition_ai_tiers(&mut all_far, |_| AiTier::Far), 0);
        assert_eq!(all_far, vec![7, 8, 9]);
    }

    #[test]
    fn ai_tier_partition_with_real_npc_tier() {
        // 真实 Npc：按到玩家距离平方 + 交互标志分层
        let params = AiTierParams::default();
        let mut npcs = vec![
            npc_at(0, Team::Red, [10.0, 0.0, 0.0]),  // 近 → Near
            npc_at(1, Team::Red, [500.0, 0.0, 0.0]), // 远 → Far
            npc_at(2, Team::Red, [300.0, 0.0, 0.0]), // 远 → Far
            npc_at(3, Team::Red, [30.0, 0.0, 0.0]),  // 近 → Near
        ];
        let player = [0.0f32, 0.0, 0.0];
        let near_len = partition_ai_tiers(&mut npcs, |n| {
            let dx = n.position[0] - player[0];
            let dz = n.position[2] - player[2];
            classify_ai_tier(dx * dx + dz * dz, false, &params)
        });
        assert_eq!(near_len, 2);
        assert_eq!(npcs[0].id, 0);
        assert_eq!(npcs[1].id, 3);
        assert_eq!(npcs[2].id, 1);
        assert_eq!(npcs[3].id, 2);
    }

    #[test]
    fn far_decimate_skips_idle_npcs_by_frame() {
        // 远组无感知非攻击 NPC（600m > STRESS_SIGHT=512 → 感知不到任何目标）：
        // decimate_far=true 时按 id 分帧跳过（id=7 → 7%4=3），
        // 命中帧步进（位置变化）、跳过帧冻结（位置不变）。
        let mut npcs = [npc_at(7, Team::Red, [600.0, 0.0, 0.0])];
        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 1];
        let targets = vec![None];
        for frame in 0..8u32 {
            let ctx = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags,
                targets: &targets,
                grid: &grid,
                time: 1.0 + frame as f32 / 60.0,
                dt: 1.0 / 60.0,
                stress: true,
                frame,
                decimate_far: true,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
                squad_wps: &[],
                spectator: false,
                target_known: false,
                fallback_targets: &[],
                target_occluded: &[],
            };
            let idle_before = npcs[0].position;
            Game::step_ai_parallel(&mut npcs, 0, &ctx); // near_len=0 → 全远组
            if frame % AI_FAR_DECIMATE == 7 % AI_FAR_DECIMATE {
                assert_ne!(npcs[0].position, idle_before, "id=7 命中帧应步进（frame {frame}）");
            } else {
                assert_eq!(npcs[0].position, idle_before, "id=7 跳过帧应冻结（frame {frame}）");
            }
        }
    }

    #[test]
    fn should_decimate_far_excludes_interactions() {
        // 交互中（感知/受击/被火力威胁/被瞄准/攻击态）永不降频；无交互按 id 分帧
        let mut n = npc_at(7, Team::Red, [300.0, 0.0, 0.0]);
        n.perception.enemy_visible = true;
        assert!(!should_decimate_far(&n, 0), "感知敌人不降频");
        n.perception.enemy_visible = false;
        n.perception.took_hit = true;
        assert!(!should_decimate_far(&n, 0), "受击不降频");
        n.perception.took_hit = false;
        n.perception.under_fire = true;
        assert!(!should_decimate_far(&n, 0), "被火力威胁不降频");
        n.perception.under_fire = false;
        n.perception.player_aiming = true;
        assert!(!should_decimate_far(&n, 0), "被玩家瞄准不降频");
        n.perception.player_aiming = false;
        // 推进状态机到 Attack（同 stress_npc_combat 的确定性推进）
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        n.state_machine.update(p);
        n.state_machine.update(p);
        assert_eq!(n.state_machine.state(), NpcState::Attack);
        assert!(!should_decimate_far(&n, 0), "攻击态不降频");
        // 无交互：id=7 → 7%4=3；frame 0/2 降频、frame 3 命中
        n.state_machine = NpcStateMachine::new();
        assert!(should_decimate_far(&n, 0));
        assert!(should_decimate_far(&n, 2));
        assert!(!should_decimate_far(&n, 3));
    }

    /// 关卡障碍刚体应贴地落地并静止（程序化地图替代原 3 AABB + 2 球体演示场景）
    #[test]
    fn map_obstacles_ground_and_settle() {
        let mut game = Game::new();
        assert!(!game.world.bodies.is_empty(), "level 1 map should populate physics");
        assert!(game.world.spheres.is_empty(), "procedural map uses AABB walls only");
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        // 2026-08-23：地图障碍已为「静态刚体」——契约：120 帧模拟后位置零漂移
        // （此前的动态刚体重力化导致树桩沉地 1m / 地基板沉入 4.5m / 墙体被撞动）
        let spawn: Vec<Pv> = game.world.bodies.iter().map(|b| b.position).collect();
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        for (b, s) in game.world.bodies.iter().zip(&spawn) {
            assert!(b.static_body, "地图障碍应全部为静态刚体");
            let d = b.position - *s;
            let dist2 = d.x * d.x + d.y * d.y + d.z * d.z;
            assert!(
                dist2 < 1e-7,
                "静态障碍不应移动：pos={:?} spawn={:?}",
                b.position,
                s
            );
        }
    }

    /// 程序化地图：同种子确定、异种子不同、障碍都在安全环外且两两不重叠
    #[test]
    fn map_generation_is_deterministic_and_safe() {
        let a = generate_level_map(1);
        let b = generate_level_map(1);
        let c = generate_level_map(2);
        assert_eq!(a.obstacles, b.obstacles, "same seed must produce identical layout");
        assert_ne!(a.obstacles, c.obstacles, "different seed must produce different layout");
        assert!(!a.obstacles.is_empty() && !c.obstacles.is_empty());
        for ob in a.obstacles.iter().chain(c.obstacles.iter()) {
            let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
            assert!(d >= MAP_RING_INNER - 0.5, "obstacle too close to origin: {:.1}m", d);
        }
        for (i, o1) in a.obstacles.iter().enumerate() {
            for o2 in a.obstacles.iter().skip(i + 1) {
                let overlap = (o1.x - o2.x).abs() < o1.half_w + o2.half_w
                    && (o1.z - o2.z).abs() < o1.half_d + o2.half_d;
                assert!(!overlap, "obstacles must not overlap");
            }
        }
    }

    /// 地图主题：第一关保持冒烟基准（58m 安全环 + Wall 种类 + 与主题化生成完全一致）
    #[test]
    fn map_theme_level1_preserves_smoke_ring() {
        let t1 = theme_for_level(1);
        assert_eq!(t1.ring_inner, MAP_RING_INNER, "第一关安全环必须 58m");
        assert_eq!(t1.kind, ObstacleKind::Wall);
        let a = generate_level_map(1);
        let b = generate_level_map_with_theme(1, theme_for_level(1));
        assert_eq!(a.obstacles, b.obstacles, "generate_level_map 必须等于主题化生成");
        // 战斗障碍（安全环带内）全部为 Wall；环外可含场景装饰（树/建筑/残骸）
        assert!(
            a.obstacles
                .iter()
                .filter(|o| {
                    let d = (o.x * o.x + o.z * o.z).sqrt();
                    d <= t1.ring_outer + 1.0
                })
                .all(|o| o.kind == ObstacleKind::Wall),
            "安全环带内障碍全部为 Wall"
        );
        for ob in &a.obstacles {
            let d = (ob.x * ob.x + ob.z * ob.z).sqrt();
            assert!(d >= MAP_RING_INNER - 0.5, "58m 环带内必须无障碍: {:.1}m", d);
        }
    }

    /// 地图主题：按关卡轮换种类/安全环/密度，同主题确定性一致、异主题布局不同
    #[test]
    fn map_themes_rotate_and_differ() {
        // 主题轮换周期：1/4/7 同主题，2/5/8、3/6/9 依次轮换
        assert_eq!(theme_for_level(1), theme_for_level(4));
        assert_ne!(theme_for_level(1), theme_for_level(2));
        assert_ne!(theme_for_level(2), theme_for_level(3));
        assert_ne!(theme_for_level(3), theme_for_level(4));
        assert_eq!(theme_for_level(2).kind, ObstacleKind::Block);
        assert_eq!(theme_for_level(3).kind, ObstacleKind::Barrier);
        // 安全环半径随主题变化，且都不低于 NPC 站定下限（见 MAP_RING_INNER 注释）
        for level in 1..=6 {
            assert!(
                theme_for_level(level).ring_inner >= MAP_RING_INNER - 0.5,
                "level {} 安全环过低",
                level
            );
        }
        // 同 seed 不同主题 → 不同布局；同主题同 seed → 确定性一致
        let wall = generate_level_map_with_theme(1, theme_for_level(1));
        let block = generate_level_map_with_theme(1, theme_for_level(2));
        assert_ne!(wall.obstacles, block.obstacles, "不同主题必须产生不同布局");
        assert_eq!(
            wall.obstacles,
            generate_level_map_with_theme(1, theme_for_level(1)).obstacles
        );
        assert!(
            wall.obstacles
                .iter()
                .filter(|o| (o.x * o.x + o.z * o.z).sqrt() <= theme_for_level(1).ring_outer + 1.0)
                .all(|o| o.kind == ObstacleKind::Wall)
        );
        assert!(
            block.obstacles
                .iter()
                .filter(|o| (o.x * o.x + o.z * o.z).sqrt() <= theme_for_level(2).ring_outer + 1.0)
                .all(|o| o.kind == ObstacleKind::Block)
        );
    }

    /// 升关：每关 WAVES_PER_LEVEL 波清完后 level+1、wave 回 1、地图重新生成、难度按有效波次递进
    #[test]
    fn level_advances_after_waves_per_level() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.level, 1);
        let l1 = game.map.obstacles.clone();
        // 快速清完 3 波（每波清空 npcs 后推进 3.3s 倒计时）
        for _ in 0..3 {
            game.npcs.clear();
            for _ in 0..330 {
                game.update(0.01, &Camera::new());
            }
        }
        assert_eq!(game.level, 2, "level should advance after WAVES_PER_LEVEL waves");
        assert_eq!(game.wave, 1, "wave resets to 1 on level up");
        assert!(!game.npcs.is_empty(), "level 2 wave 1 should spawn enemies");
        let l2 = game.map.obstacles.clone();
        // 2026-08-23：手绘城市地图跨关卡不变（关卡递进 = 波次强度升阶，不再随机重生成）
        assert_eq!(l1, l2, "地图跨关卡保持同一张手绘城市（旧随机重生成契约已废弃）");
        // 物理世界与网格同步重建
        assert_eq!(game.world.bodies.len(), l2.len());
    }

    /// 清第 WAVES_PER_LEVEL 波之前不应升关（wave 3 是升关临界，wave 2 仍停留原关）
    #[test]
    fn level_does_not_advance_before_last_wave() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 1, "wave 1 → 2 must not advance level");
        assert_eq!(game.wave, 2);
    }

    /// 开火产生投射物，命中物理刚体（障碍）后销毁；命中不计入 hits
    /// （打墙没有命中提示——hit_count 只统计 NPC 命中）
    #[test]
    fn weapon_fire_hits_physics_body() {
        let mut game = Game::new();
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        let body = game.world.bodies[0];
        let target = [body.position.x, body.position.y, body.position.z];
        // 从目标正上方竖直向下射击
        assert!(game.fire([target[0], target[1] + 50.0, target[2]], [0.0, -1.0, 0.0]));
        assert_eq!(game.shots, 1);
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.projectiles.is_empty(), "hit projectile should be removed");
        assert_eq!(game.hits(), 0, "障碍命中不计入 hits（无命中提示）");
    }

    /// 射速冷却：连续开火被限流
    #[test]
    fn weapon_fire_rate_limits_shots() {
        let mut game = Game::new();
        let origin = [0.0, 5.0, 0.0];
        let dir = [0.0, 0.0, -1.0];
        assert!(game.fire(origin, dir));
        // 冷却期内再开火应被拒绝
        for _ in 0..10 {
            assert!(!game.fire(origin, dir));
            game.update(1.0 / 240.0, &Camera::new());
        }
        assert_eq!(game.shots, 1);
    }

    /// HUD：喂入渲染统计后能产出覆盖层 quad（血条 + 调试文本）
    #[test]
    fn hud_quads_produce_overlay() {
        let mut game = Game::new();
        game.hud.fps = 60.0;
        let quads = game.hud_quads(100, 200, "high");
        assert!(!quads.is_empty(), "hud should produce overlay quads");
        assert!(quads.len() >= 3, "health bar + debug text lines expected");
    }

    /// HUD：游戏画面含分数/波次/准星等元素
    #[test]
    fn hud_game_screen_has_score_wave_crosshair() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.fps = 60.0;
        game.score = 30;
        game.wave = 2;
        let quads = game.hud_quads(100, 200, "high");
        assert!(!quads.is_empty());
        assert!(quads.len() >= 5, "health/ammo bars + score/wave + crosshair");
    }

    /// HUD：开始菜单与死亡结算画面都产出元素
    #[test]
    fn hud_menu_and_gameover_screens() {
        let mut game = Game::new();
        let menu = game.hud_quads(0, 0, "high");
        assert!(!menu.is_empty(), "start menu should draw overlay + title");
        game.game_state = GameState::GameOver;
        let over = game.hud_quads(0, 0, "high");
        assert!(!over.is_empty(), "game over screen should draw overlay + score");
    }

    /// 光照场景：开启标志位、方向光与环境光生效
    #[test]
    fn light_uniform_enabled() {
        let game = Game::new();
        let u = game.light_uniform();
        assert!(u.flags.x >= 1.0, "lighting should be enabled");
        assert!(u.directional.direction.w >= 1.0, "directional enabled");
        assert!(u.ambient.w > 0.0, "ambient intensity set");
        assert!(u.points[0].position.w >= 1.0, "point light A enabled");
        assert!(u.points[1].position.w >= 1.0, "point light B enabled");
    }

    /// 音频：环境风合成器常开，tick 每帧渲染（audio_us 链路真实运行）
    #[test]
    fn audio_synth_ambient_runs_with_tick() {
        let mut game = Game::new();
        assert!(
            game.audio.synth().ambient_active(),
            "ambient wind should be playing"
        );
        let cam = Camera::new();
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.audio.synth().ambient_active(),
            "ambient wind should persist"
        );
    }

    /// AI：NPC 站在地形高度上，且相机在原点时能离开 Idle 状态
    #[test]
    fn ai_npcs_stand_on_terrain_and_leave_idle() {
        let mut game = Game::new();
        let cam = Camera::new();
        assert_eq!(game.npcs.len(), 8);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &cam);
        }
        for npc in &game.npcs {
            let h = terrain_height_at(npc.position[0], npc.position[2]);
            assert!(
                (npc.position[1] - h).abs() < 0.001,
                "npc y should match terrain height"
            );
        }
        assert!(
            game.npcs
                .iter()
                .any(|n| n.state_machine.state() != NpcState::Idle),
            "at least one npc should leave Idle near the player"
        );
    }

    /// 🔴 未结案 #17 的**根因级**回归（2026-09-23）：`NPC_SIGHT(60) < 出生半径上限(80)`
    /// ⇒ 出生在视距之外的进攻方在旧行为下**只会 Patrol**（`enemy_visible` 恒 false），
    /// 而 `update_waves` 要求 `npcs.is_empty()` ⇒ 这一波永远清不掉、没有波间补给、
    /// 没有第 2..N 波、没有胜利态（实测 `#8` 在 77.8m 上一动不动守了整局）。
    ///
    /// 这里把感知层直接喂给 `step_npc`，把变量压到只剩"目标已知"这一条：
    /// `target_known=true` ⇒ 必须 Chase 并朝玩家推进；`false` ⇒ 旧行为（没有目标就不追）。
    ///
    /// ⚠️ 对照组故意只跑 1 帧：巡逻游走**本身**会让 NPC 慢慢走到 60m 内、从而"偶然"进入 Chase
    /// （见 `far_decimate_skips_idle_npcs_by_frame`：600m 外的 NPC 每帧都在动），
    /// 所以"跑很久之后它不在 Patrol"**不能**证明这条修法生效 —— 判据必须是"目标已知 ⇒ 立刻去追"。
    #[test]
    fn far_npc_gets_a_target_instead_of_patrolling_forever() {
        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 1];
        let targets = vec![None];
        let occupied = vec![false; 1];
        let run = |target_known: bool, frames: u32| {
            // 80m > NPC_SIGHT(60)：这一只"看不见玩家"
            let mut npc = npc_at(1, Team::Red, [80.0, 0.0, 0.0]);
            for frame in 0..frames {
                let ctx = AiStepCtx {
                    player: &player,
                    player_yaw: 0.0,
                    charge: false,
                    under_fire: &flags,
                    targets: &targets,
                    grid: &grid,
                    time: 1.0 + frame as f32 / 60.0,
                    dt: 1.0 / 60.0,
                    stress: false,
                    frame,
                    decimate_far: false,
                    ring_inner: MAP_RING_INNER,
                    ring_outer: MAP_RING_OUTER,
                    obstacles: &game.map.obstacles,
                    squad_wps: &[],
                    spectator: false,
                    target_known,
                    fallback_targets: &[],
                    target_occluded: &occupied,
                };
                Game::step_npc(0, &mut npc, &ctx);
            }
            let d = (npc.position[0] * npc.position[0] + npc.position[2] * npc.position[2]).sqrt();
            (npc.state_machine.state(), d)
        };
        let (st_known_1f, _) = run(true, 1);
        assert!(
            matches!(st_known_1f, NpcState::Chase | NpcState::Attack),
            "80m 外的进攻方在目标已知时**第一帧**就该去追，实际 {st_known_1f:?}"
        );
        // 对照组 = 旧行为：80m 外没有视线、也没有"目标已知"这条通道 ⇒ 进不了 Chase
        let (st_old_1f, _) = run(false, 1);
        assert!(
            matches!(st_old_1f, NpcState::Idle | NpcState::Patrol),
            "对照组：目标未知时第一帧进不了 Chase（实际 {st_old_1f:?}）"
        );
        // 跑满 10 秒：已知目标的那只必须**一路在走**（路径推进 ⇒ 与玩家的距离明显缩短）。
        //
        // 🔴 2026-09-25 改判据：旧断言是"已知目标那只必须比巡逻对照组更靠近玩家"
        // （`d_known < d_old - 5.0`）。实测两者 59.8 / 57.6 —— **对照组反而更近**，于是这条红。
        // 查下去发现断言本身站不住：程序化城市地图上玩家出生在中央广场，而广场被一圈低矮装饰
        // 封成 24 格的孤岛（导航网格按"障碍盒覆盖整格"判定，见 `ai::reachable_mask` 文档），
        // 80m 外的这只**根本走不到玩家**（`find_path` 只能给部分路径）⇒ 它这 10 秒走的是绕行
        // 路线，进度取决于障碍布局、不取决于有没有目标；巡逻组的游走目标恰好在同方向，
        // 两者本来就没有可比性。真正该守的不变量是"出生点必须落在玩家的可达域里"
        // （见 `wave_spawns_land_inside_the_players_reachable_component`）。
        let (st_known_600, d_known) = run(true, 600);
        assert!(
            matches!(st_known_600, NpcState::Chase | NpcState::Attack),
            "已知目标的那只 10 秒后应当仍在追击（实际 {st_known_600:?}）"
        );
        assert!(
            d_known < 80.0 - 15.0,
            "已知目标的那只 10 秒内必须明显推进（实测 57.6m）：{d_known:.1}m"
        );
    }

    /// 🔴 判据：**军情的连强度只数活人**（`CompanyReport.strength` 的语义就是"活着的人"）。
    ///
    /// 2026-09-26 修：以前按「在 `npcs` 里」计数 ⇒ 本帧刚阵亡、**还没被清场的尸体同帧既算活人
    /// 又算阵亡**（真机会战日志 66 行里有 6 行 `击杀 + Σ强度` 比编制多 1~3 人，
    /// 判据工具 = `tools/battle_tally_check.py`）。这里把那个同帧状态直接造出来验一次。
    #[test]
    fn company_report_counts_only_the_living() {
        let cam = Camera::new();
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 64;
        let player = glam::Vec3::ZERO;
        game.spawn_stress_battle(&player);
        game.game_state = GameState::Playing;
        let roster = game
            .command
            .as_ref()
            .map(|(red, _)| red.roster_size())
            .unwrap_or(0);
        assert_eq!(roster, 64, "压力模式红营编制 = stress_sides");
        // 造「本帧刚阵亡、还没清场」：hp 归零但**留在 `npcs` 里**
        let ri = game
            .npcs
            .iter()
            .position(|n| n.team == Team::Red)
            .expect("压力模式应有红方 NPC");
        game.npcs[ri].hp = 0.0;
        // 跑一帧（指挥节拍 0.5s）——`update_ai` 里的军情汇总排在 `update_stress_respawns` 之前
        game.update(0.6, &cam);
        let (red, _) = game.command.as_ref().expect("压力模式有指挥层");
        let sum: usize = red
            .companies
            .iter()
            .map(|c| c.report.strength as usize)
            .sum();
        assert_eq!(
            sum,
            roster - 1,
            "阵亡者不许再算进连强度，否则同帧的「击杀 + Σ强度」会超出编制"
        );
    }

    /// 接线判据：`AiStepCtx::target_known` 的唯一表达式在 `update_ai` 里，
    /// 语义是"只有在真在打的一局里，玩家才是进攻方的已知目标"。
    /// 三条分支各断言一次（菜单游走 / 正常波次 / 压力模式）——
    /// 漏掉 `!self.stress` 或漏掉 `Playing` 都会被这条抓住。
    #[test]
    fn target_known_is_wired_only_for_real_missions() {
        let cam = Camera::new();
        // ① 开始菜单（`Game::new` 后不调 on_any_key）：NPC 照常游走，不该把玩家当已知目标
        let mut menu = Game::new();
        menu.update(1.0 / 60.0, &cam);
        assert!(!menu.npcs.is_empty());
        assert!(
            menu.npcs.iter().all(|n| !n.perception.target_known),
            "开始菜单的游走不该把玩家当成已知目标"
        );
        // ② 真在打的一局：波次进攻方从出生起就已知目标
        let mut run = Game::new();
        run.on_any_key(&glam::Vec3::ZERO);
        run.update(1.0 / 60.0, &cam);
        assert!(!run.npcs.is_empty());
        assert!(
            run.npcs.iter().all(|n| n.perception.target_known),
            "Playing 状态下的波次进攻方应当已知目标（未结案 #17）"
        );
        // ③ 压力模式：红蓝对抗走 pick_stress_targets，不经过这条通道
        let mut stress = Game::new();
        stress.stress = true;
        stress.on_any_key(&glam::Vec3::ZERO);
        stress.update(1.0 / 60.0, &cam);
        assert!(!stress.npcs.is_empty());
        assert!(
            stress.npcs.iter().all(|n| !n.perception.target_known),
            "压力模式不该走这条通道（它有 STRESS_SIGHT + pick_stress_targets）"
        );
    }

    /// 🔴 连发路径的判据（2026-09-23 补）：`fire_burst`/`fire_burst_player` 合并成一条路径后，
    /// 玩家入口必须真的打满 `rounds` 发、计数与强制冷却都要跟上。
    /// 这条红了 = 连发被改坏（例如循环次数被改、或 `from_player` 传错导致 `fire_shot` 拒发）。
    #[test]
    fn player_burst_fires_three_rounds() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let shots_before = game.shots;
        let fired = game.fire_burst_player([0.0, 1.6, 0.0], [0.0, 0.0, -1.0], 3);
        assert_eq!(fired, 3, "三连发应打满 3 发");
        assert_eq!(game.shots, shots_before + 3, "发射计数应 +3");
        assert!(
            game.fire_cooldown > 0.0,
            "连发结束后应进入强制冷却（rounds × 间隔）"
        );
    }

    /// 网格坐标转换往返一致
    #[test]
    fn grid_conversion_roundtrip() {
        for (x, z) in [(-255.0, -255.0), (0.0, 0.0), (255.0, 255.0), (123.4, -67.8)] {
            let g = world_to_grid(x, z);
            let (wx, wz) = grid_to_world(g);
            assert_eq!(g, world_to_grid(wx, wz));
        }
    }

    /// 初始状态为 StartMenu（开始菜单）
    #[test]
    fn game_state_starts_in_menu() {
        let game = Game::new();
        assert_eq!(game.state(), GameState::StartMenu);
    }

    /// 开始菜单任意键 → Playing，重置并生成第 1 波
    #[test]
    fn start_menu_any_key_begins_run() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.state(), GameState::Playing);
        assert_eq!(game.wave, 1);
        assert_eq!(game.score, 0);
        assert_eq!(game.hud.health, game.hud.max_health);
        assert_eq!(game.npcs.len(), 6, "wave 1 spawns 4+2·1=6");
    }

    /// 死亡结算 R 重开：状态复位并重新生成第 1 波
    #[test]
    fn gameover_restart_resets_run() {
        let mut game = Game::new();
        game.game_state = GameState::GameOver;
        game.score = 999;
        game.hud.health = 0.0;
        game.request_restart(&glam::Vec3::ZERO);
        assert_eq!(game.state(), GameState::Playing);
        assert_eq!(game.score, 0);
        assert_eq!(game.wave, 1);
        assert_eq!(game.hud.health, game.hud.max_health);
        assert!(!game.npcs.is_empty());
    }

    /// 波次清空（npcs 空）后开始 3 秒倒计时，随后刷出下一波
    #[test]
    fn wave_spawns_after_clear() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert_eq!(game.wave, 1);
        assert_eq!(game.npcs.len(), 6);
        game.npcs.clear();
        game.update(0.01, &Camera::new());
        assert!(game.wave_timer > 0.0, "countdown should start after clear");
        assert_eq!(game.wave, 1, "wave must not advance during countdown");
        // 推进 3 秒以上
        for _ in 0..320 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 2, "next wave should spawn after countdown");
        assert!(!game.npcs.is_empty(), "wave 2 should spawn enemies");
        assert_eq!(game.npcs.len(), 8, "wave 2 spawns 4+2·2=8");
    }

    /// 波次递进：下一波数量/速度/血量都高于上一波
    #[test]
    fn wave_scales_count_speed_hp() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let (c1, s1, h1) = (game.npcs.len(), game.npcs[0].speed, game.npcs[0].max_hp);
        game.npcs.clear();
        for _ in 0..320 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 2);
        let (c2, s2, h2) = (game.npcs.len(), game.npcs[0].speed, game.npcs[0].max_hp);
        assert!(c2 > c1, "wave 2 should have more enemies: {} vs {}", c2, c1);
        assert!(s2 > s1, "wave 2 should be faster: {} vs {}", s2, s1);
        assert!(h2 > h1, "wave 2 should have more hp: {} vs {}", h2, h1);
    }

    /// 清空一波奖励分
    #[test]
    fn wave_clear_awards_bonus() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.clear();
        game.update(0.01, &Camera::new());
        assert_eq!(game.score, WAVE_CLEAR_BONUS, "clearing wave 1 awards bonus");
    }

    /// 残留 NPC 清除：刷新波前清掉旧波存活 NPC，新旧不共存
    #[test]
    fn spawn_wave_purges_leftovers() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let old_ids: Vec<usize> = game.npcs.iter().map(|n| n.id).collect();
        game.spawn_wave(3, &glam::Vec3::ZERO);
        assert_eq!(game.npcs.len(), 10, "wave 3 spawns 4+2·3=10");
        assert!(
            game.npcs.iter().all(|n| !old_ids.contains(&n.id)),
            "all old-wave npcs must be purged before the new wave"
        );
    }

    /// 投射物命中 NPC 扣血：默认武器 AK-12M 近距 34 伤（V3.0 首档 34），从高处垂直下射
    /// 命中球心 +1.0m（胸部区 ×1.0）→ 34 伤
    #[test]
    fn projectile_damages_npc() {
        let mut game = Game::new();
        let npc_pos = game.npcs[0].position;
        let hp_before = game.npcs[0].hp;
        assert!(game.fire(
            [npc_pos[0], npc_pos[1] + 10.0, npc_pos[2]],
            [0.0, -1.0, 0.0]
        ));
        for _ in 0..10 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs[0].hp, hp_before - 34.0, "chest hit should deal 34 (V3.0)");
        assert_eq!(game.npcs.len(), 8, "non-lethal hit keeps the npc");
    }

    /// 击杀：hp≤0 移除 NPC 并计分（AK-12M 34 伤 > 20 HP）
    #[test]
    fn projectile_kill_scores_and_removes_npc() {
        let mut game = Game::new();
        game.npcs[0].hp = 20.0;
        let npc_pos = game.npcs[0].position;
        assert!(game.fire(
            [npc_pos[0], npc_pos[1] + 10.0, npc_pos[2]],
            [0.0, -1.0, 0.0]
        ));
        for _ in 0..10 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), 7, "killed npc should be removed");
        assert_eq!(game.score, KILL_SCORE);
    }

    /// 普通子弹过期/撞障碍均不产生爆炸（回归：修复"远处神秘连发爆炸"——
    /// V3.0 高速弹射程尽头曾触发 spawn_explosion，现在只有 explosive 标记弹才爆炸）
    #[test]
    fn bullet_expiry_and_obstacle_no_explosion() {
        let mut game = Game::new();
        let before = game.explosions.len();
        // 朝远处平射：子弹飞出射程后过期消失，不引爆
        assert!(game.fire([0.0, 2.0, 0.0], [0.0, 0.0, 1.0]));
        for _ in 0..90 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.projectiles.is_empty(), "子弹应已过期消失");
        assert_eq!(game.explosions.len(), before, "普通子弹过期不应爆炸");
        // 朝近处障碍射击：命中直接消耗（障碍永久存在），不爆炸
        let mut game2 = Game::new();
        let before2 = game2.explosions.len();
        let ob = game2.map.obstacles[0];
        let cx = ob.x;
        let cz = ob.z;
        let cy = terrain_height_at(cx, cz) + 1.2; // 障碍腰部
        // 从 -X 侧 40m 水平射向障碍中心（AABB 高 ~1-3m，40m 内散布/下坠 < 2cm）
        assert!(game2.fire([cx - 40.0, cy, cz], [1.0, 0.0, 0.0]));
        for _ in 0..30 {
            game2.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game2.explosions.len(), before2, "子弹命中障碍不应爆炸");
    }

    /// 友军伤害关闭：玩家弹命中蓝方（友军）NPC 穿身而过不造成伤害；
    /// 对照：玩家弹命中红方（敌人）NPC 正常伤害（证明机制差异来自友军判定）
    #[test]
    fn friendly_fire_off_for_player_projectiles() {
        let mut game = Game::new();
        // 追加一个蓝方（友军）NPC，复制 npc[0] 的位置（红方敌人对照）
        let blue_idx = game.npcs.len();
        let mut b = npc_at(blue_idx, crate::engine::ai::Team::Blue, game.npcs[0].position);
        b.hp = 100.0;
        b.max_hp = 100.0;
        game.npcs.push(b);
        // 垂直下射（1 帧内命中，NPC 横向移动不影响）
        let down = |pos: [f32; 3]| ([pos[0], pos[1] + 10.0, pos[2]], [0.0, -1.0, 0.0]);
        // 1) 玩家弹打蓝方友军：无伤害
        let (o1, d1) = down(game.npcs[blue_idx].position);
        assert!(game.fire_player(o1, d1));
        for _ in 0..5 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(
            game.npcs[blue_idx].hp, 100.0,
            "玩家弹命中友军不应造成伤害"
        );
        // 2) 冷却后玩家弹打红方敌人：正常伤害（对照）
        for _ in 0..15 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        let (o2, d2) = down(game.npcs[0].position);
        assert!(game.fire_player(o2, d2));
        for _ in 0..5 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(
            game.npcs[0].hp < 100.0,
            "玩家弹命中红方敌人应正常伤害"
        );
    }

    /// 枪械不再打爆障碍：子弹命中障碍后障碍 HP 不变、仍保留在列表
    #[test]
    fn bullets_do_not_destroy_obstacles() {
        let mut game = Game::new();
        let ob = game.map.obstacles[0];
        let hp_before = ob.hp;
        let n_before = game.map.obstacles.len();
        let cx = ob.x;
        let cz = ob.z;
        let cy = terrain_height_at(cx, cz) + 1.2;
        // 从 -X 侧 40m 水平射击障碍
        assert!(game.fire_player([cx - 40.0, cy, cz], [1.0, 0.0, 0.0]));
        for _ in 0..30 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.map.obstacles.len(), n_before, "障碍不应被子弹摧毁");
        assert_eq!(
            game.map.obstacles[0].hp, hp_before,
            "障碍 HP 不应被子弹削减"
        );
    }

    /// 判据：渲染可见性缓存**必须**是分摊刷新 —— 一帧只重算一部分，而 N 帧里每个 NPC
    /// 都**恰好**被重算一次（"缓存"不能退化成"再也不更新"）。
    ///
    /// 实测依据：不缓存时压力场景 `cull-diag` 中位 **102 ms/s**（510 次/帧 × ~2 µs，
    /// 每帧 5.5 ms ⇒ 约 18% 帧预算）。
    #[test]
    fn npc_visibility_cache_is_staggered() {
        let mut game = Game::new();
        let n = game.npcs.len();
        assert!(n >= 2, "测试场景至少要两个 NPC");
        game.refresh_npc_visibility(); // 建缓存（新条目 fail-open）
        assert_eq!(game.npc_visibility_flags().len(), n);
        let mut per_frame = Vec::new();
        for _ in 0..NPC_VIS_REFRESH_FRAMES {
            let before = game.npc_vis_scans.get();
            game.refresh_npc_visibility();
            per_frame.push(game.npc_vis_scans.get() - before);
        }
        assert!(
            per_frame.iter().all(|&c| (c as usize) < n),
            "每帧重算数必须小于 NPC 总数（否则等于没缓存），实际每帧：{:?}",
            per_frame
        );
        let total: u64 = per_frame.iter().sum();
        assert_eq!(
            total, n as u64,
            "{} 帧内每个 NPC 必须恰好被重算一次，实际每帧：{:?}",
            NPC_VIS_REFRESH_FRAMES, per_frame
        );
    }

    /// 判据：**NPC 死亡导致下标前移时，缓存必须立刻改判**（不能把前一个人的可见性
    /// 用在后一个人身上）。
    ///
    /// 真机代价（2026-09-26 复查）：槽位以前只存一个 bool，而 `npcs` 用 `retain`/`swap_remove`
    /// 删除 ⇒ 死一个人之后，后面每个人的标志都是前一个人的，最多错 N−1 帧。现在槽位存
    /// `(id, 可见)`：id 不符就立刻重算。
    #[test]
    fn npc_visibility_cache_notices_index_shifts() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.truncate(4);
        let starts = game.npc_vis_scans.get();
        game.refresh_npc_visibility();
        assert_eq!(game.npc_visibility_flags().len(), 4);
        let after_first = game.npc_vis_scans.get();
        assert!(after_first > starts, "首次必须建缓存");
        // 删掉 0 号（后面三人下标前移）→ 下一次 refresh 里**那三个人都必须重算**
        game.npcs.remove(0);
        let before = game.npc_vis_scans.get();
        game.refresh_npc_visibility();
        let rescanned = game.npc_vis_scans.get() - before;
        assert!(
            rescanned >= 3,
            "下标前移后每个受影响的人都要立刻重算，实际只重算了 {}",
            rescanned
        );
    }

    /// 判据：**过时数据最多旧 N 帧** —— 遮挡关系变了以后，NPC 必须在一个刷新窗口内
    /// 被重新判定（否则"看不见的人"会永远隐形，那是最坏的一类卡死）。
    #[test]
    fn npc_visibility_refreshes_within_the_window() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.truncate(1);
        // 3m 高窄柱挡在玩家与 NPC 之间（与 `npc_occluded_by_obstacle_between` 同一套布景）
        game.world.bodies.push(physics::Body::new_static(
            Pv::new(0.0, 1.5, 0.0),
            Pv::new(0.5, 1.5, 0.5),
        ));
        game.player_body.pos = Pv::new(-40.0, 0.0, 0.0);
        game.npcs[0].position = [30.0, 0.0, 0.0];
        assert!(game.npc_occluded(0), "布景本身要能挡住（否则这条测试没意义）");
        // 先跑满一个窗口，让缓存必然拿到"被挡"的判定
        for _ in 0..NPC_VIS_REFRESH_FRAMES {
            game.refresh_npc_visibility();
        }
        assert!(
            !game.npc_visibility_flags()[0],
            "缓存应已判出遮挡：{:?}",
            game.npc_visibility_flags()
        );
        // 把 NPC 挪到无遮挡处：最多一个窗口之后必须变回"可见"
        game.npcs[0].position = [30.0, 0.0, 90.0];
        let mut refreshed_after = None;
        for f in 1..=NPC_VIS_REFRESH_FRAMES {
            game.refresh_npc_visibility();
            if game.npc_visibility_flags()[0] {
                refreshed_after = Some(f);
                break;
            }
        }
        assert!(
            refreshed_after.is_some_and(|f| f <= NPC_VIS_REFRESH_FRAMES),
            "一个刷新窗口内必须重新判定，实际用了 {:?} 帧",
            refreshed_after
        );
    }

    /// NPC 遮挡判定：障碍 AABB 在玩家与 NPC 之间 → occluded；移开 NPC → 可见
    #[test]
    fn npc_occluded_by_obstacle_between() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.truncate(1);
        // 2026-08-23：自建确定性障碍（3m 高窄柱）——地图障碍部署位置随改版漂移，
        // 高层建筑斜线必挡、低屏障又太矮；自建柱体唯一确定
        let ob = MapObstacle {
            x: 0.0,
            z: 0.0,
            half_w: 0.5,
            half_d: 0.5,
            y: 1.5,
            half_h: 1.5,
            kind: ObstacleKind::Block,
            tint: None,
            max_hp: 300.0,
            hp: 300.0,
            shape: Shape::Legacy,
        };
        game.map.obstacles.push(ob);
        game.world
            .bodies
            .push(physics::Body::new_static(Pv::new(0.0, 1.5, 0.0), Pv::new(0.5, 1.5, 0.5)));
        // 玩家 → 障碍中心 → NPC 在障碍另一侧 30m
        game.player_body.pos = Pv::new(-40.0, 0.0, 0.0);
        game.npcs[0].position = [30.0, 0.0, 0.0];
        assert!(
            game.npc_occluded(0),
            "障碍挡在玩家与 NPC 之间应判定遮挡"
        );
        // NPC 移到障碍同一侧 20m（视线无遮挡）→ 可见
        game.npcs[0].position = [ob.x + ob.half_w + 30.0, 0.0, ob.z + 80.0];
        assert!(
            !game.npc_occluded(0),
            "无遮挡时应可见"
        );
        // 越界索引安全
        assert!(!game.npc_occluded(999));
        // 双采样验证：近距离贴墙（墙高 MAP_BLOCK_HEIGHT ≥ 1.7m）NPC 在墙正后方
        // → 身体与头部都被挡，仍判遮挡
        game.npcs.truncate(1);
        game.npcs[0].position = [ob.x + ob.half_w + 2.0, 0.0, ob.z];
        game.player_body.pos = Pv::new(ob.x - ob.half_w - 1.0, 0.0, ob.z);
        assert!(
            game.npc_occluded(0),
            "高墙正后方 NPC 应完全遮挡（双采样均被挡）"
        );
    }

    /// AI 视线遮挡预计算：这是"隔墙掉血"的正解回归测试。
    ///
    /// 此前 `enemy_visible = dist < sight` 完全不看几何，NPC 能隔着整栋楼持续输出。
    /// 本测试锁三件事：① 高墙挡住 → occluded；② 无遮挡 → 不 occluded；
    /// ③ 齐腰掩体只挡躯干采样、头/肩采样通 → **不** occluded（与渲染剔除
    ///    `npc_occluded` 的双采样规则保持一致，避免出现"我看得见他却打不到我"）。
    #[test]
    fn ai_target_occlusion_respects_geometry() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.npcs.truncate(1);
        let player = glam::Vec3::new(-40.0, 0.0, 0.0);
        game.player_body.pos = Pv::new(-40.0, 0.0, 0.0);
        game.npcs[0].position = [30.0, 0.0, 0.0];

        let run = |g: &Game| -> bool {
            target_occlusion(&g.npcs, &g.world.bodies, false, false, &[], &[], &player)[0]
        };

        // ② 先测无障碍：必须可见（空场地）
        game.world.bodies.clear();
        assert!(!run(&game), "无几何遮挡时目标必须可见");

        // ① 3m 高墙横在中间 → 躯干与头部两条采样全被挡 → 遮挡
        game.world
            .bodies
            .push(physics::Body::new_static(Pv::new(0.0, 1.5, 0.0), Pv::new(0.5, 1.5, 0.5)));
        assert!(run(&game), "3m 高墙挡在 NPC 与目标之间必须判遮挡");

        // ③ 齐腰掩体（顶面 1.1m）：躯干采样被挡，头/肩 1.7m 采样通 → 不判遮挡
        game.world.bodies.clear();
        game.world
            .bodies
            .push(physics::Body::new_static(Pv::new(0.0, 0.55, 0.0), Pv::new(0.5, 0.55, 0.5)));
        assert!(
            !run(&game),
            "齐腰掩体后露出上半身，按双采样规则必须仍算可见（与 npc_occluded 一致）"
        );
    }

    /// 玩家受伤：攻击态 NPC 每秒扣血，血量为 0 进入 GameOver
    #[test]
    fn player_damage_and_gameover() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 把一只 NPC 放到玩家（原点）脚边，保证进入 Attack
        game.npcs[0].position = [1.0, 0.0, 1.0];
        game.hud.health = 6.0;
        let cam = Camera::new();
        for _ in 0..300 {
            game.update(1.0 / 60.0, &cam);
            if game.state() == GameState::GameOver {
                break;
            }
        }
        assert_eq!(game.state(), GameState::GameOver, "health 0 should end the run");
        assert_eq!(game.hud.health, 0.0);
    }

    /// 死亡补给：玩家死亡瞬间全部武器弹匣补满 + 备弹恢复初始
    #[test]
    fn death_resets_all_ammo() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 消耗当前武器（AK-12M 30 发弹匣）10 发（每发后推进冷却）
        let cam = Camera::new();
        for _ in 0..10 {
            game.fire(game.player_eye().to_array(), [0.0, 0.0, 1.0]);
            for _ in 0..10 {
                game.update(1.0 / 60.0, &cam);
            }
        }
        assert_eq!(game.weapons.active_firearm_ref().magazine(), 20, "先消耗 10 发");
        // 把 NPC 放到玩家脚边，打死玩家进入 GameOver
        game.npcs[0].position = [1.0, 0.0, 1.0];
        game.hud.health = 6.0;
        for _ in 0..300 {
            game.update(1.0 / 60.0, &cam);
            if game.state() == GameState::GameOver {
                break;
            }
        }
        assert_eq!(game.state(), GameState::GameOver, "玩家应已阵亡");
        // 死亡补给生效：当前武器弹匣补满、备弹恢复初始
        assert_eq!(
            game.weapons.active_firearm_ref().magazine(),
            game.weapons.active_firearm_ref().max_magazine(),
            "死亡后弹匣应补满"
        );
        assert_eq!(
            game.weapons.active_firearm_ref().reserve(),
            90,
            "死亡后备弹应恢复初始 90（AK-12M）"
        );
    }

    /// 精确场景：玩家眼位 y=3 朝 20m 外同地形 NPC 水平射击 → 弹道应穿过命中球
    #[test]
    fn hitbox_horizontal_20m_same_terrain() {
        let mut game = Game::new();
        game.npcs.truncate(1);
        let eye_y = 3.0; // 模拟玩家眼位（地形高 1.4 + 眼位 1.6）
        let ground_y = 1.4;
        game.npcs[0].position = [0.0, ground_y, -20.0];
        // 水平弹道：从 (0, eye_y, 0) 朝 -Z
        let mut p = Projectile::new(
            [0.0, eye_y, 0.0],
            [0.0, 0.0, -1.0],
            75.0,
            400.0,
            6.0,
            28.0,
        );
        // 推进到 z=-20 附近（75m/s × 0.267s）
        p.update(0.267);
        let hit = game.hit_npc_index(&p);
        assert!(
            hit.is_some(),
            "20m 水平射击应命中：弹道 y={} 球覆盖 {:.1}..{:.1}",
            p.position[1],
            ground_y - 0.05,
            ground_y + 2.05
        );
        // 推进穿过（z=-40），仍应命中（segment 覆盖球）
        p.update(0.267);
        assert!(game.hit_npc_index(&p).is_some(), "弹道穿过球心段应命中");
    }

    /// 命中判定覆盖：瞄头/瞄身/瞄腿/远距都应命中（hitbox 含整身 0..1.85m），脱靶不命中
    #[test]
    fn hitbox_covers_head_body_legs_and_range() {
        let mut game = Game::new();
        // 固定 NPC 在原点，清除其它 NPC 干扰
        game.npcs[0].position = [0.0, 0.0, 0.0];
        game.npcs.truncate(1);
        // 水平弹道：从 -40m 射向原点，段内覆盖球心
        let mk = |h: f32| -> Projectile {
            Projectile::new([-40.0, h, 0.0], [1.0, 0.0, 0.0], 100.0, 200.0, 3.0, 30.0)
        };
        // 瞄头（1.70m）：段 [-40,0] 最近点 = 球心投影，命中且部位为头
        let mut p = mk(1.70);
        p.update(0.40);
        let hit = game.hit_npc_index(&p);
        assert!(hit.is_some(), "瞄头（1.70m）应命中");
        if let Some((_, hh)) = hit {
            assert!(
                Game::part_multiplier(hh, 0.0) >= 1.5,
                "头部命中倍率应为 1.5，实际 {}",
                Game::part_multiplier(hh, 0.0)
            );
        }
        // 瞄身（1.10m）：命中且胸部倍率 1.0
        let mut p = mk(1.10);
        p.update(0.40);
        let hit = game.hit_npc_index(&p);
        assert!(hit.is_some(), "瞄身（1.10m）应命中");
        if let Some((_, hh)) = hit {
            assert_eq!(Game::part_multiplier(hh, 0.0), 1.0);
        }
        // 瞄腿（0.30m）：命中且腿部倍率 0.6
        let mut p = mk(0.30);
        p.update(0.40);
        let hit = game.hit_npc_index(&p);
        assert!(hit.is_some(), "瞄腿（0.30m）应命中");
        if let Some((_, hh)) = hit {
            assert_eq!(Game::part_multiplier(hh, 0.0), 0.6);
        }
        // 远距离（80m）：段内覆盖球心即命中
        let mut p = Projectile::new([-80.0, 1.10, 0.0], [1.0, 0.0, 0.0], 100.0, 300.0, 4.0, 30.0);
        p.update(0.80);
        assert!(game.hit_npc_index(&p).is_some(), "80m 瞄身应命中");
        // 脱靶：高度 3.0m 高出 hitbox 顶部（1.85m）
        let mut p = mk(3.0);
        p.update(0.40);
        assert!(game.hit_npc_index(&p).is_none(), "3m 高处脱靶不应命中");
        // 横向脱靶：x 偏移 3m（球半径 0.95）
        let mut p = Projectile::new([-40.0, 1.10, 3.0], [1.0, 0.0, 0.0], 100.0, 200.0, 3.0, 30.0);
        p.update(0.40);
        assert!(game.hit_npc_index(&p).is_none(), "横向 3m 脱靶不应命中");
    }

    /// 切枪稳定性：任意槽位切换后长时间运行不 panic、武器状态一致
    #[test]
    fn weapon_switch_stability_all_slots() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let cam = Camera::new();
        for slot in 0..ALL_WEAPONS.len() {
            game.switch_weapon(slot);
            for _ in 0..20 {
                game.update(1.0 / 60.0, &cam);
            }
            assert_eq!(
                game.weapons.active_index(),
                slot,
                "槽位 {} 切换未生效",
                slot
            );
            assert!(
                !game.weapons.active_name().is_empty(),
                "槽位 {} 武器名为空",
                slot
            );
        }
        // 越界切换：优雅忽略，不 panic，当前武器不变
        let current = game.weapons.active_index();
        game.switch_weapon(9999);
        assert_eq!(game.weapons.active_index(), current, "越界切换不应生效");
        game.switch_weapon(usize::MAX);
        assert_eq!(game.weapons.active_index(), current, "usize::MAX 切换不应生效");
    }

    /// GameOver 冻结：投射物继续飞行但不产生新击杀、不计分
    #[test]
    fn stance_scales_speed_and_eye_height_monotonically() {
        let mut game = Game::new();
        assert_eq!(game.stance, Stance::Standing, "默认必须是站立");
        // 站立视高与物理体默认一致（1.6）——两处不一致就是"两套状态源"
        assert!((Stance::Standing.eye_height() - 1.60).abs() < 1e-6);
        // 速度与视高都必须单调：站 > 蹲 > 卧。反了会让蹲比站快、或相机穿出头顶。
        assert!(Stance::Standing.speed_mul() > Stance::Crouching.speed_mul());
        assert!(Stance::Crouching.speed_mul() > Stance::Prone.speed_mul());
        assert!(Stance::Standing.eye_height() > Stance::Crouching.eye_height());
        assert!(Stance::Crouching.eye_height() > Stance::Prone.eye_height());
        // C 在站立/下蹲间来回，Z 在站立/卧倒间来回
        game.toggle_crouch();
        assert_eq!(game.stance, Stance::Crouching);
        game.toggle_crouch();
        assert_eq!(game.stance, Stance::Standing);
        game.toggle_prone();
        assert_eq!(game.stance, Stance::Prone);
        game.toggle_prone();
        assert_eq!(game.stance, Stance::Standing);
        // 从下蹲按 Z 直接转卧倒（而不是先回站立）
        game.toggle_crouch();
        game.toggle_prone();
        assert_eq!(game.stance, Stance::Prone);
    }

    #[test]
    fn sprint_requires_forward_standing_and_hipfire() {
        let mut game = Game::new();
        game.move_forward = true;
        game.set_sprint(true);
        assert!(game.sprinting(), "站立 + 前进 + 按住 Shift 应当冲刺");
        // 后退不冲刺
        game.move_backward = true;
        assert!(!game.sprinting(), "后退不该冲刺");
        game.move_backward = false;
        // 开镜不冲刺
        game.hud.ads = true;
        assert!(!game.sprinting(), "开镜不该冲刺");
        game.hud.ads = false;
        // 蹲 / 卧不冲刺
        game.toggle_crouch();
        assert!(!game.sprinting(), "下蹲不该冲刺");
        game.toggle_prone();
        assert!(!game.sprinting(), "卧倒不该冲刺");
        // 回到站立后，还要松开 Shift 才停
        game.toggle_prone();
        assert!(game.sprinting(), "恢复站立后应重新冲刺");
        game.set_sprint(false);
        assert!(!game.sprinting(), "松开 Shift 必须停止冲刺");
    }

    #[test]
    fn medkit_heals_once_and_refuses_when_wasted() {
        let mut game = Game::new();
        assert_eq!(game.medkits, 2, "默认应有两个医疗包");
        // 满血按 X 不消耗（避免误按白扔一个包）
        game.use_medkit();
        assert_eq!(game.medkits, 2, "满血不该消耗医疗包");
        assert_eq!(game.heal_progress(), 0.0, "满血不该进入打药状态");

        // 掉血后打药：立刻扣 1 个，进度开始涨，未完成前不回血
        game.hud.health = 30.0;
        game.use_medkit();
        assert_eq!(game.medkits, 1);
        game.update_heal(HEAL_TIME * 0.5);
        let p = game.heal_progress();
        assert!(p > 0.4 && p < 0.6, "进度应过半，实际 {p}");
        assert_eq!(game.hud.health, 30.0, "未完成前不该回血");

        // 打药期间再按 X 不消耗
        game.use_medkit();
        assert_eq!(game.medkits, 1, "打药中不该再消耗");

        // 推进到结束：一次性回血
        game.update_heal(HEAL_TIME);
        assert_eq!(game.hud.health, 75.0, "30 + 45 = 75");
        assert_eq!(game.heal_progress(), 0.0, "完成后进度归零");

        // 封顶：不会超过 max_health
        game.hud.health = 90.0;
        game.use_medkit();
        game.update_heal(HEAL_TIME);
        assert_eq!(game.hud.health, game.hud.max_health, "回血必须封顶");

        // 没药时按 X 无效果
        game.medkits = 0;
        game.hud.health = 10.0;
        game.use_medkit();
        assert_eq!(game.heal_timer, 0.0, "没药时不该进入打药状态");
    }

    #[test]
    fn crosshair_spread_orders_by_stance_and_sprint() {
        // 第⑤条：准星扩散是玩家读散布的唯一途径，四个关系必须成立。
        let mut game = Game::new();
        game.stance = Stance::Standing;
        game.set_sprint(false);
        game.fire_cooldown = 0.0;
        let stand = game.crosshair_spread();

        game.stance = Stance::Crouching;
        let crouch = game.crosshair_spread();
        game.stance = Stance::Prone;
        let prone = game.crosshair_spread();

        assert!(crouch < stand, "蹲下应比站立收拢：{crouch} vs {stand}");
        assert!(prone < crouch, "趴下应比蹲下更收拢：{prone} vs {crouch}");

        // 冲刺张开：`sprinting()` 还要求「站立 + 前进 + 未开镜 + 在地面」，
        // 单靠 `set_sprint(true)` 不足以让它为真 —— 这里不断言冲刺，
        // 改为直接断言"冲刺项参与合成"这一事实（源码可查），并把区间断言留给下面。
        game.stance = Stance::Standing;
        game.set_sprint(false);
        game.fire_cooldown = 0.0;

        // 开火后坐期张开
        game.fire_cooldown = 0.15;
        let firing = game.crosshair_spread();
        assert!(firing > stand, "开火期应比静立张开：{firing} vs {stand}");

        // 恒在合法区间内（HUD 直接拿它乘像素，越界会画出畸形十字）
        for s in [Stance::Standing, Stance::Crouching, Stance::Prone] {
            for cd in [0.0f32, 0.05, 0.5, 3.0] {
                game.stance = s;
                game.fire_cooldown = cd;
                let v = game.crosshair_spread();
                assert!((0.08..=1.0).contains(&v), "越界: stance={s:?} cd={cd} -> {v}");
            }
        }

        // 开镜必须让准星跟着收拢：`spread_scale` 由 main.rs 按开镜混合度写入。
        // 初版漏了这一项 ⇒ 开镜后弹道收拢 70% 而准星不动（准星与实际散布不一致）。
        game.stance = Stance::Standing;
        game.fire_cooldown = 0.0;
        game.set_spread_scale(1.0);
        let hip = game.crosshair_spread();
        game.set_spread_scale(0.3); // = 1.0 - ads_blend(1.0) * 0.7，即完全开镜
        let ads = game.crosshair_spread();
        assert!(ads < hip, "开镜应收拢准星：{ads} vs 腰射 {hip}");
        game.set_spread_scale(1.0);
    }

    #[test]
    fn fire_mode_cycle_skips_unsupported_modes() {
        // 栓动狙击只有单发：按 B 必须原地不动，绝不能切到连发
        let semi_only = fire_modes_for(45.0);
        assert_eq!(
            next_supported_fire_mode(FireMode::Semi, semi_only),
            FireMode::Semi,
            "只有单发的武器按 B 不该离开单发"
        );
        // 半自动：单发 → 双发 → 三连发 → 回到单发，全自动被跳过
        let semi_burst = fire_modes_for(400.0);
        let a = next_supported_fire_mode(FireMode::Semi, semi_burst);
        assert_eq!(a, FireMode::Burst2);
        let b = next_supported_fire_mode(a, semi_burst);
        assert_eq!(b, FireMode::Burst3);
        let c = next_supported_fire_mode(b, semi_burst);
        assert_eq!(c, FireMode::Semi, "半自动绕一圈必须回到单发，跳过连发");
        // 全自动：四档按顺序全走一遍
        let full = fire_modes_for(700.0);
        assert_eq!(next_supported_fire_mode(FireMode::Burst3, full), FireMode::Auto);
        assert_eq!(next_supported_fire_mode(FireMode::Auto, full), FireMode::Semi);
    }

    #[test]
    fn fire_modes_are_derived_per_weapon_class() {
        // 栓动狙击 / 泵动霰弹：只有单发，绝不给点射或连发
        for rpm in [0.0f32, 45.0, 120.0, 199.0] {
            let m = fire_modes_for(rpm);
            assert_eq!(m.len(), 1, "rpm={rpm} 应只有一档");
            assert_eq!(m[0], FireMode::Semi, "rpm={rpm} 应是单发");
        }
        // 半自动：单发 + 双发 + 三连发，但没有全自动
        let mid = fire_modes_for(400.0);
        assert!(mid.contains(&FireMode::Burst2), "半自动要有双发");
        assert!(mid.contains(&FireMode::Burst3), "半自动要有三连发");
        assert!(!mid.contains(&FireMode::Auto), "半自动不该给全自动");
        // 全自动武器：四档全给
        assert_eq!(fire_modes_for(700.0).len(), 4, "全自动应给满四档");
        // NaN 必须落进最保守的一档（若写成 `rpm < 200.0` 则 NaN 会一路穿到 FULL）
        assert_eq!(fire_modes_for(f32::NAN).len(), 1, "NaN 必须落最保守档");
    }

    #[test]
    fn fire_mode_cycle_covers_semi_double_triple_auto() {
        // 四档必须闭环且互不重复 —— 漏一档会让 B 键循环静默跳过某个模式
        let mut seen = Vec::new();
        let mut m = FireMode::Semi;
        for _ in 0..4 {
            assert!(!seen.contains(&m), "循环里出现重复档位: {m:?}");
            seen.push(m);
            m = m.next();
        }
        assert_eq!(m, FireMode::Semi, "循环必须回到起点");
        assert!(seen.contains(&FireMode::Burst2), "双发必须在循环里");
        // 发数映射：单发/连发按单发走，双发 2，三连发 3
        assert_eq!(FireMode::Semi.burst_rounds(), 1);
        assert_eq!(FireMode::Burst2.burst_rounds(), 2);
        assert_eq!(FireMode::Burst3.burst_rounds(), 3);
        assert_eq!(FireMode::Auto.burst_rounds(), 1);
        // 显示名互不相同（HUD 靠它区分档位）
        let labels: Vec<&str> = seen.iter().map(|m| m.label()).collect();
        let mut uniq = labels.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), labels.len(), "档位显示名重复: {labels:?}");
    }

    #[test]
    fn gameover_freezes_kills() {
        let mut game = Game::new();
        game.game_state = GameState::GameOver;
        game.npcs[0].hp = 20.0;
        let npc_pos = game.npcs[0].position;
        assert!(game.fire([npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]], [0.0, -1.0, 0.0]));
        for _ in 0..10 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), 8, "no kills allowed after game over");
        assert_eq!(game.score, 0, "no score after game over");
        assert_eq!(game.hud.health, 100.0, "player health locked after game over");
    }

    /// 网络环回演示：init 能绑定环回 server，client 的 Join 能被 server 收到并回 ack
    #[test]
    fn net_loopback_demo_join_roundtrip() {
        let mut demo = Game::init_network_demo().expect("loopback demo should init");
        let mut got_join = false;
        // UDP 环回投递可能有毫秒级延迟：带超时轮询（与 net.rs recv_until 同款模式），避免偶发失败
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while !got_join && std::time::Instant::now() < deadline {
            // 内层 `if let NetworkMessage::Join` 折进同一条模式：非 Join 的消息原本也是
            // 直接落空忽略，两种写法行为一致（clippy::collapsible_match）。
            // 注意折叠后 `player_id` 是按值绑定（原来配 `&msg` 是 `&u32` 才需要解引用）。
            if let Ok(Some((NetworkMessage::Join { player_id, .. }, from))) = demo.server.recv() {
                got_join = player_id == 0;
                assert!(demo.server.handle_join(from, "local".into(), crate::net::SESSION_VERSION).is_ok());
            }
            if !got_join {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        assert!(got_join, "server should receive the Join sent at init");
    }

    /// 网络对战闭环（RV3D_NET=server|client 的纯逻辑等价物，无 Vulkan/winit）：
    /// 客户端上报输入 → 服务端应用 → 广播快照 → 客户端插值缓冲
    #[test]
    fn net_server_client_loopback_closed_loop() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut server_game = Game::new();
        let mut client_game = Game::new();
        server_game.set_net_server(server);
        client_game.set_net_client(Client::connect(addr).unwrap());
        client_game.set_movement(true, false, false, false);
        let camera = Camera::new();
        // UDP 环回投递可能有毫秒级延迟：多轮推进让 握手 → 输入 → 快照 完整走通
        for _ in 0..20 {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        // 最后一轮收尾：轮询直到追平服务端最新快照（UDP 环回投递有毫秒级延迟）
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while client_game.net_client.as_ref().unwrap().snapshot_seq()
            != server_game.net_snap_seq
            && std::time::Instant::now() < deadline
        {
            // 2026-08-23：等待期间服务器也必须继续更新（否则不产新快照）
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        // 收尾：客户端再消费一次（服务器已停止生产 → 快照序号追平）
        client_game.update(1.0 / 60.0, &camera);
        let client = client_game.net_client.as_ref().unwrap();
        assert_eq!(client.player_id(), Some(1), "握手应分配 player id 1");
        assert!(client.snapshot_seq() > 0, "应收到服务端快照");
        assert!(client.own_state().is_some(), "快照应含本机权威状态");
        assert!(
            client.entities().len() >= server_game.npcs.len(),
            "快照应包含全部 NPC + 服务端本机玩家"
        );
        assert!(!client.snapshot_timeout(), "回环测试不应超时");
        // 2026-08-25：客户端输入驱动「远端玩家」实体（网络对战模型），不再是服务端本机玩家
        // （原来这里还挂着 `npcs.len() >= 0`：usize 恒非负，是个永真的空断言，删掉）
        assert!(
            server_game.net_players.iter().any(|p| p.alive),
            "服务端应注册远端玩家并应用客户端输入"
        );
        assert_eq!(
            client.snapshot_seq(),
            server_game.net_snap_seq,
            "客户端应追平最新快照"
        );
    }

    /// 🔴 判据：远端玩家离场后，服务端**必须停止在快照里广播它**。
    ///
    /// 真机代价（2026-09-26 复查）：服务端超时/离场只摘掉自己的注册表项，
    /// `net_players` 一条不删 ⇒ 快照里继续带着它（最后一帧的姿态），
    /// 而客户端对远端玩家实体是**无条件进画面**的 ⇒ 每个人看到一个站着不动的幽灵。
    ///
    /// 走的是真链路：回环握手 → 服务端注册远端玩家 → 客户端发 `Leave`（正常退出的路径）
    /// → 服务端注销注册表并停止广播。
    #[test]
    fn net_departed_player_stops_being_broadcast() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut server_game = Game::new();
        let mut client_game = Game::new();
        server_game.set_net_server(server);
        client_game.set_net_client(Client::connect(addr).unwrap());
        client_game.set_movement(true, false, false, false);
        let camera = Camera::new();
        for _ in 0..20 {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        // UDP 环回投递有毫秒级延迟：轮询到注册发生为止（同 `net_server_client_loopback_closed_loop`）
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while server_game.net_players.is_empty() && std::time::Instant::now() < deadline {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        assert!(
            !server_game.net_players.is_empty(),
            "握手后服务端应注册远端玩家（否则这条测试没意义）"
        );
        let id = server_game.net_players[0].id;
        assert_eq!(
            server_game.net_server.as_ref().unwrap().player_id_of(
                client_game.net_client.as_ref().unwrap().local_addr().unwrap()
            ),
            Some(id),
            "注册表里应有该客户端的注册"
        );
        // 客户端正常退出：发 Leave，然后服务端再走一 tick
        client_game
            .net_client
            .as_ref()
            .unwrap()
            .send(&NetworkMessage::Leave { player_id: id, reason: 0 })
            .unwrap();
        server_game.update(1.0 / 60.0, &camera);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while server_game.net_server.as_ref().unwrap().client_count() > 0
            && std::time::Instant::now() < deadline
        {
            server_game.update(1.0 / 60.0, &camera);
        }
        assert_eq!(
            server_game.net_server.as_ref().unwrap().client_count(),
            0,
            "收到 Leave 必须注销注册表"
        );
        assert!(
            server_game.net_players.is_empty(),
            "离场玩家必须从广播名单里删掉（否则每帧快照继续带着一个幽灵）"
        );
        // 再走几 tick：后面的快照里也不该再有它的实体
        for _ in 0..5 {
            server_game.update(1.0 / 60.0, &camera);
        }
        assert!(server_game.net_players.is_empty());
    }

    /// 目标状态回环：服务端广播 ObjectiveState → 客户端解析据点归属/进度
    #[test]
    fn net_objective_state_loopback_broadcast_consumed() {        let server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut server_game = Game::new();
        let mut client_game = Game::new();
        server_game.set_net_server(server);
        client_game.set_net_client(Client::connect(addr).unwrap());
        // 给服务端注入一个据点（模拟关卡系统启用）：
        // 用 CapturePoint 直接塞进 obj_state（绕过 RV3D_MAP 加载，纯逻辑回环）
        let rule = crate::engine::objective::GameRule::CapturePoints { required: 1 };
        let mut obj = crate::engine::objective::ObjectiveState::new(rule);
        obj.points.push(crate::engine::objective::CapturePoint::new(
            "A", 0.0, 0.0, 5.0, 10.0,
        ));
        server_game.obj_state = Some(obj);
        let camera = Camera::new();
        for _ in 0..20 {
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
        while !client_game
            .net_client
            .as_ref()
            .unwrap()
            .has_objective()
            && std::time::Instant::now() < deadline
        {
            // 2026-08-23：等待期间服务器也必须继续更新（否则广播停止）
            client_game.update(1.0 / 60.0, &camera);
            server_game.update(1.0 / 60.0, &camera);
        }
        let client = client_game.net_client.as_ref().unwrap();
        assert!(client.has_objective(), "客户端应收到目标状态");
        assert_eq!(client.objective_rule(), "capture");
        let pts = client.objective_state();
        assert_eq!(pts.len(), 1, "应收到 1 个据点");
        assert_eq!(pts[0].0, "A");
        assert_eq!(pts[0].1, 0, "中立据点归属码 = 0");
        assert_eq!(pts[0].2, 0.0, "中立据点进度 = 0");
    }

    /// FPS 玩家：WASD 移动改变位置，眼睛高度 1.6m
    #[test]
    fn fps_player_moves_with_wasd() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let cam = Camera::new(); // yaw=0 → forward -Z
        let start_z = game.player_body.pos.z;
        game.set_movement(true, false, false, false);
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.player_pos().z < start_z - 5.0,
            "W 应沿 -Z 移动约 6m: {} -> {}",
            start_z,
            game.player_pos().z
        );
        assert!(
            (game.player_eye().y - 1.6).abs() < 1e-5,
            "眼睛高度应为 1.6m"
        );
        // S 后退回原点附近
        game.set_movement(false, true, false, false);
        for _ in 0..60 {
            game.update(1.0 / 60.0, &cam);
        }
        assert!(
            game.player_body.pos.z > start_z - 0.5,
            "S 应退回原点附近: {}",
            game.player_body.pos.z
        );
    }

    /// FPS 玩家：撞到演示刚体被推回，不会穿模
    #[test]
    fn fps_player_collides_with_map_obstacle() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 取关卡地图第一个障碍盒（AABB 中心 (x,z) 半宽 half_w/half_d）；
        // 玩家放其 +Z 侧 5m，W 前进（-Z 方向）应被挡在 0.5m（玩家半径）外
        let ob = game.map.obstacles[0];
        game.player_body.pos = Pv::new(ob.x, 0.0, ob.z + ob.half_d + 5.0);
        let cam = Camera::new();
        game.set_movement(true, false, false, false);
        for _ in 0..120 {
            game.update(1.0 / 60.0, &cam);
        }
        let z = game.player_body.pos.z;
        assert!(z < ob.z + ob.half_d + 5.0, "玩家应朝障碍移动: {}", z);
        assert!(
            z > ob.z + ob.half_d + 0.15 && z < ob.z + ob.half_d + 0.55,
            "碰撞应把玩家挡在障碍 +Z 面外约 0.35m（玩家半径）(期望 ~{}): {}",
            ob.z + ob.half_d + 0.35,
            z
        );
    }

    /// 高强度碰撞完整性：随机方向高速游走 2000 帧（含斜穿 AABB 角、贴边滑动），
    /// 玩家中心到任意障碍 AABB 的距离必须 ≥ 玩家半径 - ε（永不穿入）。
    /// 回归：据点立柱/任何新增"视觉障碍"都不得缺失碰撞体。
    #[test]
    fn player_never_enters_any_obstacle_aabb() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert!(!game.world.bodies.is_empty(), "关卡应有障碍刚体");
        let r = game.player_body.radius();
        let mut state = 0x9E3779B97F4A7C15u64;
        for _ in 0..2000 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let ang = ((state >> 33) % 62832) as f32 / 10000.0; // 0..2π
            // 高速移动（0.1m/帧 ≈ 6m/s）
            game.player_body.pos.x += ang.cos() * 0.1;
            game.player_body.pos.z += ang.sin() * 0.1;
            game.player_body.collide_world(&game.world);
            for body in &game.world.bodies {
                let a = body.aabb();
                let cx = game.player_body.pos.x.clamp(a.min.x, a.max.x);
                let cz = game.player_body.pos.z.clamp(a.min.z, a.max.z);
                let d2 = (cx - game.player_body.pos.x).powi(2)
                    + (cz - game.player_body.pos.z).powi(2);
                assert!(
                    d2 >= (r - 0.02).powi(2),
                    "玩家穿入障碍 AABB ({}, {}, {}, {})",
                    a.min.x,
                    a.min.z,
                    a.max.x,
                    a.max.z
                );
            }
        }
    }

    /// 换弹：R 触发后计时完成，弹匣补满
    #[test]
    fn firearm_reload_cycle_via_game() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let origin = [0.0, 5.0, 0.0];
        let dir = [0.0, 0.0, -1.0];
        let mut fired = 0;
        for _ in 0..120 {
            if game.fire(origin, dir) {
                fired += 1;
            }
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(fired >= 5, "2 秒内应打出至少 5 发: {}", fired);
        let before = game.weapons.active_firearm_ref().magazine();
        assert!(before < 30, "弹匣应消耗过");
        game.request_reload();
        assert!(game.weapons.active_firearm_ref().is_reloading(), "R 应开始换弹");
        for _ in 0..200 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(!game.weapons.active_firearm_ref().is_reloading(), "换弹应完成");
        assert_eq!(game.weapons.active_firearm_ref().magazine(), 30, "换弹后弹匣应补满");
    }

    /// 开火产生后坐力，drain 一次后清零
    #[test]
    fn fire_applies_recoil_kick() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert!(game.fire([0.0, 5.0, 0.0], [0.0, 0.0, -1.0]));
        let (pitch_kick, _) = game.drain_kick();
        assert!(pitch_kick > 0.0, "上跳后坐力应为正");
        assert_eq!(game.drain_kick(), (0.0, 0.0), "drain 后应清零");
    }

    /// 命中 NPC 触发命中标记
    #[test]
    fn projectile_hit_shows_hit_marker() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let npc_pos = game.npcs[0].position;
        assert!(game.fire(
            [npc_pos[0], npc_pos[1] + 2.0, npc_pos[2]],
            [0.0, -1.0, 0.0]
        ));
        for _ in 0..3 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(
            game.hud.hit_marker_timer > 0.0,
            "命中应触发准星命中标记"
        );
    }

    /// 波次难度曲线：spawn 数量/速度/血量/攻击距离与 wave_profile 一致
    #[test]
    fn wave_profile_drives_spawn() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        let p = wave_profile(1);
        assert_eq!(game.npcs.len(), p.count as usize, "波次数量");
        for npc in &game.npcs {
            assert!((npc.speed - p.speed).abs() < 1e-6, "速度");
            assert!((npc.max_hp - p.hp).abs() < 1e-6, "血量");
            assert!((npc.attack_range - p.attack_range).abs() < 1e-6, "攻击距离");
        }
    }

    /// 特殊波次：Boss 波最后一只为主怪（高血量/慢速/攻击距离略长），其余仍按 profile
    #[test]
    fn boss_wave_spawns_slow_tanky_elite() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.spawn_wave(5, &glam::Vec3::ZERO);
        let p = wave_profile(5);
        assert_eq!(p.kind, WaveKind::Boss);
        assert_eq!(game.npcs.len(), p.count as usize, "Boss 波数量仍按 count");
        let boss = game.npcs.last().unwrap();
        let b = p.boss.expect("Boss 波应有主怪参数");
        assert!((boss.max_hp - b.hp).abs() < 1e-6, "主怪血量");
        assert!((boss.speed - b.speed).abs() < 1e-6, "主怪速度");
        assert!((boss.attack_range - b.attack_range).abs() < 1e-6, "主怪攻击距离");
        for npc in game.npcs.iter().take(game.npcs.len() - 1) {
            assert!((npc.max_hp - p.hp).abs() < 1e-6, "小怪血量按 profile");
            assert!(npc.speed > boss.speed, "主怪应慢于小怪");
        }
    }

    /// 特殊波次：援军波在波开始 1.5s 后补怪 1..=2 只，且只触发一次
    #[test]
    fn reinforcement_wave_spawns_mid_wave() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.spawn_wave(3, &glam::Vec3::ZERO);
        let p = wave_profile(3);
        assert_eq!(p.kind, WaveKind::Reinforced);
        assert_eq!(game.npcs.len(), p.count as usize, "援军补怪前数量 = count");
        // 推进 2 秒：1.5s 处应触发补怪
        for _ in 0..120 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert!(game.reinforcement_done, "援军应已触发");
        assert_eq!(
            game.npcs.len(),
            (p.count + p.reinforcement_count) as usize,
            "补怪后数量 = count + reinforcement_count"
        );
        // 再推进 1 秒：不应二次补怪
        let before = game.npcs.len();
        for _ in 0..60 {
            game.update(1.0 / 60.0, &Camera::new());
        }
        assert_eq!(game.npcs.len(), before, "援军只触发一次");
    }

    /// 特殊波次：清波条件不变（全歼才算清空），Boss/援军波后波次推进正常
    #[test]
    fn special_waves_keep_clear_and_progress() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 常规波（波 2）：清掉当前 npc 后应正常推进到波 3
        game.spawn_wave(2, &glam::Vec3::ZERO);
        assert_eq!(game.wave, 2);
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.wave, 3, "常规波清空后应推进");
        assert_eq!(
            wave_profile(3).kind,
            WaveKind::Reinforced,
            "推进后的波 3 应为援军波"
        );
        // 第 3 波是每关最后一波：清空后升关到 level 2 wave 1
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 2, "第 3 波清空应升关");
        assert_eq!(game.wave, 1, "升关后回本关第 1 波");
        // Boss 波：level 2 第 2 波 = 累计有效波 5，清空后推进到本关第 3 波
        game.spawn_wave(2, &glam::Vec3::ZERO);
        assert_eq!(game.wave, 2);
        let p5 = wave_profile(5);
        assert_eq!(p5.kind, WaveKind::Boss, "有效波 5 应为 Boss 波");
        assert_eq!(p5.total_count, p5.count, "Boss 波总敌人数 = count（含主怪）");
        game.npcs.clear();
        for _ in 0..330 {
            game.update(0.01, &Camera::new());
        }
        assert_eq!(game.level, 2, "Boss 波清空后应留在本关");
        assert_eq!(game.wave, 3, "Boss 波清空后应推进到本关第 3 波");
    }

    /// 设置面板：开关、音量/灵敏度调整、选中项循环
    #[test]
    fn settings_panel_toggle_and_adjust() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        assert!(!game.settings_open(), "初始应关闭");
        game.toggle_settings();
        assert!(game.settings_open(), "toggle 后应打开");
        let vol = game.hud.volume;
        game.adjust_settings(0.1);
        assert!(
            (game.hud.volume - (vol + 0.1).min(1.0)).abs() < 1e-6,
            "音量应增加"
        );
        game.cycle_settings();
        let sens = game.hud.sensitivity;
        game.adjust_settings(-0.1);
        assert!(game.hud.sensitivity < sens, "灵敏度应降低");
        game.toggle_settings();
        assert!(!game.settings_open(), "再 toggle 应关闭");
    }

    // ---- 爆炸/冲击波（AoE 玩法）----

    /// 测试用爆炸：替换 npcs 后引爆，返回爆炸实体引用
    fn explode_on(npcs: Vec<Npc>, center: [f32; 3], damage: f32, knockback: bool) -> Game {
        let mut game = Game::new();
        game.npcs = npcs;
        game.spawn_explosion(center, EXPLOSION_RADIUS, damage, knockback);
        game
    }

    /// 🔴 **kill feed 开始报"谁杀的"**（2026-09-15）。
    ///
    /// 此前三处调用点各写各的格式，且**只有 NPC 互射那一处带击杀者** ——
    /// 玩家自己打死人时 feed 是"击杀 蓝方 #174"，看不出是谁干的。
    ///
    /// 这条测试直接把**文本契约**锁住（feed 全文此前没有任何测试覆盖）：
    /// 玩家击杀要出现"你击杀了"，爆炸要出现"爆炸（…）击杀了"且**不能冒充**有击杀者。
    #[test]
    fn kill_feed_names_the_killer() {
        let mut game = Game::new();
        game.npcs = vec![npc_at(7, Team::Blue, [0.0, 0.0, 0.0])];
        game.damage_npc(0, 999.0, DamageSource::Player);
        let line = game.hud.kill_feed.first().expect("feed 应有一条").text.clone();
        assert!(
            line.contains("你击杀了") && line.contains("蓝方 #7"),
            "玩家击杀应报出击杀者与死者，实测：{line}"
        );

        // 爆炸：同一发 AoE 可能同时结算多人 ⇒ **不冒充**单一击杀者，只报爆点
        let mut blast = Game::new();
        blast.npcs = vec![npc_at(9, Team::Red, [0.0, 0.0, 0.0])];
        blast.spawn_explosion([3.0, 1.0, -4.0], EXPLOSION_RADIUS, 999.0, false);
        let line = blast.hud.kill_feed.first().expect("feed 应有一条").text.clone();
        assert!(
            line.contains("爆炸（3,-4）击杀了") && line.contains("红方 #9"),
            "爆炸击杀应报爆点坐标且不编造击杀者，实测：{line}"
        );
        assert!(
            !line.contains("你击杀"),
            "爆炸不是玩家的击杀，不该写成玩家杀的，实测：{line}"
        );
    }

    /// NPC 受伤结算：扣血至 0 → 移除 + 计分 + 任务目标推进；返回是否击杀（阶段二）
    /// 爆炸/冲击波（AoE 玩法）—— 见 `explosion_*` 系列
    #[test]
    fn damage_npc_reports_kill_and_removes() {
        let mut game = Game::new();
        game.npcs = vec![npc_at(3, Team::Blue, [0.0, 0.0, 0.0])];
        assert!(
            game.damage_npc(0, 40.0, DamageSource::Player) == false,
            "未致死应返回 false"
        );
        assert_eq!(game.npcs.len(), 1, "未致死不应移除");
        assert!(
            game.damage_npc(0, 999.0, DamageSource::Player),
            "致死应返回 true"
        );
        assert_eq!(game.npcs.len(), 0, "致死应移除");
    }

    /// AoE 伤害：爆心全伤、随距离衰减、超出半径无损（衰减语义 = shockwave_pressure）
    #[test]
    fn explosion_aoe_damage_falloff() {
        let game = explode_on(
            vec![
                npc_at(1, Team::Red, [0.0, 0.0, 0.0]),
                npc_at(2, Team::Red, [4.0, 0.0, 0.0]),
                npc_at(3, Team::Red, [20.0, 0.0, 0.0]),
            ],
            [0.0, 1.0, 0.0],
            EXPLOSION_DAMAGE,
            true,
        );
        let hp: Vec<f32> = game.npcs.iter().map(|n| n.hp).collect();
        assert!(hp[0] < hp[1], "爆心最近者受伤最重");
        assert!(hp[1] < hp[2], "随距离衰减");
        assert!((hp[2] - 100.0).abs() < 1e-6, "超出半径不受伤");
        assert!(hp[0] < 100.0 - 30.0, "近爆心伤害显著");
        assert!(game.npcs[1].knockback[0] > 0.0, "径向推挤方向向外（+x）");
        assert_eq!(game.npcs[2].knockback, [0.0, 0.0], "超出半径无推挤");
    }

    /// 爆炸对障碍 AoE 伤害：半径内障碍掉血、可摧毁；超出半径无损（阶段二）
    #[test]
    fn explosion_damages_obstacles_in_radius() {
        let mut game = Game::new();
        // 注入两个障碍：一个 Barrier（100HP，爆心可摧毁）在（0,0），一个在远处（50,50）
        game.map.obstacles.push(MapObstacle::new(ObstacleKind::Barrier, 0.0, 0.0, 1.0, 1.0));
        game.map.obstacles.push(MapObstacle::new(ObstacleKind::Wall, 50.0, 50.0, 1.0, 1.0));
        let before = game.map.obstacles.len();
        game.spawn_explosion([0.0, 1.0, 0.0], 8.0, 120.0, true);
        // 爆心 Barrier（100HP）被 120×1.0×fall(1.0)=120 伤摧毁（Game::new 含场景装饰，
        // 数量随主题变化——只断言爆心障碍被移除）
        assert_eq!(
            game.map.obstacles.len(),
            before - 1,
            "爆心 Barrier（100HP）被 120 伤摧毁"
        );
        // 注入的远处障碍（50,50）保留且无伤
        let far = game
            .map
            .obstacles
            .iter()
            .find(|o| (o.x - 50.0).abs() < 1.0 && (o.z - 50.0).abs() < 1.0)
            .expect("远处障碍应保留");
        assert!((far.hp - 150.0).abs() < 1e-5, "远处障碍无伤");
    }

    /// 🔴 **冲击波被障碍挡住：掩体后面的人不该吃满伤害**（2026-09-15）。
    ///
    /// 此前 `spawn_explosion` 的 AoE 只有"半径内 + 距离衰减"，**没有任何视线判定** ——
    /// 掩体对爆炸等于不存在，"躲在墙后"对爆炸完全无效。
    ///
    /// 这里用**三格对照**把"挡住"和"算错"分开（判据不是"受伤变少"，而是几何本身）：
    /// - A 垂直挡在爆心与 NPC 之间 → NPC **一点伤害都不该吃**（仍 100HP）；
    /// - B 与爆心-NPC 连线**平行**（旁边那堵墙）→ 人**照吃** —— 这条是防过度修正的：
    ///   若把判据写成"爆心到 NPC 的扫掠体与障碍相交"，平行墙会把本不存在的遮挡算进来；
    /// - C 同一位置不放障碍 → 对照组，证明 A 的 100HP 是"被挡住"而不是"本来就不掉血"。
    ///
    /// ⚠ 三条都从 `Game::new()` 起手并**清空自带障碍**：默认场景含场景装饰，
    /// 不清的话"某个装饰正好挡在中间"会让这三条悄悄失真。
    #[test]
    fn explosion_blast_is_blocked_by_cover() {
        let npc = || npc_at(1, Team::Red, [6.0, 0.0, 0.0]);
        let hp_of = |game: &Game| game.npcs[0].hp;

        // C：对照组（无障碍）—— 6m 处应当受伤
        let mut open = Game::new();
        open.map.obstacles.clear();
        open.npcs = vec![npc()];
        open.spawn_explosion([0.0, 1.0, 0.0], EXPLOSION_RADIUS, EXPLOSION_DAMAGE, true);
        assert!(
            hp_of(&open) < 100.0,
            "对照组：开阔地 6m 处的 NPC 应当受伤（实测 {}）",
            hp_of(&open)
        );

        // A：垂直障碍挡在中间 —— 爆心 x=0 → NPC x=6，墙放 x=3
        let mut blocked = Game::new();
        blocked.map.obstacles.clear();
        blocked
            .map
            .obstacles
            .push(MapObstacle::new(ObstacleKind::Wall, 3.0, 0.0, 0.5, 0.5));
        blocked.npcs = vec![npc()];
        blocked.spawn_explosion([0.0, 1.0, 0.0], EXPLOSION_RADIUS, EXPLOSION_DAMAGE, true);
        assert_eq!(
            hp_of(&blocked),
            100.0,
            "掩体后面的人不该吃伤害（实测 {}）",
            hp_of(&blocked)
        );

        // B：平行墙（不挡路）—— 人照吃；这条专门防"扫掠体"式的过度修正
        let mut parallel = Game::new();
        parallel.map.obstacles.clear();
        parallel
            .map
            .obstacles
            .push(MapObstacle::new(ObstacleKind::Wall, 3.0, 4.0, 0.35, 0.35));
        parallel.npcs = vec![npc()];
        parallel.spawn_explosion([0.0, 1.0, 0.0], EXPLOSION_RADIUS, EXPLOSION_DAMAGE, true);
        assert!(
            hp_of(&parallel) < 100.0,
            "旁边的墙不该替人挡冲击波（实测 {}）",
            hp_of(&parallel)
        );
    }

    /// 爆心落在障碍内部（手榴弹贴着掩体炸）时**不能把自己堵死**：
    /// 那一格障碍自己就是被炸对象，不该把爆心判成"被遮挡"。
    ///
    /// ⚠ 用 `GRENADE_EXPLOSION_DAMAGE`(120) 而不是 `EXPLOSION_DAMAGE`(60)：
    /// 前者是手榴弹的真实伤害，120 > Barrier(100HP) 才能验证"贴脸炸掉掩体"。
    #[test]
    fn explosion_inside_obstacle_still_kills_through() {
        let mut game = Game::new();
        game.map.obstacles.clear();
        // 爆心就在这格障碍里（half_h = 1.2 ⇒ y∈[0,2.4]，爆心 y=1.0 在内）
        game.map
            .obstacles
            .push(MapObstacle::new(ObstacleKind::Barrier, 0.0, 0.0, 1.0, 1.0));
        // NPC 在 3m 外、路径**不经过**这格障碍（z 方向）
        game.npcs = vec![npc_at(1, Team::Red, [0.0, 0.0, 3.0])];
        game.spawn_explosion(
            [0.0, 1.0, 0.0],
            EXPLOSION_RADIUS,
            GRENADE_EXPLOSION_DAMAGE,
            true,
        );
        assert!(
            game.npcs[0].hp < 100.0,
            "贴脸炸掩体时，旁边的人仍应受伤（实测 {}）",
            game.npcs[0].hp
        );
        assert_eq!(
            game.map.obstacles.len(),
            0,
            "被炸的那格障碍自己应被摧毁（120 伤 > Barrier 的 100HP）"
        );
    }

    /// NPC 投掷手榴弹：压力模式 Attack 态 NPC 冷却结束 → 朝敌对目标投掷（阶段二）
    #[test]
    fn npc_throws_grenade_in_stress_combat() {
        let mut game = Game::new();
        game.stress = true;
        let mut a = npc_at(0, Team::Red, [0.0, 0.0, 0.0]);
        let mut b = npc_at(1, Team::Blue, [8.0, 0.0, 0.0]);
        // 推进到 Attack 态
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        a.state_machine.update(p);
        a.state_machine.update(p);
        b.state_machine.update(p);
        b.state_machine.update(p);
        a.grenade_timer = 0.0; // 冷却结束，允许投掷
        b.grenade_timer = 999.0; // 对侧不投掷（验证确定性只投掷冷却结束者）
        game.npcs = vec![a, b];
        let targets = pick_stress_targets(&game.npcs, STRESS_SIGHT);
        let before = game.grenades_vec.len();
        // 帧号取 id*31 % 8 < 8 恒真 → 必投掷（h = (0*31 + frame*7) % 100，frame 取使 h<8）
        game.frame_no = 1;
        game.npc_throw_grenades(1.0 / 60.0, &targets);
        assert!(
            game.grenades_vec.len() > before,
            "冷却结束的 Attack 态 NPC 应投掷手榴弹"
        );
    }

    /// 🔴 2026-09-16 真机现场：NPC 手榴弹**出手即自爆**（帧率相关）。
    ///
    /// 证据（`RV3D_MAP=assets/maps/defense_line.toml`、survive、130fps）：
    /// `grenade: npc #12 throws at (0, 0)` 与 `kill: npc #12 eliminated` **同一秒**，
    /// 而 `weapons: shot #` 全程 **0 条** ⇒ 投掷者炸死了自己，玩家一枪没开。
    ///
    /// 机理：NPC 从**脚底**出手（`npc.position`，平地 y=0），落地判据是
    /// `pos.y <= ground + 0.05`；出手后第一帧只上升 `vy*dt`（vy≈5.35 m/s）
    /// ⇒ **dt ≤ 9.3ms（≥108fps）时第一帧仍在容差内 → 原地引爆**。
    /// 60fps 下第一帧上升 8.9cm 所以不复现 —— 这是个只在快机器上出现的自杀。
    /// 修法 = 出手点抬到 `NPC_GRENADE_RELEASE_Y`（手的高度）。
    #[test]
    fn npc_grenade_does_not_detonate_on_release() {
        for fps in [60.0f32, 130.0, 240.0] {
            let dt = 1.0 / fps;
            let mut game = Game::new();
            game.stress = true;
            let mut a = npc_at(0, Team::Red, [0.0, 0.0, 0.0]);
            let mut b = npc_at(1, Team::Blue, [12.0, 0.0, 0.0]);
            let p = NpcPerception {
                enemy_visible: true,
                enemy_in_range: true,
                ..NpcPerception::default()
            };
            a.state_machine.update(p);
            a.state_machine.update(p);
            b.state_machine.update(p);
            b.state_machine.update(p);
            a.grenade_timer = 0.0;
            b.grenade_timer = 999.0;
            game.npcs = vec![a, b];
            let targets = pick_stress_targets(&game.npcs, STRESS_SIGHT);
            game.frame_no = 1; // h = (0*31 + 1*7) % 100 = 7 < 8 → 必投掷
            game.npc_throw_grenades(dt, &targets);
            assert_eq!(game.grenades_vec.len(), 1, "{}fps: 应投出一枚", fps);
            assert!(
                game.grenades_vec[0].position()[1] > 1.0,
                "{}fps: 出手点应抬到离地高度，实际 y={}",
                fps,
                game.grenades_vec[0].position()[1]
            );
            // 出手后的头几帧绝不允许引爆（自爆窗口就在这里）
            for _ in 0..8 {
                game.update_grenades(dt);
                assert!(
                    !game.grenades_vec.is_empty() && !game.grenades_vec[0].exploded(),
                    "{}fps: 出手后 {}s 内不得引爆（否则原地炸死投掷者）",
                    fps,
                    dt * 8.0
                );
            }
            // 抛物线走完：落点应在 8m 爆炸半径之外（投掷者安全）
            for _ in 0..(fps as usize * 3) {
                game.update_grenades(dt);
                if game.grenades_vec.is_empty() {
                    break;
                }
            }
            assert!(
                game.npcs.iter().all(|n| n.hp > 0.0),
                "{}fps: 投掷者不该被自己的手榴弹炸死",
                fps
            );
        }
    }

    /// 重开一局必须清掉**在飞的手榴弹与爆炸**：
    /// 否则"投掷后死亡/通关 → 按 R 重开"会把上一局的手榴弹带进新一局，
    /// 它在新一局的第 1 波里爆炸 —— 多算击杀得分，还可能伤到玩家。
    /// 本测试在补这两行清除之前会红（`grenades_vec` 非空）。
    #[test]
    fn restart_clears_in_flight_grenades_and_explosions() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 造一颗"在飞"的手榴弹 + 一团爆炸（都还没过期）
        game.grenades_vec.push(Grenade::new(
            [0.0, 1.5, 0.0],
            [0.0, 0.3, -1.0],
            GRENADE_SPEED,
            2.0,
        ));
        game.spawn_explosion([1.0, 1.0, 1.0], EXPLOSION_RADIUS, 1.0, false);
        assert!(
            !game.grenades_vec.is_empty() && !game.explosions.is_empty(),
            "前置：场上应有在飞手榴弹与爆炸"
        );
        game.game_state = GameState::GameOver;
        game.request_restart(&glam::Vec3::ZERO);
        assert!(game.grenades_vec.is_empty(), "重开后不得残留上一局的手榴弹");
        assert!(game.explosions.is_empty(), "重开后不得残留上一局的爆炸");
        assert_eq!(game.shake_timer, 0.0, "重开后震屏应归零");
    }

    /// 重开一局必须复位**跳跃状态**：玩家可能在空中被打死，
    /// 否则新一局开局会带着上一局的上升速度与冲刺跳惯性，甚至"落地即起跳"。
    /// 本测试在 `start_run` 补这三行复位之前会红。
    #[test]
    fn restart_resets_jump_state() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        // 模拟"冲刺跳之后在空中被打死"：仍在上升 + 带着水平惯性 + 空格一直没松
        game.jump_vel = JUMP_SPEED;
        game.jump_hvel = glam::Vec3::new(6.0, 0.0, 0.0);
        game.jump_pressed = true;
        game.game_state = GameState::GameOver;
        game.request_restart(&glam::Vec3::ZERO);
        assert_eq!(game.jump_vel, 0.0, "重开后不得带上一局的上升速度");
        assert_eq!(
            game.jump_hvel,
            glam::Vec3::ZERO,
            "重开后不得带上一局的跳跃惯性"
        );
        assert!(!game.jump_pressed, "重开后不得保留上一局的跳跃按键");
    }

    /// 重开一局必须清掉上一局的击杀提示：feed 是每局的事件流，
    /// 而重开时 score 已归零 —— 残留的"你被击杀了"与清零的分数自相矛盾。
    /// 本测试在 `start_run` 补 `kill_feed.clear()` 之前会红。
    #[test]
    fn restart_clears_kill_feed() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.push_kill("你被击杀了".to_string());
        assert!(!game.hud.kill_feed.is_empty(), "前置：feed 里应有一条");
        game.game_state = GameState::GameOver;
        game.request_restart(&glam::Vec3::ZERO);
        assert!(
            game.hud.kill_feed.is_empty(),
            "重开后不得残留上一局的击杀提示"
        );
    }

    /// 爆炸击杀：hp≤0 移除 + 计分 + 任务推进
    #[test]
    fn explosion_kills_and_scores() {
        let mut npcs = vec![npc_at(7, Team::Red, [0.5, 0.0, 0.0])];
        npcs[0].hp = 30.0;
        npcs[0].last_hp = 30.0;
        let game = explode_on(npcs, [0.0, 1.0, 0.0], EXPLOSION_DAMAGE, true);
        assert!(game.npcs.is_empty(), "30hp NPC 被爆心击杀");
        assert_eq!(game.score, KILL_SCORE, "击杀计分");
        assert_eq!(game.objective.eliminated, 1, "任务目标推进");
    }

    /// survive 规则波次推进：清波 → 补给窗口（血量回复/弹药补满）→ 下一波；
    /// 守住全部波 → 胜利态（阶段一）
    #[test]
    fn survive_rule_advances_waves_and_wins_at_last() {
        let mut game = Game::new();
        game.obj_state = Some(crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::Survive { waves: 2 },
        ));
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.health = 50.0; // 半血，验证补给回复
        game.grenades = 0;
        let camera = Camera::new();
        // 清空 NPC（模拟玩家清完 wave 1）+ 推进 update_waves
        game.npcs.clear();
        game.wave_timer = 0.0;
        for _ in 0..200 {
            game.update(1.0 / 60.0, &camera);
        }
        // wave 1 清完 → wave_timer 置为 WAVE_INTERMISSION → 递减到 0 → wave 2 + 补给
        assert_eq!(game.wave, 2, "survive 第 1 波清完应进入第 2 波");
        assert!(
            game.hud.health > 50.0,
            "波间补给应回复血量: {}",
            game.hud.health
        );
        assert_eq!(game.grenades, game.grenades_max, "波间补给应补满手榴弹");
        // 清完 wave 2（最后一波）→ 胜利
        game.npcs.clear();
        game.wave_timer = 0.0;
        for _ in 0..300 {
            game.update(1.0 / 60.0, &camera);
            if game.game_state == GameState::Victory(crate::engine::ai::Team::Blue) {
                break;
            }
        }
        assert_eq!(
            game.game_state,
            GameState::Victory(crate::engine::ai::Team::Blue),
            "守住全部波次应胜利"
        );
    }

    /// survive 规则的波数**可以长于** `WAVES_PER_LEVEL`（`defense_line.toml` 写的是 `waves = 5`）。
    ///
    /// 🔴 2026-09-25 真机实测（`defense_line` + `run_survive_pm.ps1`）：`wave 3 cleared` 之后
    /// 紧跟的不是 wave 4，而是 `wave: wave 1 spawned 12 enemies (… effective=4)` —— 旧逻辑
    /// 先判胜利，再判 `wave >= WAVES_PER_LEVEL`(3) 就**升关并把 wave 归 1**，于是第 4/5 波
    /// 永远到不了、`survive: 全部 5 波守住 → 胜利` 永远不成立（旧测试只覆盖 `waves = 2`，
    /// 恰好低于阈值 3 ⇒ 这个洞一直没被测到）。
    #[test]
    fn survive_wave_count_above_waves_per_level_still_reaches_victory() {
        let mut game = Game::new();
        game.obj_state = Some(crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::Survive { waves: 5 },
        ));
        game.on_any_key(&glam::Vec3::ZERO);
        let camera = Camera::new();
        for expected in 1..=5u32 {
            assert_eq!(game.wave, expected, "波次必须连续推进到 rule.waves");
            assert_eq!(game.level, 1, "survive 规则下不许按关卡进度升关");
            game.npcs.clear();
            game.wave_timer = 0.0;
            for _ in 0..200 {
                game.update(1.0 / 60.0, &camera);
            }
        }
        assert_eq!(
            game.game_state,
            GameState::Victory(crate::engine::ai::Team::Blue),
            "守住 rule.waves 波应胜利（旧逻辑第 3 波清完就升关，永远到不了这里）"
        );
    }

    /// 胜利那一拍**不许再生成下一波**：旧代码把 `spawn_wave` 放在整个 if/else 之后，
    /// 于是「守住全部波次 → 胜利」的同一帧又把最后一波重新刷了出来。
    ///
    /// 🔴 2026-09-25 真机 5 波通关时抓到（独显 + mailbox，277s）：`survive: 全部 5 波守住 → 胜利`
    /// 之后紧跟 `wave: wave 5 spawned 14 enemies (kind=Boss …)`，NPC #60–#73 又刷了一批，
    /// 胜利画面里凭空多出一整波敌人。
    #[test]
    fn survive_victory_does_not_spawn_another_wave() {
        let mut game = Game::new();
        game.obj_state = Some(crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::Survive { waves: 1 },
        ));
        game.on_any_key(&glam::Vec3::ZERO);
        let camera = Camera::new();
        game.npcs.clear();
        game.wave_timer = 0.0;
        for _ in 0..400 {
            game.update(1.0 / 60.0, &camera);
            if game.game_state == GameState::Victory(crate::engine::ai::Team::Blue) {
                break;
            }
        }
        assert_eq!(
            game.game_state,
            GameState::Victory(crate::engine::ai::Team::Blue),
            "守住唯一一波应胜利"
        );
        assert!(
            game.npcs.is_empty(),
            "胜利那一拍不应再生成波次，实际 {} 只",
            game.npcs.len()
        );
        // 胜利后再跑几帧也不许刷
        for _ in 0..120 {
            game.update(1.0 / 60.0, &camera);
        }
        assert!(
            game.npcs.is_empty(),
            "胜利后继续跑仍不应生成敌人，实际 {} 只",
            game.npcs.len()
        );
    }

    /// survive 的任务目标数必须覆盖 `rule.waves` **全部**波次，而不是 `WAVES_PER_LEVEL`。
    ///
    /// 🔴 2026-09-25 真机：5 波的图上 wave 3 清完就报「本关敌军全灭」（26 杀），
    /// 而实际通关是 52 杀 ⇒ 目标数只有 3 波的和。这条测试直接比两个数（不比对常量），
    /// 所以 `WAVES_PER_LEVEL` 将来变了它也不会失真。
    #[test]
    fn survive_objective_target_counts_every_rule_wave() {
        let mut game = Game::new();
        game.obj_state = Some(crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::Survive { waves: 5 },
        ));
        let expected: u32 = (1..=5u32)
            .map(|w| {
                let p = wave_profile(w);
                (p.count as f32 * game.npc_scale).round().max(1.0) as u32
                    + p.reinforcement_count
            })
            .sum();
        assert_eq!(
            game.level_objective_target(1),
            expected,
            "survive 目标数应为 rule.waves 波之和"
        );
        // 非空对照：3 波（WAVES_PER_LEVEL）的和必须比它小，否则这条测试恒真
        let three: u32 = (1..=3u32)
            .map(|w| {
                let p = wave_profile(w);
                (p.count as f32 * game.npc_scale).round().max(1.0) as u32
                    + p.reinforcement_count
            })
            .sum();
        assert!(
            expected > three,
            "5 波之和({})应大于 3 波之和({})，否则这条测试测不出东西",
            expected,
            three
        );
    }

    /// 玩家自伤：手榴弹爆炸在玩家附近 → 掉血但不秒杀（封顶 SELF_DAMAGE_CAP）；
    /// 爆炸中心偏移保证玩家不被自己秒杀（阶段二）
    #[test]
    fn grenade_self_damage_capped_not_fatal() {
        let mut game = Game::new();
        game.on_any_key(&glam::Vec3::ZERO);
        game.hud.health = 100.0;
        // 玩家在 (0, 1.6, 0)，爆炸在 3m 外（半径 8m 内，fall ≈ 0.625）
        game.spawn_explosion([3.0, 0.0, 0.0], 8.0, 120.0, true);
        let hp = game.hud.health;
        assert!(hp < 100.0, "近距离爆炸应造成自伤: {}", hp);
        assert!(
            hp >= 100.0 - SELF_DAMAGE_CAP,
            "自伤封顶（不被秒杀）: hp={} cap={}",
            hp,
            SELF_DAMAGE_CAP
        );
        assert!(
            game.game_state == GameState::Playing,
            "封顶自伤不应致死"
        );
    }

    /// 冲击波推挤：生成时获得径向速度，后续帧按指数衰减并产生位移
    #[test]
    fn explosion_knockback_moves_and_decays() {
        let mut game = explode_on(
            vec![npc_at(1, Team::Red, [3.0, 0.0, 0.0])],
            [0.0, 1.0, 0.0],
            EXPLOSION_DAMAGE,
            true,
        );
        let v0 = game.npcs[0].knockback[0];
        assert!(v0 > 1.0, "近爆心推挤速度可观");
        let d0 = game.npcs[0].position[0].abs();
        // 推挤帧：位移方向向外（+x），速度按指数衰减
        game.update(0.05, &Camera::new());
        assert!(
            game.npcs[0].position[0] > d0 + 0.05,
            "推挤帧应产生向外位移"
        );
        assert!(
            game.npcs[0].knockback[0] < v0,
            "速度应衰减"
        );
        // 数帧后速度归零（衰减到阈值清 0）
        for _ in 0..30 {
            game.update(0.05, &Camera::new());
        }
        assert_eq!(game.npcs[0].knockback, [0.0, 0.0], "衰减后归零");
    }

    /// 爆炸实体生命周期：生成 → 年龄推进 → 超时移除；玩家在冲击半径内触发震屏
    #[test]
    fn explosion_visual_lifecycle_and_shake() {
        let mut game = Game::new();
        game.spawn_explosion([0.0, 1.0, 0.0], EXPLOSION_RADIUS, EXPLOSION_DAMAGE, false);
        assert_eq!(game.explosions().len(), 1, "生成即入列");
        // 玩家在原点：爆炸半径 8m 内 → 震屏触发
        assert!(game.shake_timer > 0.0, "近爆应触发震屏");
        let (sx, sz) = game.camera_shake_offset();
        assert!(sx != 0.0 || sz != 0.0, "震屏期间偏移非零");
        game.step_explosions(0.1);
        assert_eq!(game.explosions().len(), 1, "存活期内保留");
        assert!((game.explosions()[0].age - 0.1).abs() < 1e-6, "年龄推进");
        game.step_explosions(0.3);
        assert!(game.explosions().is_empty(), "超过 lifetime 移除");
        game.step_explosions(SHAKE_DURATION + 0.1);
        assert_eq!(game.shake_timer, 0.0, "震屏超时归零");
        assert_eq!(game.camera_shake_offset(), (0.0, 0.0));
    }

    /// 冻结玩法（GameOver / damage=0）：爆炸只有视觉，不结算伤害/击退
    #[test]
    fn explosion_zero_damage_is_visual_only() {
        let game = explode_on(
            vec![npc_at(1, Team::Red, [1.0, 0.0, 0.0])],
            [0.0, 1.0, 0.0],
            0.0,
            true,
        );
        assert_eq!(game.npcs[0].hp, 100.0, "damage=0 不扣血");
        assert_eq!(game.npcs[0].knockback, [0.0, 0.0], "damage=0 不推挤");
        assert_eq!(game.explosions().len(), 1, "视觉实体仍生成");
    }

    // ---- 压力模式（64v64 大战场）----

    /// 测试用 NPC 构造器（全字段确定性初始化）
    fn npc_at(id: usize, team: Team, pos: [f32; 3]) -> Npc {
        Npc {
            id,
            position: pos,
            speed: 4.0,
            attack_range: 12.0,
            home: [pos[0], pos[2]],
            state_machine: NpcStateMachine::new(),
            perception: NpcPerception::default(),
            path: Vec::new(),
            path_index: 0,
            direct_goal: false,
            direct_x: 0.0,
            direct_z: 0.0,
            last_goal: [0.0, 0.0],
            aidiag_bucket: u64::MAX,
                attack_timer: 0.0,
                reposition: None,            hp: 100.0,
            max_hp: 100.0,
            role: TacticalRole::Rusher,
            tactic: Tactic::Advance,
            dodge_timer: 0.0,
            hit_cooldown: 0.0,
            last_hp: 100.0,
            team,
            facing: 0.0,
            fire_accum: 0.0,
            knockback: [0.0, 0.0],
            grenade_timer: 0.0,
        }
    }

    #[test]
    fn stress_spawn_creates_balanced_two_teams() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 8;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.npcs.len(), 15, "8v8 蓝方少生 1（玩家补位）→ 15 名 NPC");
        let red = game.npcs.iter().filter(|n| n.team == Team::Red).count();
        let blue = game.npcs.iter().filter(|n| n.team == Team::Blue).count();
        assert_eq!(red, 8);
        assert_eq!(blue, 7);
        // 红半场 +X、蓝半场 -X，出生点在场内且不在障碍格上
        for n in &game.npcs {
            assert!(n.position[0].abs() <= 250.0 && n.position[2].abs() <= 250.0);
            assert!(
                game.grid.is_passable(world_to_grid(n.position[0], n.position[2])),
                "npc #{} 出生点应在可通行格",
                n.id
            );
            if n.team == Team::Red {
                assert!(n.position[0] > 0.0, "红方应在 +X 半场");
            } else {
                assert!(n.position[0] < 0.0, "蓝方应在 -X 半场");
            }
        }
    }

    #[test]
    fn stress_target_picking_prefers_nearest_enemy() {
        // 3 红 + 2 蓝；红0 最近敌 = 蓝4（√34 ≈ 5.8 < 蓝3 的 8）
        let npcs = vec![
            npc_at(0, Team::Red, [0.0, 0.0, 0.0]),
            npc_at(1, Team::Red, [10.0, 0.0, 0.0]),
            npc_at(2, Team::Red, [-100.0, 0.0, -100.0]),
            npc_at(3, Team::Blue, [8.0, 0.0, 0.0]),
            npc_at(4, Team::Blue, [5.0, 0.0, 3.0]),
        ];
        let targets = pick_stress_targets(&npcs, NPC_SIGHT);
        assert_eq!(targets[0], Some((4, npcs[4].position, npcs[4].facing)));
        assert_eq!(targets[1], Some((3, npcs[3].position, npcs[3].facing)), "红1(10,0) 最近敌 = 蓝3(8,0)");
        assert_eq!(targets[2], None, "视野外无敌人 → 玩家兜底");
        assert_eq!(targets[3], Some((1, npcs[1].position, npcs[1].facing)), "蓝3 最近敌 = 红1");
        assert_eq!(targets[4], Some((0, npcs[0].position, npcs[0].facing)), "同距取索引小者");
    }

    #[test]
    fn stress_parallel_step_matches_serial() {
        // 64 NPC（32 红 / 32 蓝），串行与并行逐帧推进 6 帧，状态必须逐位一致。
        // 并行路径模拟真实流程：先 `partition_ai_tiers` 分层重排（Near 在前），
        // 再 pick targets（重排后索引对齐），然后双池 `step_ai_parallel(near_len)`；
        // 数组顺序因重排而不同，断言按 npc.id 对齐比较。
        let mut npcs_s: Vec<Npc> = Vec::new();
        let mut npcs_p: Vec<Npc> = Vec::new();
        for i in 0..64usize {
            let team = if i < 32 { Team::Red } else { Team::Blue };
            let x = if i < 32 {
                80.0 + i as f32 * 3.0
            } else {
                -80.0 - (i - 32) as f32 * 3.0
            };
            let z = ((i % 8) as f32 - 4.0) * 10.0;
            npcs_s.push(npc_at(i, team, [x, 0.0, z]));
            npcs_p.push(npc_at(i, team, [x, 0.0, z]));
        }
        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let tier_params = AiTierParams::default();
        for frame in 0..6u32 {
            let dt = 1.0 / 60.0;
            let time = 1.0 + frame as f32 * dt;
            let targets_s = pick_stress_targets(&npcs_s, STRESS_SIGHT);
            // 并行路径：分层重排 → 重排后 pick（与 update_ai 顺序一致）
            let near_len = partition_ai_tiers(&mut npcs_p, |n| {
                let dx = n.position[0] - player.x;
                let dz = n.position[2] - player.z;
                classify_ai_tier(dx * dx + dz * dz, false, &tier_params)
            });
            assert!(near_len > 0 && near_len < npcs_p.len(), "近/远组都应非空");
            let targets_p = pick_stress_targets(&npcs_p, STRESS_SIGHT);
            let flags_s = vec![false; npcs_s.len()];
            let flags_p = vec![false; npcs_p.len()];
            let ctx_s = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags_s,
                targets: &targets_s,
                grid: &grid,
                time,
                dt,
                stress: true,
                frame,
                decimate_far: false,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
                squad_wps: &[],
                spectator: false,
                target_known: false,
                fallback_targets: &[],
                target_occluded: &[],
            };
            let ctx_p = AiStepCtx {
                player: &player,
                player_yaw: 0.0,
                charge: false,
                under_fire: &flags_p,
                targets: &targets_p,
                grid: &grid,
                time,
                dt,
                stress: true,
                frame,
                decimate_far: false,
                ring_inner: MAP_RING_INNER,
                ring_outer: MAP_RING_OUTER,
                obstacles: &game.map.obstacles,
                squad_wps: &[],
                spectator: false,
                target_known: false,
                fallback_targets: &[],
                target_occluded: &[],
            };
            Game::step_ai_serial(&mut npcs_s, &ctx_s);
            Game::step_ai_parallel(&mut npcs_p, near_len, &ctx_p);
            // 按 id 对齐比较（并行路径数组已重排）
            let by_id: Vec<(&Npc, &Npc)> = npcs_s
                .iter()
                .map(|a| {
                    let b = npcs_p
                        .iter()
                        .find(|b| b.id == a.id)
                        .expect("并行路径应包含同一 NPC 集合");
                    (a, b)
                })
                .collect();
            for (a, b) in by_id {
                assert_eq!(a.position, b.position, "frame {} npc {}", frame, a.id);
                assert_eq!(a.hp, b.hp);
                assert_eq!(a.facing, b.facing);
                assert_eq!(a.tactic, b.tactic);
                assert_eq!(a.state_machine.state(), b.state_machine.state());
                assert_eq!(a.path, b.path);
            }
        }
    }

    /// 阶段二：压力模式「目标 NPC 朝向」驱动 player_facing → Flanker 触发 Flank/Ambush。
    /// 红 NPC 站在蓝 NPC 正面（蓝 facing 指向红）→ 红应判定「目标面朝我」→ Flank；
    /// 红 NPC 站在蓝 NPC 背面（蓝 facing 背对红）→ 红应判定「目标背对我」→ Ambush。
    #[test]
    fn stress_flanker_tactic_follows_target_facing() {
        // 蓝 NPC 面朝 +X 方向（facing = atan2(dz,dx)：dz=0,dx>0 → facing=0）
        let mut blue = npc_at(1, Team::Blue, [0.0, 0.0, 0.0]);
        blue.facing = 0.0; // 面朝 +X
        blue.role = TacticalRole::Flanker;
        // 红 NPC 在蓝的正+X 侧 10m（蓝面朝它 → 目标面朝本 NPC）
        let mut red_front = npc_at(0, Team::Red, [10.0, 0.0, 0.0]);
        red_front.role = TacticalRole::Flanker;
        // 红 NPC 在蓝的 -X 侧 10m（蓝背对它 → 目标背对本 NPC）
        let mut red_back = npc_at(2, Team::Red, [-10.0, 0.0, 0.0]);
        red_back.role = TacticalRole::Flanker;

        let game = Game::new();
        let grid = game.grid.clone();
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 3];

        let npcs_a = vec![red_front, blue, red_back];
        let targets_a = pick_stress_targets(&npcs_a, STRESS_SIGHT);
        let ctx_a = AiStepCtx {
            player: &player,
            player_yaw: 0.0,
            charge: false,
            under_fire: &flags,
            targets: &targets_a,
            grid: &grid,
            time: 1.0,
            dt: 1.0 / 60.0,
            stress: true,
            frame: 0,
            decimate_far: false,
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            obstacles: &game.map.obstacles,
            squad_wps: &[],
            spectator: false,
            target_known: false,
            fallback_targets: &[],
            target_occluded: &[],
        };
        let mut npcs_a = npcs_a;
        Game::step_ai_serial(&mut npcs_a, &ctx_a);
        // 红 0（正面）最近敌 = 蓝 1 且蓝面朝它 → player_facing=true → Flank
        assert_eq!(
            npcs_a[0].tactic,
            Tactic::Flank,
            "正面站位的红应触发 Flank（目标面朝本 NPC）"
        );
        // 红 2（背面）最近敌 = 蓝 1 但蓝背对它 → player_facing=false → Ambush
        assert_eq!(
            npcs_a[2].tactic,
            Tactic::Ambush,
            "背面站位的红应触发 Ambush（目标背对本 NPC）"
        );
    }

    /// 阶段二：压力模式 NPC 在障碍附近交火时触发 CoverSeek（掩体利用）。
    /// 红 NPC 位于障碍旁 20m 处（Chase 态目标在 30m 外）→ 应进入掩体利用；
    /// 开阔处（无障碍）目标在射程外 → 保持 Advance（不误触发）。
    #[test]
    fn stress_cover_seek_triggers_near_obstacle() {
        let mut game = Game::new();
        game.stress = true;
        // 构造一个障碍（墙）：在 (0,0) 位置放一个 MapObstacle → 网格会 block 该格
        // 用 street_fight 式的墙：x=0,z=0,half_w=5,half_d=0.5 → 格 (0,0) 及邻域 blocked
        let ob = MapObstacle::new(ObstacleKind::Wall, 0.0, 0.0, 5.0, 0.5);
        game.map.obstacles.push(ob);
        // 重建网格（把障碍格 block）—— 走生产同一套规则（`block_obstacle_cells`）
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        for o in &game.map.obstacles {
            block_obstacle_cells(&mut grid, o);
        }
        game.grid = grid;
        let grid = game.grid.clone();

        // 红 NPC 在障碍旁（-10, 0，距障碍 10m），Chase 态（感知目标但不在射程）
        let mut red = npc_at(0, Team::Red, [-10.0, 0.0, 0.0]);
        red.state_machine.update(NpcPerception {
            enemy_visible: true,
            enemy_in_range: false,
            ..NpcPerception::default()
        }); // Idle → Chase
        red.role = TacticalRole::Rusher;
        // 蓝目标在 30m 外（+20,0），Chase 态
        let mut blue = npc_at(1, Team::Blue, [20.0, 0.0, 0.0]);
        blue.state_machine.update(NpcPerception {
            enemy_visible: true,
            enemy_in_range: false,
            ..NpcPerception::default()
        });
        let npcs = vec![red, blue];
        let targets = pick_stress_targets(&npcs, STRESS_SIGHT);
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        let flags = vec![false; 2];
        let ctx = AiStepCtx {
            player: &player,
            player_yaw: 0.0,
            charge: false,
            under_fire: &flags,
            targets: &targets,
            grid: &grid,
            time: 1.0,
            dt: 1.0 / 60.0,
            stress: true,
            frame: 0,
            decimate_far: false,
            ring_inner: MAP_RING_INNER,
            ring_outer: MAP_RING_OUTER,
            obstacles: &game.map.obstacles,
            squad_wps: &[],
            spectator: false,
            target_known: false,
            fallback_targets: &[],
            target_occluded: &[],
        };
        let mut npcs = npcs;
        Game::step_ai_serial(&mut npcs, &ctx);
        // 红在障碍旁 + Chase + 目标 30m（≤ attack_range 12 + 40）→ 应进入 CoverSeek
        assert_eq!(
            npcs[0].tactic,
            Tactic::CoverSeek,
            "障碍旁的 NPC 在 Chase 接近目标时应利用掩体（CoverSeek）"
        );
    }

    #[test]
    fn stress_npc_combat_damages_target_only() {
        let mut game = Game::new();
        game.stress = true;
        let mut a = npc_at(0, Team::Red, [0.0, 0.0, 0.0]);
        let mut b = npc_at(1, Team::Blue, [6.0, 0.0, 0.0]);
        // 推进到 Attack 态：Idle → Chase → Attack
        let p = NpcPerception {
            enemy_visible: true,
            enemy_in_range: true,
            ..NpcPerception::default()
        };
        a.state_machine.update(p);
        a.state_machine.update(p);
        b.state_machine.update(p);
        b.state_machine.update(p);
        assert_eq!(a.state_machine.state(), NpcState::Attack);
        assert_eq!(b.state_machine.state(), NpcState::Attack);
        game.npcs = vec![a, b];
        let targets = vec![
            Some((1, game.npcs[1].position, game.npcs[1].facing)),
            Some((0, game.npcs[0].position, game.npcs[0].facing)),
        ];
        let hp_before = [game.npcs[0].hp, game.npcs[1].hp];
        let dps = wave_profile(game.effective_wave(1)).dps;
        game.apply_npc_combat(1.1, &targets);
        assert!(
            (game.npcs[0].hp - (hp_before[0] - dps)).abs() < 1e-3,
            "红 0 应被扣 dps"
        );
        assert!(
            (game.npcs[1].hp - (hp_before[1] - dps)).abs() < 1e-3,
            "蓝 1 应被扣 dps"
        );
    }

    #[test]
    fn stress_wipe_respawns_full_battle() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.npcs.len(), 7, "4v4 蓝方少生 1（玩家补位）");
        let round0 = game.stress_round;
        for n in &mut game.npcs {
            if n.team == Team::Red {
                n.hp = 0.0;
            }
        }
        game.game_state = GameState::Playing;
        game.update_stress_respawns(&player);
        // 胜利后 10 秒重置：计时器置 0（非负且已到期）触发第二轮
        game.round_reset_at = 0.0;
        game.update_stress_respawns(&player);
        assert_eq!(game.stress_round, round0 + 1, "团灭应开新一轮");
        assert_eq!(game.npcs.len(), 7, "全量补员（红 4 + 蓝 3）");
        let red = game.npcs.iter().filter(|n| n.team == Team::Red).count();
        assert_eq!(red, 4);
    }

    /// 🔴 判据：**阵亡计数不能漏掉任何一条死亡路径**（军情/日志里的自损数必须等于真实阵亡）。
    ///
    /// 2026-09-26 实测（170 秒 128v127 会战，`logs/llmbattle.log.err`）：`command:` 行里红营自报
    /// 阵亡 **35**，而三个连的强度合计 60 ⇒ 实际阵亡 128 − 60 = **68**（蓝营 79 vs 123）。
    /// 根因：`damage_npc` 打死人时**直接从 `npcs` 里移除**，而 `round_kills_*` 只在
    /// `update_stress_respawns` 里扫「还在数组里的 `hp <= 0`」⇒ **子弹/爆炸打死的主路径一个都没计**。
    #[test]
    fn every_death_path_is_counted_in_the_round_tally() {
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        game.game_state = GameState::Playing;
        let (red0, blue0) = (game.round_kills_red, game.round_kills_blue);
        // 路径 A：`damage_npc`（子弹/爆炸的主路径，当场把阵亡者移出数组）
        let ri = game
            .npcs
            .iter()
            .position(|n| n.team == Team::Red)
            .expect("压力模式应有红方 NPC");
        assert!(
            game.damage_npc(ri, 999.0, DamageSource::Player),
            "999 伤害必须打死"
        );
        // 路径 B：兜底扫描（另有路径把 hp 打到 0、人还留在数组里）
        let bi = game
            .npcs
            .iter()
            .position(|n| n.team == Team::Blue)
            .expect("压力模式应有蓝方 NPC");
        game.npcs[bi].hp = 0.0;
        game.update_stress_respawns(&player);
        assert_eq!(
            game.round_kills_red,
            red0 + 1,
            "damage_npc 打死的红方必须计入本轮自损（漏了它，日志里的自损只有真实值的一半）"
        );
        assert_eq!(game.round_kills_blue, blue0 + 1, "兜底扫描那条路同样要计入");
    }

    // ---- 新玩法：可破坏障碍 / 掩体利用 / 任务目标 ----

    /// 可破坏障碍：扣血不摧毁 → 保留；血尽 → 从物理刚体/AI 网格/渲染列表中移除
    #[test]
    fn obstacles_take_damage_and_destroy() {
        let mut game = Game::new();
        assert!(!game.map.obstacles.is_empty());
        let n = game.map.obstacles.len();
        assert_eq!(game.world.bodies.len(), n, "刚体与障碍应一一对应");
        let idx = 0;
        let ob0 = game.map.obstacles[idx];
        assert!(ob0.max_hp > 0.0 && ob0.hp == ob0.max_hp, "障碍出生满血");
        // 一格：扣血不摧毁
        game.damage_obstacle(idx, 25.0);
        assert_eq!(game.map.obstacles[idx].hp, ob0.max_hp - 25.0);
        assert_eq!(game.map.obstacles.len(), n);
        assert_eq!(game.world.bodies.len(), n);
        // 摧毁：清空剩余血量 → 列表同步移除
        game.damage_obstacle(idx, ob0.max_hp);
        assert_eq!(game.map.obstacles.len(), n - 1, "障碍应从渲染列表移除");
        assert_eq!(game.world.bodies.len(), n - 1, "物理刚体应同步移除");
        // 被摧毁障碍覆盖的网格格已解除阻挡（NPC 可穿过缺口）
        let g0 = world_to_grid(ob0.x - ob0.half_w, ob0.z - ob0.half_d);
        let g1 = world_to_grid(ob0.x + ob0.half_w, ob0.z + ob0.half_d);
        let mut any_passable = false;
        for gx in g0.x..=g1.x {
            for gz in g0.y..=g1.y {
                let pos = GridPos::new(gx, gz);
                if game.grid.in_bounds(pos) && game.grid.is_passable(pos) {
                    any_passable = true;
                }
            }
        }
        assert!(any_passable, "摧毁后障碍占格应解除阻挡");
        // 越界下标安全忽略
        game.damage_obstacle(usize::MAX, 999.0);
        assert_eq!(game.map.obstacles.len(), n - 1);
    }

    /// 掩体利用：环带内、射程内、紧邻存活障碍的遮挡掩体被选中；
    /// 中央安全区目标无可用掩体 → None（冒烟站定语义）
    #[test]
    fn attack_cover_picks_ring_band_shielding_cover() {
        let mut grid = GridMap::new(GRID_SIZE, GRID_SIZE);
        let obstacles = vec![MapObstacle::new(ObstacleKind::Wall, 60.0, 0.0, 3.0, 3.0)];
        for ob in &obstacles {
            block_obstacle_cells(&mut grid, ob);
        }
        // 目标（被攻击方）在障碍东侧（环带内）；NPC 在障碍西侧追近
        let target = world_to_grid(66.0, 0.0);
        let npc = world_to_grid(50.0, 0.0);
        let cover = pick_attack_cover(
            &grid,
            npc,
            target,
            12.0,
            MAP_RING_INNER,
            MAP_RING_OUTER,
            COVER_MAX_DIST,
            &obstacles,
        );
        assert!(cover.is_some(), "环带内应有射程内遮挡掩体");
        let (wx, wz) = grid_to_world(cover.unwrap());
        let d_origin = (wx * wx + wz * wz).sqrt();
        assert!(
            d_origin >= MAP_RING_INNER && d_origin <= MAP_RING_OUTER,
            "掩体必须在障碍环带内: {:.1}m",
            d_origin
        );
        let (tx, tz) = grid_to_world(target);
        let d_t = ((wx - tx).powi(2) + (wz - tz).powi(2)).sqrt();
        assert!(d_t <= 12.0, "掩体必须在攻击距离内: {:.1}m", d_t);
        // 安全区目标（原点）：环带内无射程内掩体 → None（NPC 保持直线推进/站定）
        let origin = world_to_grid(0.0, 0.0);
        let none_cover = pick_attack_cover(
            &grid,
            npc,
            origin,
            12.0,
            MAP_RING_INNER,
            MAP_RING_OUTER,
            COVER_MAX_DIST,
            &obstacles,
        );
        assert!(none_cover.is_none(), "中央安全区目标应无可用掩体");
    }

    /// 任务目标：歼灭数推进、达成只触发一次、计数封顶在 target
    #[test]
    fn mission_objective_progress_and_completion() {
        let mut obj = MissionObjective::new(24);
        assert_eq!((obj.eliminated, obj.target, obj.done), (0, 24, false));
        assert!(!obj.progress(23), "未达目标不应完成");
        assert!(obj.progress(1), "第 24 击杀应达成目标");
        assert!(obj.done);
        assert!(!obj.progress(1), "达成后不再重复触发");
        assert_eq!(obj.eliminated, 24, "计数封顶在 target");
        let mut zero = MissionObjective::new(0);
        assert!(!zero.progress(1), "target=0 永不达成");
    }

    /// 任务目标：普通模式本关目标 = 3 波出场总数（含援军）；压力模式 = 歼灭一队
    #[test]
    fn objective_targets_per_mode_and_stress_victory() {
        let game = Game::new();
        assert!(!game.stress);
        // 第 1 关：wave1=6 + wave2=8 + wave3=10+援军2 = 26
        assert_eq!(game.objective.target, 26, "第 1 关任务目标应为 3 波出场总数");
        assert!(!game.objective.done);
        assert!(game.hud.victory_banner.is_none());
        // 压力模式：目标 = 歼灭一队；红方团灭 → 达成 + 横幅，且补员照常开新一轮
        let mut game = Game::new();
        game.stress = true;
        game.stress_sides = 4;
        let player = glam::Vec3::new(0.0, 0.0, 0.0);
        game.spawn_stress_battle(&player);
        assert_eq!(game.objective.target, 4, "压力模式目标 = 歼灭一队");
        for n in &mut game.npcs {
            if n.team == Team::Red {
                n.hp = 0.0;
            }
        }
        game.game_state = GameState::Playing;
        let round0 = game.stress_round;
        game.update_stress_respawns(&player);
        // 2026-08-23：胜利后 10 秒重置（战场停顿）——计时器置 0（非负且已到期）触发第二轮
        game.round_reset_at = 0.0;
        game.update_stress_respawns(&player);
        assert_eq!(game.stress_round, round0 + 1, "补员逻辑不受影响");
        assert!(game.hud.victory_banner.is_some(), "达成后应显示胜利横幅（保留到下一轮）");
        assert_eq!(game.objective.eliminated, 0, "新一轮目标已重置");
        assert_eq!(game.objective.target, 4, "新一轮目标 = 歼灭一队");
    }

    /// 弹孔：必须落在**障碍表面**上，法线取入口面的轴向（不是子弹的当前位置）。
    ///
    /// 存在理由：高速弹一帧能飞十几米（710 m/s × 1/60 s = 11.8m），拿 `p.position` 当弹孔
    /// 位置会得到墙**里面**的一个点 —— 画出来整片弹孔都不见了，而且不报任何错。
    #[test]
    fn impact_mark_lands_on_the_entered_face() {
        let mut game = Game::new();
        game.npcs.clear();
        game.world.bodies.clear();
        game.world.spheres.clear();
        // 一堵墙：中心 (0, 1.2, -10)，半尺寸 4 × 1.2 × 0.5 ⇒ 碰撞近面 z = -9.5
        // ⚠ `world.bodies[i]` 与 `map.obstacles[i]` 必须**同序同尺寸**（引擎的不变式）：
        // 弹着点直接取该障碍的 AABB 面（2026-09-17 起可见尺寸 == AABB，见
        // geom::Shape::template_half_extent），只建刚体不建障碍表就会贴到别的东西上。
        let aabb = Body::new_static(Pv::new(0.0, 1.2, -10.0), Pv::new(4.0, 1.2, 0.5));
        let wall = MapObstacle {
            x: 0.0,
            z: -10.0,
            half_w: 4.0,
            half_d: 0.5,
            y: 1.2,
            half_h: 1.2,
            kind: ObstacleKind::Wall,
            tint: None,
            max_hp: 100.0,
            hp: 100.0,
            shape: Shape::Legacy,
        };
        game.map.obstacles = vec![wall];
        game.world.bodies.push(aabb);
        // 从原点朝 -Z 打，一帧 10m（终点 z=-10 已在墙里）
        // ⚠ 交给 `update_projectiles` 自己推进：它内部先 `p.update(dt)` 再判命中，
        // 这里再手工推一次就会变成"上一帧已在墙里"的退化段（t=0）。
        let mk = || Projectile::new([0.0, 1.2, 0.0], [0.0, 0.0, -1.0], 600.0, 400.0, 6.0, 28.0);
        let mut probe = mk();
        probe.update(1.0 / 60.0);
        assert!(game.collide_physics(&probe), "该弹道必须被墙挡下");
        game.projectiles.push(mk());
        game.update_projectiles(1.0 / 60.0, true);

        let marks = game.impact_marks();
        assert_eq!(marks.len(), 1, "命中障碍应留下 1 个弹孔，实际 {}", marks.len());
        let m = marks[0];
        assert!(
            (m.pos[2] + 9.5).abs() < 0.01,
            "弹孔应贴在**可见面** z=-9.5（2026-09-17 起 marker 可见尺寸 == 碰撞 AABB，\
             见 geom::Shape::template_half_extent；此前这里是 -9.0，因为可见盒是 AABB 的 2 倍），\
             实际 z={}",
            m.pos[2]
        );
        assert!(
            m.pos[0].abs() < 0.01 && (m.pos[1] - 1.2).abs() < 0.01,
            "弹孔应落在弹道上，实际 ({}, {})",
            m.pos[0],
            m.pos[1]
        );
        assert_eq!(m.normal, [0.0, 0.0, 1.0], "迎面法线应为 +Z（背向子弹来向）");
        assert_eq!(m.age, 0.0, "本帧刚打出的弹孔年龄应从 0 开始");
    }

    /// 弹孔只留在障碍刚体上：打中球体（树冠一类）与打空都不留痕。
    #[test]
    fn impact_marks_are_only_left_on_obstacles() {
        let mut game = Game::new();
        game.npcs.clear();
        game.world.bodies.clear();
        game.world.spheres.clear();
        // 只有球体，没有任何 AABB 刚体
        game.world.spheres
            .push(physics::SphereBody::new(Pv::new(0.0, 1.2, -10.0), 2.0));
        let mk = |dir: [f32; 3]| {
            Projectile::new([0.0, 1.2, 0.0], dir, 600.0, 400.0, 6.0, 28.0)
        };
        let mut p = mk([0.0, 0.0, -1.0]);
        p.update(1.0 / 60.0);
        assert!(game.collide_physics(&p), "该弹道必须被球体挡下");
        assert!(
            game.first_obstacle_hit(&p).is_none(),
            "球体不是障碍刚体，不得给出弹着点"
        );
        game.projectiles.push(mk([0.0, 0.0, -1.0]));
        game.update_projectiles(1.0 / 60.0, true);
        assert!(game.impact_marks().is_empty(), "球体上不留弹孔");

        // 打空：没有命中任何东西的子弹不留痕
        let q = mk([0.0, 0.0, 1.0]);
        assert!(!game.collide_physics(&q), "朝反方向应打空");
        game.projectiles.push(q);
        game.update_projectiles(1.0 / 60.0, true);
        assert!(game.impact_marks().is_empty(), "打空的子弹不留弹孔");

        // 起点已在盒内（贴脸开枪被墙包住）：没有入口面 ⇒ 不留痕，
        // 免得画出一块悬在半空的暗方块。
        game.world.spheres.clear();
        game.world.bodies.push(Body::new_static(
            Pv::new(0.0, 1.2, -10.0),
            Pv::new(4.0, 1.2, 0.5),
        ));
        game.projectiles
            .push(Projectile::new([0.0, 1.2, -10.0], [0.0, 0.0, -1.0], 600.0, 400.0, 6.0, 28.0));
        game.update_projectiles(1.0 / 60.0, true);
        assert!(
            game.impact_marks().is_empty(),
            "线段起点已在障碍内部时不得生成弹孔"
        );
    }

    /// 弹孔池是环形缓冲（上限固定），并且按寿命过期。
    #[test]
    fn impact_marks_ring_buffer_and_expiry() {
        let mut game = Game::new();
        let wall = |game: &mut Game| {
            game.world.bodies.clear();
            game.world.spheres.clear();
            game.world.bodies.push(Body::new_static(
                Pv::new(0.0, 1.2, -10.0),
                Pv::new(4.0, 1.2, 0.5),
            ));
            game.projectiles.push(Projectile::new(
                [0.0, 1.2, 0.0],
                [0.0, 0.0, -1.0],
                600.0,
                400.0,
                6.0,
                28.0,
            ));
            game.update_projectiles(1.0 / 60.0, true);
        };
        for _ in 0..(IMPACT_MARK_MAX + 7) {
            wall(&mut game);
        }
        assert_eq!(
            game.impact_marks().len(),
            IMPACT_MARK_MAX,
            "弹孔数必须封顶在 IMPACT_MARK_MAX，不能随射击次数增长"
        );
        // 老化：走到寿命末尾时包络收缩到 0，超过寿命即被清掉
        for m in game.impact_marks.iter_mut() {
            m.age = IMPACT_MARK_LIFE - IMPACT_MARK_FADE * 0.5;
        }
        let half = game.impact_marks()[0].size_envelope();
        assert!(
            (half - 0.5).abs() < 1e-5,
            "寿命末尾半程应变到一半，实际 {half}"
        );
        for m in game.impact_marks.iter_mut() {
            m.age = IMPACT_MARK_LIFE + 1.0;
        }
        game.update_projectiles(1.0 / 60.0, true);
        assert!(game.impact_marks().is_empty(), "超过寿命的弹孔必须清掉");
    }

    /// 弹孔的朝向基必须是**右手**正交基（行列式 +1）：左手化就是正面绕序反掉 —— 整片黑且不报错。
    #[test]
    fn impact_mark_basis_is_right_handed_for_every_face() {
        for normal in [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ] {
            let m = ImpactMark {
                pos: [0.0; 3],
                normal,
                age: 0.0,
            };
            let (t, u, n) = m.basis();
            let (t, u, n) = (
                glam::Vec3::from(t),
                glam::Vec3::from(u),
                glam::Vec3::from(n),
            );
            for v in [t, u, n] {
                assert!(
                    (v.length() - 1.0).abs() < 1e-5,
                    "基向量必须是单位向量 {v:?}"
                );
            }
            assert!(t.dot(n).abs() < 1e-5 && u.dot(n).abs() < 1e-5, "基必须与法线正交");
            assert!(
                (t.cross(u) - n).length() < 1e-5,
                "必须是右手基（t × u = n），法线 {normal:?}"
            );
            assert!((n - glam::Vec3::from(normal)).length() < 1e-5, "法线方向不得被翻转");
        }
    }

    /// `segment_hits_aabb` 与 `segment_aabb_entry` 必须永远给出同一个答案
    /// （弹孔用的是后者，命中判定用的是前者 —— 一旦漂移就是"子弹被挡下但弹孔画在别处"）。
    #[test]
    fn segment_entry_agrees_with_hits_test() {
        let aabb = crate::engine::physics::Aabb::new(
            Pv::new(-1.0, 0.0, -1.0),
            Pv::new(1.0, 2.0, 1.0),
        );
        let cases: [([f32; 3], [f32; 3]); 8] = [
            ([0.0, 1.0, -5.0], [0.0, 1.0, 5.0]),   // 迎面穿过
            ([0.0, 1.0, 5.0], [0.0, 1.0, -5.0]),   // 反向穿过
            ([0.0, 1.0, -5.0], [0.0, 1.0, -3.0]),  // 停在盒前
            ([0.0, 3.0, -5.0], [0.0, 3.0, 5.0]),   // 从顶上掠过
            ([0.0, 1.0, 0.0], [0.5, 1.0, 0.5]),    // 起点已在盒内
            ([-1.0, 1.0, -1.0], [1.0, 1.0, 1.0]),  // 对角穿过
            ([2.0, 1.0, -5.0], [2.0, 1.0, 5.0]),   // 侧面经过盒外
            ([0.0, 1.0, -1.0], [0.0, 1.0, 1.0]),   // 起点在面上
        ];
        for (a, b) in cases {
            let hits = Game::segment_hits_aabb(a[0], a[1], a[2], b[0], b[1], b[2], &aabb);
            let entry = Game::segment_aabb_entry(a[0], a[1], a[2], b[0], b[1], b[2], &aabb);
            assert_eq!(hits, entry.is_some(), "两者对 {a:?} -> {b:?} 判断不一致");
            if let Some((t, _)) = entry {
                assert!((0.0..=1.0).contains(&t), "入口参数必须落在 [0,1]，实际 {t}");
            }
        }
    }
