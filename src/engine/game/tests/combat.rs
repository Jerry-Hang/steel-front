// 由 src/engine/game/tests.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `tests` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

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
