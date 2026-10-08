// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// NPC 是否被障碍物完全遮挡：玩家眼位 → NPC 的采样点（身体中心 +1.0m 与头部 +1.7m）
    /// 的线段与任一障碍 AABB 相交。两处都被挡才算完全遮挡；任一处可见（如从矮墙
    /// 上方露出头/肩）即不遮挡——修复"隔墙透视"同时避免"半身可见却消失"。
    ///
    /// ⚠️ `RV3D_CULL_DIAG=1` 时这里会计时（见 `cull_diag_on`），用于**先量再改**：
    /// 一次调用最坏要扫 2 × `world.bodies.len()` 个 AABB，而 `main.rs` 每帧对全部 NPC
    /// 调它**两遍**。
    pub(crate) fn npc_occluded(&self, idx: usize) -> bool {
        if !cull_diag_on() {
            return self.npc_occluded_untimed(idx);
        }
        let t0 = std::time::Instant::now();
        let r = self.npc_occluded_untimed(idx);
        self.occl_us.set(self.occl_us.get() + t0.elapsed().as_micros() as u64);
        self.occl_calls.set(self.occl_calls.get() + 1);
        r
    }
    pub(crate) fn npc_occluded_untimed(&self, idx: usize) -> bool {
        let Some(n) = self.npcs.get(idx) else {
            return false;
        };
        let eye = self.cull_eye_override.unwrap_or_else(|| self.player_eye());
        for h in [NPC_HIT_CENTER_Y, 1.7] {
            let (ax, ay, az) = (eye.x, eye.y, eye.z);
            let (bx, by, bz) = (n.position[0], n.position[1] + h, n.position[2]);
            let blocked = self.world.bodies.iter().any(|body| {
                Self::segment_hits_aabb(ax, ay, az, bx, by, bz, &body.aabb())
            });
            if !blocked {
                return false;
            }
        }
        true
    }
    /// 刷新渲染用的「玩家看得见否」缓存（分摊刷新，见 `NPC_VIS_REFRESH_FRAMES`）。
    ///
    /// 每个 NPC 有一个固定槽位：只有 `(frame + i) % N == 0` 的那些这一帧才真做遮挡测试
    /// ⇒ 每帧重算 **1/N** 的 NPC，而每个 NPC 的判定最多旧 N−1 帧（≈40ms @100fps）。
    /// 分摊（而不是"每 N 帧全体重算一次"）是为了让过时数据**散在几个 NPC 上**，
    /// 不会整场一起跳变 —— 视觉上只是一两个士兵晚 40ms 才消失。
    ///
    /// 🔴 槽位里存的是 **`(npc.id, 可见)` 而不是单一个 bool**：NPC 死亡会把后面所有元素
    /// 的**下标前移**（`npcs.retain` / `swap_remove`），只按下标缓存 ⇒ 死一个人之后，
    /// 后面每个人的标志都变成了**前一个人的**（最多错 3 帧）。带上 id 就能一眼看出错位：
    /// id 不符 ⇒ 无论槽位轮没轮到，当帧立刻重算（这是"精确性"那一边，不受分摊限制）。
    ///
    /// 新出现的 NPC 一律**先按可见处理**（fail-open）：宁可多画一个也不让新兵隐形，
    /// 它的第一次刷新最迟 N−1 帧后到。
    pub(crate) fn refresh_npc_visibility(&mut self) {
        let n = self.npcs.len();
        if self.npc_vis.len() != n {
            self.npc_vis.resize(n, (usize::MAX, true));
        }
        let shift = self.npc_vis_frame % NPC_VIS_REFRESH_FRAMES;
        for i in 0..n {
            let due = (i as u32 + shift) % NPC_VIS_REFRESH_FRAMES == 0;
            // 下标错位（有人死了）：立刻重算，不等自己的槽位
            let shifted = self.npc_vis[i].0 != self.npcs[i].id;
            if !due && !shifted {
                continue;
            }
            self.npc_vis[i] = (self.npcs[i].id, !self.npc_occluded(i));
            self.npc_vis_scans.set(self.npc_vis_scans.get() + 1);
        }
        self.npc_vis_frame = self.npc_vis_frame.wrapping_add(1);
    }
    /// 上一帧 [`Game::refresh_npc_visibility`] 的结果（`true` = 玩家看得见）。
    /// 索引越界返回 `true`（fail-open，理由同上）。
    pub(crate) fn npc_visibility_flags(&self) -> Vec<bool> {
        // 每帧一次（255 个 bool）：把 `(id, flag)` 摊平成调用方要的形状。
        // 就地返回 `&[bool]` 需要第二份并行数组，收益不值得（一次 255 字节的拷贝 ≈ 几十纳秒）。
        self.npc_vis.iter().map(|(_, v)| *v).collect()
    }
    /// NPC 受击闪白剩余强度（0..1；0 = 无闪白）。渲染侧按此混合白色 tint。
    pub(crate) fn npc_flash(&self, id: usize) -> f32 {
        self.npc_hit_flash
            .get(&id)
            .map(|t| (t / 0.15).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }
    /// 推进单个 NPC：感知 → 状态机 → 战术决策 → 躲避 → A* 路径 → 移动 → 朝向。
    /// 与旧版串行循环体逐行为一致（普通波次目标=玩家，行为不变；压力模式目标=敌对 NPC）。
    pub(crate) fn step_npc(index: usize, npc: &mut Npc, ctx: &AiStepCtx) {
        // 视野半径：压力模式全场可见（两军立即接火），普通模式保持原值
        let sight = if ctx.stress { STRESS_SIGHT } else { NPC_SIGHT };
        // 目标位置：与遮挡预计算共用 resolve_ai_target，杜绝"按 A 算遮挡、按 B 决策"
        let target_pos = resolve_ai_target(
            index,
            ctx.stress,
            ctx.spectator,
            ctx.targets,
            ctx.fallback_targets,
            ctx.player,
        );
        let dx = npc.position[0] - target_pos.x;
        let dz = npc.position[2] - target_pos.z;
        let dist = (dx * dx + dz * dz).sqrt();
        let yaw_to = yaw_to_target(target_pos.x, target_pos.z, npc.position[0], npc.position[2]);
        let facing_angle = angle_diff(ctx.player_yaw, yaw_to).abs();
        // 绕背判定用的"目标朝向"：普通模式 = 玩家视角；压力模式 = 朝向目标 NPC 的方向
        // （facing 坐标系：atan2(dz,dx)，与 npc.facing 同源可比）
        let target_yaw = if ctx.stress && ctx.targets.get(index).copied().flatten().is_some() {
            (npc.position[2] - target_pos.z).atan2(npc.position[0] - target_pos.x)
        } else {
            ctx.player_yaw
        };
        // 「目标是否面朝本 NPC」：普通模式 = 玩家视角（行为不变）；压力模式 = 目标 NPC 的
        // 朝向 facing 是否大致指向本 NPC（总指挥指令单 #1 阶段二：让 NPC-vs-NPC 触发包抄/偷袭）。
        // 坐标系：target_yaw = (本NPC.z - 目标.z).atan2(本NPC.x - 目标.x) 即「目标→本NPC」方向，
        // 与 npc.facing（atan2(dz,dx)）同源可直接比。angle_diff 已处理 ±π 环绕。
        let target_facing = match (ctx.stress, ctx.targets.get(index).copied().flatten()) {
            (true, Some((_, _, tf))) => angle_diff(tf, target_yaw).abs() < std::f32::consts::FRAC_PI_2,
            _ => facing_angle < std::f32::consts::FRAC_PI_2,
        };
        let prev = npc.state_machine.state();
        let took_hit = npc.hp < npc.last_hp - 0.001;
        let under_fire = ctx.under_fire.get(index).copied().unwrap_or(false);
        // 目标被几何体挡住 = 不可见。缺数据（空 slice）时按"未挡住"处理，保持旧行为。
        let occluded = ctx.target_occluded.get(index).copied().unwrap_or(false);
        let can_see_target = dist < sight && !occluded;
        npc.perception = NpcPerception {
            enemy_visible: can_see_target,
            // 🔴「知道要打谁」与「现在看得见」分两条通道（未结案 #17 的修法②）：
            // 出生半径 40–80m 而 `NPC_SIGHT = 60` ⇒ 出生在 60m 外的那批**永远**看不见玩家，
            // 旧行为下它们恒为 Patrol，而 `update_waves` 要求 `npcs.is_empty()` ⇒
            // **这一波永远清不掉**（实测 #8 在 77.8m 上守了整局）。进攻方不该靠视距才知道要打哪。
            // ⚠️ 开火不受影响：`Chase → Attack` 仍要求 `enemy_visible`（上面那行）。
            target_known: ctx.target_known,
            // 压力模式攻击距离放宽 x2.5（约 30m）：同速互追刷步机的残局收敛（2026-08-23）
            enemy_in_range: dist < npc.attack_range * if ctx.stress { 2.5 } else { 1.0 },
            start_patrol: prev == NpcState::Idle,
            patrol_finished: false,
            player_aiming: facing_angle < AIM_ANGLE && can_see_target,
            player_facing: target_facing,
            took_hit,
            low_hp: npc.hp < npc.max_hp * LOW_HP_RATIO,
            under_fire,
        };
        let state = npc.state_machine.update(npc.perception);
        // 诊断埋点（`RV3D_AI_DIAG=1`）：把"这个 NPC 为什么不进 Attack"压成一行。
        // 未结案 #17：survive 第 1 波残余 1–2 只十余分钟恒为 `Patrol`，波次永远清不掉；
        // 而 `Patrol` 只能由 `enemy_visible == false` 维持（`ai.rs::NpcStateMachine`），
        // `enemy_visible = dist < sight && !occluded` ⇒ 只可能是"太远"或"被挡"，
        // 这一行把二者分开（`occluded` 是原始判据；`lines` 为 0 = 遮挡数据缺失、按未挡处理）。
        // 🔴 键 = 时间桶(5s) + id，但**节流状态必须挂在每只 NPC 自己身上**。
        //   旧实现是 `static SEEN: AtomicU64` 只记"最后一个 key"：两只以上卡住的 NPC
        //   逐帧交替 ⇒ 每帧都"变了" ⇒ 恒真，300s 刷 16.4 万行（§20.6 实测），
        //   而这里的注释一直声称"每个卡住的 NPC 每 5s 一行"——注释与代码同时是错的。
        //   `step_npc` 对每个 `npc` 是 `&mut` 独占（`par_for_each_mut`）⇒ 无需 atomic。
        if ai_diag() && state != NpcState::Attack {
            if aidiag_due(&mut npc.aidiag_bucket, ctx.time, npc.id) {
                // 2026-09-25 扩展：实测 NPC 有路径却在**爬行**（0.02 m/s vs 设定 4 m/s），
                // 只靠 state/dist 判不出来 ⇒ 把"移动为什么没发生"的四个候选一起打出来：
                //   path=idx/len（路点推进到哪）wp_d（离当前路点多远）
                //   dodge（>0 = 正在侧向弹开，这一支会**跳过前进**）
                //   goal=direct|path（走的是直行兜底还是路径）
                let wp_d = if npc.path_index < npc.path.len() {
                    let (wx, wz) = grid_to_world(npc.path[npc.path_index]);
                    ((wx - npc.position[0]).powi(2) + (wz - npc.position[2]).powi(2)).sqrt()
                } else {
                    -1.0
                };
                log::info!(
                    "aidiag: #{} state={:?} dist={:.1} sight={:.0} known={} occluded={} lines={} pos=({:.1}, {:.1}) path={}/{} wp_d={:.1} dodge={:.1} goal={}",
                    npc.id,
                    state,
                    dist,
                    sight,
                    ctx.target_known,
                    occluded,
                    ctx.target_occluded.len(),
                    npc.position[0],
                    npc.position[2],
                    npc.path_index,
                    npc.path.len(),
                    wp_d,
                    npc.dodge_timer,
                    if npc.direct_goal { "direct" } else { "path" }
                );
            }
        }
        // 火-机动交替打（2026-08-26）：攻击态站打数秒 → 换下一个掩体/侧移点（再站打）
        if state == NpcState::Attack {
            npc.attack_timer += ctx.dt;
            if npc.reposition.is_none() {
                // 站打阈值：3.4s 后开始换位；换位冷却由 timer 重置管理
                if npc.attack_timer > 3.4 {
                    // 优先换到掩体点（周围障碍环带内遮挡点）；无掩体则沿垂直方向侧移 9m
                    let npc_g = world_to_grid(npc.position[0], npc.position[2]);
                    let goal = crate::engine::ai::find_cover_points(ctx.grid, npc_g, COVER_MAX_DIST)
                        .into_iter()
                        .filter(|c| {
                            let (gx, gz) = grid_to_world(c.pos);
                            let dx = gx - npc.position[0];
                            let dz = gz - npc.position[2];
                            dx * dx + dz * dz > 9.0
                        })
                        .min_by_key(|c| c.dist)
                        .map(|c| {
                            let (gx, gz) = grid_to_world(c.pos);
                            [gx, gz]
                        });
                    let goal = goal.unwrap_or_else(|| {
                        // 无掩体：垂直目标方向侧移（id 奇偶定左右，制造交替推进感）
                        let dx = target_pos.x - npc.position[0];
                        let dz = target_pos.z - npc.position[2];
                        let dl = (dx * dx + dz * dz).sqrt().max(1.0);
                        let side = if npc.id % 2 == 0 { 1.0 } else { -1.0 };
                        [
                            npc.position[0] - dz / dl * 9.0 * side,
                            npc.position[2] + dx / dl * 9.0 * side,
                        ]
                    });
                    npc.reposition = Some(goal);
                }
            } else {
                // 已在换位中：到位（<2m）或换位超时（5s）→ 回站打并重置计时
                let [gx, gz] = npc.reposition.unwrap();
                let dx = gx - npc.position[0];
                let dz = gz - npc.position[2];
                if dx * dx + dz * dz < 4.0 || npc.attack_timer > 8.4 {
                    npc.reposition = None;
                    npc.attack_timer = 0.0;
                }
            }
        } else {
            // 非攻击态：清换位/计时（保持其它状态机行为）
            npc.attack_timer = 0.0;
            npc.reposition = None;
        }
        // 躲避触发：仅移动态（Attack 站定是冒烟瞄准依据）；受击反应更强、冷却更久
        if state != NpcState::Attack
            && npc.hit_cooldown <= 0.0
            && dist < DODGE_TRIGGER_DIST
            && (took_hit || under_fire)
        {
            npc.dodge_timer = if took_hit {
                DODGE_HIT_TIME
            } else {
                DODGE_THREAT_TIME
            };
            npc.hit_cooldown = DODGE_COOLDOWN;
        }
        npc.last_hp = npc.hp;
        // 战术决策：低血量撤退 / 角色行为 / 目标是否面朝（偷袭）；冲锋覆盖为突进。
        // Flanker 的包抄/偷袭战术不被冲锋覆盖（保持侧翼机动，实现互射战场的包抄/偷袭——
        // 总指挥指令单 #1 阶段二）；其余角色冲锋时全队直突（行为与原设计一致）。
        let mut tactic = pick_tactic(npc.role, &npc.perception);
        let is_flank_maneuver = matches!(tactic, Tactic::Flank | Tactic::Ambush);
        // 🔴 2026-09-26：**掩体爬行者（CoverCrawler）不再被冲锋覆盖**。
        // 实测依据（survive 模式 270s / 4 波，`RV3D_AI_DIAG=1` 的 `aidiag: tactic 1s`）：
        // `CoverSeek=0%` **整场**、`CoverAdvance=0%` —— 不是"地图没掩体"（defense_line 内圈
        // 有 8 段沙袋），而是 `should_charge`（≥50% 的 NPC 在追/打就全队冲锋）**几乎一直成立**，
        // 而下面那条 CoverSeek 升级要求 `!ctx.charge`，冲锋覆盖又把 CoverCrawler 也改成 Advance
        // ⇒ 掩体战术在生存模式里**一次都没生效**。现在只豁免这一个角色（约 1/6，第 3 波起存在），
        // "冲锋 = 其余角色全队直突"的原设计保持不变。
        let is_cover_crawler = npc.role == TacticalRole::CoverCrawler;
        if ctx.charge
            && npc.role != TacticalRole::Suppressor
            && !is_cover_crawler
            && tactic != Tactic::Retreat
            && !is_flank_maneuver
        {
            tactic = Tactic::Advance;
        }
        // 掩体利用：突击/压制手接近射程边缘时先评估障碍环带掩体（先移动到掩体再推进开火）。
        // 环带内无射程内掩体（如玩家处于中央安全区）时保持原直线推进 → 冒烟站定语义不变；
        // 只在 Chase 态生效且冲锋时不做（冲锋 = 全队直突）——**唯一的例外是掩体爬行者**。
        // 压力模式（NPC-vs-NPC）：目标在射程内且本 NPC 附近（40m）存在障碍格 → 也进入
        // 掩体利用（互射战场用障碍环带/关卡掩体，总指挥指令单 #2 阶段二）。
        if state == NpcState::Chase
            && (!ctx.charge || is_cover_crawler)
            && matches!(
                tactic,
                Tactic::Advance | Tactic::Suppress | Tactic::CoverAdvance
            )
        {
            // 压力模式：目标在射程附近（≤ attack_range + 40m）即进入掩体利用——
            // advance 沿目标方向找遮挡掩体（NPC 穿越障碍带时自然利用），不要求当前位置附近有障碍。
            let range = if ctx.stress {
                npc.attack_range + COVER_SEEK_RANGE * 2.0
            } else {
                npc.attack_range + COVER_SEEK_RANGE
            };
            if dist <= range {
                tactic = Tactic::CoverSeek;
            }
        }
        // 压力模式 Attack 态：若 NPC 站定于障碍掩体旁（贴掩体探头射击），战术标记为
        // CoverSeek——让互射战场中「利用掩体交火」的 NPC 持续可见（供战术分布采样与观察）。
        // 普通模式行为不变（冒烟依赖 Attack 站定日志与纯 Advance/Suppress 语义）。
        if ctx.stress && state == NpcState::Attack {
            let npc_g = world_to_grid(npc.position[0], npc.position[2]);
            if !crate::engine::ai::find_cover_points(ctx.grid, npc_g, COVER_MAX_DIST).is_empty() {
                tactic = Tactic::CoverSeek;
            }
        }
        npc.tactic = tactic;
        let (bx, bz) = (npc.position[0], npc.position[2]);
        advance_npc(
            npc,
            state,
            tactic,
            &target_pos,
            target_yaw,
            ctx.grid,
            ctx.ring_inner,
            ctx.ring_outer,
            ctx.obstacles,
            ctx.time,
            ctx.dt,
            ctx.stress,
            ctx.squad_wps.get(index).copied().flatten(),
        );
        // 朝向更新：移动时朝移动方向；站定时面向目标（渲染士兵模型用）
        let mdx = npc.position[0] - bx;
        let mdz = npc.position[2] - bz;
        if mdx * mdx + mdz * mdz > 1e-6 {
            npc.facing = mdz.atan2(mdx);
        } else {
            npc.facing = (npc.position[2] - target_pos.z).atan2(npc.position[0] - target_pos.x);
        }
        // 攻击态站定：打位置日志，冒烟 harness 读日志后从对跖点瞄准点射
        if state == NpcState::Attack && prev != NpcState::Attack {
            log::info!(
                "npc: #{} stand ({:.1}, {:.1}, {:.1})",
                npc.id,
                npc.position[0],
                npc.position[1],
                npc.position[2]
            );
        }
    }
    /// NPC 相互推离（空间哈希；1.6m 内斥力，位置微调，不影响路径规划下一帧重算）
    pub(crate) fn apply_npc_separation(npcs: &mut [Npc]) {
        if npcs.len() < 2 {
            return;
        }
        // 空间哈希：cell = 4m；键 = (gx, gy)
        let cell = 4.0f32;
        let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> =
            std::collections::HashMap::with_capacity(npcs.len() / 2 + 1);
        for (i, n) in npcs.iter().enumerate() {
            let key = ((n.position[0] / cell).floor() as i32, (n.position[2] / cell).floor() as i32);
            grid.entry(key).or_default().push(i);
        }
        let min_d = 1.6f32;
        let mut pushes = vec![[0.0f32, 0.0]; npcs.len()];
        for (_, bucket) in grid.iter() {
            for &i in bucket {
                let (xi, zi) = (npcs[i].position[0], npcs[i].position[2]);
                // 邻域 3x3 cell
                let (cx, cy) = ((xi / cell).floor() as i32, (zi / cell).floor() as i32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let key = (cx + dx, cy + dy);
                        if let Some(other) = grid.get(&key) {
                            for &j in other {
                                if j <= i {
                                    continue;
                                }
                                let dxp = npcs[j].position[0] - xi;
                                let dzp = npcs[j].position[2] - zi;
                                let d2 = dxp * dxp + dzp * dzp;
                                if d2 >= min_d * min_d {
                                    continue;
                                }
                                let d = d2.sqrt().max(1e-3);
                                let push = (min_d - d) * 0.5;
                                let ux = dxp / d;
                                let uz = dzp / d;
                                pushes[i][0] -= ux * push;
                                pushes[i][1] -= uz * push;
                                pushes[j][0] += ux * push;
                                pushes[j][1] += uz * push;
                            }
                        }
                    }
                }
            }
        }
        for (i, n) in npcs.iter_mut().enumerate() {
            let (px, pz) = (pushes[i][0], pushes[i][1]);
            if px != 0.0 || pz != 0.0 {
                // 归因埋点（#17）：分离力是**每帧一次的纯位置位移**（不乘 dt、不设上限），
                // 手感上"半步" = 0.08m（≈25fps 下一帧的行走位移 0.16m 的一半）。
                if ai_diag() {
                    let ord = std::sync::atomic::Ordering::Relaxed;
                    NOTE_SEP_PUSHED.fetch_add(1, ord);
                    if (px * px + pz * pz).sqrt() >= 0.08 {
                        NOTE_SEP_BIG.fetch_add(1, ord);
                    }
                }
                n.position[0] += px;
                n.position[2] += pz;
                n.position[1] = terrain_height_at(n.position[0], n.position[2]);
            }
        }
    }
    /// 串行推进全部 NPC（普通波次路径；与并行路径逐 NPC 行为一致）
    pub(crate) fn step_ai_serial(npcs: &mut [Npc], ctx: &AiStepCtx) {
        for (i, npc) in npcs.iter_mut().enumerate() {
            Self::step_npc(i, npc, ctx);
        }
    }
    /// 双池并行推进全部 NPC（线程优化第 2 步，2026-08-11）：
    /// 数组已由 `partition_ai_tiers` 稳定重排为 [Near..., Far...]，`near_len` 为分界。
    /// - 近组（Near）：延迟敏感 → `cpu::scene_pool()`（AMD CCD0 / Intel 仅 P-core），
    ///   调用线程参与首段，与主线程同簇通信延迟最低；
    /// - 远组（Far）：延迟不敏感重计算 → `cpu::ai_pool()`（AMD CCD1 / Intel E-core）。
    /// 各 NPC 更新彼此独立（目标/感知/路径均为本帧快照），并行与串行结果逐位一致。
    pub(crate) fn step_ai_parallel(npcs: &mut [Npc], near_len: usize, ctx: &AiStepCtx) {
        let (near, far) = npcs.split_at_mut(near_len);
        let near_pool = crate::engine::cpu::scene_pool();
        near_pool.par_for_each_mut(near, |_, start, slice| {
            for (k, npc) in slice.iter_mut().enumerate() {
                Self::step_npc(start + k, npc, ctx);
            }
        });
        let far_pool = crate::engine::cpu::ai_pool();
        far_pool.par_for_each_mut(far, |_, start, slice| {
            for (k, npc) in slice.iter_mut().enumerate() {
                // 远组降频（压力模式）：无感知/非交互远 NPC 按 id 分帧跳过，
                // 交互中（攻击/感知/受击/被瞄准）恒每帧步进。
                if ctx.decimate_far && should_decimate_far(npc, ctx.frame) {
                    continue;
                }
                Self::step_npc(near_len + start + k, npc, ctx);
            }
        });
    }
    /// 推进 NPC：感知 → 状态机 → 战术决策 → A* 路径 → 移动 → 地形高度
    ///
    /// 战术层（见 ai.rs）：
    /// - 角色分工：每波确定性分配 突击/包抄/压制/掩体跃进，左右包抄按 id 奇偶分工
    /// - 进攻协同：Chase/Attack 过半 → 同步冲锋（压制手除外，保持压制）
    /// - 躲避攻击：移动态受击/被火力威胁 → 侧向弹开（Attack 站定是冒烟瞄准依据，不躲）
    /// - 偷袭绕路：包抄手在玩家未面朝时绕大圈逼近，被发现转侧翼
    pub(crate) fn update_ai(&mut self, dt: f32, camera: &Camera) {
        let player = camera.position();
        // 2026-08-24：借用代替克隆（16KB/帧网格拷贝消除；与 self.npcs/map 字段级借用拆分共存）
        let grid = &self.grid;
        let time = self.time;
        let player_yaw = camera.yaw;
        // 分层调度（2026-08-11）：稳定重排 npcs 为 [Near..., Far...]，
        // 返回 Near 段长度。Near = 与玩家实时交互/近距离（延迟敏感，走 scene_pool
        // = P 核/CCD0），Far = 远距离重计算（走 ai_pool = CCD1/E 核）。
        // 重排后 under_fire / targets 均在当前数组顺序上构建，帧内索引对齐；
        // 各 NPC 步进彼此独立（AiStepCtx 只读），重排不改变步进语义。
        let stress = self.stress;
        // 观战模式（玩家无敌）：NPC 目标兜底 = 敌方重心（红/蓝互搏，火力不浪费在无敌玩家上）
        // 计时埋点（RV3D_AI_PROF=1，第 47 轮）：`update_ai` 顶部三处**每帧固定 O(n) 工作**。
        // 动机：第 46 轮实测"并行池 vs 串行"在 255 NPC 下**完全无法区分**
        // （ai_us 1460 vs 1391、fps 132.8 vs 132.7）—— 若开销在逐 NPC 步进上，
        // 并行应当明显更快。既然没有，说明 ai_us 被与"是否并行"无关的每帧工作主导。
        let t_a = std::time::Instant::now();
        let (rc, bc) = team_centroids(&self.npcs);
        let t_b = std::time::Instant::now();
        let fallback_targets: Vec<[f32; 3]> = if self.player_invincible {
            self.npcs
                .iter()
                .map(|n| match n.team {
                    Team::Red => [bc[0], 0.0, bc[1]],
                    Team::Blue => [rc[0], 0.0, rc[1]],
                })
                .collect()
        } else {
            Vec::new()
        };
        let t_c = std::time::Instant::now();
        let tier_params = AiTierParams::default();
        let near_len = partition_ai_tiers(&mut self.npcs, |npc| {
            ai_tier_of(npc, &player, stress, &tier_params)
        });
        let t_d = std::time::Instant::now();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_AI_PROF").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 119 == 0
            {
                log::info!(
                    "aiprof: 重心={}us 兜底目标={}us 分层重排={}us 三处合计={}us",
                    t_b.duration_since(t_a).as_micros(),
                    t_c.duration_since(t_b).as_micros(),
                    t_d.duration_since(t_c).as_micros(),
                    t_d.duration_since(t_a).as_micros()
                );
            }
        }
        // 同步冲锋判定：本帧开始时 Chase/Attack 数量过半 → 全队突进
        let active = self
            .npcs
            .iter()
            .filter(|n| {
                matches!(
                    n.state_machine.state(),
                    NpcState::Chase | NpcState::Attack
                )
            })
            .count() as u32;
        let charge = should_charge(active, self.npcs.len() as u32, self.charge_active);
        self.charge_active = charge;
        // 指挥节拍：0.5s 营司令评估 + 战士班目标点（未接敌编队推进，接敌由逐人战术接管）
        if self.stress {
            if let Some(cmd) = self.command.as_mut() {
                // rc/bc 复用 update_ai 顶部已算的敌我重心（不再重复 O(n) 扫描）
                // LLM 指挥官：红蓝各一独立上下文窗口互搏（无/失效 → 该侧启发式）
                let to_ov = |cmds: Vec<crate::llm_cmd::CompanyCmd>| {
                    cmds.into_iter()
                        .map(|c| crate::engine::ai_command::CmdOverride {
                            order: match c.order {
                                crate::llm_cmd::LlmOrder::Assault => {
                                    crate::engine::ai_command::CompanyOrder::Assault
                                }
                                crate::llm_cmd::LlmOrder::Hold => {
                                    crate::engine::ai_command::CompanyOrder::Hold
                                }
                                crate::llm_cmd::LlmOrder::FlankL => {
                                    crate::engine::ai_command::CompanyOrder::Flank(1)
                                }
                                crate::llm_cmd::LlmOrder::FlankR => {
                                    crate::engine::ai_command::CompanyOrder::Flank(-1)
                                }
                                crate::llm_cmd::LlmOrder::Regroup => {
                                    crate::engine::ai_command::CompanyOrder::Regroup
                                }
                            },
                            x: c.x,
                            z: c.z,
                        })
                        .collect::<Vec<_>>()
                };
                let llm_red: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_red()).map(to_ov);
                let llm_blue: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_blue()).map(to_ov);
                // 军情两个口径：本营阵亡（round_kills_<自己>）与战果（= 敌方阵亡）。
                // 🔴 重组判据读的是**战果** —— 喂本营阵亡会让那条分支永远不触发（未结案 27）。
                cmd.0.update(
                    &self.npcs,
                    &grid,
                    dt,
                    self.round_kills_red,
                    self.round_kills_blue,
                    bc,
                    llm_red.as_deref(),
                );
                cmd.1.update(
                    &self.npcs,
                    &grid,
                    dt,
                    self.round_kills_blue,
                    self.round_kills_red,
                    rc,
                    llm_blue.as_deref(),
                );
                // 态势推送（红/蓝各独立上下文）
                if let Some(l) = &self.llm {
                    let sr = build_llm_situation(&cmd.0);
                    let sb = build_llm_situation(&cmd.1);
                    l.push_red(&sr, cmd.0.companies.len());
                    l.push_blue(&sb, cmd.1.companies.len());
                }
                if self.time - self.command_log_at >= 5.0 {
                    self.command_log_at = self.time;
                    log::info!("command: 红{} | 蓝{}", cmd.0.summary(), cmd.1.summary());
                }
            }
        }
        // 计时：以下三段都在**并行分派之前**、每帧做超线性扫描，
        // 正是"并行≈串行 + 中位 1178/最大 10677 尖峰"的形态（第 49 轮）。
        let t_pre = std::time::Instant::now();
        let squad_wps: Vec<Option<[f32; 2]>> = if self.stress {
            if let Some(cmd) = self.command.as_ref() {
                self.npcs.iter().map(|n| {
                    let army = match n.team { Team::Red => &cmd.0, Team::Blue => &cmd.1 };
                    army.squad_waypoint(n.id)
                }).collect()
            } else { vec![None; self.npcs.len()] }
        } else { vec![None; self.npcs.len()] };
        // 弹道威胁预扫：存活子弹水平距离 < THREAT_RADIUS 且朝 NPC 方向飞行 → 该 NPC 受火力威胁
        let t_uf = std::time::Instant::now();
        let under_fire = {
            let mut flags = vec![false; self.npcs.len()];
            for p in &self.projectiles {
                if !p.is_alive() {
                    continue;
                }
                let v = p.velocity();
                for (i, npc) in self.npcs.iter().enumerate() {
                    if flags[i] {
                        continue;
                    }
                    let dx = npc.position[0] - p.position[0];
                    let dz = npc.position[2] - p.position[2];
                    if dx * dx + dz * dz > THREAT_RADIUS * THREAT_RADIUS {
                        continue;
                    }
                    if dx * v[0] + dz * v[2] > 0.0 {
                        flags[i] = true;
                    }
                }
            }
            flags
        };
        // ⚠️ 上一轮 `t_occ` 插错了位置（落在 `target_occlusion` 之后），导致"威胁预扫"
        // 这个标签把下面三段都算了进去。现在按真实区间拆成三个计时点。
        let t_pick0 = std::time::Instant::now();
        // 压力模式：每 NPC 预选最近敌对目标（敌对 NPC 优先、玩家兜底；O(n²) 纯读，串行）
        let targets: Vec<Option<(usize, [f32; 3], f32)>> = if self.stress {
            pick_stress_targets(&self.npcs, STRESS_SIGHT)
        } else {
            Vec::new()
        };
        // 掩体利用评估用的当前关卡障碍环带（theme 随关卡轮换）
        let theme = theme_for_level(self.level);
        // 视线遮挡预计算：必须在 ctx 之前算完（返回 owned Vec），否则会与
        // step_ai_* 对 self.npcs 的可变借用冲突。resolve_ai_target 在 stress=false 时
        // 忽略 targets、spectator=false 时忽略 fallback_targets，所以一条调用通吃三种模式。
        let t_pick = std::time::Instant::now();
        // 🔴 视线遮挡**每 N 帧重算一次**（2026-09-12 第 102 轮）。
        //
        // 第 50 轮实测：本节占 `ai_us` 的 **18–39%（244–519µs）**，成本是
        // **每帧为全部 NPC** 做线段-AABB 扫描（255 × 2 采样 × 约 1100 刚体 ≈ **56 万次/帧**）。
        //
        // 而遮挡关系在相邻帧之间几乎不变，**AI 的反应时间在 100–300ms 量级** ⇒
        // 3 帧（约 23ms @130fps）的陈旧**完全在容差内**。
        //
        // ⚠️ A/B 开关（第 103 轮加）：`RV3D_OCCL_REFRESH=1` 回到"每帧重算"的旧行为。
        // 每次 `update_ai` 只读一次环境变量（**每帧 1 次，不是每实例**，见教训 32）⇒ 不必 OnceLock。默认 4。
        let occlusion_refresh: u32 = std::env::var("RV3D_OCCL_REFRESH")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(4);
        let recompute = self.occl_cache_age == 0 || self.occl_cache.len() != self.npcs.len();
        let target_occluded: Vec<bool> = if recompute {
            let v = target_occlusion(
                &self.npcs,
                &self.world.bodies,
                self.stress,
                self.player_invincible,
                &targets,
                &fallback_targets,
                &player,
            );
            self.occl_cache = v.clone();
            self.occl_cache_age = occlusion_refresh;
            v
        } else {
            self.occl_cache_age -= 1;
            self.occl_cache.clone()
        };
        let t_occ = std::time::Instant::now();
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_AI_PROF").is_ok()
                && TICK.fetch_add(1, Ordering::Relaxed) % 119 == 0
            {
                log::info!(
                    "aiprof2: 班目标点={}us 威胁预扫={}us 目标选择={}us 视线遮挡={}us 四段合计={}us（子弹 {} / NPC {}）",
                    t_uf.duration_since(t_pre).as_micros(),
                    t_pick0.duration_since(t_uf).as_micros(),
                    t_pick.duration_since(t_pick0).as_micros(),
                    t_occ.duration_since(t_pick).as_micros(),
                    t_occ.duration_since(t_pre).as_micros(),
                    self.projectiles.iter().filter(|p| p.is_alive()).count(),
                    self.npcs.len()
                );
            }
        }
        {
            let ctx = AiStepCtx {
                player: &player,
                player_yaw,
                charge,
                under_fire: &under_fire,
                targets: &targets,
                grid: &grid,
                time,
                dt,
                stress: self.stress,
                frame: self.frame_no,
                decimate_far: self.stress
                    && std::env::var("RV3D_AI_DECIMATE")
                        .map_or(true, |v| v != "off" && v != "0"),
                ring_inner: theme.ring_inner,
                ring_outer: theme.ring_outer,
                obstacles: &self.map.obstacles,
                squad_wps: &squad_wps,
                spectator: self.player_invincible,
                // 只有在"真在打的一局"里才把玩家当作已知目标：
                // 开始菜单的游走（StartMenu 也调 update_ai）必须保持"没看见就随便走"的观感，
                // 压力模式有自己的一套选目标逻辑（target_known 会让红蓝绕过 pick_stress_targets）。
                target_known: self.game_state == GameState::Playing && !self.stress,
                fallback_targets: &fallback_targets,
                target_occluded: &target_occluded,
            };
            if self.npcs.len() >= PARALLEL_AI_MIN && self.ai_parallel {
                // safety: 已分层（Near 在前），双池分片互不相交；ctx 只含共享只读数据
                Self::step_ai_parallel(&mut self.npcs, near_len, &ctx);
            } else {
                Self::step_ai_serial(&mut self.npcs, &ctx);
            }
            // 分离力（2026-08-23 三三制防重叠）：空间哈希 4m 单元，1.6m 内施加推离，
            // 消除阵型/搜索点的人叠人（楔形队形+残局集结处生效）
            Self::apply_npc_separation(&mut self.npcs);
        }
        // NPC 投掷手榴弹：压力模式 / survive 防守波次中，交火 NPC 低概率（5-8%）投掷
        // （仅对敌对目标方向；阵营区分不炸友军）。普通波次（打玩家）不投掷 → 行为零回归。
        if self.stress || self.is_survive_rule() {
            self.npc_throw_grenades(dt, &targets);
        }
        // 压力模式：攻击态 NPC 对目标 NPC 结算伤害（每满 1 秒 dps；玩家无敌旁观）
        if self.stress {
            self.apply_npc_combat(dt, &targets);
        }
        // 压力模式：移除阵亡 NPC；任一阵营团灭 → 全量补员开新一轮
        if self.stress {
            self.update_stress_respawns(&player);
        }
        if self.time - self.ai_log_time >= 1.0 {
            self.ai_log_time = self.time;
            let mut counts = [0u32; 4];
            let mut tactics = [0u32; 8];
            for npc in &self.npcs {
                counts[npc.state_machine.state() as usize] += 1;
                tactics[npc.tactic as usize] += 1;
            }
            log::info!(
                "ai: npcs={} near={} far={} idle={} patrol={} chase={} attack={} tactics={:?} red={} blue={} ra={} ba={} rc=({:.0},{:.0}) bc=({:.0},{:.0})",
                self.npcs.len(),
                near_len,
                self.npcs.len() - near_len,
                counts[0],
                counts[1],
                counts[2],
                counts[3],
                tactics,
                self.npcs.iter().filter(|n| n.team == Team::Red).count(),
                self.npcs.iter().filter(|n| n.team == Team::Blue).count(),
                self.npcs
                    .iter()
                    .filter(|n| n.team == Team::Red && n.state_machine.state() == NpcState::Attack)
                    .count(),
                self.npcs
                    .iter()
                    .filter(|n| n.team == Team::Blue && n.state_machine.state() == NpcState::Attack)
                    .count(),
                bc[0], bc[1],
                rc[0], rc[1],
            );
        }
        // 攻击态 NPC 对玩家造成伤害（1 秒一次），驱动 HUD 血条
        // 伤害值取当前有效波次的 dps（Boss 波更高，见 wave_profile）
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        if !self.stress && !self.player_invincible
            && self.time - self.last_damage_time >= 1.0
            && self.game_state == GameState::Playing
            && self
                .npcs
                .iter()
                .any(|n| n.state_machine.state() == NpcState::Attack)
            && self.hud.health > 0.0
        {
            self.hud.health = (self.hud.health - dps).max(0.0);
            self.last_damage_time = self.time;
            if self.hud.health <= 0.0 {
                // 击杀提示：玩家被敌方击杀
                self.hud.push_kill("你被击杀了".to_string());
                // survive 规则：玩家死亡即失败（Defeat 结算）；否则普通 GameOver
                if self.is_survive_rule() {
                    if let Some(obj) = self.obj_state.as_mut() {
                        obj.won_team = Some(crate::engine::ai::Team::Red);
                    }
                    self.game_state = GameState::Defeat;
                    log::info!(
                        "survive: 玩家阵亡于第 {} 波 → 失败",
                        self.wave
                    );
                } else {
                    self.game_state = GameState::GameOver;
                    log::info!(
                        "game: player down, score={} wave={} (GameOver: gameplay frozen, projectiles coast without kills)",
                        self.score,
                        self.wave
                    );
                }
                // 死亡补给：全部武器弹匣补满 + 备弹恢复初始
                self.weapons.reset_all_ammo();
            }
        }
    }
    /// 压力模式 NPC 互射：攻击态且目标在攻击距离内 → 每满 1 秒对目标结算 dps。
    /// 目标索引在帧内有效（互射结算后统一移除，不在中途删）。友军永远不被伤害。
    pub(crate) fn apply_npc_combat(&mut self, dt: f32, targets: &[Option<(usize, [f32; 3], f32)>]) {
        if self.npcs.len() < 2 {
            return;
        }
        let dps = wave_profile(self.effective_wave(self.wave)).dps;
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (i, npc) in self.npcs.iter().enumerate() {
            if npc.state_machine.state() != NpcState::Attack {
                continue;
            }
            if let Some(Some((t, _, _))) = targets.get(i) {
                if *t >= self.npcs.len() {
                    continue;
                }
                let dx = self.npcs[*t].position[0] - npc.position[0];
                let dz = self.npcs[*t].position[2] - npc.position[2];
                // 伤害距离与感知对齐（2026-08-23：压力模式 2.5 倍攻击距离，
                // 否则全员感知 30m 内站定但无法开火 → 僵持观望）
                let rng = npc.attack_range * if self.stress { 2.5 } else { 1.0 };
                if dx * dx + dz * dz <= rng * rng {
                    pairs.push((i, *t));
                }
            }
        }
        for (i, t) in pairs {
            self.npcs[i].fire_accum += dt;
            if self.npcs[i].fire_accum >= 1.0 {
                self.npcs[i].fire_accum = 0.0;
                self.npcs[t].hp -= dps;
                // 击杀提示：NPC 互射击杀（击杀者阵营 + id，与玩家击杀同一套拼装）
                if self.npcs[t].hp <= 0.0 && self.npcs[t].hp > -dps {
                    let (aid, a, v) = (self.npcs[i].id, self.npcs[i].team, self.npcs[t].team);
                    let vid = self.npcs[t].id;
                    let killer = KillerLabel::Named(format!("{} #{aid}", team_name(a)));
                    let victim = format!("{} #{vid}", team_name(v));
                    self.hud.push_kill(kill_line(killer, &victim, ""));
                }
            }
        }
    }
}
