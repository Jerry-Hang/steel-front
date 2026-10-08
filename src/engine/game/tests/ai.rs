// 由 src/engine/game/tests.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `tests` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
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
