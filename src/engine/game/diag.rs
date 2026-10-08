// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

/// `RV3D_NPC_POS=1`：每秒给**每只** NPC 打一行机器可读位置 `npcpos: #id x y z state`，
/// 供注入 harness 跟踪**活靶**（`npc: #N stand` 只在进 Attack 那一刻打一次，移动靶全程打空）。
/// 与 `RV3D_AI_DIAG` 分开：harness 只要位置，不需要那一堆 AI 归因统计。
pub(crate) fn npc_pos_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_NPC_POS").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}
/// 换弹动作包络（纯函数，可单测）：`p` = 换弹进度 0→1（刚按下 → 完成），返回 `[0, 1]`，
/// **两端位移为 0、中点为 1**。
///
/// 与切枪用同一套 `sin(π·p)`：**两端取 0** ⇒ 枪不会在换弹开始/结束那一帧"啪"地跳一下
/// （本仓枪模历史上的残影/抖动都出自位置跳变）。
/// ⚠️ 这条包络**两端速度最大**（导数 = π·cos(πp)，在 p=0/1 处为 ±π），所以它是
/// "很快沉下去、再抬回来"，而不是缓入缓出 —— 想要缓入缓出得用 `sin²` 或 smoothstep，
/// 但那会让中段变慢、看起来像卡顿。**别把这条写成"速度也连续"。**
/// `main.rs` 传的是 `1 - hud.reload_progress`（该进度是 1→0 递减的）。
pub(crate) fn reload_envelope(p: f32) -> f32 {
    (std::f32::consts::PI * p.clamp(0.0, 1.0)).sin()
}
/// `RV3D_CULL_DIAG=1`：把「NPC 遮挡剔除」的 CPU 成本按秒量出来（默认关；关着时每个调用
/// 只多一次已缓存的 bool 比较）。
///
/// **为什么要这把尺子**：`npc_occluded` 每帧对**每个** NPC 做 2 条线段 × `world.bodies`
/// 的 AABB 扫描（城市图 1240 个障碍）⇒ 255 人一帧约 63 万次相交测试，而 `main.rs` 里
/// 它被**调两遍**（上屏列表 + 枪口焰筛选）⇒ 一帧约 126 万次。这个数量级**不许靠推理**：
/// 先量出来（教训 20 / 25），再决定要不要缓存或换宽相。
pub(crate) fn cull_diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("RV3D_CULL_DIAG").is_ok_and(|v| v == "1" || v == "on" || v == "true")
    })
}
/// `npcpos:` 的发送周期（秒）—— 纯函数，可单测。
///
/// `RV3D_NPC_POS_HZ`（默认 1）决定频率，夹在 `[1, 30]`：小于 1 没意义，大于 30 只会刷屏。
/// 🔴 2026-09-25 加：注入 harness 打移动靶时，位置样本的**年龄**直接决定命中率 ——
/// 1 Hz 意味着它可能瞄一个 1 秒前的位置（NPC 4–5 m/s ⇒ 差出好几米）。
/// 这条旋钮让 harness 能按需取到 ~10 Hz 的新鲜位置，而默认仍是原来的 1 Hz（不改变既有日志量）。
pub(crate) fn npc_pos_period(hz_env: Option<u32>) -> f32 {
    let hz = hz_env.unwrap_or(1).clamp(1, 30);
    1.0 / hz as f32
}
/// `RV3D_NPC_POS_HZ`（只读一次；非法值视为缺省 = 1 Hz）。
pub(crate) fn npc_pos_hz() -> Option<u32> {
    static HZ: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *HZ.get_or_init(|| std::env::var("RV3D_NPC_POS_HZ").ok().and_then(|v| v.parse::<u32>().ok()))
}
/// 诊断通道的刷新周期（秒）：`RV3D_NPC_POS=1` 时按 `RV3D_NPC_POS_HZ`，否则保持 1 Hz。
///
/// 🔴 2026-09-25：`main.rs` 的 `cam:` 行（yaw/pitch）**也**要用它。这是当天最重要的一个发现：
/// 注入 harness 的瞄准环是拿 `cam:` 行做**回读**的（注入一像素 → 读回当前角度 → 再算误差），
/// 而 `cam:` 原本 1 Hz，瞄准环每轮只 sleep 0.5s ⇒ **常常读到同一行**（旧角度）⇒ 把同一个
/// 修正量**再注入一次** ⇒ 过冲 / 假装收敛。弹道埋点正是这么露的马脚：过期弹 117/118
/// 差最近的人 **2m 以上**（12m 交火距离上那是 ~10° 的偏差，只可能是回读失灵）。
/// 判据：同频后 `RV3D_PROJ_DIAG` 的 `>2m` 桶必须显著下降（PROGRESS §21.22）。
pub(crate) fn diagnostic_period_secs() -> f32 {
    if npc_pos_log() {
        npc_pos_period(npc_pos_hz())
    } else {
        1.0
    }
}
/// `RV3D_PROJ_DIAG=1`：弹道诊断通道（弹丸去向 / 过期分桶 / 每枪瞄得准不准）。
pub(crate) fn proj_diag_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV3D_PROJ_DIAG").as_deref() == Ok("1"))
}
/// feed 里一行"某人击杀某人"的文本（`cause` 是武器/爆炸说明，可空）。
/// **三处调用点（`damage_npc` / NPC 互射 / 联机）共用它** ——
/// 免得三处各写一遍格式、越改越不一致（此前正是三处各写一遍）。
pub(crate) fn kill_line(killer: KillerLabel, victim: &str, cause: &str) -> String {
    match killer {
        KillerLabel::You => format!("你击杀了{victim}{cause}"),
        KillerLabel::Named(name) => format!("{name} 击杀了{victim}{cause}"),
    }
}
/// 爆炸击杀单独成句：**不冒充有击杀者**。
pub(crate) fn blast_kill_line(center: [f32; 3], victim: &str) -> String {
    format!("爆炸（{:.0},{:.0}）击杀了{victim}", center[0], center[2])
}
/// 红蓝阵营存活 NPC 的平均 x/z（阵营为空 → [0.0, 0.0]；命令行军/军情用）
/// 生成红营态势 JSON（LLM 指挥官输入；严格字段：兵力/重心/接敌/当前命令）
pub(crate) fn build_llm_situation(a: &crate::engine::ai_command::Army) -> String {
    let side = match a.side {
        Team::Red => "red",
        Team::Blue => "blue",
    };
    let mut s = format!(
        "{{\"battle\":\"128v128\",\"side\":\"{side}\",\"map_half\":270,\"enemy\":{{\"x\":{:.0},\"z\":{:.0}}},\"companies\":[",
        a.enemy_centroid[0], a.enemy_centroid[1]
    );
    for (i, c) in a.companies.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let cur = match c.order {
            crate::engine::ai_command::CompanyOrder::Assault => "Assault",
            crate::engine::ai_command::CompanyOrder::Hold => "Hold",
            crate::engine::ai_command::CompanyOrder::Flank(1) => "FlankL",
            crate::engine::ai_command::CompanyOrder::Flank(_) => "FlankR",
            crate::engine::ai_command::CompanyOrder::Regroup => "Regroup",
        };
        s.push_str(&format!(
            "{{\"id\":{i},\"strength\":{:.0},\"x\":{:.0},\"z\":{:.0},\"contact\":{},\"current\":\"{cur}\"}}",
            c.report.strength, c.report.centroid[0], c.report.centroid[1], c.report.contact
        ));
    }
    s.push_str("]}");
    s
}
pub(crate) fn team_centroids(npcs: &[Npc]) -> ([f32; 2], [f32; 2]) {
    let mut rc = [0.0f32; 2];
    let mut bc = [0.0f32; 2];
    let mut rn = 0usize;
    let mut bn = 0usize;
    for n in npcs {
        match n.team {
            Team::Red => { rc[0] += n.position[0]; rc[1] += n.position[2]; rn += 1; }
            Team::Blue => { bc[0] += n.position[0]; bc[1] += n.position[2]; bn += 1; }
        }
    }
    let avg = |c: [f32; 2], n: usize| -> [f32; 2] {
        if n > 0 { [c[0] / n as f32, c[1] / n as f32] } else { [0.0, 0.0] }
    };
    (avg(rc, rn), avg(bc, bn))
}

// 子模块（见 docs/refactor-plan.md）
