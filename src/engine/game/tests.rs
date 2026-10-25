// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
    use super::*;


















































































    /// 测试用爆炸：替换 npcs 后引爆，返回爆炸实体引用
    fn explode_on(npcs: Vec<Npc>, center: [f32; 3], damage: f32, knockback: bool) -> Game {
        let mut game = Game::new();
        game.npcs = npcs;
        game.spawn_explosion(center, EXPLOSION_RADIUS, damage, knockback);
        game
    }





















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

















// 子模块（见 docs/refactor-plan.md）
mod ai;
mod combat;
mod session;

    /// 🔴 **重开一局必须清掉 ESC 菜单状态**（2026-10-20 自查发现）。
    ///
    /// 菜单打开时 `update()` 照常推进 ⇒ 玩家可以"开着菜单被打死"→ 结算按 R 重开；
    /// 旧实现只复位 `settings_open`/`confirm_quit`，`esc_menu_open` 会残留到新一局
    /// （遮罩盖着新局、光标不抓取、选中项停在上一局的位置）。
    /// 反向验证：删掉 `start_run` 里那两行复位 ⇒ 本判据红。
    #[test]
    fn start_run_clears_stale_escape_menu() {
        let mut game = Game::new();
        game.hud.esc_menu_open = true;
        game.hud.esc_menu_selection = 1;
        game.start_run();
        assert!(!game.hud.esc_menu_open, "重开后 ESC 菜单必须关闭");
        assert_eq!(game.hud.esc_menu_selection, 0, "重开后菜单选中项必须归零");
    }

    /// 🔴 **重开一局必须复位姿态/冲刺/治疗/战术道具**（2026-10-20 第 14 批复核第 1 条）。
    /// 反向验证：删掉 `start_run_at` 里那几行复位 ⇒ 本判据红。
    #[test]
    fn start_run_resets_stance_sprint_heal_and_consumables() {
        let mut g = Game::new();
        g.on_any_key();
        g.toggle_prone();
        g.set_sprint(true);
        g.heal_timer = 1.5;
        g.grenades = 0;
        g.medkits = 0;
        g.start_run();
        assert_eq!(g.stance, Stance::Standing, "重开后必须回到站姿");
        assert!(
            (g.player_body.eye_height - Stance::Standing.eye_height()).abs() < 1e-5,
            "重开后物理身高必须同步回站姿"
        );
        assert!(!g.sprint_held, "重开后冲刺按住状态必须清零");
        assert_eq!(g.heal_timer, 0.0, "重开后打药计时必须清零");
        assert_eq!(g.grenades, g.grenades_max, "重开后手榴弹必须补满");
        assert_eq!(g.medkits, g.medkits_max, "重开后医疗包必须补满");
    }

    /// 🔴 **蹲/趴必须同步物理碰撞身高**（2026-10-20 第 14 批复核第 4 条）。
    /// `clamp_vertical` / `push_out_of_aabb` 读的是 `player_body.eye_height`，
    /// 只改 `stance` 会让物理身高恒为 1.6m。
    /// 反向验证：删掉 `toggle_crouch`/`toggle_prone` 里的 `eye_height` 同步 ⇒ 红。
    #[test]
    fn stance_toggle_syncs_physics_eye_height() {
        let mut g = Game::new();
        g.on_any_key();
        assert!((g.player_body.eye_height - Stance::Standing.eye_height()).abs() < 1e-5);
        g.toggle_crouch();
        assert_eq!(g.stance, Stance::Crouching);
        assert!(
            (g.player_body.eye_height - Stance::Crouching.eye_height()).abs() < 1e-5,
            "下蹲后物理身高 {} 未同步到 {}",
            g.player_body.eye_height,
            Stance::Crouching.eye_height()
        );
        g.toggle_prone();
        assert_eq!(g.stance, Stance::Prone);
        assert!(
            (g.player_body.eye_height - Stance::Prone.eye_height()).abs() < 1e-5,
            "卧倒后物理身高未同步"
        );
        g.toggle_crouch();
        assert_eq!(g.stance, Stance::Standing);
        assert!((g.player_body.eye_height - Stance::Standing.eye_height()).abs() < 1e-5);
    }

    /// 🔴 **进下一关必须停在"下一关"**（2026-10-20 第 14 批复核第 1 条后半）：
    /// 旧实现 `advance_level` 调 `start_run()`，而 `start_run` 内部硬编码 `apply_level(1)`
    /// ⇒ 关卡号被重置回 1、跨关难度加成全部失效、地图白建两次。
    /// 反向验证：把 `start_run_at((self.level_idx + 1) as u32)` 改回 `self.start_run()` ⇒ 红。
    #[test]
    fn advance_level_keeps_the_next_level_number() {
        let mut g = Game::new();
        g.on_any_key();
        g.level_list = vec![
            "assets/maps/street_fight.toml".to_string(),
            "assets/maps/open_field.toml".to_string(),
        ];
        let first = crate::engine::map::MapManager::load(&g.level_list[0]).expect("第 1 关必须可加载");
        g.map_mgr = Some(first);
        g.level_idx = 0;
        g.level = 1;
        assert!(g.advance_level(), "还有下一关时必须成功");
        assert_eq!(g.level_idx, 1);
        assert_eq!(g.level, 2, "进下一关后 `level` 必须变成 2（旧实现被重置回 1）");
        let want = crate::engine::map::MapManager::load(&g.level_list[1])
            .expect("第 2 关必须可加载")
            .data()
            .name
            .clone();
        assert_eq!(
            g.map_mgr.as_ref().expect("map_mgr 应已切换").data().name,
            want,
            "必须切到第 2 关的地图（open_field）"
        );
    }

    /// 🔴 **据点/限时胜利必须有可见提示**（2026-10-20 第 14 批复核第 6 条）：
    /// `view.rs` 把 `Victory` 映射成 `HudScreen::Game` 且非 Playing 时提前返回，
    /// 只有 `victory_banner` 能显示文字；不置它玩家会以为卡死。
    /// 反向验证：删掉 `update_objectives` 里设置横幅那行 ⇒ 红。
    #[test]
    fn objective_victory_sets_a_visible_banner() {
        use crate::engine::ai::Team;
        use crate::engine::objective::{CapturePoint, GameRule, ObjectiveState};
        let mut g = Game::new();
        g.on_any_key();
        let mut st = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        let mut pt = CapturePoint::new("A", 0.0, 0.0, 5.0, 5.0);
        pt.owner = Some(Team::Blue);
        st.points.push(pt);
        g.obj_state = Some(st);
        g.update_objectives(0.0, &glam::Vec3::ZERO);
        assert!(
            matches!(g.game_state, GameState::Victory(_)),
            "占领 required 个据点后应判胜利"
        );
        let banner = g.hud.victory_banner.as_deref().unwrap_or("");
        assert!(
            banner.contains("VICTORY"),
            "胜利后 HUD 必须有 VICTORY 提示（否则画面冻结、玩家以为卡死），实际 {banner:?}"
        );
    }

    /// 🔴 **连发必须受射速冷却约束**（2026-10-20 第 15 批复核第 1 条）。
    /// 旧 `fire_burst` 只查切枪计时、不查 `fire_cooldown`，而末尾又把冷却设成
    /// `interval*rounds` ⇒ 连点左键可按帧连打三连发。
    /// 反向验证：删掉 `fire_burst` 里的 `if self.fire_cooldown > 0.0` ⇒ 本判据红。
    #[test]
    fn burst_fire_respects_the_fire_cooldown() {
        let mut g = Game::new();
        g.on_any_key();
        let first = g.fire_burst_player([0.0, 1.6, 0.0], [0.0, 0.0, -1.0], 3);
        assert!(first > 0, "首次三连发应打出子弹（前提：默认武器有弹）");
        assert!(g.fire_cooldown > 0.0, "三连发之后必须立起冷却计时");
        let second = g.fire_burst_player([0.0, 1.6, 0.0], [0.0, 0.0, -1.0], 3);
        assert_eq!(
            second, 0,
            "冷却期内再次扣扳机必须被拦下（实打 {second} 发）—— 否则射速冷却形同虚设"
        );
    }

    /// 🔴 **切枪必须复位上一把枪的冷却与控枪热量**（2026-10-20 第 15 批复核第 3 条）：
    /// 不清 ⇒ 高射速/三连发后切手枪「切枪动画已结束却扣不动扳机」，
    /// 或步枪连射热量污染狙击枪首发后坐力。
    /// 反向验证：删掉 `reset_weapon_transients_after_switch` 里的两行 ⇒ 本判据红。
    #[test]
    fn switching_weapon_clears_previous_cooldown_and_heat() {
        let mut g = Game::new();
        g.on_any_key();
        g.fire_cooldown = 2.5;
        g.auto_heat = 1.0;
        g.switch_weapon(1);
        assert_eq!(g.fire_cooldown, 0.0, "切枪后上一把枪的冷却必须清零");
        assert_eq!(g.auto_heat, 0.0, "切枪后上一把枪的控枪热量必须清零");
        // 结束切枪硬直后，新武器必须能立即开火
        g.weapons.update(0.7);
        assert!(
            g.fire_player([0.0, 1.6, 0.0], [0.0, 0.0, -1.0]),
            "切枪完成（0.6s）后新武器应能立即开火，不应被上一把枪的残留冷却卡住"
        );
    }

    
    /// 🔴 **远端玩家开火不得污染房主机体**（2026-10-20 第 15 批复核第 7 条）。
    /// 旧实现服务端用 `self.fire(...)` 处理远端开火 ⇒ 扣房主弹匣、把后坐力加到房主相机、
    /// 并把房主自己的 `fire_cooldown` 立起来。现在走无状态的 `fire_remote`。
    /// 反向验证：把 net.rs 里 `fire_remote` 改回 `self.fire` ⇒ 本判据红。
    #[test]
    fn remote_player_fire_does_not_touch_the_host_weapon_state() {
        let mut g = Game::new();
        g.on_any_key();
        g.net_players.push(NetPlayer {
            id: 1,
            pos: [50.0, 0.0, 50.0],
            yaw: 0.0,
            pitch: 0.0,
            hp: 100.0,
            alive: true,
            last_rx: 0.0,
            fire_accum: 0.5,
            last_fire: 0.0,
            last_input_seq: 0,
            last_input_time: 0.0,
        });
        g.last_dt = 0.016;
        let ammo_before = g.weapons.active_firearm_ref().magazine();
        let kick_before = g.pending_kick;
        let projectiles_before = g.projectiles.len();
        let input = NetInput {
            forward: false,
            backward: false,
            left: false,
            right: false,
            fire: true,
            yaw: 0.0,
            pitch: 0.0,
        };
        g.apply_net_player_input(1, input, 1);
        assert_eq!(
            g.weapons.active_firearm_ref().magazine(),
            ammo_before,
            "远端玩家开火不应扣减服务端房主本机的弹匣"
        );
        assert_eq!(
            g.pending_kick, kick_before,
            "远端玩家开火不应给房主本机相机施加后坐力"
        );
        assert_eq!(g.fire_cooldown, 0.0, "远端玩家开火不应阻塞房主本机的开火冷却");
        assert!(
            g.projectiles.len() > projectiles_before,
            "远端玩家开火必须真的生成一发世界弹丸"
        );
    }

    // ===== 定向复查探针（2026-10-21）：跨文件不变量，通过后留作回归判据 =====

    /// 探针 1：**障碍摧毁后 `world.bodies` 与 `map.obstacles` 必须继续逐项对齐**
    /// （两表按下标一一对应是命中结算/弹孔/伤害的前提；任何 remove 不同步 =
    /// 以后所有子弹打到错误的刚体上）。
    #[test]
    fn obstacle_destruction_keeps_body_and_obstacle_tables_aligned() {
        let mut g = Game::new();
        g.map.obstacles.clear();
        g.world.bodies.clear();
        for (i, (x, z)) in [(0.0f32, -5.0f32), (10.0, -5.0)].into_iter().enumerate() {
            let half_w = 1.5f32;
            let half_d = 0.5f32;
            g.map.obstacles.push(MapObstacle {
                x,
                z,
                half_w,
                half_d,
                y: 1.2,
                half_h: 1.2,
                kind: ObstacleKind::Wall,
                tint: None,
                max_hp: 100.0,
                hp: 100.0,
                shape: Shape::Legacy,
            });
            g.world.bodies.push(Body::new_static(
                Pv::new(x, 1.2, z),
                Pv::new(half_w, 1.2, half_d),
            ));
            assert!(i < 2);
        }
        g.damage_obstacle(0, 1.0e9);
        assert_eq!(g.map.obstacles.len(), 1, "只剩一个障碍");
        assert_eq!(
            g.world.bodies.len(),
            g.map.obstacles.len(),
            "两表长度必须始终相等（下标一一对应）"
        );
        let ob = &g.map.obstacles[0];
        let aabb = g.world.bodies[0].aabb();
        assert!(
            (aabb.min.x - (ob.x - ob.half_w)).abs() < 1e-3
                && (aabb.max.x - (ob.x + ob.half_w)).abs() < 1e-3
                && (aabb.min.z - (ob.z - ob.half_d)).abs() < 1e-3,
            "摧毁 #0 后，剩下的刚体与障碍必须还是同一件（body min.x={} vs 障碍 x-half={})",
            aabb.min.x,
            ob.x - ob.half_w
        );
    }

    /// 探针 2：**`damage_npc` 击杀移除后，剩下的数组下标语义必须一致**
    /// （`nearest_segment_hit` 返回的是 `self.npcs` 的下标；乱序/错位 = 子弹打错人）。
    #[test]
    fn killing_an_npc_keeps_remaining_indices_consistent() {
        let mut g = Game::new();
        g.npcs.clear();
        g.world.bodies.clear();
        g.world.spheres.clear();
        let mut a = npc_at(7, Team::Red, [0.0, 0.0, -5.0]);
        a.hp = 30.0;
        let mut b = npc_at(9, Team::Red, [0.0, 0.0, -12.0]);
        b.hp = 100.0;
        g.npcs = vec![a, b];
        let killed = g.damage_npc(0, 1.0e9, DamageSource::Player);
        assert!(killed, "#7 30 血应被一击打死");
        assert_eq!(g.npcs.len(), 1);
        assert_eq!(g.npcs[0].id, 9, "剩下的必须是 #9");
        // 从原点向 −Z 打一发，线段 0..−10：应命中 z=−12 之外？改为线段 0..−20 直接构造
        let mut p = crate::engine::weapons::Projectile::new(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            600.0,
            500.0,
            2.0,
            28.0,
        )
        .with_player();
        p.update(1.0 / 30.0); // 一帧飞 20m，穿过 z=−12
        match g.nearest_segment_hit(&p) {
            Some(crate::engine::game::weapons::SegmentHit::Npc { idx, .. }) => {
                assert_eq!(idx, 0, "命中下标应指向剩下的 #9（数组下标 0）");
                assert_eq!(g.npcs[idx].id, 9);
            }
            other => panic!("应命中剩下的 NPC，实际 {:?}", other.map(|h| h.t())),
        }
    }

    /// 探针 3：**换弹中切枪再切回，换弹不能永久卡住**（状态机跨武器残留）。
    #[test]
    fn reload_state_does_not_stick_across_a_weapon_switch() {
        let mut g = Game::new();
        g.on_any_key();
        // 打一发消耗弹匣，再请求换弹
        assert!(g.fire_player([0.0, 1.6, 0.0], [0.0, 0.0, -1.0]));
        g.weapons.update(1.0); // 冷却清零
        g.request_reload();
        assert!(
            g.weapons.active_firearm_ref().is_reloading(),
            "前提：应当进入换弹状态"
        );
        // 切走再切回
        g.switch_weapon(1);
        g.weapons.update(0.7);
        g.switch_weapon(0);
        g.weapons.update(0.7);
        // 推进足够长时间：换弹必须能完成（不能永久卡在 reloading）
        for _ in 0..400 {
            g.weapons.update(0.02);
        }
        assert!(
            !g.weapons.active_firearm_ref().is_reloading(),
            "换弹中切枪再切回后，换弹永久卡住（remaining={:.2}s）",
            g.weapons.active_firearm_ref().reload_progress()
        );
        assert!(g.weapons.active_firearm_ref().magazine() > 1, "换弹应已补弹");
    }

    /// 探针 4：**`start_run` 必须幂等**（连开两局后所有"每局状态"一致）。
    /// 任何新增的每局字段只要在 `start_run_at` 里漏复位，这里就会分叉。
    #[test]
    fn start_run_is_idempotent_for_per_run_state() {
        let mut g = Game::new();
        g.on_any_key();
        g.start_run();
        let snap1 = (
            g.hud.health,
            g.hud.ammo,
            g.hud.reserve,
            g.score,
            g.wave,
            g.level,
            g.heal_timer,
        );
        let snap1b = (
            g.grenades,
            g.medkits,
            g.projectiles.len(),
            g.grenades_vec.len(),
            g.explosions.len(),
            g.npcs.len(),
            g.map.obstacles.len(),
            g.map.props.len(),
        );
        assert_eq!(g.stance, Stance::Standing);
        // 改动一批每局状态后再重开
        g.hud.health = 1.0;
        g.hud.ammo = 0;
        g.score = 999;
        g.level = 7;
        g.toggle_prone();
        g.grenades = 0;
        g.medkits = 0;
        g.heal_timer = 2.0;
        g.projectiles.clear();
        g.start_run();
        let snap2 = (
            g.hud.health,
            g.hud.ammo,
            g.hud.reserve,
            g.score,
            g.wave,
            g.level,
            g.heal_timer,
        );
        let snap2b = (
            g.grenades,
            g.medkits,
            g.projectiles.len(),
            g.grenades_vec.len(),
            g.explosions.len(),
            g.npcs.len(),
            g.map.obstacles.len(),
            g.map.props.len(),
        );
        assert_eq!(snap1, snap2, "两局起始状态必须一致（有每局字段漏复位）");
        assert_eq!(snap1b, snap2b, "两局起始状态必须一致（有每局字段漏复位）");
        assert_eq!(g.stance, Stance::Standing, "重开后必须回到站姿");
    }

    /// 探针 5：**据点胜利判定必须幂等**（重复调用不得重复播报/改状态）。
    #[test]
    fn objective_victory_evaluation_is_idempotent() {
        use crate::engine::ai::Team;
        use crate::engine::objective::{CapturePoint, GameRule, ObjectiveState};
        let mut g = Game::new();
        g.on_any_key();
        let mut st = ObjectiveState::new(GameRule::CapturePoints { required: 1 });
        let mut pt = CapturePoint::new("A", 0.0, 0.0, 5.0, 5.0);
        pt.owner = Some(Team::Blue);
        st.points.push(pt);
        g.obj_state = Some(st);
        g.update_objectives(0.0, &glam::Vec3::ZERO);
        let state1 = g.game_state;
        let banner1 = g.hud.victory_banner.clone();
        let kills1 = g.hud.kill_feed.len();
        g.update_objectives(0.0, &glam::Vec3::ZERO);
        assert_eq!(g.game_state, state1, "重复判定不得改变状态");
        assert_eq!(g.hud.victory_banner, banner1, "重复判定不得重写横幅");
        assert_eq!(g.hud.kill_feed.len(), kills1, "重复判定不得重复播报");
    }

/// 复核 S5：远端弹丸并发数必须有上限（旧版 100 包/帧可堆上千发）。
/// 反向验证：删掉 `fire_remote` 里的上限分支 ⇒ 本判据红。
#[test]
fn projectile_pileup_is_capped() {
    let mut g = Game::new();
    for _ in 0..400 {
        g.fire_remote([0.0, 1.6, 0.0], [0.0, 0.0, -1.0], 1);
    }
    assert!(
        g.projectiles.len() <= 256,
        "远端弹丸堆到 {} 发（应 ≤256，S5）",
        g.projectiles.len()
    );
}

/// S8：远端玩家的子弹必须能打到玩家（旧版只认 `from_player`，PvP 事实不存在）。
/// 反向验证：去掉 `nearest_segment_hit` 里的 `|| p.remote_owner().is_some()` ⇒ 本判据红。
#[test]
fn remote_bullet_can_hit_players() {
    let mut g = Game::new();
    g.npcs.clear();
    g.world.bodies.clear();
    g.world.spheres.clear();
    for (id, z) in [(1u32, 0.0f32), (2, -6.0)] {
        g.net_players.push(NetPlayer {
            id,
            pos: [0.0, 0.0, z],
            yaw: 0.0,
            pitch: 0.0,
            hp: 100.0,
            alive: true,
            last_rx: 0.0,
            fire_accum: 0.0,
            last_fire: 0.0,
            last_input_seq: 0,
            last_input_time: 0.0,
        });
    }
    let before = g.net_players[1].hp;
    g.fire_remote([0.0, 1.2, 0.0], [0.0, 0.0, -1.0], 1);
    for _ in 0..20 {
        g.update_projectiles(0.004, true);
    }
    assert!(g.net_players[1].hp < before, "远端弹打不到玩家（S8）");
}

/// S8：远端玩家（蓝方）的子弹必须穿过蓝方 NPC（旧版只有本机玩家弹才豁免友军）。
#[test]
fn remote_bullet_ignores_own_team_npcs() {
    let mut g = Game::new();
    g.npcs.clear();
    g.world.bodies.clear();
    g.world.spheres.clear();
    g.npcs.push(npc_at(7, Team::Blue, [0.0, 0.0, -6.0]));
    let before = g.npcs[0].hp;
    g.fire_remote([0.0, 1.2, 0.0], [0.0, 0.0, -1.0], 1);
    for _ in 0..20 {
        g.update_projectiles(0.004, true);
    }
    assert_eq!(g.npcs[0].hp, before, "远端弹打到了友军 NPC（S8）");
}

/// S8：远端玩家的击杀不得记到房主的分数上（旧版 `score += KILL_SCORE` 无来源判断）。
#[test]
fn remote_kill_does_not_credit_the_host() {
    let mut g = Game::new();
    g.npcs.clear();
    g.world.bodies.clear();
    g.world.spheres.clear();
    g.npcs.push(npc_at(8, Team::Red, [0.0, 0.0, -6.0]));
    g.npcs[0].hp = 1.0;
    let score_before = g.score;
    g.fire_remote([0.0, 1.2, 0.0], [0.0, 0.0, -1.0], 1);
    for _ in 0..20 {
        g.update_projectiles(0.004, true);
    }
    assert!(g.npcs.iter().all(|n| n.id != 8), "远端弹没打死红方 NPC");
    assert_eq!(g.score, score_before, "远端击杀记到了房主分数（S8）");
}

/// S13：远端玩家出生点必须按 id 散开（旧版全部 `(-110, 60)` ⇒ 多人重叠）。
/// 反向验证：把半径/角度改成常量 0 ⇒ 本判据红。
#[test]
fn net_spawn_points_are_spread_around_the_base() {
    let base = [-110.0f32, 60.0f32];
    let mut seen = std::collections::HashSet::new();
    for id in 1..=12u32 {
        let p = Game::net_spawn_point_for(id);
        let (dx, dz) = (p[0] - base[0], p[1] - base[1]);
        let r = (dx * dx + dz * dz).sqrt();
        assert!((2.9..=6.6).contains(&r), "id={id} 半径 {r:.2} 不在 2.9..6.6");
        let key = (format!("{:.2},{:.2}", p[0], p[1]));
        assert!(seen.insert(key), "id={id} 与前面的出生点重叠（S13）");
    }
}

/// 夜间视距必须真的接到 AI（`night_vision_factor` 读关卡 TOML 的太阳高度）。
/// 反向验证：把 `map_sun` 分支删掉 ⇒ 恒 1.0 ⇒ 红。
#[test]
fn night_vision_factor_reads_map_sun() {
    let mut g = Game::new();
    g.map_sun = Some((4.0, 200.0));
    let night = g.night_vision_factor();
    assert!(night < 0.75, "低太阳（4°）时视距系数应明显下降，实际 {night}");
    g.map_sun = Some((70.0, 233.0));
    let day = g.night_vision_factor();
    assert!((day - 1.0).abs() < 0.02, "高太阳（70°）应接近 1.0，实际 {day}");
}

/// 手电开关：有电可开、空电拒绝。反向验证：删掉 `battery > 0.02` 判断 ⇒ 红。
#[test]
fn flashlight_toggle_respects_battery() {
    let mut g = Game::new();
    g.battery = 0.5;
    g.flashlight = false;
    g.toggle_flashlight();
    assert!(g.flashlight, "有电时应能打开");
    g.battery = 0.0;
    g.flashlight = false;
    g.toggle_flashlight();
    assert!(!g.flashlight, "空电时不应能打开");
    assert!(!g.flashlight_lit(), "空电时不应点亮");
}

/// 据点电源：站在据点半径内回充（20 s 满），站在外面不充。
/// 反向验证：把 `inside` 改成恒 true ⇒ 远处也充；恒 false ⇒ 红（本判据两侧都钉）。
#[test]
fn battery_recharges_inside_objective_radius() {
    use crate::engine::objective::CapturePoint;
    let mut g = Game::new();
    let mut obj = g
        .obj_state
        .take()
        .unwrap_or_else(|| crate::engine::objective::ObjectiveState::new(
            crate::engine::objective::GameRule::CapturePoints { required: 1 },
        ));
    obj.points.push(CapturePoint::new("A", 0.0, 0.0, 5.0, 10.0));
    g.obj_state = Some(obj);
    g.battery = 0.5;
    g.recharge_battery_near_points([0.0, 0.0], 10.0);
    assert!((g.battery - 1.0).abs() < 1e-5, "据点内 10s 应充满：{}", g.battery);
    g.battery = 0.4;
    g.recharge_battery_near_points([100.0, 100.0], 10.0);
    assert!((g.battery - 0.4).abs() < 1e-5, "据点外不应回充：{}", g.battery);
}

/// 火把光源取**离相机最近的存活敌人**（不是列表第一个）。反向验证：改成 `.first()` ⇒ 红。
#[test]
fn torch_picks_the_nearest_alive_enemy() {
    let mut g = Game::new();
    g.npcs.clear();
    g.npcs.push(npc_at(1, Team::Red, [40.0, 0.0, 0.0]));
    g.npcs.push(npc_at(2, Team::Red, [3.0, 0.0, 0.0]));
    let near = g.nearest_npc_for_torch([0.0, 1.6, 0.0]).expect("应有敌人");
    assert!((near[0] - 3.0).abs() < 1e-3, "应选最近的（x=3），实际 {near:?}");
    g.npcs[1].hp = 0.0; // 最近的死掉 ⇒ 退到另一个
    let next = g.nearest_npc_for_torch([0.0, 1.6, 0.0]).expect("应有敌人");
    assert!((next[0] - 40.0).abs() < 1e-3, "跳过死者后应选 x=40，实际 {next:?}");
}
