// 由 src/engine/game/tests.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `tests` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

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
