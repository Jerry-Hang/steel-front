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
        g.apply_net_player_input(1, input);
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
