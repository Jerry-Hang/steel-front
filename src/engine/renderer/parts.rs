// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 计算一名 NPC 的 15 段人形士兵实例数据（大腿/小腿/脚/骨盆/胸/颈/头/上臂/前臂/枪）。
    /// 比例贴近真人：总高约 1.78m，肩宽 ~0.55m，腿/臂分上下两段，行走时髋/膝/肩/肘
    /// 各自绕枢轴摆动（不再是积木式整体摆动）。全部同 tint。
    /// 每段矩阵 = T(pos) * R_y(yaw) * T(枢轴) * R_anim * T(段心) * S(尺寸)：
    /// 动画旋转在枢轴平移之后、段心平移之前，绕枢轴（髋/膝/肩/肘）旋转。
    /// 枪局部偏移在 +Z（yaw=0 时枪口朝向 +Z），随 yaw 绕 y 轴旋转。
    /// 动画：moving 时髋/膝/肩/肘按 phase 正弦对向摆动（走路步态）；
    /// firing 时枪沿 -Z 后坐脉冲（高频正弦），胸微俯。
    /// 返回 (盒体组, 圆柱组, 球体组) 三段实例：躯干/脚/枪为盒体，
    /// 四肢为圆柱（半径 = 盒宽/厚的一半），头为球体 —— 真人比例、非方块人。
    pub(crate) fn soldier_part_matrices(
        pos: [f32; 3],
        yaw: f32,
        tint: [f32; 4],
        phase: f32,
        moving: bool,
        firing: bool,
    ) -> (Vec<InstanceData>, Vec<InstanceData>, Vec<InstanceData>) {
        // (枢轴, 缩放, 段心相对枢轴偏移, 动画类型, 几何: 0盒 1圆柱 2球)
        // 动画类型：0 无 1 左大腿 2 右大腿 3 左小腿 4 右小腿 5 左上臂 6 右上臂
        //          7 左前臂 8 右前臂 9 枪(后坐) 10 胸(前俯)
        // 人形比例（总高 ~1.79m）：头小、躯干桶形（圆柱）、四肢粗细适中、有脚。
        // 圆柱 scale = (半径, 高, 半径)；盒 scale = (宽, 高, 厚)。
        // 段数预算：255 人 × 每组 3072 ⇒ **每组每人最多 12 段**。本表 = 9 盒 + 8 圆柱
        // （2295 / 2040，分别占 75% / 66%），都留了余量。加段之前先复核这个预算。
        // ── 持枪姿态角（2026-09-12 第④条修）：负角 = 肢体向 +Z（面朝方向）抬起。
        //
        // 旧版两条手臂只有走路摆动（kind 5~8 绕 X 随 `stride` 摆），而枪固定在身前
        // `(x=+0.16, z=+0.36, y=1.18)` ⇒ **枪悬在胸前、手臂垂在身侧，两者永不相交**。
        // 实机放大图（`screenshots/soldier_zoom.png`）里读作"左肩伸出一根悬空的横条"、
        // 且"看不见手臂" —— 这是用户说的"神人样子"的主要来源。
        //
        // 改成持枪后**手臂不再随步伐摆动**：真人端枪行进时本来就不摆臂，比摆动更真。
        // 只改姿态角、**不加段数**，实例预算（9 盒 + 8 圆柱）不变。
        const HOLD_R_UPPER: f32 = -1.00; // 右上臂前抬 ~57°
        const HOLD_R_FORE: f32 = -0.85;  // 右前臂再前抬 ~49° ⇒ 手到握把高度
        const HOLD_L_UPPER: f32 = -1.15; // 左上臂前抬 ~66°（托护木，比右手更前）
        const HOLD_L_FORE: f32 = -0.70;  // 左前臂 ~40°
        // 段尾的 f32 = **逐段明暗系数**（2026-09-12 第④条修）。
        //
        // 引擎对 NPC 走 `flat_flag=2` 纯色路径，`tint` **就是**外观色，顶点色是白化的。
        // 因此 17 段共用一个 tint ⇒ 整个人是一个**均匀饱和色块**，
        // 平着色下没有任何结构可读（实机放大图上就是一团橙色塑料）——
        // 这是"神人样子"的第二大来源（第一大是枪悬空，已修）。
        //
        // 真人身上的装备本来就有明显的**明度层次**：盔最暗、背心次之、作训服中、靴最暗。
        // 这里给每段一个亮度系数，**不加段数、不加 draw call**（还是同一批实例）。
        let parts: [([f32; 3], [f32; 3], [f32; 3], u8, u8); 18] = [
            // ── 四肢：保持圆柱（转动最自然），但**加粗到真人尺寸**。旧值大腿 φ0.13 /
            //    小腿 φ0.10 / 前臂 φ0.084 —— 比真人细一倍，远看只剩躯干那根柱子，
            //    这就是"远距离读作蓝色平板"的来源。
            ([-0.10, 0.84, 0.0], [0.085, 0.38, 0.085], [0.0, -0.19, 0.0], 1, 1), // 左大腿（髋）圆柱
            ([0.10, 0.84, 0.0], [0.085, 0.38, 0.085], [0.0, -0.19, 0.0], 2, 1),  // 右大腿（髋）圆柱
            ([-0.095, 0.46, 0.0], [0.062, 0.36, 0.062], [0.0, -0.18, 0.0], 3, 1), // 左小腿（膝）圆柱
            ([0.095, 0.46, 0.0], [0.062, 0.36, 0.062], [0.0, -0.18, 0.0], 4, 1),  // 右小腿（膝）圆柱
            ([-0.09, 0.10, 0.0], [0.11, 0.10, 0.26], [0.0, -0.05, 0.02], 0, 0),   // 左脚（盒，抬到踝高）
            ([0.09, 0.10, 0.0], [0.11, 0.10, 0.26], [0.0, -0.05, 0.02], 0, 0),    // 右脚（盒）
            // ── 躯干：**扁箱**而不是圆柱。真人胸廓宽 > 深（0.36 × 0.24），圆柱躯干
            //    在剪影上就是一根柱子；而且旧胸半径 0.20 < 上臂挂点 0.28，
            //    手臂是**悬空挂在躯干外面**的。
            ([0.0, 0.96, 0.0], [0.32, 0.20, 0.24], [0.0, -0.10, 0.0], 0, 0),      // 骨盆（扁箱）
            // 🔴 2026-09-12 第④条收窄（第 122 轮）：实宽由 0.72 收到 **0.48**。
            //    依据：`tools/measure_silhouette.py` 量出躯干**投影**宽 1.12m；
            //    扣除投影角度（宽 0.78/深 0.58、约 37 度 ⇒ 最大 0.97）与透视放大（1.07）
            //    后，模型实宽约 0.78m —— 真人含护甲约 0.52m ⇒ **宽约 1.5 倍**。
            //    （第 120 轮误记为"2.2~2.5 倍"，那是把投影宽当成了模型宽。）
            ([0.0, 1.26, 0.0], [0.36, 0.46, 0.24], [0.0, -0.01, -0.01], 10, 0),   // 胸廓（扁箱，含前俯）
            // 🔴 2026-09-12 第④条修：背心/头的 y 区间原本**重叠 0.13m**
            //    （背心 1.23~1.53 vs 头 1.40~1.64）⇒ **头有 43% 埋在背心里**，
            //    平着色下读作"一整块大方块"，这就是"神人样子"最直接的一条。
            //    按真人重排：背心 1.20~1.50（pivot 1.35），头 1.50~1.74（pivot 1.62），
            //    头盔 1.645~1.795（pivot 1.72）⇒ 总高 1.795，且**三段首尾相接不重叠**。
            ([0.0, 1.35, 0.0], [0.39, 0.30, 0.29], [0.0, 0.0, 0.0], 10, 0),       // 防弹背心（套在胸外）
            // ── 头：方块 + 头盔两段。旧版是一个 φ0.30 的**球**，没有下颌、没有头盔、
            //    没有朝向；平着色下一个球就是一块均匀色斑。
            // 🔴🔴 2026-09-12 第 136 轮**回退第 133 轮的收小**：同上是"半宽"外推的产物。
            //    盒 scale 是全尺寸 ⇒ 头 0.17x0.24x0.20、盔 0.205x0.15x0.235 **本来就是真人尺寸**
            //    （真人头约 0.16x0.24x0.20、盔约 0.22x0.15x0.28）。第 133 轮把它们改小了一半，是错的。
            ([0.0, 1.62, 0.0], [0.17, 0.24, 0.20], [0.0, 0.0, 0.0], 0, 0),        // 头（含下颌，方块）
            ([0.0, 1.72, 0.0], [0.205, 0.15, 0.235], [0.0, 0.0, 0.0], 0, 0),      // 头盔壳
            // ── 手臂：圆柱加粗，并把挂点从 ±0.28 收到 ±0.235 —— 贴着胸廓外侧
            ([-0.235, 1.38, 0.02], [0.068, 0.26, 0.068], [0.0, -0.13, 0.0], 5, 1), // 左上臂（肩）圆柱
            ([0.235, 1.38, 0.02], [0.068, 0.26, 0.068], [0.0, -0.13, 0.0], 6, 1),  // 右上臂（肩）圆柱
            ([-0.235, 1.10, 0.02], [0.058, 0.24, 0.058], [0.0, -0.12, 0.02], 7, 1), // 左前臂（肘）圆柱
            ([0.235, 1.10, 0.02], [0.058, 0.24, 0.058], [0.0, -0.12, 0.02], 8, 1),  // 右前臂（肘）圆柱
            // ── 武器：枪身 + 枪托两段（旧版是一个 0.26×0.10×0.95 的**纯方块**）
            // 🔴 2026-09-12 第 137 轮回退第 132 轮：scale[2] 0.47 -> 0.62（原值）。上一条注释已写明旧版是 0.26x0.10x0.95 的整块 => 0.95m 正是 AK-12 长度，枪身0.62+枪托0.24 就是拆成两段的结果 => 原值本来就对。第132轮按错误的"半宽"前提把0.62读成1.24m才去缩短；第136轮定案盒scale是全尺寸。
            //    （真 AK-12 全枪约 0.94 m）。原值在侧视投影里是一根 1.24m 的"水平细长横杆"，
            //    我连续三轮（114/129/130）把它误认成手臂 —— 直到第 131 轮"改上臂角 26° 它却不动"才定案。
            ([0.16, 1.18, 0.36], [0.07, 0.10, 0.62], [0.0, 0.0, 0.0], 9, 0),      // 枪身
            ([0.16, 1.14, -0.04], [0.06, 0.13, 0.24], [0.0, 0.0, 0.0], 9, 0),     // 枪托
            // ── 背包（2026-09-12 第④条加）：盒体第 10 段。盒预算 9→10，
            //    10×255 = 2550 / 3072 = 83%，**仍留 17% 余量**（上限 12 段 = 3060/3072）。
            //    纯侧影改造：平着色下"背上有东西"是区分士兵与方柱最省的一段几何。
            // 🔴 2026-09-12 修正：初版做成 0.30 x **0.42** x 0.16 —— 从背后看**整个背面被它盖住**，
            //    实机特写（`screenshots/soldier_zoom5.png`）里读作"一个大方块躯干"，
            //    反而比不加更糟。真人背包约 0.30 宽 x 0.40 高但**贴身**（深 0.16），
            //    关键是它不该高过肩胛 —— 收到 0.26 x 0.30，读作"背上有东西"即可，不抢主体。
            ([0.0, 1.28, -0.20], [0.26, 0.30, 0.14], [0.0, 0.0, 0.0], 10, 0),     // 背包（跟着胸俯仰）
        ];
        // 逐段明暗系数（顺序严格对应上面的 17 段）：盔最暗、靴最暗、枪近黑、背心最亮。
        // 真人装备本来就有明显明度层次，平着色下这是**唯一**能读出结构的手段。
        let shade: [f32; 18] = [
            0.72, 0.72, // 左/右大腿
            0.66, 0.66, // 左/右小腿
            0.42, 0.42, // 左/右脚（靴，最暗）
            0.80, // 骨盆
            0.95, // 胸廓
            1.00, // 防弹背心（装备主体，最亮）
            // 🔴 2026-09-12 修：这两个系数原本是**反的**（头 0.70 亮于盔 0.55）
            //    ⇒ 平着色下"脸比盔亮"，读作一块均匀方块，**头没有正面**。
            //    真人脸上有盔影 + 护目镜，是全身最暗的一块；盔是受光面，最亮之一。
            //    对调之后头才第一次有了"朝向"。
            0.42, // 头/脸（暗：盔影 + 护目镜）
            0.78, // 头盔壳（亮：受光面）
            0.78, 0.78, // 左/右上臂
            0.74, 0.74, // 左/右前臂
            0.30, 0.30, // 枪身/枪托（近黑）
            0.62, // 背包（比胸廓暗、比盔亮，读作"背上的织物"）
        ];
        let trans = glam::Mat4::from_translation(glam::Vec3::from(pos));
        let rot = glam::Mat4::from_rotation_y(yaw);
        // 步态：髋/膝/肩/肘绕各自枢轴对向摆动，频率 ~2.2Hz 视觉节奏
        let stride = if moving {
            (phase * 13.8).sin().clamp(-1.0, 1.0) * 0.55
        } else {
            0.0
        };
        // 开火后坐：枪沿 -Z 脉冲（~7Hz 快速衰减），胸轻微前俯
        let (kick, torso_lean) = if firing {
            let k = ((phase * 44.0).sin().abs()).min(1.0);
            (0.09 * k, -0.06 * k)
        } else {
            (0.0, 0.0)
        };
        let mut box_out: Vec<InstanceData> = Vec::with_capacity(6);
        let mut cyl_out: Vec<InstanceData> = Vec::with_capacity(8);
        let mut sph_out: Vec<InstanceData> = Vec::with_capacity(1);
        for (i, (pivot, scale, center, kind, geom)) in parts.iter().enumerate() {
            let mut anim = glam::Mat4::IDENTITY;
            match kind {
                1 => anim *= glam::Mat4::from_rotation_x(stride),        // 左大腿
                2 => anim *= glam::Mat4::from_rotation_x(-stride),       // 右大腿
                3 => anim *= glam::Mat4::from_rotation_x(-stride * 0.5), // 左小腿（膝弯反向）
                4 => anim *= glam::Mat4::from_rotation_x(stride * 0.5),  // 右小腿
                5 => anim *= glam::Mat4::from_rotation_x(HOLD_L_UPPER), // 左上臂：持枪（托护木）
                6 => anim *= glam::Mat4::from_rotation_x(HOLD_R_UPPER), // 右上臂：持枪（握把）
                7 => anim *= glam::Mat4::from_rotation_x(HOLD_L_FORE),  // 左前臂
                8 => anim *= glam::Mat4::from_rotation_x(HOLD_R_FORE),  // 右前臂
                9 => anim *= glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, -kick)), // 枪后坐
                10 => anim *= glam::Mat4::from_rotation_x(torso_lean),   // 胸微俯
                _ => {}
            }
            let model = trans
                * rot
                * glam::Mat4::from_translation(glam::Vec3::from(*pivot))
                * anim
                * glam::Mat4::from_translation(glam::Vec3::from(*center))
                * glam::Mat4::from_scale(glam::Vec3::from(*scale));
            let inst = InstanceData {
                model: model.to_cols_array(),
                // 逐段明暗：NPC 走纯色路径，`tint` 就是外观色；乘上本段系数即得装备层次。
                tint: [tint[0] * shade[i], tint[1] * shade[i], tint[2] * shade[i], tint[3]],
            };
            match geom {
                1 => cyl_out.push(inst),
                2 => sph_out.push(inst),
                _ => box_out.push(inst),
            }
        }
        (box_out, cyl_out, sph_out)
    }
    /// 倒地尸体姿态：14 段人体绕 X 轴躺倒（-90° 侧卧）贴地摊开，枪横置身侧，
    /// tint 按阵营保留（尸体可辨识）。
    pub(crate) fn dead_part_matrices(
        pos: [f32; 3],
        yaw: f32,
        tint: [f32; 4],
    ) -> (Vec<InstanceData>, Vec<InstanceData>, Vec<InstanceData>) {
        // (立姿局部偏移, 缩放, 几何: 0盒 1圆柱 2球)：躺倒时偏移 (x, y_stand*0.3, rest_z)，
        // 经 lie 旋转后世界位置 = (x, rest_z, -y_stand*0.3)：rest_z 保证部件贴地不埋入。
        let parts: [([f32; 3], [f32; 3], u8); 14] = [
            ([-0.10, 0.285, 0.24], [0.065, 0.38, 0.065], 1),  // 左大腿（圆柱）
            ([0.10, 0.285, 0.24], [0.065, 0.38, 0.065], 1),   // 右大腿（圆柱）
            ([-0.09, 0.153, 0.23], [0.05, 0.36, 0.05], 1),    // 左小腿（圆柱）
            ([0.09, 0.153, 0.23], [0.05, 0.36, 0.05], 1),     // 右小腿（圆柱）
            ([-0.09, 0.015, 0.16], [0.09, 0.05, 0.24], 0),    // 左脚（盒）
            ([0.09, 0.015, 0.16], [0.09, 0.05, 0.24], 0),     // 右脚（盒）
            ([0.0, 0.294, 0.15], [0.17, 0.18, 0.19], 1),      // 骨盆（圆柱）
            ([0.0, 0.372, 0.24], [0.20, 0.48, 0.20], 1),      // 胸（桶形圆柱）
            ([0.0, 0.441, 0.07], [0.05, 0.06, 0.05], 0),      // 颈（盒）
            ([0.0, 0.489, 0.155], [0.15, 0.17, 0.15], 2),     // 头（球体，φ≈0.30m）
            ([-0.28, 0.42, 0.17], [0.05, 0.26, 0.05], 1),     // 左上臂（圆柱）
            ([0.28, 0.42, 0.17], [0.05, 0.26, 0.05], 1),      // 右上臂（圆柱）
            ([-0.28, 0.33, 0.16], [0.042, 0.24, 0.042], 1),   // 左前臂（圆柱）
            ([0.28, 0.33, 0.16], [0.042, 0.24, 0.042], 1),    // 右前臂（圆柱）
        ];
        let trans = glam::Mat4::from_translation(glam::Vec3::from(pos));
        let rot = glam::Mat4::from_rotation_y(yaw);
        let lie = glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let mut box_out: Vec<InstanceData> = Vec::with_capacity(6);
        let mut cyl_out: Vec<InstanceData> = Vec::with_capacity(8);
        let mut sph_out: Vec<InstanceData> = Vec::with_capacity(1);
        for (off, scale, geom) in parts.iter() {
            let model = trans
                * rot
                * lie
                * glam::Mat4::from_translation(glam::Vec3::from(*off))
                * glam::Mat4::from_scale(glam::Vec3::from(*scale));
            let inst = InstanceData {
                model: model.to_cols_array(),
                tint,
            };
            match geom {
                1 => cyl_out.push(inst),
                2 => sph_out.push(inst),
                _ => box_out.push(inst),
            }
        }
        // 枪横置身侧：绕 Y 转 90° 使枪管沿 +X，贴地平放
        let gun = trans
            * rot
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.08, 0.62))
            * glam::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
            * glam::Mat4::from_scale(glam::Vec3::new(0.13, 0.10, 0.95));
        box_out.push(InstanceData {
            model: gun.to_cols_array(),
            tint,
        });
        (box_out, cyl_out, sph_out)
    }
    pub(crate) fn set_npc_visuals(&mut self, visuals: &[NpcVisual]) {
        // 临时埋点（RV3D_NPC_POS=1）：每 120 次调用打一次。放在 `clear()` **之前**，
        // 于是 `self.npc_*_parts` 里还是**上一次循环的最终结果** —— 不必去找函数尾部，
        // 也不会被 clear 掉。目的是回答"NPC 到底有没有被交给渲染器"（第 18 轮的结论：
        // 机位几何已经证明是对的，人却不在画面里 ⇒ 该问题不在取景侧）。
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static TICK: AtomicU32 = AtomicU32::new(0);
            if std::env::var("RV3D_NPC_POS").as_deref() == Ok("1")
                && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0
            {
                log::info!(
                    "npcvis: 收到 {} 个 NPC；上一帧段数 盒={} 柱={} 球={}（各组上限 {}）",
                    visuals.len(),
                    self.npc_box_parts.len(),
                    self.npc_cyl_parts.len(),
                    self.npc_sph_parts.len(),
                    MAX_NPC_INSTANCES
                );
                // 判据（第 25 轮）：把**前 3 段盒体段实例矩阵的平移分量**打出来，与
                // `visuals[0].pos + 设计偏移` 比对。设计值（见 `soldier_part_matrices`）：
                // 段0/1 = 左右脚，中心 = pos + (∓0.09, +0.05, +0.02)。
                //   相等 ⇒ 矩阵组装正确，问题在槽位/绘制侧；
                //   不等 ⇒ 矩阵组装错了，`soldier_part_matrices` 的输出没落在 pos 上。
                if let Some(v0) = visuals.first() {
                    for (k, part) in self.npc_box_parts.iter().take(3).enumerate() {
                        log::info!(
                            "npcvis: v0.pos=({:.1},{:.1},{:.1}) 盒段[{k}] 平移=({:.2},{:.2},{:.2}) 期望x={:.2}",
                            v0.pos[0],
                            v0.pos[1],
                            v0.pos[2],
                            part.model[12],
                            part.model[13],
                            part.model[14],
                            v0.pos[0] - 0.09
                        );
                    }
                }
            }
        }
        self.npc_box_parts.clear();
        self.npc_cyl_parts.clear();
        self.npc_sph_parts.clear();
        self.soldier_parts.clear();
        let soldier_on = self.soldier_vertex_count > 0;
        for v in visuals {
            // 🪖 士兵 GLB：每个 NPC **一个实例**（根变换 = 位置 + yaw），而不是 18 段。
            // 只有在网格上传成功时才建 —— 否则白算一遍再被 `upload_soldiers` 丢掉。
            if soldier_on && (self.soldier_parts.len() as u32) < MAX_SOLDIER_INSTANCES {
                // 🚶 **步态**（2026-09-13）：GLB 是静态网格，所以用**实例矩阵**补回动作 ——
                // 不接这一段，士兵会僵直地"滑行"，那是我换掉 18 段箱体时引入的倒退
                // （箱体路径原来靠逐段矩阵摆腿）。
                //
                // 只做两件最显眼的事，不值得为此上骨骼：
                //   * **上下起伏**：一步一次，±4cm。人走路时质心确实在上下动，
                //     幅度取小 —— 大了会读成"跳"而不是"走"。
                //   * **前后倾**：与起伏同相，约 ±3°。给"迈步"一个方向感。
                // 相位用 `v.phase`（main.rs 累积时钟，`moving` 为假时冻结），
                // 所以站住的士兵是静止的，不会原地抖。
                //
                // 开火时再加一次**向后的短促后坐**（枪口冲击把人往后推），
                // 同样用 `phase` 取脉冲，避免引入新的状态量。
                let gait = if v.moving { (v.phase * 2.0).sin() } else { 0.0 };
                let bob = gait * 0.04;
                let lean = gait * 0.05;
                let kick = if v.firing { (v.phase * 18.0).sin().max(0.0) * 0.05 } else { 0.0 };
                let rot = glam::Quat::from_rotation_y(v.yaw)
                    * glam::Quat::from_rotation_x(lean - kick);
                let m = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::ONE,
                    rot,
                    glam::Vec3::new(v.pos[0], v.pos[1] + bob, v.pos[2]),
                );
                // 🔴🔴 2026-09-13 定案（第一版整身饱和红的真正原因）：
                //
                // 士兵实例槽 `SOLDIER_INSTANCE_BASE`(83011) **≥ `NPC_SLOT_BASE`**，
                // 于是顶点着色器把它归进 NPC 那条**纯色路径**（`flat_flag = 2`）——
                // 那条路上 **`tint` 就是外观色本身，顶点色是被白化掉的**
                // （见 AGENTS.md 铁律 B：18 段箱体的顶点色全部白化）。
                // 所以第一版照抄 `v.tint` 的结果是"一整块饱和的红"，军服细节全丢；
                // 我随后改成"与白色混三成"也只减轻了饱和度，**因为问题不是强度而是语义**。
                //
                // 道具（槽位 83010，同样落在那段范围里）却渲染正常 —— **靠的是 `tint.w`**：
                // `Shape::Authored` 用 `tint.w = 6.0` 作标记 ⇒ 片元判 `authored`
                // ⇒ 跳过四条程序化表面效果、`tint.rgb` 只作乘数 ⇒ 外观回到顶点色。
                //
                // ⇒ 士兵照打同一个标记，于是它就和道具一样**用 `soldier.glb` 里烘焙的
                // 军服/护甲/皮肤色**渲染。`tint.rgb = 1` 表示"顶点色原样输出"。
                //
                // ⚠️ 代价：**红蓝阵营眼下没有颜色区分**（GLB 只有一套橄榄绿军服）。
                // 要区分就得在 Blender 里出两套顶点色变体（蓝/红臂章或迷彩），
                // 那是资产侧的事，不是这里调 tint 能解决的 —— 调了只会把模型涂成一块色。
                // 🔴🔴 2026-09-13：**`tint.w = 6.0` 是"Authored"标记**（`build.rs:99`：
                // `tint.w ∈ (5.5, 6.5)` ⇒ `flat_flag = 1.25`）⇒ 片元走顶点色路径，
                // `tint.rgb` 只作**乘数**。不接这一条，槽位 83011 ≥ `NPC_SLOT_BASE`
                // 会落进 NPC 的纯色路径，`tint` 被当成颜色本身 ⇒ 士兵变成一整块阵营色。
                //
                // ⚠️ **`tint.rgb` 取"向白靠拢的阵营色"，不是原色**。
                // 我早前试过"原色 × 0.30"却仍是纯红，**那次实验是被污染的** ——
                // 当时 18 段箱体还在同时画，红来自箱体。箱体关掉后才量得准。
                // 取 0.35：橄榄绿军服被染成"偏红/偏蓝的军服"，敌我一眼可辨，
                // 而布料与护甲的明暗层次**保留**（这正是"像人"的前提）。
                //
                // ⇒ 若要把阵营差异做得更硬（臂章/迷彩），正解是在 Blender 里出两套
                // 顶点色变体，而不是继续加 tint 强度 —— 那只会把模型重新涂成一块色。
                let t = v.tint;
                const TEAM_MIX: f32 = 0.35;
                let tint = [
                    1.0 - (1.0 - t[0]) * TEAM_MIX,
                    1.0 - (1.0 - t[1]) * TEAM_MIX,
                    1.0 - (1.0 - t[2]) * TEAM_MIX,
                    6.0,
                ];
                self.soldier_parts.push(InstanceData {
                    model: m.to_cols_array(),
                    tint,
                });
            }
            // 🔴🔴 2026-09-13 定案：**GLB 生效时不再生成 18 段箱体**。
            //
            // 此前两条路**同时在画**，玩家看到的是"GLB 士兵叠在箱体堆上"：
            // 形状大体是对的（所以前几轮我没看出来），但同一个身上有两种颜色。
            // 是**品红探针**把它逼出来的 —— 把 tint 临时改成 `[1,0,1,6]` 后
            // **四肢变品红、躯干仍是红的** ⇒ 红的那部分根本不是我的 draw。
            //
            // 这同时把每个 NPC 的实例数从 **18 降到 1**。
            //
            // ⚠️ `soldier_part_matrices` **不删** —— 它是 `soldier.glb` 缺失/上传失败时的
            // 回退路径（`soldier_on == false`），也是将来做"远距 LOD 用箱体"的现成备选。
            if !soldier_on {
                let (box_parts, cyl_parts, sph_parts) = Self::soldier_part_matrices(
                    v.pos, v.yaw, v.tint, v.phase, v.moving, v.firing,
                );
                for part in box_parts {
                    if (self.npc_box_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_box_parts.push(part);
                    }
                }
                for part in cyl_parts {
                    if (self.npc_cyl_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_cyl_parts.push(part);
                    }
                }
                for part in sph_parts {
                    if (self.npc_sph_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_sph_parts.push(part);
                    }
                }
                if (self.npc_box_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_cyl_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_sph_parts.len() as u32) >= MAX_NPC_INSTANCES
                {
                    self.warn_npc_cap_once();
                    break;
                }
            }
        }
    }
    /// 追加倒地尸体（由 main.rs 传入位置/朝向/阵营）。
    ///
    /// 🪖 2026-09-13：**GLB 生效时改用同一套实例化**（原来走 15 段躺倒姿态）。
    /// 一具尸体 = 一个实例，矩阵把 `soldier.glb` 放倒：绕 X 转 −90°，
    /// **绕原点（脚底）转** ⇒ 脚留在 `pos`、身体沿 +Z 平躺、**自然贴地**
    /// （GLB 的 y∈[0,1.84] 转到 z∈[0,1.84]，正好躺在地上）。
    /// 再抬 0.12m，免得半个身子陷进路面。
    ///
    /// 与活体共用 `soldier_parts` / 同一段实例区 ⇒ 一次 draw 里全画完。
    pub(crate) fn set_dead_bodies(&mut self, bodies: &[NpcVisual]) {
        let soldier_on = self.soldier_vertex_count > 0;
        for v in bodies {
            if soldier_on && (self.soldier_parts.len() as u32) < MAX_SOLDIER_INSTANCES {
                let rot = glam::Quat::from_rotation_y(v.yaw)
                    * glam::Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
                let m = glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::ONE,
                    rot,
                    glam::Vec3::new(v.pos[0], v.pos[1] + 0.12, v.pos[2]),
                );
                // 阵营色沿用与活体同一套（含 Authored 标记）
                let t = v.tint;
                const TEAM_MIX: f32 = 0.35;
                self.soldier_parts.push(InstanceData {
                    model: m.to_cols_array(),
                    tint: [
                        1.0 - (1.0 - t[0]) * TEAM_MIX,
                        1.0 - (1.0 - t[1]) * TEAM_MIX,
                        1.0 - (1.0 - t[2]) * TEAM_MIX,
                        6.0,
                    ],
                });
            }
            if !soldier_on {
                let (box_parts, cyl_parts, sph_parts) =
                    Self::dead_part_matrices(v.pos, v.yaw, v.tint);
                for part in box_parts {
                    if (self.npc_box_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_box_parts.push(part);
                    }
                }
                for part in cyl_parts {
                    if (self.npc_cyl_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_cyl_parts.push(part);
                    }
                }
                for part in sph_parts {
                    if (self.npc_sph_parts.len() as u32) < MAX_NPC_INSTANCES {
                        self.npc_sph_parts.push(part);
                    }
                }
                if (self.npc_box_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_cyl_parts.len() as u32) >= MAX_NPC_INSTANCES
                    && (self.npc_sph_parts.len() as u32) >= MAX_NPC_INSTANCES
                {
                    self.warn_npc_cap_once();
                    break;
                }
            }
        }
    }
    /// 上传第一人称枪模程序化网格（2026-08-16 高模路线）：顶点已是世界空间
    /// （main.rs 用 view⁻¹ × 锚点烘焙），颜色已含材质×烘焙光照。
    /// 用主管线（深度测试关）以 identity 实例（槽 INSTANCE_COUNT）绘制——
    /// 深度测试关闭 = 枪模恒可见（不再需要 z 覆盖 hack，也不写脏深度）。
    pub(crate) fn set_first_person_gun_mesh(
        &mut self,
        verts: &[crate::engine::meshgen::GVertex],
        indices: &[u32],
    ) {
        // 🔴 2026-09-22 复查：先记下"旧枪模的计数"。
        // 扩容失败时旧 buffer 仍完好，只有把计数也恢复成旧值，两者才继续自洽；
        // 否则会出现"旧缓冲 + 新计数" ⇒ 按新计数抓取旧缓冲的索引 = 越界（静默的错误几何）。
        let prev_vcount = self.gun_vertex_count;
        let prev_icount = self.gun_index_count;
        self.gun_vertex_count = verts.len() as u32;
        self.gun_index_count = indices.len() as u32;
        if verts.is_empty() || indices.is_empty() {
            return;
        }
        // 枪模缓冲容量：预分配全局最大（当前最大 verts=63283 / idx=70479 ⇒
        // next_power_of_two = 65536 / 262144）。**只增不减**：切枪永不重建缓冲
        // （重建会 destroy 正在被 GPU 使用的 buffer → NVIDIA 驱动 device lost）。
        //
        // 🔴 2026-09-15 修的正是这句注释与代码不符：旧判据是
        // `need != capacity` 就重建，于是**换成更小的枪也会重建** ——
        // 实测按一下 "2"（AK-12M 63283 顶点 / 容量 65536 → AK-104 11705 顶点 /
        // 需要 32768 ≠ 65536）当场 `vkQueueSubmit` 返回 `VK_ERROR_DEVICE_LOST`，
        // 画面上是"切枪 = 整台设备消失"。
        // ⇒ 判据必须是 `need > capacity`（与 `set_props` 同一写法），
        //   并且真扩容前无条件 `device_wait_idle()`。
        let need_verts = 32768u32.max((verts.len() as u32).next_power_of_two());
        let need_idx = 262_144u32.max((indices.len() as u32).next_power_of_two());
        if need_verts > self.gun_buffer_capacity_verts
            || need_idx > self.gun_buffer_capacity_idx
            || self.gun_mapped.is_null()
            || self.gun_vertex_buffer == vk::Buffer::null()
        {
            // 真扩容（或首次创建）才等：等待发生在帧与帧之间、不在命令缓冲记录期间，安全。
            unsafe {
                self.wait_idle_checked();
            }
            // 🔴 2026-09-22 复查（灰色地带修复）：**先建新的，成功了再毁旧的**。
            // 旧写法是「先 destroy 两个旧 buffer，再 `create_host_buffer(..).expect(..)`」：
            //  ① 分配失败（显存碎片 / OOM）直接 panic —— 游戏在切枪瞬间整个进程没了；
            //  ② 更糟的是**即使不 panic，旧句柄也已经毁掉了**（自留悬空句柄，
            //     之后任何一次 destroy/free 都是二次释放）。
            // 现在失败路径只 log::error 并**保留原缓冲**（降级：这一枪不换，其余照常跑）；
            // 成功路径多占一份旧缓冲的显存（几 MB）直到销毁，代价可忽略。
            let v_size = need_verts as u64 * std::mem::size_of::<Vertex>() as u64;
            let i_size = need_idx as u64 * 4; // 索引容量独立按实际索引数
            let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, v_size)
            {
                Ok(pair) => pair,
                Err(e) => {
                    log::error!("枪模顶点缓冲扩容失败，保留原枪模：{e}");
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, i_size) {
                Ok(pair) => pair,
                Err(e) => {
                    log::error!("枪模索引缓冲扩容失败，保留原枪模：{e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                    }
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            let mapped = match unsafe {
                self.device
                    .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
            } {
                Ok(p) => p,
                Err(e) => {
                    log::error!("枪模顶点缓冲映射失败，保留原枪模：{e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                        self.device.destroy_buffer(ib, None);
                        self.device.free_memory(im, None);
                    }
                    self.gun_vertex_count = prev_vcount;
                    self.gun_index_count = prev_icount;
                    return;
                }
            };
            // 新的三件套齐了，才拆旧的（前面已 `device_wait_idle()`，不会撞在飞帧）
            if self.gun_vertex_buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(self.gun_vertex_buffer, None) };
            }
            if self.gun_vertex_buffer_memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(self.gun_vertex_buffer_memory, None) };
            }
            if self.gun_index_buffer != vk::Buffer::null() {
                unsafe { self.device.destroy_buffer(self.gun_index_buffer, None) };
            }
            if self.gun_index_buffer_memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(self.gun_index_buffer_memory, None) };
            }
            self.gun_vertex_buffer = vb;
            self.gun_vertex_buffer_memory = vm;
            self.gun_index_buffer = ib;
            self.gun_index_buffer_memory = im;
            self.gun_mapped = mapped;
            self.gun_buffer_capacity_verts = need_verts;
            self.gun_buffer_capacity_idx = need_idx;
        }

        // 写入顶点（GVertex → Vertex: pos/color/uv；color 已含烘焙光照）
        let vptr = self.gun_mapped as *mut Vertex;
        for (i, v) in verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: v.pos,
                    color: v.color,
                    uv: v.uv,
                };
            }
        }
        // 2026-08-28 终极可见性修复：unmap → remap（host-coherent 亦可能被驱动缓存延迟可见）
        //
        // 🔴 2026-09-22 复查补（灰色地带）：这里原来是 `.expect("枪模顶点缓冲重映射失败")` ——
        // 映射失败 = 切枪瞬间 panic。更要紧的是失败时 `gun_mapped` 会**停在悬空指针上**：
        // unmap 已经执行，旧指针指向已解除映射的内存 ⇒ 下一枪的顶点写入是野地址写。
        // 现在改成与其它失败路径同款：报错 + 指针置空（入口判据 `gun_mapped.is_null()`
        // 会让**下一枪**走重建分支、重新上传并恢复）。
        // ⚠️ 计数**不回退**：新顶点数据此刻已经写进新缓冲了，回退计数反而会让 draw
        // 按旧数量去抓新缓冲（也就是 5227 行注释里那种"新缓冲 + 旧计数"的错配）。
        unsafe {
            self.device.unmap_memory(self.gun_vertex_buffer_memory);
            match self.device.map_memory(
                self.gun_vertex_buffer_memory,
                0,
                self.gun_buffer_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => self.gun_mapped = p,
                Err(e) => {
                    log::error!(
                        "枪模顶点缓冲重映射失败：本枪仍按已写入的数据绘制，指针置空，下一枪重建: {e}"
                    );
                    self.gun_mapped = std::ptr::null_mut();
                }
            }
        }
        // 索引上传（独立映射窗口，用一次性的暂存：直接再 map 索引内存）
        unsafe {
            let iptr = match self.device.map_memory(
                self.gun_index_buffer_memory,
                0,
                self.gun_index_count as u64 * 4,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    // 索引没写进去 ⇒ 缓冲里是**上一把枪的索引**（或未初始化数据）。
                    // 按新计数抓取 = 画错几何，最坏是越界索引 ⇒ 设备消失。
                    // ⇒ 本枪不画（计数归零），并把顶点指针置空，让下一枪整体重建后重传两件套。
                    log::error!("枪模索引缓冲映射失败：本枪不画，下一枪重建: {e}");
                    self.gun_index_count = 0;
                    self.gun_mapped = std::ptr::null_mut();
                    return;
                }
            };
            std::ptr::copy_nonoverlapping(
                indices.as_ptr() as *const u8,
                iptr as *mut u8,
                self.gun_index_count as usize * 4,
            );
            self.device.unmap_memory(self.gun_index_buffer_memory);
        }
    }
    /// 上传 GLB 道具：把摆放列表在 CPU 上烘成一份静态几何，再传上 GPU。
    ///
    /// 只在**地图重载**时调用（`main.rs` 用 `Game::map_generation()` 判定），不要每帧调：
    /// 一次合并是百万级顶点的拷贝。
    ///
    /// 缓冲**只增不减**，且扩容前无条件 `device_wait_idle()`。这两条都是照着枪模的
    /// 事故写的：2026-08-18 那次"切到小网格武器触发重建"直接 destroy 了正在被 GPU 使用
    /// 的 buffer，NVIDIA 驱动 device lost、画面卡死。地图重载发生在帧与帧之间、不在命令
    /// 缓冲记录期间，所以这里的等待是安全的；缩小容量同样走这条路，因此必须等。
    pub(crate) fn set_props(
        &mut self,
        set: &crate::engine::props::PropSet,
        placements: &[crate::engine::props::PropPlacement],
    ) {
        let merged =
            crate::engine::props::merge_binned(set, placements, PROP_BIN_CELL_M, |x, z| {
                terrain_height_at(x, z)
            });
        // 按**摆放类型**统计（RV3D_PROP_STATS=1 时打一行）：道具是最大单项（≈38% 帧时间），
        // 但此前只有"总共多少三角形"，不知道是哪几类资产贡献的。**顶点数才是本仓的帧率**
        // （无正常线槽位 ⇒ 每帧提交的顶点跨度就是成本），所以两类数一起给，并按**顶点**排序。
        if std::env::var("RV3D_PROP_STATS").is_ok() {
            let mut per_type: Vec<(String, u32, u64, u64)> = Vec::new();
            for p in placements {
                let Some(mesh) = set.get(p.mesh) else { continue };
                let tris = (mesh.indices.len() / 3) as u64;
                let verts = mesh.verts.len() as u64;
                match per_type.iter_mut().find(|t| t.0 == mesh.name) {
                    Some(t) => {
                        t.1 += 1;
                        t.2 += tris;
                        t.3 += verts;
                    }
                    None => per_type.push((mesh.name.clone(), 1, tris, verts)),
                }
            }
            per_type.sort_by(|a, b| b.3.cmp(&a.3));
            let total_v: u64 = per_type.iter().map(|t| t.3).sum();
            let total: u64 = per_type.iter().map(|t| t.2).sum();
            let top: Vec<String> = per_type
                .iter()
                .take(8)
                .map(|t| {
                    let pct = if total_v > 0 {
                        100.0 * t.3 as f64 / total_v as f64
                    } else {
                        0.0
                    };
                    format!(
                        "{} x{} = {} tri / {} v ({:.0}% v)",
                        t.0, t.1, t.2, t.3, pct
                    )
                })
                .collect();
            log::info!(
                "proptypes: 摆放 {} 处 / 场景三角形合计 {} / 顶点合计 {}；按顶点前 8 类：{}",
                placements.len(),
                total,
                total_v,
                top.join(" / ")
            );
        }
        self.prop_vertex_count = 0;
        self.prop_index_count = 0;
        self.prop_bins.clear();
        // 任何提前返回都先把阴影几何清零：shadow loop 会退回全量，绝不会引用
        // 上一张地图的分桶。
        self.prop_sh_index_count = 0;
        self.prop_sh_bins.clear();
        // 🏢 PT 道具属性表同理随道具一起作废（重建在下面上传成功后进行）。
        // 先静默再销毁：旧表可能正被上一帧的 PT dispatch 读着。
        unsafe {
            self.wait_idle_checked();
            if self.prop_attr_buf != vk::Buffer::null() {
                self.device.destroy_buffer(self.prop_attr_buf, None);
                self.prop_attr_buf = vk::Buffer::null();
            }
            if self.prop_attr_mem != vk::DeviceMemory::null() {
                self.device.free_memory(self.prop_attr_mem, None);
                self.prop_attr_mem = vk::DeviceMemory::null();
            }
        }
        self.prop_attr_tris = 0;
        if merged.verts.is_empty() || merged.indices.is_empty() {
            log::info!("props: 无摆放几何（套件 {} 件 / 摆放 {} 处）", set.len(), placements.len());
            return;
        }
        // RV3D_NO_PROPS=1：完全跳过道具上传（于是 draw 因 index_count==0 自然不发）。
        // 这是性能 A/B 的诊断门，与 RV3D_NO_SHADOW / RV3D_NO_GROUND_TEX 同一套惯例——
        // 优化前先量清"这个东西到底值多少帧"，否则很可能在优化错的对象。
        if std::env::var("RV3D_NO_PROPS").as_deref() == Ok("1") {
            log::info!("props: RV3D_NO_PROPS=1，跳过上传（合并结果 {} 顶点未提交）", merged.verts.len());
            return;
        }
        let need_v = merged.verts.len() as u32;
        let need_i = merged.indices.len() as u32;
        let mapped_ok = self.prop_mapped != std::ptr::null_mut()
            && self.prop_vertex_buffer != vk::Buffer::null();
        // 只在"要得更多"或"还没有"时重建（判据 `need > capacity`，不是 `!=`：与枪模同一条理由）
        if prop_buffer_growth_needed(
            need_v,
            self.prop_capacity_verts,
            need_i,
            self.prop_capacity_idx,
            mapped_ok,
        ) {
            // 等待发生在帧与帧之间、不在命令缓冲记录期间，安全。
            unsafe {
                self.wait_idle_checked();
            }
            // 2 的幂向上取整：地图尺寸只会小幅波动，避免每次重载都重建
            let cap_v = need_v.next_power_of_two().max(65_536);
            let cap_i = need_i.next_power_of_two().max(65_536);
            // 🏢 道具进 BLAS（2026-09-19，用户决策）：PT 的第二个三角形几何**零拷贝**
            //   直接引用这两个缓冲 ⇒ usage 必须叠加 SHADER_DEVICE_ADDRESS +
            //   ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY；顶点色还要给 PT 着色器当
            //   albedo ⇒ VB 再加 STORAGE_BUFFER。主 pass 不受影响（usage 只做加法）。
            //
            // 🔴 2026-09-26 复查（与枪模 5585 起同一形态）：**先建新的，成功了再拆旧的**。
            // 旧写法是「先 unmap/destroy/free 旧的 → 再 create 新的」，而 create 失败时只
            // log + return ⇒ 句柄字段里留着**已销毁、却非 null** 的 VkBuffer：
            //   ① 阴影 pass 只判 `!= null` 就把它绑上（10866 行那条）；
            //   ② 下一次扩容 / 退出清理会对同一个句柄**二次 destroy_buffer**（双重释放）。
            // 现在失败路径原样保留旧缓冲（降级 = 这一张图的道具不画，其余照常跑，
            // 而且因为旧映射没动，下一次 set_props 还能自己恢复）；
            // 成功路径多占一份旧显存，直到下面 swap 时销毁，代价可忽略。
            let (vb, vm) = match self.create_host_buffer(
                vk::BufferUsageFlags::VERTEX_BUFFER
                    | vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                cap_v as u64 * std::mem::size_of::<Vertex>() as u64,
            ) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("props: 顶点缓冲创建失败，跳过道具绘制: {e}");
                    return;
                }
            };
            let (ib, im) = match self.create_host_buffer(
                vk::BufferUsageFlags::INDEX_BUFFER
                    | vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                cap_i as u64 * 4,
            ) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("props: 索引缓冲创建失败，跳过道具绘制: {e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                    }
                    return;
                }
            };
            let new_mapped = match unsafe {
                self.device.map_memory(
                    vm,
                    0,
                    cap_v as u64 * std::mem::size_of::<Vertex>() as u64,
                    vk::MemoryMapFlags::empty(),
                )
            } {
                Ok(p) => p,
                Err(e) => {
                    log::error!("props: 顶点缓冲映射失败，跳过道具绘制: {e}");
                    unsafe {
                        self.device.destroy_buffer(vb, None);
                        self.device.free_memory(vm, None);
                        self.device.destroy_buffer(ib, None);
                        self.device.free_memory(im, None);
                    }
                    return;
                }
            };
            // 新的三件套齐了，才拆旧的（前面已 `device_wait_idle()`，不会撞在飞帧）
            unsafe {
                if self.prop_mapped != std::ptr::null_mut() {
                    self.device.unmap_memory(self.prop_vertex_memory);
                }
                if self.prop_vertex_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(self.prop_vertex_buffer, None);
                }
                if self.prop_vertex_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(self.prop_vertex_memory, None);
                }
                if self.prop_index_buffer != vk::Buffer::null() {
                    self.device.destroy_buffer(self.prop_index_buffer, None);
                }
                if self.prop_index_memory != vk::DeviceMemory::null() {
                    self.device.free_memory(self.prop_index_memory, None);
                }
            }
            self.prop_vertex_buffer = vb;
            self.prop_vertex_memory = vm;
            self.prop_index_buffer = ib;
            self.prop_index_memory = im;
            self.prop_mapped = new_mapped;
            self.prop_capacity_verts = cap_v;
            self.prop_capacity_idx = cap_i;
            log::info!(
                "props: 缓冲扩容 顶点 {}/{} 索引 {}/{}（{:.1} MB）",
                need_v, cap_v, need_i, cap_i,
                (cap_v as u64 * std::mem::size_of::<Vertex>() as u64
                    + cap_i as u64 * 4) as f64 / 1048576.0
            );
        }

        // [f32;11]（pos/normal/uv/color）→ Vertex（pos/color/uv）。
        // normal 与枪模一样在上传时丢弃：本引擎的着色法线由屏幕空间导数重建，
        // 顶点格式里没有它的槽位。因此**绕序必须正确**，反面的面会直接黑掉而不报错。
        let vptr = self.prop_mapped as *mut Vertex;
        for (i, v) in merged.verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: [v[0], v[1], v[2]],
                    color: [v[8], v[9], v[10]],
                    uv: [v[6], v[7]],
                };
            }
        }
        // 与枪模同样的 unmap→remap：host-coherent 内存也可能被驱动延迟可见
        let v_bytes =
            self.prop_capacity_verts as u64 * std::mem::size_of::<Vertex>() as u64;
        unsafe {
            self.device.unmap_memory(self.prop_vertex_memory);
            match self
                .device
                .map_memory(self.prop_vertex_memory, 0, v_bytes, vk::MemoryMapFlags::empty())
            {
                Ok(p) => self.prop_mapped = p,
                Err(e) => {
                    log::error!("props: 顶点缓冲重映射失败，跳过道具绘制: {e}");
                    self.prop_mapped = std::ptr::null_mut();
                    return;
                }
            }
        }
        unsafe {
            match self.device.map_memory(
                self.prop_index_memory,
                0,
                need_i as u64 * 4,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(iptr) => {
                    std::ptr::copy_nonoverlapping(
                        merged.indices.as_ptr() as *const u8,
                        iptr as *mut u8,
                        need_i as usize * 4,
                    );
                    self.device.unmap_memory(self.prop_index_memory);
                }
                Err(e) => {
                    log::error!("props: 索引缓冲映射失败，跳过道具绘制: {e}");
                    return;
                }
            }
        }
        self.prop_vertex_count = need_v;
        self.prop_index_count = need_i;
        // 桶数量级只有几十，clone 成本可忽略；存下来供 record_command_buffer 逐桶剔除
        self.prop_bins = merged.bins.clone();
        // 🏢 PT 道具逐三角属性表（2026-09-19）：每三角 2×u32 = 量化面法线（(v·127+127)
        //   每轴 u8）+ 平均顶点色（u8×3）。PT 着色器一次命中读一个 8B u32x2——
        //   device-local，**不再随机读 host-visible 主 VB**（pt3 实测那是 80× 的 PCIe 风暴）。
        //   烘焙本体是纯函数 pt_bake_prop_attrs（判据在 pt_prop_attrs_tests）。
        {
            let attrs = pt_bake_prop_attrs(&merged.verts, &merged.indices);
            let ntri = attrs.len() / 2;
            let mut bytes: Vec<u8> = Vec::with_capacity(attrs.len() * 4);
            for w in &attrs {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
            match self.create_device_local_buffer(
                vk::BufferUsageFlags::STORAGE_BUFFER,
                &bytes,
                "prop-attr",
            ) {
                Ok((b, m)) => {
                    self.prop_attr_buf = b;
                    self.prop_attr_mem = m;
                    self.prop_attr_tris = ntri as u32;
                }
                Err(e) => {
                    // 属性表失败 ⇒ 道具不进 BLAS（build_pt_as 以 prop_attr_tris 为准），
                    // PT 退回盒体原型——退化方向是"少几何"，不是越界读
                    log::error!("props/PT: 属性表上传失败，道具不进 BLAS: {e}");
                }
            }
        }
        log::info!(
            "props: 上传完成 顶点 {} / 三角 {} / 摆放 {} 处 / 分桶 {} 个（cell={}m），包围盒 x∈[{:.1},{:.1}] y∈[{:.1},{:.1}] z∈[{:.1},{:.1}]",
            need_v, need_i / 3, placements.len(), self.prop_bins.len(), PROP_BIN_CELL_M,
            merged.min[0], merged.max[0], merged.min[1], merged.max[1],
            merged.min[2], merged.max[2]
        );
        self.set_shadow_props(set, placements);
    }
    /// 2026-08-28：第一人称枪的实例模型矩阵 per-frame（bob/后坐走矩阵，顶点静态）
    /// 枪槽 75841 的唯一写者：顶点缓冲 = 视空间静态（仅首次上传），矩阵每帧更新
    pub(crate) fn set_first_person_gun_model(&mut self, m: glam::Mat4) {
        let slot = match self.instance_mapped.get(self.current_frame) {
            Some(&p) if !p.is_null() => p as *mut u8,
            _ => return,
        };
        let stride = std::mem::size_of::<InstanceData>();
        unsafe {
            let p = slot.add(GUN_INSTANCE_INDEX as usize * stride);
            // InstanceData { model: [f32; 16], tint: [f32; 4] }
            let model = m.to_cols_array();
            std::ptr::copy_nonoverlapping(model.as_ptr(), p as *mut f32, 16);
        }
    }
    /// 🪖 **上传士兵 GLB 网格**（2026-09-13）。只调用一次（启动时）。
    ///
    /// **与枪模的两处关键差别**：
    /// 1. **按常量容量一次分配、永不重建**（照 `props` 那条纪律）。士兵网格不存在切枪那种
    ///    "容量忽大忽小"的场景，而重建会 destroy 在飞 buffer → NVIDIA device lost。
    /// 2. 顶点来源是 GLB 的 `[f32; 11]`（`pos(3) normal(3) uv(2) color(3)`，见 `assets.rs`），
    ///    这里按 `pos=[0..3] / uv=[6,7] / color=[8..11]` 取 —— **与 `upload_props` 完全同一套
    ///    映射**（本引擎顶点格式 `stride=32, pos/color/uv`，没有法线槽位，法线由屏幕空间
    ///    导数重建，所以 GLB 的法线直接丢弃）。
    pub(crate) fn set_soldier_mesh(&mut self, verts: &[[f32; 11]], indices: &[u32]) {
        // 🔴🔴 2026-09-13：**幂等守卫**。调用方在每帧的渲染准备段里调用本函数，
        // 而它每次都会 `create_host_buffer` 出一套新的 GPU 缓冲 ⇒ **每帧泄漏一份显存**，
        // 几分钟就 OOM / device lost。实测日志里同一个 "士兵 GLB 已上传" 一段内出现 3+ 次。
        // 士兵网格与武器不同：它**只在启动时上传一次**，之后永不改变，所以直接早退即可。
        if self.soldier_vertex_count > 0 {
            return;
        }
        if verts.is_empty() || indices.is_empty() {
            log::info!("soldier: 未提供网格，NPC 继续用 18 段箱体");
            return;
        }
        if verts.len() > SOLDIER_MESH_VERTS as usize || indices.len() > SOLDIER_MESH_INDICES as usize {
            log::error!(
                "soldier: 网格超出预留容量（{} > {} 顶点 / {} > {} 索引）—— 必须同步放大 \
                 SOLDIER_MESH_VERTS/SOLDIER_MESH_INDICES，否则写越界（host buffer 不报 VUID）",
                verts.len(), SOLDIER_MESH_VERTS, indices.len(), SOLDIER_MESH_INDICES
            );
            return;
        }
        let v_size = SOLDIER_MESH_VERTS as u64 * std::mem::size_of::<Vertex>() as u64;
        let i_size = SOLDIER_MESH_INDICES as u64 * 4;
        // 失败路径统一收尾：释放**已经建好、但还没存进 `self`** 的 buffer/memory。
        // 直接 `return` 等于永久泄漏这几份显存 —— 没有任何别的引用还找得到它们。
        fn free_pair(device: &ash::Device, b: vk::Buffer, m: vk::DeviceMemory) {
            unsafe {
                if b != vk::Buffer::null() {
                    device.destroy_buffer(b, None);
                }
                if m != vk::DeviceMemory::null() {
                    device.free_memory(m, None);
                }
            }
        }
        let (vb, vm) = match self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, v_size) {
            Ok(x) => x,
            Err(e) => {
                log::error!("soldier: 顶点缓冲创建失败，退回 18 段箱体: {e}");
                return;
            }
        };
        let (ib, im) = match self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, i_size) {
            Ok(x) => x,
            Err(e) => {
                log::error!("soldier: 索引缓冲创建失败，退回 18 段箱体: {e}");
                // 🔴 2026-09-22 复查补：顶点那一对已经建好了，必须先释放再退出。
                free_pair(&self.device, vb, vm);
                return;
            }
        };
        let mapped = match unsafe {
            self.device
                .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
        } {
            Ok(p) => p,
            Err(e) => {
                log::error!("soldier: 顶点缓冲映射失败，退回 18 段箱体: {e}");
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
        };
        let vptr = mapped as *mut Vertex;
        for (i, v) in verts.iter().enumerate() {
            unsafe {
                *vptr.add(i) = Vertex {
                    pos: [v[0], v[1], v[2]],
                    color: [v[8], v[9], v[10]],
                    uv: [v[6], v[7]],
                };
            }
        }
        // 索引也要 host-visible：单独 map 索引缓冲
        //
        // 🔴 2026-09-22 复查补：这里原来是把 `map_memory` 的结果用 `if let Ok` 吞掉 ——
        // **映射失败被静默吞掉**：索引一个都没写进去，函数却照常往下走，把
        // `soldier_index_count` 设成 `indices.len()` ⇒ draw call 按这个数去读
        // **未初始化的显存**（几何错乱，最坏是越界索引 = 设备消失），且不报任何错。
        // 现在与其它失败路径同款：报错 + 释放已建资源 + 退回 18 段箱体。
        let ip = match unsafe {
            self.device
                .map_memory(im, 0, i_size, vk::MemoryMapFlags::empty())
        } {
            Ok(p) => p,
            Err(e) => {
                log::error!("soldier: 索引缓冲映射失败，退回 18 段箱体: {e}");
                unsafe { self.device.unmap_memory(vm) };
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
        };
        let iptr = ip as *mut u32;
        for (i, idx) in indices.iter().enumerate() {
            unsafe { *iptr.add(i) = *idx };
        }
        unsafe { self.device.unmap_memory(im) };
        // 与枪模/道具同样的 unmap→remap：host-coherent 内存也可能被驱动延迟可见。
        // 顶点只上传这一次，所以重映射后**不留指针**（枪模留是因为它要反复重写）。
        unsafe {
            self.device.unmap_memory(vm);
            if let Err(e) = self
                .device
                .map_memory(vm, 0, v_size, vk::MemoryMapFlags::empty())
            {
                log::error!("soldier: 顶点缓冲重映射失败，退回 18 段箱体: {e}");
                // vm 刚刚已 unmap（不能重复 unmap），直接释放两对句柄即可。
                free_pair(&self.device, vb, vm);
                free_pair(&self.device, ib, im);
                return;
            }
            self.device.unmap_memory(vm);
        }
        self.soldier_vertex_buffer = vb;
        self.soldier_vertex_buffer_memory = vm;
        self.soldier_index_buffer = ib;
        self.soldier_index_buffer_memory = im;
        self.soldier_vertex_count = verts.len() as u32;
        self.soldier_index_count = indices.len() as u32;
        log::info!(
            "soldier: 士兵 GLB 已上传（{} 顶点 / {} 索引，实例区起点 {}，容量 {}）",
            self.soldier_vertex_count,
            self.soldier_index_count,
            SOLDIER_INSTANCE_BASE,
            MAX_SOLDIER_INSTANCES
        );
    }
}
