// 由 src/engine/game/session.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `session` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 构建 HUD quad 列表：血条/弹药/FPS + 调试行（LOD、实体数、NPC 状态、碰撞/命中）
    pub(crate) fn hud_quads(&mut self, near: u32, far: u32, lod: &str) -> Vec<crate::ui::Quad> {
        use crate::ui::{render_text, Color, HudScreen};
        // 每帧同步 HUD 显示字段
        self.hud.score = self.score;
        self.hud.wave = self.wave;
        self.hud.countdown = self.wave_timer.max(0.0);
        self.hud.survive_waves = self.survive_total_waves();
        self.hud.objective = (self.objective.eliminated, self.objective.target);
        // 关卡系统：据点状态同步到 HUD（id/归属/进度）。
        // 单机 = 本机 obj_state；联机客户端 = 网络 ObjectiveState 广播（归属码 0=中立/1=Red/2=Blue）。
        // 无联机且无关卡系统 → 空列表（HUD 不显示进度条，行为零回归）。
        // 小地图快照：单位（含阵营）/障碍（含种类与尺寸）/玩家位置（朝向由 main.rs 同步）
        self.hud.mm_units = self
            .npcs
            .iter()
            .map(|n| {
                (
                    n.position[0],
                    n.position[2],
                    if n.team == crate::engine::ai::Team::Red { 0 } else { 1 },
                )
            })
            .collect();
        self.hud.mm_obstacles = self
            .map
            .obstacles
            .iter()
            .filter(|o| o.hp > 0.0)
            .map(|o| {
                let kind = match o.kind {
                    ObstacleKind::Wall => 0u8,
                    ObstacleKind::Block => 1,
                    ObstacleKind::Barrier => 2,
                    ObstacleKind::Tree => 3,
                    ObstacleKind::Building => 4,
                    ObstacleKind::Ruin => 5,
                };
                (o.x, o.z, o.half_w, o.half_d, kind)
            })
            .collect();
        {
            let eye = self.player_eye();
            self.hud.mm_player = [eye.x, eye.z];
        }
        self.hud.capture_points = if let Some(o) = self.obj_state.as_ref() {
            o.points
                .iter()
                .map(|p| (p.id.clone(), p.owner, p.progress))
                .collect()
        } else if let Some(client) = self.net_client.as_ref() {
            client
                .objective_state()
                .iter()
                .map(|(id, code, progress)| {
                    let owner = match code {
                        1 => Some(crate::engine::ai::Team::Red),
                        2 => Some(crate::engine::ai::Team::Blue),
                        _ => None,
                    };
                    (id.clone(), owner, *progress)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.hud.screen = if self.hud.settings_open {
            HudScreen::Settings
        } else {
            match self.game_state {
                GameState::StartMenu => HudScreen::Start,
                GameState::LoadingMap => HudScreen::Start,
                GameState::GameOver | GameState::Defeat => HudScreen::GameOver,
                GameState::Victory(_) => HudScreen::Game,
                GameState::Playing => HudScreen::Game,
            }
        };
        let mut quads = self.hud.layout();
        if self.game_state != GameState::Playing {
            return quads;
        }
        let mut counts = [0u32; 4];
        for npc in &self.npcs {
            counts[npc.state_machine.state() as usize] += 1;
        }
        let line1 = format!(
            "LOD: {}  entities: {}/65536  npc: I{} P{} C{} A{}",
            lod,
            near + far,
            counts[0],
            counts[1],
            counts[2],
            counts[3]
        );
        let hud_s = self.hud.ui_scale();
        render_text(&line1, 10.0 * hud_s, 44.0 * hud_s, Color::YELLOW, 1.3 * hud_s, &mut quads);
        // 当前武器规格：口径 + 阵营配色（联合体=红 / 同盟=蓝）
        let active_spec = ALL_WEAPONS.get(self.weapons.active_index());
        let (wcolor, caliber_txt) = match active_spec {
            Some(s) if s.faction == crate::engine::weapon_data::Faction::Union => (
                Color::new(0.95, 0.45, 0.35, 1.0),
                s.caliber,
            ),
            Some(s) => (Color::new(0.35, 0.65, 0.98, 1.0), s.caliber),
            None => (Color::CYAN, ""),
        };
        let line2 = format!(
            "{} ({}) [{}]  ammo: {:.0}%  hits: {}  col: {}",
            self.weapons.active_name(),
            caliber_txt,
            self.fire_mode.label(),
            self.weapons.active_firearm_ref().ammo_ratio() * 100.0,
            self.hits(),
            self.total_collisions()
        );
        render_text(&line2, 10.0 * hud_s, 62.0 * hud_s, wcolor, 1.3 * hud_s, &mut quads);
        quads
    }
    /// 构建默认光照场景（方向光 + 环境光 + 2 点光；阴影默认开，`RV3D_NO_SHADOW=1` 关）
    ///
    /// 强度配平（2026-09-01 建模重构第 1 批）：旧值 sun=1.5 / ambient=0.5×(0.5,0.55,0.6)
    /// 使任何 NdotL>0.5 的面全部撞上 `min(radiance,1)` 截顶，明暗比只有 4:1 且高光端全糊。
    /// 片元改用不截顶的指数压缩后，批次 1 先降到 sun=1.15 / 环境 0.30，把
    /// [背光 0.20, 迎光 0.87] 整个区间放回曲线未饱和段；同日质量 pass（c5a2a67）
    /// 因阴影面街道读不清再把两者上调到**当前的 sun=1.35 / 环境 0.55**。
    /// 改这两个数必须同时看 build.rs::apply_lighting 的曲线。
    pub(crate) fn light_uniform(&self) -> super::lighting::LightUniform {
        use super::lighting::{DirectionalLight, LightUniform, PointLight, ShadowConfig};
        let sun = DirectionalLight::new(
            glam::Vec3::new(-0.4, 0.9, -0.3).normalize(),
            glam::Vec3::new(1.0, 0.95, 0.85),
            1.35,
        );
        let point_a = PointLight::new(
            glam::Vec3::new(0.0, 6.0, 0.0),
            glam::Vec3::new(0.9, 0.6, 0.4),
            1.5,
        );
        let point_b = PointLight::new(
            glam::Vec3::new(-24.0, 5.0, -16.0),
            glam::Vec3::new(0.4, 0.7, 1.0),
            1.0,
        );
        // 阴影贴图（2026-08-11）：正交光空间以地图中心为 target、半宽 250m，
        // 覆盖障碍环带（58-130m）与两军接火区；相机无现成引用，取原点近似。
        // ShadowConfig.light_dir 语义 = 表面→光源方向，与 sun.direction 一致直接传入
        // （旧实现传 -sun.direction 使光相机在地面下方仰视：阴影图只剩背面剔除后的
        //   竖面，地面/地形整片缺失，阴影完全失效）。
        // RV3D_NO_SHADOW=1 关闭阴影（仅环境光+点光源），用于 A/B 验证与阴影 pass 性能对比。
        // 2026-08-22：城市高楼群下阴影图覆盖异常（全图判黑），默认改用环境光+方向光直照
        // （RV3D_NO_SHADOW=0 强制回阴影贴图路径，用于后续排查）
        // 2026-08-28：用户机器全高实测——默认开阴影（RV3D_NO_SHADOW=1 显式关闭）；
        // 阴影覆盖范围加大以适配城市高楼群（扩展 250→400 米，远平面 500→800）
        let shadow = if std::env::var("RV3D_NO_SHADOW").as_deref() == Ok("1") {
            None
        } else {
            Some(ShadowConfig::new(sun.direction, glam::Vec3::ZERO, 400.0, 1.0, 800.0))
        };
        LightUniform::build(
            Some(&sun),
            &[point_a, point_b],
            glam::Vec3::new(0.5, 0.55, 0.6),
            0.55,
            shadow.as_ref(),
        )
    }
}
