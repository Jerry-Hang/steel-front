// 由 src/lib.rs 拆出（2026-11-12，纯移动 + 仅加 pub(crate)）：一帧的渲染准备与提交。
//
// 这里放 `render()` 本体与它内部切出来的五段（HUD 文本 / 世界 marker / NPC / 尸体 / PT 参数）。
// 拆出来的动因：`lib.rs` 曾把事件循环、输入、渲染全挤在一处（6320 行），
// 而这几段已经是"参数进出、无隐藏状态"的形态，整体搬出来最干净。
use crate::engine;
use crate::net;
use crate::engine::ai::Team;
use crate::engine::camera::CameraMode;
use crate::{gun_diag_on, idle_sway, GameApp, GameState, Particle};
use crate::{DECAL_LIFT_M, DECAL_SIZE_M, DECAL_THICK_M};

impl GameApp {
    /// HUD 的**文本叠加层**：命令输入框（含提示行）与伤害飘字。
    ///
    /// 从 `render()` 里整块抽出来的（2026-11-11，纯提取，行为不变）。抽成
    /// **参数化的关联函数**而不是 `&self` 方法，是因为调用点在 `if let Some(renderer) =
    /// &mut self.renderer` 里面 —— 那时 `self` 已被可变借用，再取 `&self` 直接编译失败
    /// （E0502）。把"只读输入 + 可变输出"显式列出来，顺带说清了这段到底依赖什么。
    #[allow(clippy::too_many_arguments)]
    fn append_hud_text_overlays(
        command_open: bool,
        state: GameState,
        command_buf: &str,
        anim_clock: f32,
        hud: &crate::ui::HudState,
        popups: &[(f32, f32)],
        quads: &mut Vec<crate::ui::Quad>,
    ) {
        // 命令输入窗口（Minecraft 风格左下角）：深色半透明底 + 提示符 + 闪烁光标
        if command_open && state == GameState::Playing {
            let s = hud.ui_scale();
            let prompt = format!("> {}{}", command_buf, {
                if (anim_clock * 2.0).sin() > 0.0 { '_' } else { ' ' }
            });
            let box_x = 10.0 * s;
            // 🔴 按**实际屏幕高度**算（2026-10-20，第 14 批复核第 8 条）：
            // 写死 800 基准时 720p 下 y=754 > 720 ⇒ 命令框整个画到屏幕外。
            let box_y = crate::ui::HudState::command_box_y(hud.screen_h, s);
            let box_h = 36.0 * s;
            let text_scale = 2.0 * s;
            let text_w = crate::ui::text_width(&prompt, text_scale);
            let box_w = (text_w + 26.0 * s).max(180.0 * s);
            quads.push(crate::ui::Quad::new(
                crate::ui::Rect::new(box_x, box_y, box_w, box_h),
                crate::ui::Color::new(0.06, 0.06, 0.12, 0.72),
            ));
            crate::ui::render_text(
                &prompt,
                box_x + 10.0 * s,
                box_y + 8.0 * s,
                crate::ui::Color::WHITE,
                text_scale,
                quads,
            );
            // 提示行：武器编号范围说明
            crate::ui::render_text(
                "武器编号 1-35（回车切换，Esc 关闭）",
                box_x + 2.0 * s,
                box_y - 22.0 * s,
                crate::ui::Color::YELLOW,
                1.3 * s,
                quads,
            );
        }
        // 伤害飘字：准星下方逐条显示（红色，随剩余时间上浮淡出）
        let s = hud.ui_scale();
        let mut popup_y = 120.0 * s;
        for (dmg, remain) in popups {
            let alpha = (remain / 0.6).clamp(0.0, 1.0);
            crate::ui::render_text(
                &format!("-{:.0}", dmg),
                (hud.screen_w / s) * 0.5 * s + 12.0 * s,
                popup_y,
                crate::ui::Color::new(1.0, 0.35 * alpha, 0.25 * alpha, alpha),
                1.6 * s,
                quads,
            );
            popup_y += 20.0 * s;
        }
    }

    /// 自发光 marker：**粒子**（按相机距离由近及远占剩余槽位）+ **手雷**可见小方块。
    ///
    /// 从 `render()` 整块抽出（2026-11-12，纯提取）。参数化关联函数的理由同
    /// `append_hud_text_overlays`：调用点在 `&mut self.renderer` 借用内，取 `&self` 会 E0502。
    /// 这段只依赖 `particles` / `eye` / `grenade_positions` / 已占槽位数。
    fn append_particle_emissive_markers(
        particles: &[Particle],
        eye: glam::Vec3,
        grenade_positions: &[[f32; 3]],
        reserving: usize,
        emissive_markers: &mut Vec<engine::renderer::WorldMarker>,
    ) {
    // 自发光槽位只有 64 个（与 build.rs 的 EMISSIVE_INSTANCE_BASE + 64 严格同步）：
    // 128v128 压力下上百个 NPC 同时开火，按插入顺序截断会让「远处/将熄的焰」占坑、
    // 「近处的新焰」被丢弃 —— 玩家面前因此悬浮着几团本不该存在的琥珀色圆盘（D8）。
    // 策略：爆炸特效保底，剩余槽位按相机距离由近及远分配。
    const MAX_EMISSIVE: usize = 64;
    // 预留：爆炸特效已入列的 + 紧随其后要画的手雷（数量很小），剩下的才给粒子
    let reserved = reserving + grenade_positions.len();
    let mut cand: Vec<(f32, engine::renderer::WorldMarker)> = particles
        .iter()
        .map(|p| {
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let size = if p.kind == 0 {
                p.size * (1.0 - t * 0.7) // 焰：快速收缩
            } else {
                p.size
            };
            let fade = 1.0 - t;
            let d = (p.pos[0] - eye.x).powi(2)
                + (p.pos[1] - eye.y).powi(2)
                + (p.pos[2] - eye.z).powi(2);
            (
                d,
                engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::from(p.pos))
                        * glam::Mat4::from_scale(glam::Vec3::splat(size)),
                    tint: [
                p.tint[0] * fade,
                p.tint[1] * fade,
                p.tint[2] * fade,
                if p.kind == 0 { 0.0 } else { 1.0 }, // 焰=火，壳=固体
            ],
                },
            )
        })
        .collect();
    cand.sort_by(|a, b| a.0.total_cmp(&b.0));
    emissive_markers.extend(
        cand.into_iter()
            .take(MAX_EMISSIVE.saturating_sub(reserved))
            .map(|(_, m)| m),
    );
    // 手雷可见实体：深橄榄色小方块（飞行/落地均可见，复用 emissive 通道）
    for gp in grenade_positions {
        emissive_markers.push(engine::renderer::WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::from(*gp))
                * glam::Mat4::from_scale(glam::Vec3::splat(0.16)),
            tint: [0.35, 0.4, 0.12, 1.0],
        });
    }
    }

    /// 构建本帧的**世界 marker**（障碍盒 / 占领据点 / 自发光 / 弹孔）。
    ///
    /// 从 `render()` 整块抽出（2026-11-12，纯提取）。输出三条向量，`renderer.set_*` 留在调用点：
    /// 这样 `render()` 只负责"算出来 → 交给渲染器"两件事，而这段纯数据构造可以被单独读。
    ///
    /// 缓存按 `map_generation` 失效 —— 关卡不变时直接复用上一次的障碍 marker（省掉每帧几百次
    /// 矩阵构造）。
    #[allow(clippy::too_many_arguments)]
    fn build_world_markers(
        game: &crate::engine::game::Game,
        cache: &mut Vec<engine::renderer::WorldMarker>,
        cache_gen: &mut Option<u64>,
        particles: &[Particle],
        eye: glam::Vec3,
    ) -> (
        Vec<engine::renderer::WorldMarker>,
        Vec<engine::renderer::WorldMarker>,
        Vec<engine::renderer::WorldMarker>,
    ) {
    let cur_gen = game.map_generation();
    if *cache_gen != Some(cur_gen) {
        // 🔴 **建筑不再画碰撞盒 marker**（2026-10-29 真机定位）：
        // 建筑本体已经由 GLB 画出来，而它那条"碰撞盒 marker"会在同一位置再画一个
        // 灰色混凝土壳 —— 壳与立面共面/略大的地方就 z-fighting，用户看到的
        // "转视角房子在棕和灰之间跳"正是它（A/B 实测：关掉后露出的是棕色 GLB 立面）。
        // 需要回退排查时用 `RV3D_BUILDING_MARKERS=1`。
        // `RV3D_NO_MARKERS=1`：连装饰件一起关，用于判断"白色大板"到底来自 marker 还是 GLB。
        // 🏙 chunk 开启时，程序化构件（1230 件）的可见面由 chunk 提供 ⇒ 不再画 marker，
        //    否则两套几何共面重复（z-fighting + 白付 1.8 万实例）。
        let chunks_on = !crate::syscfg::flag("RV3D_NO_CITY_CHUNKS")
            && std::path::Path::new("assets/city_chunks").is_dir();
        let no_markers = chunks_on || crate::syscfg::flag("RV3D_NO_MARKERS");
        let skip_building = no_markers || !crate::syscfg::flag("RV3D_BUILDING_MARKERS");
        *cache = game
            .render_geometry()
            .filter(|o| o.shape != engine::geom::Shape::None)
            .filter(|o| !skip_building || o.kind != engine::game::ObstacleKind::Building)
            // 🔴 高空装饰件（挑檐/窗带/女儿墙）是挂在建筑盒上的：建筑盒不再画之后，
            // 它们就成了天空里一排悬空黑条（真机截图）。把离地 4m 以上的 Block 装饰件
            // 一并跳过 —— 可见的建筑表面已由 GLB 提供。
            .filter(|o| {
                !(skip_building && o.kind == engine::game::ObstacleKind::Block && o.y > 4.0)
            })
            .filter(|_| !no_markers)
            .map(engine::renderer::WorldMarker::for_obstacle)
            .collect();
        *cache_gen = Some(cur_gen);
    }
    let markers: Vec<engine::renderer::WorldMarker> =
        cache.clone();
    // 占领据点世界标记（关卡系统 RV3D_MAP/RV3D_MAPS 启用时非空）：
    // 每据点 = 细高立柱（归属色）+ 扁平板状底盘（归属色，不透明）。
    // 复用 WorldMarker 通道（主 pipeline 实例化），零渲染管线改动。
    let capture_markers: Vec<engine::renderer::WorldMarker> = game
        .capture_points()
        .into_iter()
        .flat_map(|(_id, x, z, radius, owner, _progress)| {
            let tint = match owner {
                Some(crate::engine::ai::Team::Blue) => [0.08, 0.35, 0.98, 1.0],
                Some(crate::engine::ai::Team::Red) => [0.95, 0.12, 0.08, 1.0],
                None => [0.45, 0.45, 0.45, 1.0],
            };
            // 底盘配色：三通道等比缩放 → 色相/饱和度不变，归属色语义（蓝/红/灰）保持
            let base_tint = [tint[0] * 0.8, tint[1] * 0.8, tint[2] * 0.8, 0.6];
            [
                // 立柱（旗杆）：0.4m 见方、高 4m、底面落在地面。
                // 🔴 `from_scale` 传的是**半尺寸**（立方体模板 ±1，
                // `renderer.rs:1019 obstacle_model` 用 `scale = half / tmpl`）。
                // 旧值 (0.4, 4.0, 0.4) 是 2026-09-17 之前"scale = 全尺寸"的写法，
                // 改约定后没跟着改 ⇒ 实际画出 0.8m 宽、8m 高、底部埋进地里 2m。
                engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(x, 2.0, z))
                        * glam::Mat4::from_scale(glam::Vec3::new(0.2, 2.0, 0.2)),
                    tint,
                },
                // 地面底盘：**半径直接取玩法的占领判定半径**，不再写魔数。
                // D10 根因（保留）：旧值 y=0.08 + 厚 0.15 → 实体跨 y∈[0.005,0.155]，
                // 而地面实例平面在 y=+0.05 正好从中间穿过，顶面只高出 ~10cm；
                // 玩家视线 ~1.7m 看一层 10cm 的板几乎完全侧向 → 投影不足一像素 →
                // 底盘"消失"，据点读起来只剩两根电线杆。改为 0.5m 厚低台
                // （底面埋进地里 5cm 避免与地形之间留缝），顶面离地 ~40cm。
                // 🔴 第二个根因（本轮修）：这里原先硬编码 `from_scale(10.0, 0.5, 10.0)`
                // 并注释"半径 5.0 → scale 10.0"——那是"scale = 全尺寸"的旧约定。
                // 按现在的约定它画的是**半径 10m**，而玩法判定半径是
                // street_fight 5.0 / bridgehead 5.0·6.0 / defense_line 12.0，
                // ⇒ 领地标记在前两者上是真实圈子的 **2 倍**、在后者上**反而小一圈**，
                // 玩家靠它判断"进圈了没有"会被系统性误导。改成由 `radius` 推导，
                // 魔数消失，这类漂移不可能再回来。
                // 半高 0.25 + 中心 y=0.20 ⇒ 跨 [−0.05, +0.45]，与上面 D10 的意图逐值一致。
                engine::renderer::WorldMarker {
                    model: glam::Mat4::from_translation(glam::Vec3::new(x, 0.20, z))
                        * glam::Mat4::from_scale(glam::Vec3::new(radius, 0.25, radius)),
                    tint: base_tint,
                },
            ]
        })
        .collect();
    let mut markers = markers;
    markers.extend(capture_markers);
    // 爆炸闪光：冲击波球壳随年龄膨胀、颜色转淡；走自发光路径（emissive 槽位，
    // shader 直出纯色跳过光照/贴图混合），夜间等暗光环境下依然清晰可见
    // 爆炸多层视觉（4 层同源演算，立体感：火球核 + 贴地冲击波环 + 火柱 + 烟柱）
    let mut emissive_markers: Vec<engine::renderer::WorldMarker> = game
        .explosions()
        .iter()
        .flat_map(|ex| {
        let t = (ex.age / ex.lifetime).clamp(0.0, 1.0);
        let cx = ex.center[0];
        let cz = ex.center[2];
        let r = ex.radius;
        // ① 火球核：亮黄白，快速膨胀 + 快速淡出（0-0.35 寿命为主）；半透明球形
        let fireball_t = (t * 2.8).min(1.0);
        let fb_s = r * (0.2 + 1.2 * fireball_t);
        let mut out = vec![engine::renderer::WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.2, cz))
                * glam::Mat4::from_scale(glam::Vec3::splat(fb_s)),
            tint: [
                1.0,
                0.85 * (1.0 - fireball_t) + 0.2,
                0.35 * (1.0 - fireball_t),
                0.0, // tint.w = 火（build.rs 体积光晕分支选择器）
            ],
        }];
        // ② 贴地冲击波环：扁球体（球体几何压扁）沿地面水平扩散 + 高度衰减，半透明
        let ring_s = r * (0.4 + 1.6 * t);
        let ring_h = (1.1 * (1.0 - t)).max(0.15);
        out.push(engine::renderer::WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::new(cx, ring_h * 0.5, cz))
                * glam::Mat4::from_scale(glam::Vec3::new(ring_s, ring_h, ring_s)),
            tint: [1.0, 0.55 * (1.0 - t) + 0.15, 0.06, 0.0], // 火
        });
        // ③ 火柱：垂直拉长火舌从地面向上（0.5-2 寿命段），半透明
        let col_h = 2.2 + 2.6 * t;
        out.push(engine::renderer::WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::new(cx, 1.1 + col_h * 0.5, cz))
                * glam::Mat4::from_scale(glam::Vec3::new(r * 0.5, col_h, r * 0.5)),
            tint: [1.0, 0.45 * (1.0 - t), 0.05, 0.0], // 火
        });
        // ④ 烟柱：暗色膨胀上浮（后段，营造爆炸余烟），半透明
        let smoke_s = r * (0.5 + 1.4 * t);
        let smoke_h = 2.0 + 3.0 * t;
        out.push(engine::renderer::WorldMarker {
            model: glam::Mat4::from_translation(glam::Vec3::new(cx, 0.6 + smoke_h * 0.5, cz))
                * glam::Mat4::from_scale(glam::Vec3::new(smoke_s, smoke_h, smoke_s)),
            tint: [0.16 * (1.0 - t) + 0.05, 0.13 * (1.0 - t) + 0.04, 0.1 * (1.0 - t) + 0.03, 1.0], // 烟
        });
        out
        })
        .collect();
    // 粒子（枪口焰/弹壳）转 emissive marker：枪口焰随 age 缩小淡出，弹壳保持小方块。
    let gps = game.grenade_positions();
    Self::append_particle_emissive_markers(
        &particles,
        eye,
        &gps,
        emissive_markers.len(),
        &mut emissive_markers,
    );
    // 弹孔：子弹打在障碍表面的着弹标记（`game.rs::impact_marks`）。
    // 单独一批、只追加进光栅 marker 列表 —— 不喂 PT 场景（弹孔每枪都变，
    // 会让 BLAS 指纹每帧重建，也白占 PT 盒容量）。方片的局部 +Z 用
    // `ImpactMark::basis()` 摆到表面法线上，**右手基**是硬要求（否则正面绕序反掉）。
    let decal_markers: Vec<engine::renderer::WorldMarker> = game
        .impact_marks()
        .iter()
        .map(|m| {
            let (t, u, n) = m.basis();
            let half = DECAL_SIZE_M * 0.5 * m.size_envelope();
            engine::renderer::WorldMarker {
                model: glam::Mat4::from_cols(
                    (glam::Vec3::from(t) * half).extend(0.0),
                    (glam::Vec3::from(u) * half).extend(0.0),
                    (glam::Vec3::from(n) * (DECAL_THICK_M * 0.5)).extend(0.0),
                    glam::Vec4::new(
                        m.pos[0] + m.normal[0] * DECAL_LIFT_M,
                        m.pos[1] + m.normal[1] * DECAL_LIFT_M,
                        m.pos[2] + m.normal[2] * DECAL_LIFT_M,
                        1.0,
                    ),
                ),
                // `Shape::Authored`（tint.w = 6.0）在这里是**故意**用的：
                // 它让片元跳过"给纯 tint 盒子补细节"的四条程序化效果（窗带/玻璃分格/
                // 树冠噪声/混凝土皮肤）。弹孔要的就是一块**纯色暗方片** ——
                // 走 marker 皮肤路径会给它采样一层墙纹，看起来像贴了一块小面板。
                tint: [0.035, 0.030, 0.026, engine::geom::Shape::TAG_AUTHORED],
            }
        })
        .collect();
        (markers, emissive_markers, decal_markers)
    }

    /// NPC 士兵可视化：把 `game` 里的每只 NPC 折成 `NpcVisual`（位置/朝向/阵营色/相位/动作态）。
    ///
    /// 从 `render()` 整块抽出（2026-11-12，纯提取，134 行）。抽成**参数化关联函数**的理由：
    /// 调用点在 `&mut self.renderer` 借用内，取 `&mut self` 会 E0502；而这段只碰
    /// `game` / `last_snapshot` / `corpses` / `anim_clock` 四个**互不相交**的字段，
    /// 显式传参既能编译、也把"它到底改了哪些状态"写在签名上。
    fn build_npc_visuals(
        game: &mut crate::engine::game::Game,
        last_snapshot: &mut std::collections::HashMap<usize, ([f32; 3], f32, [f32; 4])>,
        corpses: &mut Vec<([f32; 3], f32, [f32; 4], f32)>,
        anim_clock: f32,
    ) -> Vec<engine::renderer::NpcVisual> {
    // NPC 士兵可视化：每个 NPC 由 renderer 展开为 7 段积木人（头/躯干/四肢/枪），
    // 按朝向旋转，阵营配色（红=敌军、蓝=友军/玩家阵营）；
    // 动画字段：移动中摆臂摆腿（步态）、攻击态枪身后坐脉冲
    let now_ids: std::collections::HashSet<usize> =
        game.npcs.iter().map(|n| n.id).collect();
    // 尸体跟踪：本帧消失的 NPC id（被击杀移除）→ 从上一帧快照找回位置/朝向/阵营
    for id in last_snapshot.keys() {
        if !now_ids.contains(id) {
            if let Some((pos, yaw, tint)) = last_snapshot.get(id) {
                corpses.push((*pos, *yaw, *tint, 0.0));
            }
        }
    }
    // 更新快照（供下一帧 diff）
    *last_snapshot = game
        .npcs
        .iter()
        .map(|n| {
            let tint = match n.team {
                Team::Red => [0.95, 0.12, 0.08, 1.0],
                Team::Blue => [0.08, 0.35, 0.98, 1.0],
            };
            (n.id, (n.position, n.facing, tint))
        })
        .collect();
    // 客户端联机模式：显示服务器世界（快照实体：位置/朝向/血量来自服务器权威），
    // 阵营色借用本地同 id NPC 的归属（同一确定性地图/波次，id 对齐）
    //
    // 进画面的规则：id 段 >=100_000 是远端玩家、id==0 是本机玩家槽，两者无条件进；
    // 剩下的都是本地 NPC（id < 100_000），只有活着才进。
    // 原来写成 `(id<100k && hp>0) || id==0 || id>=100k`：`id==0` 已被 `id<100k`
    // 覆盖，而 `id>=100k` 恰好是 `id<100k` 的反面，吸收律化简后那段 id 判据是多余的
    // （clippy::nonminimal_bool 报的就是这个）。化简式逐档等价。
    // 🔴 渲染剔除走**分摊刷新缓存**（2026-09-26 实测优化）：
    // 原来每帧对全部 NPC 直接调 `npc_occluded`（每次最坏 2×1240 次 AABB 相交），
    // 而枪口焰那段**又调了一遍** ⇒ 压力场景 510 次/帧、`cull-diag` 实测中位
    // **102 ms/s（≈18% 帧预算）**。现在一帧只刷新 1/N 的 NPC，两条路共用同一份结果。
    // 判据 = `cull-diag: <us>/s`（`RV3D_CULL_DIAG=1`）与 `npc_vis_scans`。
    game.refresh_npc_visibility();
    let vis = game.npc_visibility_flags();
    // 🔴 用 `if let` 而不是 `is_some()` + `unwrap()`（2026-10-17）：
    // `prod_panic_sweep` 每次都会把那个 `unwrap` 报成 1 处生产 unwrap。
    // 它虽然被上一行的 `is_some()` 守着、**当前不可达**，但那是「同一个字段读两遍」
    // ——中间一旦有人插一行改动字段，就变成 panic。`if let` 把判空与取值合成一次操作，
    // 顺带把扫描基线清干净（**工具常报 1 条已知项 ⇒ 以后没人读它**）。
    let npc_visuals: Vec<engine::renderer::NpcVisual> = if let Some(client) =
        game.net_client.as_ref()
    {
        client
            .entities()
            .iter()
            .filter(|(id, e)| **id >= net::NET_PLAYER_BASE || **id == 0 || e.hp > 0.0)
            .map(|(id, e)| {
                // 阵营直接取自快照（服务器权威；NpcSnapshot.team 0=Red 1=Blue）
                let tint = if e.hp > 0.0 {
                    if e.team == 1 { [0.08, 0.35, 0.98, 1.0] } else { [0.95, 0.12, 0.08, 1.0] }
                } else {
                    [0.32, 0.32, 0.32, 1.0]
                };
                // 🔴 2026-09-26：这里以前直接用 `e.state.curr`（最新**收到**的那帧），
                // 于是远端实体按快照频率一顿一顿地跳、包抖动时更明显 ——
                // `RemotePlayer::delay` 一直是 0，插值器等于没接线。
                // 现按 `entity_state_at`（客户端内部用"now - 快照间隔"取值）取平滑位置；
                // 取不到时退回 curr（不改变任何极端情况下的行为）。
                let st = client
                    .entity_state_at(*id, client.now())
                    .unwrap_or(e.state.curr);
                engine::renderer::NpcVisual {
                    pos: st.pos,
                    yaw: st.rot,
                    tint,
                    phase: anim_clock,
                    moving: true,
                    firing: e.firing,
                }
            })
            .collect()
    } else {
        // RV3D_NO_NPC_CULL=1：跳过"玩家看不到就不画"的剔除。
        //
        // 该剔除**以玩家眼位为准**（`npc_occluded` 内部用 `player_eye()`，不是相机），
        // 这在玩法上是**正确**的：玩家隔着楼不该看见人。但**调试相机不是玩家** ——
        // 用 `RV3D_NPC_CAM` 把相机摆到 160m 外某个 NPC 跟前时，目标仍会被
        // "玩家视角"的判定剔掉。实测玩家站在原点时，255 人只有 **16 人**上屏
        //（`npcvis: 收到 16 个 NPC`，段数 144+128=272），这正是取景一路扑空的真因。
        // 关掉剔除后调试机位才看得到目标。
        let cull = !crate::syscfg::is_set("RV3D_NO_NPC_CULL");
        game
        .npcs
        .iter()
        .enumerate()
        // 隔墙透视修复：被障碍物完全遮挡的 NPC 不渲染（`vis[i]` 即该判定）
        .filter(|(i, _)| !cull || vis.get(*i).copied().unwrap_or(true))
        .map(|(_, n)| {
            let base = last_snapshot
                .get(&n.id)
                .map(|(_, _, t)| *t)
                .unwrap_or(match n.team {
                    Team::Red => [0.95, 0.12, 0.08, 1.0],
                    Team::Blue => [0.08, 0.35, 0.98, 1.0],
                });
            // 受击反馈：命中瞬间闪白（按剩余强度混合白色）
            let flash = game.npc_flash(n.id);
            let tint = if flash > 0.0 {
                let k = flash * 0.85;
                [
                    base[0] + (1.0 - base[0]) * k,
                    base[1] + (1.0 - base[1]) * k,
                    base[2] + (1.0 - base[2]) * k,
                    1.0,
                ]
            } else {
                base
            };
            engine::renderer::NpcVisual {
                pos: n.position,
                yaw: n.facing,
                tint,
                phase: anim_clock,
                moving: n.speed > 0.5
                    && matches!(
                        n.state_machine.state(),
                        crate::engine::ai::NpcState::Patrol
                            | crate::engine::ai::NpcState::Chase
                    ),
                firing: n.state_machine.state() == crate::engine::ai::NpcState::Attack,
            }
        })
        .collect()
    };
    // （士兵 GLB 的上传已移到 `Renderer::new` 之后，见那里的注释：
        npc_visuals
    }

    /// 尸体可视化：每具尸体一份 `NpcVisual`（躺倒姿态，与活体共用 NPC 槽位区）。
    ///
    /// 从 `render()` 抽出（2026-11-12，纯提取）。只读 `corpses`，因此传 `&[...]` 即可。
    fn build_dead_visuals(
        corpses: &[([f32; 3], f32, [f32; 4], f32)],
    ) -> Vec<engine::renderer::NpcVisual> {
        corpses
            .iter()
            .map(|(pos, yaw, tint, _age)| engine::renderer::NpcVisual {
                pos: *pos,
                yaw: *yaw,
                tint: *tint,
                phase: 0.0,
                moving: false,
                firing: false,
            })
            .collect()
    }

    /// PT 取景参数：与光栅化**同一相机、同一太阳方向**（路径追踪当烘焙参照，参数必须同源）。
    ///
    /// 从 `render()` 抽出（2026-11-12，纯提取）：只读 `game` 与 `camera`，返回 `PtParams`，
    /// `set_pt_params` 留在调用点。
    fn build_pt_params(
        game: &crate::engine::game::Game,
        camera: &engine::camera::Camera,
    ) -> crate::engine::ray_tracer::PtParams {
// PT 取景：与光栅化同一相机、同一太阳方向（路径追踪要当烘焙参照，参数必须同源）
let lu = game.light_uniform([camera.position().x, camera.position().y, camera.position().z], camera.yaw);
crate::engine::ray_tracer::PtParams {
    cam: camera.position(),
    fwd: camera.forward(),
    tan_half_fov: (camera.fov * 0.5).tan(),
    bounces: 6,
    sun_dir: lu.directional.direction.truncate(),
    sun_color: lu.directional.color_intensity.truncate()
        * lu.directional.color_intensity.w,
    // 曝光 = 标定值（默认 0.4，见 ray_tracer.rs 的常量注释）：光栅把反照率乘在
    // tone 之外（alb×(1-exp(-1.55L))），PT 物理正确在之内；0.4 使两模型在
    // albedo 0.1~0.8 区间分区均值互差 ≤15%。现由 config.rs 持久化、可被
    // RV3D_PT_EXPOSURE 覆盖，不再是硬编码字面量（未结案 #11 的最后一小项）。
    exposure: game.hud.pt_exposure,
}
    }

    /// 渲染一帧
    pub(crate) fn render(&mut self) {
        // 【诊断】`GameApp::render()` 的**准备段**计时（起 → 调用 `renderer.render`）。
        // 存在的理由（2026-10-13）：实测 `cyc = upd + ren` 自洽，但 `ren − frame_us ≈ 2000µs`，
        // 而 `Renderer::render()` 尾部只有 3 行 ⇒ 那 2000µs 全在本函数**包着**渲染器的那一段
        // （本函数 2531→3283 共 752 行：`hud_quads` 每帧重建、`set_hud_quads` 上传、
        // `first_person_gun_mesh`、`set_lights`、`pt_set_scene_markers` …）。
        // **这是整帧里最大的一块没有任何计时器覆盖的时间。** 先把它的占比量出来。
        let prep_start = std::time::Instant::now();
        let mut t_props: u64 = 0;
        // 第一人称枪模程序化网格（相机姿态）：在借用 renderer 前生成，避免借用冲突。
        // 🔴 **只在武器 key 变化时生成 + 上传**（见 `gun_mesh_uploaded_key` 的文档）：
        // 这把枪有几万顶点，每帧重建+拷贝约 4.6MB，而同一把枪下结果逐帧相同。
        let gun_key = self.game.active_weapon_key().to_string();
        // 🔴 **`show_gun` 必须在这里算，而且要参与"要不要生成"的判定**
        // （2026-10-15，外部复核指出第 5 条，已对着代码确认）。
        //
        // 原来 `gun_key_changed` **不看 `show_gun`**，而下面（隐藏分支）每帧把
        // `gun_mesh_uploaded_key` 清成 `None` ⇒ 形成**反馈环**：
        //   `!show_gun` ⇒ 清 key ⇒ 下一帧 `gun_key_changed` **恒为 true**
        //   ⇒ 每帧调 `first_person_gun_mesh()` **克隆整把枪**
        //     （AK-12M = 63283 顶点 × 32B + 70479 索引 × 4B ≈ **2.3MB**）
        //   ⇒ 然后在下面 `if show_gun { .. } else { .. }` 的 else 里**直接丢弃**。
        //
        // `!show_gun` 覆盖**开始菜单 / 结算画面 / 非检视的 Orbit·Flight 相机**
        // ⇒ 在菜单里坐着不动，每帧白克隆 2.3MB（分配 + 拷贝 + 释放的churn）。
        //
        // 同一个表达式原来在本函数里**写了三遍**（这里一处、`fp_gun_pre` 一处、
        // 上传处一处）—— 现在收口成这一个变量。
        let show_gun = self.inspect_weapon.is_some()
            || (self.game.state() == GameState::Playing
                && self.camera.mode == CameraMode::FirstPerson);
        let gun_key_changed =
            show_gun && self.gun_mesh_uploaded_key.as_deref() != Some(gun_key.as_str());
        let gun_mesh = if gun_key_changed {
            self.first_person_gun_mesh()
        } else {
            (Vec::new(), Vec::new())
        };
        // 诊断（每 5 秒一次）：窗口 inner_size vs swapchain extent vs HUD 尺寸
        if self.anim_clock > 5.0 && (self.anim_clock - 5.0) % 5.0 < 0.05 {
            if let Some(win) = &self.window {
                let is = win.inner_size();
                log::info!(
                    "size diag: window_inner={}x{} hud={}x{}",
                    is.width,
                    is.height,
                    self.game.hud.screen_w,
                    self.game.hud.screen_h
                );
            }
        }
        // 2026-08-28：枪实例矩阵预计算（进入 renderer 借用前）
        // `RV3D_GUN_DIAG=1`：每秒一行枪姿态诊断 —— 姿态是**动画**，静态截图只能证明
        // "画面变了"，证明不了"姿态项真的按状态在动"。这一行给出可判定的数字：
        // 冲刺时 `sprint→1`、行走时 `walk_env→1`、换弹中点 `reload≈1`、开镜 `ads→1`。
        // 判据 = `scripts\run_gunpose_probe.ps1` 的日志（见 PROGRESS §21.29）。
        if gun_diag_on() && self.last_gun_diag.elapsed().as_secs_f32() >= 1.0 {
            self.last_gun_diag = std::time::Instant::now();
            let (ix, iy) = idle_sway(self.anim_clock);
            log::info!(
                "gundiag: sprint={:.3} walk_env={:.3} reload={:.3} idle=({:+.3},{:+.3}) ads={:.2}",
                self.gun_sway.sprint,
                self.gun_walk_env(),
                crate::engine::game::reload_envelope(1.0 - self.game.hud.reload_progress),
                ix,
                iy,
                self.ads_blend
            );
        }
        let fp_gun_pre = {
            // 检视模式：顶点已居中到世界坐标，需用单位矩阵
            if self.inspect_weapon.is_some() { glam::Mat4::IDENTITY }
            else if show_gun { self.fp_gun_matrix() } else { glam::Mat4::IDENTITY }
        };

        if let Some(renderer) = &mut self.renderer {
            // 投影宽高比取实际窗口尺寸（16:10 等非 16:9 分辨率下不拉伸）
            let aspect = self
                .window
                .as_ref()
                .map(|w| {
                    let s = w.inner_size();
                    s.width.max(1) as f32 / s.height.max(1) as f32
                })
                .unwrap_or(16.0 / 9.0);
            if self.inspect_weapon.is_some() && !self.cam_logged {
                self.cam_logged = true;
                log::info!(
                    "inspect cam: pos=({:.3},{:.3},{:.3}) target=({:.3},{:.3},{:.3}) yaw={:.3} pitch={:.3} dist={:.3} fov={:.3} aspect={:.3}",
                    self.camera.position().x, self.camera.position().y, self.camera.position().z,
                    self.camera.target.x, self.camera.target.y, self.camera.target.z,
                    self.camera.yaw, self.camera.pitch, self.camera.distance,
                    self.camera.fov.to_degrees(), aspect
                );
            }
            let view = self.camera.view_matrix();
            // 投影矩阵不翻转 Y：主 shader（triangle.vert.spv）已在 gl_Position.y 上完成
            // Vulkan 翻转，若这里再翻一次会双重翻转导致画面上下颠倒（与 HUD shader 一致）。
            let proj = self.camera.projection_matrix(aspect);

            // 枪械检视模式：虚空环境——只画枪模（renderer 跳过地形/NPC/marker/阴影）
            renderer.void_mode = self.inspect_weapon.is_some();

            // HUD：用上一帧渲染统计生成覆盖层 quad 并上传（首帧统计为 0）
            let (near, far, lod) = renderer.last_stats();
            // 检视模式：无游戏 HUD（纯枪模检视画面）
            let mut quads = if self.inspect_weapon.is_some() {
                Vec::new()
            } else {
                self.game.hud_quads(near, far, lod)
            };
            // 【诊断】把 HUD 的"构建几何"与"上传"分开：`t_hud_build` = 几何构建完的时刻
            let t_hud_build = prep_start.elapsed().as_micros() as u64;
            let hud_quad_count = quads.len();
            Self::append_hud_text_overlays(
                self.command_open,
                self.game.state(),
                &self.command_buf,
                self.anim_clock,
                &self.game.hud,
                &self.hit_damage_popups,
                &mut quads,
            );
            renderer.set_hud_quads(&quads);
            renderer.set_lights(&self.game.light_uniform([self.camera.position().x, self.camera.position().y, self.camera.position().z], self.camera.yaw));
            let t_hud = prep_start.elapsed().as_micros();
            // 世界障碍 marker：关卡地图几何 → 按种类材质着色的实例（复用主 pipeline，
            // 见 renderer.rs MARKER_SLOT_BASE；模型矩阵/材质色统一由 WorldMarker::for_obstacle
            // 构建，与物理刚体 AABB（game.rs apply_level，同 half_w/half_d）严格同尺寸）。
            // 用 render_geometry() 而不是 map_obstacles()：后者只含会挡人的障碍，前者还包含
            // 挑檐/窗带/壁柱/屋顶设备这类纯装饰件（game.rs LevelMap::decor）。
            // 但要跳过 [`engine::geom::Shape::None`]：那是 GLB 道具的碰撞核，只参与物理与
            // 布局不变式检查，画出来会和 GLB 表面共面 z-fighting（正是 city.rs 零共面纪律
            // 禁止的那类穿帮）。过滤放在这里而不是 render_geometry() 内部，因为 city.rs 的
            // 布局测试需要遍历到每一个盒子。
            // 🔴 `RV3D_DUMP_NEAR=<米>`：把**相机附近**的 marker 逐件打印出来（序号/种类/位置/尺寸/形状）。
            //
            // 存在理由（2026-09-12 第 83 轮）：为画面正中一组薄板，我连试五个假设
            // （柱廊檐梁 / 退化几何 / 长椅 / 喷泉池缘 / 路缘石）**全部落空** ——
            // 因为每次都是"整类地改"（染色/搬走/压暗一整类），而**一类里有几百个实例**，
            // 这种做法**只能否证、不能定位**。
            //
            // ⇒ 这条日志把"几何是什么"变成可读的数字，而不是靠像素颜色反推
            //    （marker 的 tint 会经光照与程序化皮肤，读回来的颜色不可靠）。
            //    **量在程序里就直接打出来** —— 这是第 76 轮已经验证过的方法。
            // 🏗 `RV3D_DUMP_ALL=<路径>`（2026-11-01）：把**全部**可渲染构件导出成 TSV，
            // 供 `tools/blender/city_from_dump.py` 在 Blender 里重建整座城（把 1.8 万个实例
            // 烘焙成少量合并网格的路线，见 docs/PROGRESS.md 续125）。
            // 列：kind shape x y z half_w half_d half_h r g b
            // 与 `RV3D_DUMP_NEAR` 同样只在第一次落盘（几何在关卡加载后是静态的）。
            if let Some(path) = crate::syscfg::cfg("RV3D_DUMP_ALL") {
                use std::sync::atomic::{AtomicBool, Ordering};
                static DUMPED_ALL: AtomicBool = AtomicBool::new(false);
                if !DUMPED_ALL.swap(true, Ordering::Relaxed) {
                    let mut out = String::from("kind\tshape\tx\ty\tz\thalf_w\thalf_d\thalf_h\tr\tg\tb\n");
                    let mut n = 0usize;
                    for o in self.game.render_geometry() {
                        if o.shape == engine::geom::Shape::None {
                            continue;
                        }
                        let t = o.tint.unwrap_or([1.0, 1.0, 1.0]);
                        out.push_str(&format!(
                            "{:?}\t{:?}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\n",
                            o.kind, o.shape, o.x, o.y, o.z, o.half_w, o.half_d, o.half_h, t[0], t[1], t[2]
                        ));
                        n += 1;
                    }
                    match std::fs::write(&path, out) {
                        Ok(()) => log::info!("RV3D_DUMP_ALL: 已导出 {n} 个构件 → {path}"),
                        Err(e) => log::warn!("RV3D_DUMP_ALL: 写入 {path} 失败: {e}"),
                    }
                }
            }
            if let Some(s) = crate::syscfg::cfg("RV3D_DUMP_NEAR") {
                // ⚠ 只在**第一次**打印。这段在渲染路径里、每帧都会跑到 ——
                //    不加守卫就是几十行/秒刷屏，把真正要看的日志冲掉（第 92 轮修）。
                //    几何在关卡加载后是静态的，一次就够；要看别的机位就重跑一局。
                use std::sync::atomic::{AtomicBool, Ordering};
                static DUMPED: AtomicBool = AtomicBool::new(false);
                if !DUMPED.swap(true, Ordering::Relaxed) {
                    if let Ok(r) = s.parse::<f32>() {
                        let c = self.camera.position();
                        for (i, o) in self.game.render_geometry().enumerate() {
                        if o.shape == engine::geom::Shape::None {
                            continue;
                        }
                        let d = glam::Vec3::new(o.x - c.x, o.y - c.y, o.z - c.z).length();
                        if d <= r {
                            log::info!(
                                "near#{i} d={d:.1} {:?} @({:.1},{:.1},{:.1}) 尺寸={:.2}x{:.2}x{:.2} 高=[{:.2}..{:.2}] shape={:?}",
                                o.kind, o.x, o.y, o.z,
                                o.half_w * 2.0, o.half_d * 2.0, o.half_h * 2.0,
                                o.y - o.half_h, o.y + o.half_h,
                                o.shape
                            );
                        }
                        }
                    }
                }
            }
            // 🔴 **静态障碍 marker 走缓存**（按 `map_generation()` 失效）：
            // 逐件 `.filter(shape != None)` + `for_obstacle()` 构造矩阵实测 ~600µs，
            // 而城市场景障碍只在 `apply_level()` 时换一批。
            // 下面那次 `clone()` 只是 memcpy —— 贵的是"构造"，不是"拷贝"。
            let (markers, emissive_markers, decal_markers) = Self::build_world_markers(
                &self.game,
                &mut self.obstacle_markers_cache,
                &mut self.obstacle_markers_gen,
                &self.particles,
                self.camera.position(),
            );
            renderer.set_world_markers(&markers);
            renderer.append_markers(&decal_markers);
            renderer.set_emissive_markers(&emissive_markers);
            let t_mark = prep_start.elapsed().as_micros();
            // ---- GLB 道具几何上传 ----
            // 套件懒加载一次（重载地图不必重新解析 24 个 GLB）；几何只在**地图代号变化**
            // 时重传：一次合并是百万级顶点的 CPU 拷贝，绝不能进每帧路径。
            // 读不到 assets/props 只意味着城市退回纯程序化外观，不是错误。
            if self.prop_set.is_none() {
                self.prop_set = Some(match engine::props::PropSet::load_dir("assets/props").map(|mut ps| {
            let n = ps.sanitize_degenerate_uv();
            if n > 0 {
                log::info!("props: {n} 件退化 UV 已清零（整片采样同一格会串到别的贴图）");
            }
            ps
        }) {
                    Ok(set) => {
                        // 🏙 **整城 chunk**（2026-11-01）：Blender 里按 40m 格合并导出的
                        // 世界坐标几何，走**同一条道具管线**（同一份 VB/IB、同一套分桶剔除），
                        // 于是 1230 个程序化构件从"每帧上万实例"变成十几块合并网格。
                        // 关闭：`RV3D_NO_CITY_CHUNKS=1`。chunk 缺失时静默跳过（可选资产）。
                        log::info!("props: 渲染侧载入 {} 件网格", set.len());
                        set
                    }
                    Err(e) => {
                        log::info!("props: 渲染侧未载入（{e}），不绘制道具");
                        Default::default()
                    }
                });
            }
            let map_gen = self.game.map_generation();
            if map_gen != self.prop_map_gen {
                self.prop_map_gen = map_gen;
                if let Some(set) = self.prop_set.as_ref() {
                    if !set.is_empty() {
                        // 整城 chunk 走**独立集合**（2026-11-04）：不再并进道具套件。
                        // 理由见 `props::ChunkSet`：8 块一起走同一条合并路径会产出错位三角形。
                        if !crate::syscfg::flag("RV3D_NO_CITY_CHUNKS") {
                            match engine::props::ChunkSet::load_dir("assets/city_chunks") {
                                Some(cs) => renderer.set_city_chunks(&cs),
                                None => log::info!("chunk: assets/city_chunks 为空或不可读"),
                            }
                        }
                        renderer.set_props(set, self.game.prop_placements());
                        t_props = prep_start.elapsed().as_micros() as u64;
                    }
                }
            }
            // 🔴 `RV3D_EXPORT_CITY=<path>`：把本关**完整城市布局**（太阳/环境光 + marker
            //    盒 + GLB 道具摆放）导出为 JSON，供 `tools/blender/prerender_city.py`
            //    在无头 Blender 里合成**同布局预渲染参照帧**（建模/光照/烘焙的独立真值，
            //    不经游戏光栅管线）。坐标系原样输出游戏约定（右手 Y-up，米，道具 yaw 绕 +Y），
            //    轴变换归 Blender 脚本管。一次性导出（写成功才置位，早帧数据不全可重试）。
            if let Some(path) = crate::syscfg::cfg("RV3D_EXPORT_CITY") {
                use std::sync::atomic::{AtomicBool, Ordering};
                static CITY_EXPORTED: AtomicBool = AtomicBool::new(false);
                if !CITY_EXPORTED.load(Ordering::Relaxed) {
                    let placements = self.game.prop_placements();
                    let set_guard = self.prop_set.as_ref();
                    if let Some(set) = set_guard.filter(|s| !s.is_empty()) {
                        if !placements.is_empty() {
                            let lu = self.game.light_uniform([self.camera.position().x, self.camera.position().y, self.camera.position().z], self.camera.yaw);
                            let mut mj: Vec<String> = Vec::new();
                            for o in self.game.render_geometry() {
                                if o.shape == engine::geom::Shape::None {
                                    continue;
                                }
                                // tint 与渲染侧同源：for_obstacle 内含调色板+逐件抖动
                                let t = engine::renderer::WorldMarker::for_obstacle(&o).tint;
                                mj.push(format!(
                                    "{{\"kind\":\"{:?}\",\"x\":{:.3},\"y\":{:.3},\"z\":{:.3},\"hw\":{:.3},\"hh\":{:.3},\"hd\":{:.3},\"tint\":[{:.4},{:.4},{:.4}]}}",
                                    o.kind, o.x, o.y, o.z, o.half_w, o.half_h, o.half_d,
                                    t[0], t[1], t[2]
                                ));
                            }
                            let pj: Vec<String> = placements
                                .iter()
                                .map(|p| {
                                    format!(
                                        "{{\"mesh\":\"{}\",\"x\":{:.3},\"y\":{:.3},\"z\":{:.3},\"yaw\":{:.4},\"scale\":{:.4}}}",
                                        set.meshes[p.mesh].name, p.x, p.y, p.z, p.yaw, p.scale
                                    )
                                })
                                .collect();
                            let d = lu.directional;
                            let json = format!(
                                "{{\"version\":1,\"handedness\":\"right_y_up_meters\",\"sun\":{{\"dir\":[{:.5},{:.5},{:.5}],\"color\":[{:.4},{:.4},{:.4}],\"intensity\":{:.4}}},\"ambient\":{{\"color\":[{:.4},{:.4},{:.4}],\"intensity\":{:.4}}},\"markers\":[{}],\"props\":[{}]}}",
                                d.direction.x, d.direction.y, d.direction.z,
                                d.color_intensity.x, d.color_intensity.y, d.color_intensity.z, d.color_intensity.w,
                                lu.ambient.x, lu.ambient.y, lu.ambient.z, lu.ambient.w,
                                mj.join(","),
                                pj.join(",")
                            );
                            match std::fs::write(&path, json.as_bytes()) {
                                Ok(()) => {
                                    CITY_EXPORTED.store(true, Ordering::Relaxed);
                                    log::info!(
                                        "CITY-EXPORT: {} 字节 → {}（marker {} + 道具 {}）",
                                        json.len(), path, mj.len(), pj.len()
                                    );
                                }
                                Err(e) => log::error!("CITY-EXPORT 写入失败 {path}: {e}"),
                            }
                        }
                    }
                }
            }
            let npc_visuals = Self::build_npc_visuals(
                &mut self.game,
                &mut self.last_npc_snapshot,
                &mut self.corpses,
                self.anim_clock,
            );
            renderer.set_npc_visuals(&npc_visuals);
            let t_npc = prep_start.elapsed().as_micros();
            // RV3D_CULL_DIAG=1：每秒报一次「NPC 遮挡剔除」的实测成本 —— 先量再改的尺子。
            // 判据 = `cull-diag: <us>/s calls=<N> npcs=<M> bodies=<B>`（关着时零成本）。
            if engine::game::cull_diag_on() && self.last_cull_diag.elapsed().as_secs_f32() >= 1.0 {
                let us = self.game.occl_us.replace(0);
                let calls = self.game.occl_calls.replace(0);
                let scans = self.game.npc_vis_scans.replace(0);
                log::info!(
                    "cull-diag: {} us/s calls={} recomputed={} npcs={} bodies={}",
                    us,
                    calls,
                    scans,
                    self.game.npcs.len(),
                    self.game.world.bodies.len()
                );
                self.last_cull_diag = std::time::Instant::now();
            }
            // NPC 枪口焰/弹壳：攻击态 NPC 限流生成（每帧最多 4 个，按 id 相位轮转避免全爆发）
            //
            // 🔴 判据顺序是有意的（2026-09-26）：**先筛状态、再查可见性**。旧顺序反过来 ——
            // 于是为了挑出最多 4 个开火者，每帧对**全部 255 个** NPC 都做了一次遮挡测试
            // （就算它根本没在开火）。现在开火态先出局，遮挡测试只落在少数开火者身上，
            // 而且用的是上面那份**同一份**缓存（`vis`），不再有第二次全量扫描。
            // `vis` 原本由上面那段（现已抽成 `build_npc_visuals`）计算；这里只是**再读一次**
            // 同样的可见性标志（纯读，无副作用）—— 抽取不改变语义。
            let vis = self.game.npc_visibility_flags();
            let mut firing_npcs: Vec<[f32; 3]> = self
                .game
                .npcs
                .iter()
                .enumerate()
                .filter(|(_, n)| n.state_machine.state() == crate::engine::ai::NpcState::Attack)
                .filter(|(i, _)| vis.get(*i).copied().unwrap_or(true))
                .map(|(_, n)| n)
                .filter(|n| (n.id as f32 + self.anim_clock * 6.0) % 4.0 < 1.0)
                .take(4)
                .map(|n| {
                    // 枪口世界位置：facing 为绕 Y 旋转角（0 = +Z），枪口在身前 0.85m、高 1.3m
                    let (s, c) = n.facing.sin_cos();
                    [
                        n.position[0] + s * 0.85,
                        1.3,
                        n.position[2] + c * 0.85,
                    ]
                })
                .collect();
            // 网络远端实体开火 → 同链路枪口焰（你看到对面玩家开枪的火光）
            if let Some(client) = self.game.net_client.as_ref() {
                for (id, e) in client.entities().iter() {
                    if e.firing && e.hp > 0.0 {
                        let st = client
                            .entity_state_at(*id, client.now())
                            .unwrap_or(e.state.curr);
                        let (s, c) = st.rot.sin_cos();
                        firing_npcs.push([
                            st.pos[0] + s * 0.85,
                            st.pos[1] + 0.15,
                            st.pos[2] + c * 0.85,
                        ]);
                    }
                }
            }
            for muzzle in firing_npcs {
                self.particles.push(Particle {
                    pos: muzzle,
                    vel: [0.0, 0.0, 0.0],
                    age: 0.0,
                    life: 0.07,
                    size: 0.14,
                    tint: [1.0, 0.7, 0.2, 1.0],
                    kind: 0,
                });
            }
            // 尸体渲染（躺倒姿态，7 段/具；与活体共用 NPC 槽位区）
            let dead_visuals = Self::build_dead_visuals(&self.corpses);
            renderer.set_dead_bodies(&dead_visuals);
            let t_dead = prep_start.elapsed().as_micros();

            // 第一人称枪模高模网格（已在 render() 入口生成，此处上传）
            // 枪模仅在第一人称游玩或检视模式渲染：结算/其它相机态下隐藏
            // （否则枪模会按锚点漂浮在场景中——2-4 反馈“变成 M1 加兰德”观感）
            // 检视模式绕枪观察 ⇒ 必须用带深度测试的枪模管线（第一人称则关深度）
            renderer.set_gun_depth_test(self.inspect_weapon.is_some());
            if show_gun {
                if gun_key_changed {
                    renderer.set_first_person_gun_mesh(&gun_mesh.0, &gun_mesh.1);
                    self.gun_mesh_uploaded_key = Some(gun_key.clone());
                }
                renderer.set_first_person_gun_model(fp_gun_pre);
            } else {
                renderer.set_first_person_gun_mesh(&[], &[]);
                renderer.set_first_person_gun_model(fp_gun_pre);
                // 清掉 key：否则"隐藏→再显示同一把枪"时 key 没变 ⇒ 不会重传 ⇒ 枪不见了
                self.gun_mesh_uploaded_key = None;
            }

            // 尺寸保险（2026-08-15）：窗口实际尺寸与交换链不一致时重建——
            // 覆盖 DPI 缩放/全屏切换等任何导致 swapchain 与窗口错位的场景，
            // 根治"画面只显示左上角"（1:1 呈现但尺寸不匹配）。
            let (sw, sh) = renderer.swapchain_size();
            if let Some(win) = &self.window {
                let is = win.inner_size();
                // 无头离屏：窗口尺寸与离屏分辨率**本来就不同**（窗口只是 surface 载体），
                // 不能按窗口尺寸重建；HUD 按离屏分辨率排版。
                if crate::engine::renderer::headless_mode() {
                    let _ = self.game.hud.set_screen_size(sw as f32, sh as f32);
                } else if (is.width != sw || is.height != sh) && is.width > 0 && is.height > 0 {
                    // 🔴 降级/设备丢失时**不再每帧重试**：这一处曾每帧刷 1 条 WARN + 2 条 ERROR
                    // （实测设备丢失后 12 秒 1961 轮、5900 行日志，而画面一帧都不更新）。
                    // 判据 = `Renderer::swapchain_recovery_allowed`。
                    if renderer.swapchain_recovery_allowed() {
                        log::warn!(
                            "size mismatch: window={}x{} swapchain={}x{} → 重建交换链",
                            is.width, is.height, sw, sh
                        );
                        if let Err(e) = renderer.recreate_swapchain() {
                            log::error!("尺寸自检重建交换链失败：{}", e);
                        }
                    }
                    let _ = self.game.hud.set_screen_size(is.width as f32, is.height as f32);
                }
            }

            // 🔴 降级恢复（2026-10-12 补）：`swapchain_broken` 时**每帧**给一次重建机会。
            //
            // 上面那段"尺寸保险"只在**尺寸不一致**时才触发；而 Android 切后台再切回来时
            // 尺寸通常**不变**，`Resized` 也就不来 ⇒ 于是 `resumed()` 里那次重建**一旦失败**，
            // 就再没有任何重试入口 —— 进程活着、输入有反应、**画面永久黑屏**
            // （用户 2026-10-12 报的"切屏切回来直接黑屏"）。
            //
            // 文档此前声称有"5 秒尺寸自检"，但 `swapchain_recovery_allowed` 的四个调用点
            // **全在事件回调里**（Resized/Resumed/交换链过期），没有一个是每帧的 ——
            // 那条自检并不存在。这里把它补上。
            //
            // 不会刷屏：`swapchain_recovery_allowed` 内含 `RECREATE_RETRY_MIN_SECS` 限流；
            // `device_lost` 时它恒假 ⇒ 设备真丢了的场景仍然一次都不重试、不空转。
            if renderer.swapchain_broken() && renderer.swapchain_recovery_allowed() {
                // 同样升级：surface 死了的话，重建交换链是白试，直接走重建 surface。
                // （没有 window 就只试交换链 —— 但 `render()` 能走到这里说明窗口在。）
                if let Some(win) = &self.window {
                    let r = renderer.recreate_swapchain().or_else(|e| {
                        log::warn!("降级恢复重建交换链失败（{}）⇒ 升级为重建 Vulkan 表面", e);
                        renderer.recreate_surface(win)
                    });
                    if let Err(e) = r {
                        log::error!("降级恢复重建失败（每帧重试，已限流）：{}", e);
                    }
                } else if let Err(e) = renderer.recreate_swapchain() {
                    log::error!("降级恢复重建失败（每帧重试，已限流）：{}", e);
                }
            }
            let pt_params = Self::build_pt_params(&self.game, &self.camera);
            renderer.set_pt_params(pt_params);
            // PT 场景 = 光栅化同一批 WorldMarker（盒集合变化时才重建 BLAS，指纹判定在渲染器内）
            if let Err(e) = renderer.pt_set_scene_markers(&markers) {
                log::warn!("PT-SCENE 失败: {e}");
            }
            // 【诊断】准备段到此为止（下面才是真正的渲染器调用）
            let prep_us = prep_start.elapsed().as_micros() as u64;
            {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                if n % 120 == 0 {
                    log::info!(
                        "[prep-diag] 合计 {}µs // HUD构建 {} // HUD上传 {} // marker {} // 道具 {} // NPC {} // 尸体 {}  (prep_start 起累加µs; HUD quad 数 {})",
                        prep_us, t_hud_build, t_hud, t_mark, t_props, t_npc, t_dead,
                        hud_quad_count
                    );
                }
            }
            // 天空着色器时间：高空漂移/低角度暖色（用游戏模拟时间，headless 下也确定）。
            renderer.shader_time = self.game.time;
            renderer.rain = self.game.rain;
            if let Err(e) = renderer.render(view, proj) {
                if e == "交换链过期" {
                    log::warn!("交换链过期，尝试重建...");
                    if renderer.swapchain_recovery_allowed() {
                        if let Err(e2) = renderer.recreate_swapchain() {
                            log::error!("交换链过期后重建失败：{}", e2);
                        }
                    }
                } else {
                    log::error!("渲染错误: {}", e);
                }
            }
            // 性能日志采样（1s 一行；帧率由 perf_log 自己按窗口算，见 perf_log.rs）
            if let Some(pl) = self.perf_log.as_mut() {
                let snap = renderer.perf_snapshot();
                let (near, _, _) = renderer.last_stats();
                let _ = pl.frame(self.frame_dt_us, near, &snap);
            }
        }
    }
}
