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
        // 手电电量条：夜间且未满电才显示（白天不占画面）。
        self.hud.battery = self.battery;
        self.hud.battery_visible = self.night_vision_factor() < 0.95 && self.battery < 0.999;
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
    /// 据点电源回充：站在任意据点半径内时，`dt` 秒内按 20 s 充满回充。
    pub(crate) fn recharge_battery_near_points(&mut self, pos: [f32; 2], dt: f32) {
        if self.battery >= 1.0 {
            return;
        }
        let inside = self.obj_state.as_ref().is_some_and(|o| {
            o.points.iter().any(|p| {
                let dx = pos[0] - p.x;
                let dz = pos[1] - p.z;
                dx * dx + dz * dz <= (p.radius + 1.0) * (p.radius + 1.0)
            })
        });
        if inside {
            let before = self.battery;
            self.battery = (self.battery + dt / 20.0).min(1.0);
            if before < 1.0 && self.battery >= 1.0 {
                log::info!("flashlight: 据点电源充满");
            }
        }
    }

    /// 玩家夜里是否亮着（手电开着且有电、且当前是夜间）——AI 据此看得更远。
    pub(crate) fn player_is_lit(&self) -> bool {
        self.flashlight_lit()
            && !crate::syscfg::flag("RV3D_NO_FLASHLIGHT")
            && self.night_vision_factor() < 0.95
    }

    /// 手电开关（F 键）：电量耗空时不允许打开。
    pub(crate) fn toggle_flashlight(&mut self) {
        if self.flashlight {
            self.flashlight = false;
            log::info!("flashlight: 关闭（电量 {:.0}%）", self.battery * 100.0);
        } else if self.battery > 0.02 {
            self.flashlight = true;
            log::info!("flashlight: 打开（电量 {:.0}%）", self.battery * 100.0);
        } else {
            log::info!("flashlight: 电量不足，无法打开");
        }
    }

    /// 当前是否点亮手电（夜间 + 开关 + 有电）。
    pub(crate) fn flashlight_lit(&self) -> bool {
        self.flashlight && self.battery > 0.0
    }

    /// 离相机最近的**存活敌人**位置（火把光源用；没有敌人返回 None）。
    pub(crate) fn nearest_npc_for_torch(&self, cam: [f32; 3]) -> Option<[f32; 3]> {
        self.npcs
            .iter()
            .filter(|n| n.hp > 0.0)
            .min_by(|a, b| {
                let da = (a.position[0] - cam[0]).powi(2) + (a.position[2] - cam[2]).powi(2);
                let db = (b.position[0] - cam[0]).powi(2) + (b.position[2] - cam[2]).powi(2);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|n| {
                // 火把在**手里**，不是在眼睛上：沿朝向偏前 0.35 m、右侧 0.28 m、高度 1.15 m。
                // facing 语义 = atan2(dx, dz) ⇒ 0 = +Z，forward = (sin, cos)，right = (cos, -sin)。
                let f = n.facing;
                let (fx, fz) = (f.sin(), f.cos());
                [
                    n.position[0] + fx * 0.35 + fz * 0.28,
                    n.position[1] + 1.15,
                    n.position[2] + fz * 0.35 - fx * 0.28,
                ]
            })
    }

    /// 夜间视距系数：与 `light_uniform` 共用同一太阳高度来源（env > 关卡 TOML > 默认）。
    pub(crate) fn night_vision_factor(&self) -> f32 {
        let sun_env = super::lighting::sun_from_env().or_else(|| {
            self.map_sun.map(|(e, a)| {
                let (color, scale) = super::lighting::sun_tint(e);
                (super::lighting::sun_direction(e, a), color, scale)
            })
        });
        match sun_env {
            Some((dir, _, _)) => super::lighting::vision_factor_for_sun(dir.y),
            None => 1.0,
        }
    }

    pub(crate) fn light_uniform(&self, cam: [f32; 3], yaw: f32) -> super::lighting::LightUniform {
        use super::lighting::{DirectionalLight, LightUniform, PointLight, ShadowConfig};
        // 太阳：默认沿用历史向量 (-0.4,0.9,-0.3)/1.35；设 RV3D_SUN_ELEV（必要时加
        // RV3D_SUN_AZIM）则按角度重算，低角度自动变暖变弱（`lighting::sun_tint`）。
        // 优先级：RV3D_SUN_*（A/B）> 关卡 TOML 的 sun_* > 引擎默认向量。
        let sun_env = super::lighting::sun_from_env().or_else(|| {
            self.map_sun.map(|(e, a)| {
                let (color, scale) = super::lighting::sun_tint(e);
                (super::lighting::sun_direction(e, a), color, scale)
            })
        });
        let sun = match sun_env {
            Some((dir, color, scale)) => DirectionalLight::new(dir, color, 1.35 * scale),
            None => DirectionalLight::new(
                glam::Vec3::new(-0.4, 0.9, -0.3).normalize(),
                glam::Vec3::new(1.0, 0.95, 0.85),
                1.35,
            ),
        };
        // 低太阳预设：太阳低时环境光同步变暗（0.55→0.55*scale）并略偏冷，
        // 否则"低太阳只改方向"会被 0.55 的环境光完全盖住（实测整图仅差 0.4 亮度）。
        let (amb_color, amb_intensity) = match sun_env {
            Some((_, _, scale)) => (glam::Vec3::new(0.46, 0.47, 0.56), 0.55 * scale),
            None => (glam::Vec3::new(0.5, 0.55, 0.6), 0.55),
        };
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
        // 低太阳/夜晚：把两个点光挪到离相机最近的路灯上（暖色光池）。
        // 白天不动：白天开路灯反而会在地面留一块不合理的亮斑。
        // RV3D_NO_LAMPS=1 关掉路灯（A/B 对照用）。
        let dusk = sun_env.map(|(d, _, _)| d.y < 0.34).unwrap_or(false)
            && !crate::syscfg::flag("RV3D_NO_LAMPS");
        // 🔴 保留原来的两个装饰点光，**再加**两个路灯（uniform 有 4 个槽位）：
        // 一开始是"替换"，实测路灯 on 反而比 off 更暗（85.94 vs 95.59）——
        // 原两个本来就在给广场打光，换掉等于减光。
        // 阴雨：太阳弱化 + 环境光更冷（与天空灰化配合）。
        let (sun, amb_color, amb_intensity) = if self.rain {
            (
                super::lighting::DirectionalLight::new(
                    sun.direction,
                    sun.color * 0.92,
                    sun.intensity * 0.45,
                ),
                glam::Vec3::new(0.30, 0.33, 0.40),
                amb_intensity * 0.85,
            )
        } else {
            (sun, amb_color, amb_intensity)
        };
        let mut points = vec![point_a, point_b];
        if dusk {
            // 夜间：玩家手电（替换那个固定的装饰点光，uniform 仍是 4 槽）。
            let on = self.flashlight_lit() && !crate::syscfg::flag("RV3D_NO_FLASHLIGHT");
            if on {
                let fpos = super::lighting::flashlight_position(cam, yaw);
                let (dim, range) = super::lighting::flashlight_dim(self.battery);
                let mut fl = PointLight::new(
                    glam::Vec3::from_array(fpos),
                    glam::Vec3::new(1.0, 0.95, 0.85),
                    3.0 * dim,
                );
                // range 随电量 8..14 m：点光源没有聚光，不限制的话整片广场都会被照亮。
                fl.range = range;
                points[1] = fl;
            }
        }
        if dusk {
            let lamp_col = glam::Vec3::new(1.0, 0.78, 0.48);
            // 夜里：一个路灯（离相机最近）+ 最近敌人的火把（对等博弈：NPC 也开灯）
            let mut slots = points.len();
            if let Some(l) = super::lighting::pick_nearest_lamps(&self.map.lamps, [cam[0], cam[2]], 1)
                .into_iter()
                .next()
            {
                if slots < super::lighting::MAX_POINT_LIGHTS {
                    points.push(PointLight::new(glam::Vec3::from_array(l), lamp_col, 7.0));
                    slots += 1;
                }
            }
            if slots < super::lighting::MAX_POINT_LIGHTS
                && !crate::syscfg::flag("RV3D_NO_TORCH")
            {
                if let Some(npc) = self.nearest_npc_for_torch(cam) {
                    let mut torch = PointLight::new(
                        glam::Vec3::new(npc[0], npc[1] + 1.5, npc[2]),
                        glam::Vec3::new(1.0, 0.72, 0.42),
                        2.5,
                    );
                    torch.range = 12.0; // 点光无锥形，限制到 12 m 才像"火把"而不是探照灯
                    points.push(torch);
                }
            }
        }
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
        // 阴影覆盖半宽（米）。2026-10-22：发现 400m/2048² = **0.39m/纹素**，树冠/柱子的
        // 高频影子在广场地面上被采样成条纹状走样（关阴影后该区域 98.7% 像素变化 = 确认来源）。
        // 现在可调，默认先保持 400（历史行为），画质档位实验后再定默认值。
        let extent = crate::syscfg::parse::<f32>("RV3D_SHADOW_EXTENT")
            .map(|v| v.clamp(60.0, 800.0))
            .unwrap_or(400.0);
        let shadow = if crate::syscfg::cfg("RV3D_NO_SHADOW").as_deref() == Some("1") {
            None
        } else {
            Some(ShadowConfig::new(
                sun.direction,
                glam::Vec3::ZERO,
                extent,
                1.0,
                2.0 * extent,
            ))
        };
        let mut lu = LightUniform::build(
            Some(&sun),
            &points,
            amb_color,
            amb_intensity,
            shadow.as_ref(),
        );
        // 反光地面通道：x=阴雨开关，y=水面反光强度（阴雨即 1.0），z=秒（水光流光），w=保留。
        let rain = if self.rain { 1.0 } else { 0.0 };
        // 水面反光强度单独可关（RV3D_NO_WET=1），用于把"水光"从"阴雨天光变暗"里隔离出来做 A/B。
        let wet = if self.rain && !crate::syscfg::flag("RV3D_NO_WET") { 1.0 } else { 0.0 };
        lu.weather = glam::Vec4::new(rain, wet, self.time, 0.0);
        lu
    }
}
