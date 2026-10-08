// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

/// 阵营显示名（**只给 kill feed 用**，2026-09-15 中文化）
///
/// 改前是 `"RED"` / `"BLUE"`。全仓只有 `push_kill` 的两处调用在用它
/// （**不在日志、不在网络协议里** —— 这一点在改之前用 `rg 'team_name\('` 确认过），
/// 所以这里的字符串只影响玩家看到的那一行。
///
/// **为什么中文化是低风险的**：`ui.rs::render_text` 早就有 CJK 分支
/// （`is_cjk(ch)` → `engine::font_cjk` 的 12×12 点阵），而 HUD 的
/// `"OBJECTIVE 歼灭敌人 {}/{}"` 一直就是这么渲染的 ⇒ **路径是通的，缺的只是字符串**。
pub(crate) fn team_name(t: Team) -> &'static str {
    match t {
        Team::Red => "红方",
        Team::Blue => "蓝方",
    }
}
/// 由**射速**派生出这把武器支持的档位表。用户要求"不同武器支持不同档位"，
/// 但**不逐武器手写 35 份表** —— 手写表就是 35 个将来会分叉的地方，而本仓最贵的
/// 一类 bug 正是"同一个量有两套来源"。派生规则的阈值取自真实连发扳机的分布：
///
/// - `rpm < 200`：栓动狙击 / 泵动霰弹 —— **只有单发**，这类枪没有点射档
/// - `200 ≤ rpm < 550`：半自动步枪 / 精确射手 —— 单发 + 双发 + 三连发
/// - `rpm ≥ 550`：突击步枪 / 冲锋枪 —— 四档全给
pub(crate) fn fire_modes_for(rpm: f32) -> &'static [FireMode] {
    const SEMI: &[FireMode] = &[FireMode::Semi];
    const BURST: &[FireMode] = &[FireMode::Semi, FireMode::Burst2, FireMode::Burst3];
    const FULL: &[FireMode] = &[FireMode::Semi, FireMode::Burst2, FireMode::Burst3, FireMode::Auto];
    // 用 `!(rpm >= 200.0)` 而不是 `rpm < 200.0`，这样 NaN 落进最保守的"只有单发"
    if !(rpm >= 200.0) {
        SEMI
    } else if rpm < 550.0 {
        BURST
    } else {
        FULL
    }
}
/// 在**支持的档位集合**里取下一个，跳过不支持的。抽成纯函数是为了能直接测
/// "栓动狙击按 B 键不会切到连发"—— 否则得先构造一把特定武器才能覆盖到。
/// 一圈都找不到就原样返回（支持集合为空或只有当前档时不该崩也不该乱跳）。
pub(crate) fn next_supported_fire_mode(current: FireMode, supported: &[FireMode]) -> FireMode {
    let mut m = current;
    for _ in 0..4 {
        m = m.next();
        if supported.contains(&m) {
            return m;
        }
    }
    current
}
/// 纯函数：单个 NPC 分层判定。
/// `dist_sq` 为到目标（玩家或敌对 NPC）的距离平方；`interacting` 表示当前与目标存在
/// 实时交互（攻击态 / 感知到敌人 / 受击 / 被瞄准等）。交互中一律 Near（每帧步进），
/// 否则按距离阈值划分。
pub(crate) fn classify_ai_tier(dist_sq: f32, interacting: bool, params: &AiTierParams) -> AiTier {
    if interacting || dist_sq <= params.near_radius * params.near_radius {
        AiTier::Near
    } else {
        AiTier::Far
    }
}
/// 就地稳定分区：Near 在前、Far 在后，返回 Near 段长度；组内保持原相对顺序。
/// 各 NPC 独立读写（AiStepCtx 只读），重排不改变步进语义。泛型便于纯逻辑单测。
pub(crate) fn partition_ai_tiers<T>(items: &mut [T], tier_of: impl Fn(&T) -> AiTier) -> usize {
    items.sort_by_key(|it| tier_of(it));
    items.iter().filter(|it| tier_of(it) == AiTier::Near).count()
}
/// 分层判定：NPC 是否与玩家实时交互。
/// 普通模式目标恒为玩家（追击/攻击/感知/被瞄准/受击/被子弹威胁均算交互）；
/// 压力模式远处红蓝互射不算（玩家无敌旁观），仅玩家直接作用（瞄准/命中/子弹威胁）
/// 才算交互——互射 NPC 归远组（CCD1/E 核），不挤占玩家所在簇。
pub(crate) fn ai_tier_of(npc: &Npc, player: &glam::Vec3, stress: bool, params: &AiTierParams) -> AiTier {
    let dx = npc.position[0] - player.x;
    let dz = npc.position[2] - player.z;
    let dist_sq = dx * dx + dz * dz;
    let attacking_player = matches!(
        npc.state_machine.state(),
        NpcState::Chase | NpcState::Attack
    );
    let interacting = if stress {
        npc.perception.player_aiming || npc.perception.took_hit || npc.perception.under_fire
    } else {
        attacking_player
            || npc.perception.enemy_visible
            || npc.perception.player_aiming
            || npc.perception.took_hit
            || npc.perception.under_fire
    };
    classify_ai_tier(dist_sq, interacting, params)
}
/// 远组降频判定（第 3 步）：无感知、非追击/攻击、非受击/被瞄准的远 NPC
/// 每 `AI_FAR_DECIMATE` 帧步进一次（确定性按 npc.id 分帧，`frame % N == id % N`
/// 的帧才步进）；交互中 NPC 恒每帧（红线：攻击态/接火必须每帧）。
pub(crate) fn should_decimate_far(npc: &Npc, frame: u32) -> bool {
    if npc.perception.enemy_visible
        || npc.perception.took_hit
        || npc.perception.under_fire
        || npc.perception.player_aiming
        || matches!(npc.state_machine.state(), NpcState::Chase | NpcState::Attack)
    {
        return false;
    }
    frame % AI_FAR_DECIMATE != (npc.id as u32) % AI_FAR_DECIMATE
}
/// `RV3D_AI_DIAG=1`：NPC 停在非 Attack 态时每 5s 打一行"为什么"（未结案 #17 的定位工具，
/// 见 `step_npc` 里的调用点；默认关 ⇒ 生产行为与日志量不变）。
pub(crate) fn ai_diag() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_AI_DIAG").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}
/// step_npc 解析"本 NPC 这一帧的目标位置"的唯一规则。
/// 遮挡预计算必须与决策走同一条分支，否则会出现"按玩家算遮挡、按 NPC 行动"的错位。
pub(crate) fn resolve_ai_target(
    index: usize,
    stress: bool,
    spectator: bool,
    targets: &[Option<(usize, [f32; 3], f32)>],
    fallback_targets: &[[f32; 3]],
    player: &glam::Vec3,
) -> glam::Vec3 {
    match (stress, targets.get(index).copied().flatten()) {
        (true, Some((_, tp, _))) => glam::Vec3::new(tp[0], 0.0, tp[2]),
        (true, None) if spectator => {
            let f = fallback_targets.get(index).copied().unwrap_or([0.0, 0.0, 0.0]);
            glam::Vec3::new(f[0], 0.0, f[2])
        }
        _ => *player,
    }
}
/// 逐 NPC 计算"目标是否被障碍挡住"。
///
/// 采样规则与渲染剔除 [`Game::npc_occluded`] 严格一致：躯干中心与头/肩两条线段**全部**
/// 被挡才算挡住——低掩体后露出上半身的士兵仍然可见，避免"我能看到他却打不到我"这种
/// 新的不一致。
///
/// 成本：每 NPC 2 条线段，先用线段 XY 包围盒粗筛再跑 slab 测试。全城障碍约 1100 件时，
/// 粗筛把每次查询压到几十个候选盒，远好于对 `world.bodies` 做全量线性扫描。
pub(crate) fn target_occlusion(
    npcs: &[Npc],
    bodies: &[Body],
    stress: bool,
    spectator: bool,
    targets: &[Option<(usize, [f32; 3], f32)>],
    fallback_targets: &[[f32; 3]],
    player: &glam::Vec3,
) -> Vec<bool> {
    let samples = [NPC_HIT_CENTER_Y, 1.7];
    // 成本结构（第 50 轮实测本节占 `ai_us` 的 18–39%，即 240–520µs）：
    // 对**每一个 NPC × 2 个采样**都遍历**全部 bodies**（约 1100 件）——
    // 255 × 2 × 1100 ≈ **56 万次迭代/帧**。注释里说的"把候选盒压到几十个"
    // 是**粗筛之后**的收益，**粗筛本身仍是 O(bodies)**。
    //
    // 🔴 第 52 轮试过并把 body AABB 摊平成一维数组（想省掉 `aabb()` 构造与间接寻址）——
    // **实测无收益**（238µs vs 基线 252µs，在噪声内），却每帧多一次 1100 元素分配。
    // **已回退**：成本不在访问方式，而在**迭代次数本身**。
    // ⇒ 真正的修法是**算法级**的：给 bodies 建粗相位空间索引，把每 NPC 的候选盒
    //    从"全部 1100 件"降到"视野段覆盖的几个格子"，而不是继续在常数因子上抠。
    npcs.iter().enumerate()
        .map(|(index, npc)| {
            let t = resolve_ai_target(index, stress, spectator, targets, fallback_targets, player);
            let (ax, ay, az) = (npc.position[0], npc.position[1] + 1.4, npc.position[2]);
            // 线段包围盒粗筛（x/z/y 三轴都裁一遍，把候选盒压到几十个）
            let (minx, maxx) = (ax.min(t.x), ax.max(t.x));
            let (minz, maxz) = (az.min(t.z), az.max(t.z));
            let mut all_blocked = true;
            for h in samples {
                let ty = t.y + h;
                let (tmin, tmax) = (ay.min(ty), ay.max(ty));
                let blocked = bodies.iter().any(|body| {
                    let a = body.aabb();
                    if a.max.x < minx || a.min.x > maxx || a.max.z < minz || a.min.z > maxz
                        || a.max.y < tmin || a.min.y > tmax
                    {
                        return false;
                    }
                    Game::segment_hits_aabb(ax, ay, az, t.x, ty, t.z, &a)
                });
                if !blocked {
                    all_blocked = false;
                    break;
                }
            }
            all_blocked
        })
        .collect()
}
/// 压力模式目标预选：每 NPC 找视野内最近的敌对阵营 NPC（纯读，O(n²)）。
/// 返回 (目标索引, 目标位置快照, 目标朝向 facing)；None = 目标为玩家（兜底）。同距取索引小者，确定性。
/// facing 用于「目标是否面朝本 NPC」判定（总指挥指令单 #1 阶段二：让 NPC-vs-NPC 触发包抄/偷袭）。
pub(crate) fn pick_stress_targets(npcs: &[Npc], sight: f32) -> Vec<Option<(usize, [f32; 3], f32)>> {
    let mut out = Vec::with_capacity(npcs.len());
    for npc in npcs {
        let mut best: Option<(usize, f32)> = None;
        for (j, other) in npcs.iter().enumerate() {
            if other.team == npc.team || other.hp <= 0.0 {
                continue;
            }
            let dx = other.position[0] - npc.position[0];
            let dz = other.position[2] - npc.position[2];
            let d2 = dx * dx + dz * dz;
            if d2 >= sight * sight {
                continue;
            }
            if best.map_or(true, |(_, bd)| d2 < bd) {
                best = Some((j, d2));
            }
        }
        out.push(best.map(|(j, _)| (j, npcs[j].position, npcs[j].facing)));
    }
    out
}
/// 判断"这只 NPC 本帧该不该打一行 `aidiag`"，并把节流状态写回**它自己的**槽位。
///
/// 存在理由（2026-10-02）：旧实现是 `step_npc` 里的一个 `static SEEN: AtomicU64`，
/// 只记"上一个打印过的 key"。键里**确实**含 id，但全局只有一个槽 ⇒
/// 两只以上卡住的 NPC 逐帧交替时，每次都"与上次不同"，判据恒真，
/// 于是每帧两只各打一行（§20.6 实测 300s 刷 16.4 万行，
/// 而那行注释一直声称"每个卡住的 NPC 每 5s 一行"）。
/// ⇒ **诊断工具在它最该工作的场景（多只同时卡住）里把自己刷成了噪声。**
///
/// 状态改为随 `Npc` 携带：`step_npc` 对每个 `npc` 是 `&mut` 独占
/// （`par_for_each_mut` 切片）⇒ **不需要 atomic**。
/// 判据：`aidiag_throttle_is_per_npc`（正）与 `aidiag_single_shared_slot_would_flood`
/// （把旧实现的错误行为钉成特征测试，见其注释）。
pub(crate) fn aidiag_due(last_key: &mut u64, time: f32, id: usize) -> bool {
    let bucket = (time / AIDIAG_BUCKET_SECS) as u64;
    let key = (bucket << 32) | (id as u64 & 0xFFFF_FFFF);
    if *last_key == key {
        return false;
    }
    *last_key = key;
    true
}
/// 攻击态掩体选择：在障碍环带 `[ring_inner, ring_outer]` 内、紧邻存活障碍盒、
/// 且距目标不超过 `attack_range` 的遮挡掩体点中选最优（封闭性优先、其次离目标远——
/// 贴近射程边缘的掩体到位即可开火）。
///
/// - 掩体候选来自 `find_cover_shielding`（阻挡格挡在 NPC 与目标之间）
/// - 环带与障碍列表由调用方传入（读 MAP_RING_INNER/MAP_RING_OUTER 与关卡障碍列表；
///   摧毁后的障碍已从列表移除，其掩体点随之失效）
/// - 中央安全区内没有障碍 → 返回 None → 调用方保持直线推进/原地站定（冒烟机制不变）
pub(crate) fn pick_attack_cover(
    grid: &GridMap,
    npc: GridPos,
    target: GridPos,
    attack_range: f32,
    ring_inner: f32,
    ring_outer: f32,
    max_dist: u32,
    obstacles: &[MapObstacle],
) -> Option<GridPos> {
    let mut best: Option<(u32, u32, GridPos)> = None;
    for cover in find_cover_shielding(grid, npc, target, max_dist) {
        let (wx, wz) = grid_to_world(cover.pos);
        let d_origin = (wx * wx + wz * wz).sqrt();
        if d_origin < ring_inner || d_origin > ring_outer {
            continue;
        }
        // 掩体必须紧邻存活障碍盒（容差 GRID_CELL*2 覆盖"格中心到盒边"的最坏距离）
        let near_obstacle = obstacles.iter().any(|o| {
            (wx - o.x).abs() <= o.half_w + GRID_CELL * 2.0
                && (wz - o.z).abs() <= o.half_d + GRID_CELL * 2.0
        });
        if !near_obstacle {
            continue;
        }
        let (tx, tz) = grid_to_world(target);
        let dx = wx - tx;
        let dz = wz - tz;
        if dx * dx + dz * dz > attack_range * attack_range {
            continue;
        }
        let dist_t = target.manhattan(cover.pos);
        let better = match best {
            None => true,
            Some((bo, bd, _)) => {
                cover.openness < bo || (cover.openness == bo && dist_t > bd)
            }
        };
        if better {
            best = Some((cover.openness, dist_t, cover.pos));
        }
    }
    best.map(|(_, _, pos)| pos)
}
/// 按状态与战术推进单个 NPC：目标选择 → A* 寻路 → 移动（锯齿/躲避）→ 地形高度采样
pub(crate) fn advance_npc(
    npc: &mut Npc,
    state: NpcState,
    tactic: Tactic,
    target: &glam::Vec3,
    target_yaw: f32,
    grid: &GridMap,
    ring_inner: f32,
    ring_outer: f32,
    obstacles: &[MapObstacle],
    time: f32,
    dt: f32,
    stress: bool,
    squad_wp: Option<[f32; 2]>,
) {
    // 无路径（或已走完）时按状态 + 战术选择目标
    if npc.path.is_empty() || npc.path_index >= npc.path.len() {
        // 🔴 2026-09-25 真机实测补：**先把身体挪出阻挡格，再寻路**。
        // 起因：`direct_goal` 直行与分离力会把 NPC 顶进障碍 AABB（其所在格子不可通行），
        // 而寻路是从"最近可通行格"起算的 ⇒ 第一个路点可能在墙的**另一侧** ⇒
        // NPC 顶着墙走、永远到不了路点（`path_index` 不前进 ⇒ 也不再重规划，
        // 实测爬行速度 **0.02 m/s** vs 设定 4 m/s，`astar calls=0`）。
        // ⇒ 用与**出生**同一套判据（`passable_or_nearest` 环扫）把身体挪回可站立点，
        // 这样"身体所在格 = 路径起点"重新自洽，第一个路点就是它的邻格。
        let own = world_to_grid(npc.position[0], npc.position[2]);
        if !grid.is_passable(own) {
            if let Some(gp) = crate::engine::ai::passable_or_nearest(grid, own, 8) {
                let (wx, wz) = grid_to_world(gp);
                npc.position[0] = wx;
                npc.position[2] = wz;
                NOTE_NPC_UNSTUCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // 火-机动换位：换位目标优先（任何状态，一经设定即向换位点移动）
        let goal = if let Some(rp) = npc.reposition {
            world_to_grid(rp[0], rp[1])
        } else {
            match state {
            NpcState::Chase => match tactic {
                // 突击/压制：直线逼近（压制手到射程边缘即转 Attack 站定）
                // 班目标点：未接敌且有命令时优先向班目标点推进（>15m 时）
                Tactic::Advance | Tactic::Suppress => {
                    if let Some(wp) = squad_wp {
                        let sd2 = (npc.position[0] - wp[0]).powi(2) + (npc.position[2] - wp[1]).powi(2);
                        if sd2 > 15.0 * 15.0 {
                            world_to_grid(wp[0], wp[1])
                        } else {
                            world_to_grid(target.x, target.z)
                        }
                    } else {
                        world_to_grid(target.x, target.z)
                    }
                }
                // 侧翼包抄：垂直轴向偏移 3 格（12m），id 奇偶定左右形成钳形
                Tactic::Flank => {
                    let target_g = world_to_grid(target.x, target.z);
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let side = if npc.id % 2 == 0 { 1 } else { -1 };
                    flank_goal(grid, target_g, npc_g, side, FLANK_OFFSET)
                }
                // 偷袭绕背：玩家未面朝时绕大圈（20m 偏移）从背后逼近
                Tactic::Ambush => {
                    let target_g = world_to_grid(target.x, target.z);
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    ambush_goal(grid, target_g, npc_g, target_yaw, AMBUSH_OFFSET)
                }
                // 掩体跃进：逐掩体推进（只选比当前更靠近玩家的掩体）
                Tactic::CoverAdvance => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    let cur = npc_g.manhattan(target_g);
                    match find_cover_points(grid, npc_g, COVER_MAX_DIST)
                        .into_iter()
                        .find(|c| c.dist < cur)
                    {
                        Some(cover) => cover.pos,
                        None => target_g,
                    }
                }
                // 掩体利用：障碍环带内选"距目标 ≤ 攻击距离"的遮挡掩体，先到掩体再开火；
                // 无可用掩体（中央安全区）→ 直线推进，保持站定/站定日志语义。
                // 压力模式（NPC-vs-NPC）：沿目标方向找遮挡掩体（NPC 穿越障碍带时利用），
                // 就近取第一个；环带过滤不适用（NPC 在环带外）。
                Tactic::CoverSeek => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    if stress {
                        crate::engine::ai::find_cover_shielding(
                            grid,
                            npc_g,
                            target_g,
                            STRESS_COVER_MAX_DIST,
                        )
                        .first()
                        .map(|c| c.pos)
                        .unwrap_or(target_g)
                    } else {
                        pick_attack_cover(
                            grid,
                            npc_g,
                            target_g,
                            npc.attack_range,
                            ring_inner,
                            ring_outer,
                            COVER_MAX_DIST,
                            obstacles,
                        )
                        .unwrap_or(target_g)
                    }
                }
                // 低血量撤退：撤向最封闭且较远的遮挡掩体（阻挡格挡在 NPC 与玩家之间）
                Tactic::Retreat => {
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let target_g = world_to_grid(target.x, target.z);
                    find_cover_shielding(grid, npc_g, target_g, COVER_MAX_DIST)
                        .first()
                        .map(|c| c.pos)
                        .unwrap_or(npc_g)
                }
                Tactic::Hold => world_to_grid(npc.position[0], npc.position[2]),
            },
            NpcState::Attack => {
                // 就近掩体站定（贴障碍簇）；无掩体原地（保持攻击站定日志供冒烟瞄准）
                let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                match find_cover_points(grid, npc_g, COVER_MAX_DIST).first() {
                    Some(cover) => cover.pos,
                    None => npc_g,
                }
            }
            NpcState::Patrol | NpcState::Idle => {
                // 班目标点：有命令时优先按班目标推进；否则确定性巡逻点（随 id 相位与时间缓慢旋转）
                if let Some(wp) = squad_wp {
                    world_to_grid(wp[0], wp[1])
                } else {
                    // 🔴 #17 根因修复：旧版绕 home 画圆（r=20+id*3），出生在视距
                    // 外的敌人永远绕自家转圈不收敛（aidiag 实证：dist 83-91m、
                    // occluded=false），波次永远清不掉。现在圆心向目标推进 70%、
                    // 半径封顶 18m：扫掠仍提供变化，但每圈都在逼近，走进视距后
                    // Chase/Attack 自然接管。
                    let angle = npc.id as f32 * 2.399 + (time / 8.0).floor() * 0.7;
                    let cx = npc.home[0] + (target.x - npc.home[0]) * 0.7;
                    let cz = npc.home[1] + (target.z - npc.home[1]) * 0.7;
                    world_to_grid(cx + 18.0 * angle.cos(), cz + 18.0 * angle.sin())
                }
            }
        }
        };
        // 诊断：把本帧选定的目标记下来（`aidiag: move` 每行的 `goal=`；见 `Npc::last_goal`）
        npc.last_goal = grid_to_world(goal).into();
        let start_raw = world_to_grid(npc.position[0], npc.position[2]);
        // 🔴 2026-09-25 真机实测修：NPC 经常**站在阻挡格里**（`direct_goal` 直行会把它顶进障碍，
        // 分离力也会互相挤进去），而 `find_path` 要求起点可通行 ⇒ 每次重规划都 O(1) 失败
        // （实测 `calls=108 fails=108 起点阻挡=108`），NPC 从此**完全没有路径**，
        // 只能继续直行贴墙 ⇒ `occluded=true` ⇒ 玩家打不到 ⇒ 波次永远清不掉（#17 的可见症状）。
        // ⇒ 起点先搬到最近的可通行格（确定性的），再寻路。
        let start = crate::engine::ai::passable_or_nearest(grid, start_raw, 8).unwrap_or(start_raw);
        // 寻路兜底（2026-08-23 残局卡死修复）：目标不可达时先找最近可行格，
        // 仍无路径则直行进逼（可能蹭障碍但绝不原地踏步），保证任何局面都在动。
        let mut path = find_path(grid, start, goal);
        if path.is_none() {
            // 目标侧同样搬一次：螺旋扫描目标周边（半径 8 格）找可行的最近格
            if let Some(gp) = crate::engine::ai::passable_or_nearest(grid, goal, 8) {
                path = find_path(grid, start, gp);
            }
        }
        npc.path = path.clone().unwrap_or_default();
        npc.path_index = 0;
        // 直行机动：路径全空（含兜底失败）→ 朝世界坐标目标直线移动，绝不原地踏步
        npc.direct_goal = path.is_none();
        if npc.direct_goal {
            let (wx, wz) = grid_to_world(goal);
            npc.direct_x = wx;
            npc.direct_z = wz;
        }
    }

    // 躲避冷却/计时无条件递减（含 Attack 态，防冻结窗口；残留计时归零防"幽灵侧移"）
    npc.hit_cooldown = (npc.hit_cooldown - dt).max(0.0);
    npc.dodge_timer = (npc.dodge_timer - dt).max(0.0);

    // 爆炸冲击波推挤：覆盖本帧移动（指数衰减，约 0.25s 内衰减到 5%）
    if npc.knockback[0] != 0.0 || npc.knockback[1] != 0.0 {
        npc.position[0] += npc.knockback[0] * dt;
        npc.position[2] += npc.knockback[1] * dt;
        let decay = (-KNOCKBACK_DECAY * dt).exp();
        npc.knockback[0] *= decay;
        npc.knockback[1] *= decay;
        if npc.knockback[0].abs() < 0.05 && npc.knockback[1].abs() < 0.05 {
            npc.knockback = [0.0, 0.0];
        }
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    // 攻击态原地站定（冒烟瞄准依据 `npc: #id stand`）；火-机动换位中不站定（连续移动）
    if state == NpcState::Attack && npc.reposition.is_none() {
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    // 受击/火力威胁后侧向弹开（垂直于 目标→NPC 方向，id 奇偶定左右）
    if npc.dodge_timer > 0.0 {
        let dx = npc.position[0] - target.x;
        let dz = npc.position[2] - target.z;
        let d = (dx * dx + dz * dz).sqrt().max(1e-4);
        let side = if npc.id % 2 == 0 { 1.0 } else { -1.0 };
        let step = npc.speed * dt;
        npc.position[0] += -dz / d * side * step;
        npc.position[2] += dx / d * side * step;
        npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
        return;
    }

    let (tx, tz) = if npc.path_index < npc.path.len() {
        grid_to_world(npc.path[npc.path_index])
    } else if npc.direct_goal {
        (npc.direct_x, npc.direct_z)
    } else {
        (npc.position[0], npc.position[2])
    };
    let dx = tx - npc.position[0];
    let dz = tz - npc.position[2];
    let d = (dx * dx + dz * dz).sqrt();
    if d < 1.0 {
        npc.path_index += 1;
    } else if d > 1e-4 {
        // 推进态锯齿机动：垂直前进方向横向摆动（被瞄准/火力威胁时幅度加大）
        let dxp = npc.position[0] - target.x;
        let dzp = npc.position[2] - target.z;
        let dist_p = (dxp * dxp + dzp * dzp).sqrt();
        let (mut mx, mut mz) = (dx / d, dz / d);
        if state == NpcState::Chase && dist_p < ZIGZAG_DIST && tactic != Tactic::Retreat {
            let amp = if npc.perception.under_fire || npc.perception.player_aiming {
                ZIGZAG_AMP_HIGH
            } else {
                ZIGZAG_AMP
            };
            let off = zigzag_offset(time, npc.id as u32, amp);
            mx += -dz / d * off;
            mz += dx / d * off;
            let mlen = (mx * mx + mz * mz).sqrt().max(1e-4);
            mx /= mlen;
            mz /= mlen;
        }
        let step = npc.speed * dt;
        let (step_x0, step_z0) = (npc.position[0], npc.position[2]);
        // 2026-08-25 穿墙修复 + 2026-09-25 沿墙滑动：移动后对存活的静态障碍 AABB 推开；
        // 若整步被推回（覆盖率建网后 NPC 会贴着薄墙/家具走），改沿接触面切向滑一步。
        let (px, pz) =
            step_with_slide(obstacles, (step_x0, step_z0), (mx, mz), step, NPC_BODY_RADIUS);
        npc.position[0] = px;
        npc.position[2] = pz;
        // 归因埋点（#17）：这一步"想走"了多远、实际净位移多少 —— 净位移 < 半步 即仍被障碍推回
        // （滑动也没救回来的正撞/夹角，是真正贴着墙磨的帧）。
        if ai_diag() {
            let ord = std::sync::atomic::Ordering::Relaxed;
            NOTE_MOVE_STEP.fetch_add(1, ord);
            let net = ((px - step_x0).powi(2) + (pz - step_z0).powi(2)).sqrt();
            if net < step * 0.5 {
                NOTE_MOVE_UNDONE.fetch_add(1, ord);
            }
        }
    }
    npc.position[1] = terrain_height_at(npc.position[0], npc.position[2]);
}
/// 一步"走 + 防穿墙 + **沿墙滑动**"：返回新的水平位置。
///
/// 🔴 2026-09-25 加（覆盖率建网规则的配套修复）。导航网格改成"覆盖 ≥1/3 格才封格"之后，
/// 路径会贴着薄墙/家具走，而原实现把"走进障碍"的整步交给 `resolve_circle_obstacles` 推回来
/// ⇒ 真机实测（RV3D_AI_DIAG=1，defense_line 第 1 波残局）**1 秒 64 帧里有 18–30 帧位移被
/// 完全抵消**，NPC 实测速度掉到 **1.3–2.2 m/s**（设定 4.0）⇒ 9–14m 外磨到超时，
/// 波次照样清不掉。
///
/// 修法与玩家的 `push_out_of_aabb` **一样**：推回量超过半步时，把意图方向**投影到接触面的
/// 切向**再走一次 —— 撞墙只损失法向分量，不丢整帧。正撞（意图与法线平行）没有切向可走，
/// 保留推回点；滑动结果若还不如原地，也保留推回点（**绝不倒退**）。
pub(crate) fn step_with_slide(
    obs: &[MapObstacle],
    from: (f32, f32),
    dir: (f32, f32),
    step: f32,
    r: f32,
) -> (f32, f32) {
    let want = (from.0 + dir.0 * step, from.1 + dir.1 * step);
    let (px, pz) = resolve_circle_obstacles(obs, want.0, want.1, r);
    let net = ((px - from.0).powi(2) + (pz - from.1).powi(2)).sqrt();
    if net >= step * 0.5 {
        return (px, pz);
    }
    // 接触法线方向 = 意图点 − 推回点（即障碍把这一步顶回来的方向）
    let (nx, nz) = (want.0 - px, want.1 - pz);
    let nl = (nx * nx + nz * nz).sqrt();
    if nl < 1e-4 {
        return (px, pz);
    }
    let (ux, uz) = (nx / nl, nz / nl);
    let dot = dir.0 * ux + dir.1 * uz;
    let (mut tx, mut tz) = (dir.0 - dot * ux, dir.1 - dot * uz);
    let tl = (tx * tx + tz * tz).sqrt();
    if tl < 1e-3 {
        return (px, pz); // 正撞：没有切向可走
    }
    tx /= tl;
    tz /= tl;
    let (sx, sz) = (px + tx * step, pz + tz * step);
    let (qx, qz) = resolve_circle_obstacles(obs, sx, sz, r);
    let gain = ((qx - from.0).powi(2) + (qz - from.1).powi(2)).sqrt();
    if gain > net {
        (qx, qz)
    } else {
        (px, pz)
    }
}
/// 圆（半径 r）对存活障碍 AABB 的水平推开（NPC 移动后防穿墙；MapObstacle 版）
pub(crate) fn resolve_circle_obstacles(obs: &[MapObstacle], x: f32, z: f32, r: f32) -> (f32, f32) {
    let mut ox = x;
    let mut oz = z;
    for ob in obs {
        let (hx, hz) = (ob.half_w, ob.half_d);
        let cx = (ox - ob.x).clamp(-hx, hx);
        let cz = (oz - ob.z).clamp(-hz, hz);
        let dx = ox - (ob.x + cx);
        let dz = oz - (ob.z + cz);
        let d2 = dx * dx + dz * dz;
        if d2 < r * r {
            if d2 > 1e-6 {
                let d = d2.sqrt();
                let push = r - d;
                ox += dx / d * push;
                oz += dz / d * push;
            } else {
                let px = hx + r - (ox - ob.x).abs();
                let pz = hz + r - (oz - ob.z).abs();
                if px < pz {
                    ox += if ox > ob.x { px } else { -px };
                } else {
                    oz += if oz > ob.z { pz } else { -pz };
                }
            }
        }
    }
    (ox, oz)
}
/// 圆（半径 r）对静态障碍 AABB 的水平推开：返回 (x, z)（AABB 为 (cx±half_w, cz±half_d)）
pub(crate) fn resolve_circle_static(
    bodies: &[physics::Body],
    x: f32,
    z: f32,
    r: f32,
) -> (f32, f32) {
    let mut ox = x;
    let mut oz = z;
    for b in bodies {
        let (hx, hz) = (b.half_extents.x, b.half_extents.z);
        let cx = (ox - b.position.x).clamp(-hx, hx);
        let cz = (oz - b.position.z).clamp(-hz, hz);
        let dx = ox - (b.position.x + cx);
        let dz = oz - (b.position.z + cz);
        let d2 = dx * dx + dz * dz;
        if d2 < r * r {
            if d2 > 1e-6 {
                let d = d2.sqrt();
                let push = r - d;
                ox += dx / d * push;
                oz += dz / d * push;
            } else {
                // 圆心在盒内：沿最小穿透轴推出
                let px = hx + r - (ox - b.position.x).abs();
                let pz = hz + r - (oz - b.position.z).abs();
                if px < pz {
                    ox += if ox > b.position.x { px } else { -px };
                } else {
                    oz += if oz > b.position.z { pz } else { -pz };
                }
            }
        }
    }
    (ox, oz)
}
