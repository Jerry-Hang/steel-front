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

