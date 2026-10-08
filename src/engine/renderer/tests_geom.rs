// 由 renderer.rs 按主题拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 唯一改动是自扫源码那几处 include_str! 的相对路径（多一层目录）。
// 这些模块仍是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段与文件作用域私有项；
// `use super::*;` 把上一层的名字转发下来，子模块里的 `use super::*;` 因此仍解析到 renderer。
use super::*;

#[cfg(test)]
mod marker_scale_tests {
    use super::*;
    use crate::engine::geom::Shape;

    /// 端到端不变式：**画出来的尺寸逐轴恒等于碰撞 AABB**。
    ///
    /// 存在理由：`for_obstacle` 此前对三个轴一律写 `2*half`，而模板是 ±1 的立方体/球、
    /// r=1 的圆柱 ⇒ 全城 1700 多个程序化构件画成设计尺寸的 **2 倍**（灌木球 φ2.86 画成
    /// φ5.7、路缘石 0.55 宽画成 1.1 m 矮墙、柱头 φ0.92 画成 φ1.84 的悬空圆盘），并且
    /// 玩家能站进"看得见的那半个盒子"里 = 穿模。它存活了三周，因为**圆柱的高度恰好是对的**
    /// （模板 y 已是 ±0.5），"部分正确"让每次目视核对都能找到一条反例说服自己。
    ///
    /// 这条测试按形状逐轴验，任何一侧（模板 or 缩放推导）单独改动都会让它红。
    #[test]
    fn marker_visible_size_matches_aabb() {
        for shape in [
            Shape::Legacy,
            Shape::Cylinder,
            Shape::Sphere,
            Shape::Authored,
        ] {
            let ob = MapObstacle::new(ObstacleKind::Block, 1.0, -2.0, 0.35, 0.6)
                .shaped(0.5, 1.25, Some([0.5, 0.5, 0.5]))
                .geom(shape);
            let m = WorldMarker::for_obstacle(&ob);
            // 模型 = T × S ⇒ 三个基向量的模长就是逐轴缩放
            let scale = [
                m.model.x_axis.length(),
                m.model.y_axis.length(),
                m.model.z_axis.length(),
            ];
            let half = [ob.half_w, ob.half_h, ob.half_d];
            for axis in 0..3 {
                let drawn = scale[axis] * shape.template_half_extent(axis) * 2.0;
                let want = half[axis] * 2.0;
                assert!(
                    (drawn - want).abs() < 1e-5,
                    "{shape:?} 轴 {axis}：画出来 {drawn} m，碰撞 AABB 是 {want} m —— \
                     可见尺寸与碰撞盒不一致（判据见 geom::Shape::template_half_extent）"
                );
            }
            // 盒心必须原样落在障碍中心，不得被缩放带偏
            let t = m.model.w_axis.truncate();
            assert_eq!([t.x, t.y, t.z], [ob.x, ob.y, ob.z]);
        }
    }

    /// 缩放必须是**纯对角**的：模板按轴归一后若还带旋转/剪切，光照法线（屏幕导数重建）
    /// 与阴影深度都会错，而且绕序判定不再成立。
    #[test]
    fn marker_model_is_pure_translation_and_scale() {
        let ob = MapObstacle::new(ObstacleKind::Building, 3.0, -7.0, 1.5, 2.25).geom(Shape::Cylinder);
        let m = WorldMarker::for_obstacle(&ob).model;
        for (i, col) in [m.x_axis, m.y_axis, m.z_axis, m.w_axis].iter().enumerate() {
            for j in 0..3 {
                if i == j || (i == 3 && j != 3) {
                    continue;
                }
                let v = [col.x, col.y, col.z][j];
                assert_eq!(v, 0.0, "模型矩阵第 {i} 列第 {j} 行应为 0，实际 {v}");
            }
        }
    }
}

// ============================================================
// 实例槽位布局单元测试
// ============================================================

#[cfg(test)]
mod instance_slot_layout_tests {
    use super::*;

    /// 槽位布局钉死测试。
    ///
    /// `build.rs` 的两段 WGSL（顶点/网格着色器）里，枪模槽是**字面量** `83009u`，
    /// 而它由 `MAX_MARKER_INSTANCES` 推导。历史上这里已经因为"改了容量忘了改字面量"
    /// 出过两次真 bug（枪槽区间覆盖 NPC 圆柱/球体段 → 四肢和头被 z=0 深度覆盖，
    /// 表现为"鬼魂穿模"）。字面量没法被 Rust 类型系统检查，所以用测试兜住：
    /// 改任何一档容量都必须同时改 build.rs 的两处字面量，否则本测试失败。
    ///
    /// `#[allow(clippy::assertions_on_constants)]`：本条测试**全部内容**就是断言常量之间
    /// 的关系，这正是它的价值所在。clippy 那条 lint 针对的是 `assert!(true)` 这类笔误，
    /// 套不到故意钉死布局的回归护栏上——删掉才是丢保护。
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn gun_slot_layout_is_pinned() {
        assert_eq!(MARKER_SLOT_BASE, 65537, "marker 区起点 = 地形 identity 之后一槽");
        assert_eq!(NPC_SLOT_BASE, 65537 + 8192);
        assert_eq!(NPC_CYL_SLOT_BASE, 73729 + 3072);
        assert_eq!(NPC_SPH_SLOT_BASE, 73729 + 6144);
        assert_eq!(EMISSIVE_SLOT_BASE, 73729 + 9216);
        assert_eq!(
            GUN_INSTANCE_INDEX, 83009,
            "枪槽变了：必须同步改 build.rs 里两处 `== 83009u` 字面量"
        );
        // 枪槽必须紧贴自发光区之后、且落在自发光区间之外，否则会被当成自发光刷白。
        assert_eq!(GUN_INSTANCE_INDEX, EMISSIVE_SLOT_BASE + MAX_EMISSIVE_INSTANCES);
        assert!(
            GUN_INSTANCE_INDEX >= EMISSIVE_SLOT_BASE + 64,
            "枪槽落进自发光区间会被判成 emissive（历史 bug）"
        );
    }

    /// 🔴 `build.rs` 两段 WGSL 里的**槽位常量必须与 Rust 侧逐值相等**，且各自必须恰好两份。
    ///
    /// 存在理由（2026-10-02）：`build.rs` 的 `NPC_INSTANCE_BASE` 那行注释写着
    /// 「改容量必须同步改本行两处副本 + renderer.rs + 枪槽字面量，见 `gun_slot_layout_is_pinned`」——
    /// **但那条测试只比 Rust 常量之间是否自洽，看不见 WGSL 里的字面量**
    /// （build script 与 crate 是两个编译单元，此前没有任何测试读过 `build.rs` 的槽位块）。
    /// 于是「改了 `MAX_MARKER_INSTANCES` 忘了改 WGSL」会**编译通过、测试全绿**，
    /// 而 GPU 侧的 marker 带边界与 CPU 上传错位 ⇒ marker 被当成 NPC/自发光来画。
    /// 这正是 §22.14（PT marker 盒半尺寸）那一族「改了生产者、漏改消费者」的错法形状。
    ///
    /// 自检（教训 27：判据必须能红）：每个名字**必须找到恰好 2 份**——
    /// 解析到 0 份或 1 份都直接红，绝不允许"没解析到"被当成通过。
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn wgsl_slot_constants_match_the_rust_layout() {
        /// 收集 WGSL 里所有 `const NAME: u32 = EXPR;`，返回 (名字 → 全部副本的 EXPR, 名字 → 求值)。
        /// 🔴 两个坑：① EXPR 可以是 `65536u + 1u`，也可以是 `NPC_INSTANCE_BASE + 3072u` ——
        /// **标识项必须递归查表求值**，只把数字挑出来相加会静默丢掉标识项，
        /// 于是 `NPC_CYL_BASE` 被算成 3072（而不是 76801）；第一版就栽在这里。
        /// ② WGSL 的 `65536u` 带 `u` 后缀，`parse::<u64>()` 会**直接失败** ⇒ 必须先去掉它。
        /// 本地模拟器：`target/parsim.py`（跑一次即可复现下面 6 个期望值）。
        fn wgsl_consts(
            src: &str,
        ) -> (
            std::collections::HashMap<String, Vec<String>>,
            std::collections::HashMap<String, u64>,
        ) {
            let mut exprs: std::collections::HashMap<String, Vec<String>> = Default::default();
            for line in src.lines() {
                let t = line.trim_start();
                let Some(rest) = t.strip_prefix("const ") else { continue };
                let Some(colon) = rest.find(':') else { continue };
                let name = &rest[..colon];
                let Some(after) = rest[colon..].strip_prefix(": u32 = ") else { continue };
                let Some(semi) = after.find(';') else { continue };
                exprs
                    .entry(name.to_string())
                    .or_default()
                    .push(after[..semi].trim().to_string());
            }
            // 迭代求值到不动点（槽位布局是单向依赖，几轮就收敛）。
            let mut values: std::collections::HashMap<String, u64> = Default::default();
            loop {
                let mut progressed = false;
                for (name, es) in &exprs {
                    if values.contains_key(name) {
                        continue;
                    }
                    let mut sum = 0u64;
                    let mut done = true;
                    for term in es[0].split('+') {
                        let token: String = term
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if token.is_empty() {
                            done = false;
                            break;
                        }
                        // WGSL 整数字面量的 `u` 后缀：去掉后再解析。
                        let (digits, bare) = match token.strip_suffix('u') {
                            Some(d) if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) => {
                                (Some(d), false)
                            }
                            _ => (None, token.chars().all(|c| c.is_ascii_digit())),
                        };
                        if let Some(d) = digits {
                            sum += d.parse::<u64>().unwrap_or(0);
                        } else if bare {
                            sum += token.parse::<u64>().unwrap_or(0);
                        } else if let Some(v) = values.get(&token) {
                            sum += v;
                        } else {
                            done = false;
                            break;
                        }
                    }
                    if done {
                        values.insert(name.clone(), sum);
                        progressed = true;
                    }
                }
                if !progressed {
                    break;
                }
            }
            (exprs, values)
        }

        let build = std::fs::read_to_string("build.rs")
            .expect("读 build.rs 失败（测试工作目录应为仓库根）");
        let (exprs, values) = wgsl_consts(&build);
        let cases: [(&str, u32); 5] = [
            ("MARKER_INSTANCE_BASE", MARKER_SLOT_BASE),
            ("NPC_INSTANCE_BASE", NPC_SLOT_BASE),
            ("NPC_CYL_BASE", NPC_CYL_SLOT_BASE),
            ("NPC_SPH_BASE", NPC_SPH_SLOT_BASE),
            ("EMISSIVE_INSTANCE_BASE", EMISSIVE_SLOT_BASE),
        ];
        for (name, want) in cases {
            let es = exprs.get(name).cloned().unwrap_or_default();
            assert_eq!(
                es.len(),
                2,
                "WGSL `const {}` 应有 2 份声明（顶点段 + mesh 段），实际 {} 份 \
                 ⇒ 有副本被改名/删除/挪走，本测试需同步更新，不要直接放宽断言",
                name,
                es.len()
            );
            assert_eq!(
                es[0], es[1],
                "`{}` 的两份 WGSL 副本表达式不再逐字符相同（{:?} vs {:?}）\
                 ⇒ 顶点段与 mesh 段对槽位带的理解已经分叉",
                name,
                es[0],
                es[1]
            );
            let got = values.get(name).unwrap_or_else(|| {
                panic!("`const {}` 无法求值（表达式引用了表外的名字）⇒ 解析器需更新", name)
            });
            assert_eq!(
                *got,
                want as u64,
                "WGSL `const {}` = {} 与 Rust 侧 {} 不一致 \
                 ⇒ GPU 的槽位带边界与 CPU 上传错位，marker 会被当成 NPC/自发光来画",
                name,
                got,
                want
            );
        }

        // 枪槽在 WGSL 里是**裸字面量**（顶点段 `instance_index == 83009u`、
        // mesh 段 `slot == 83009u`），Rust 侧由 MAX_* 推导 ⇒ 这是最容易漏的一对。
        let gun_literals = build.matches("83009u").count();
        assert_eq!(
            gun_literals, 2,
            "build.rs 里枪槽字面量 `83009u` 应有 2 处（顶点段 + mesh 段），实际 {} 处",
            gun_literals
        );
        assert_eq!(
            GUN_INSTANCE_INDEX, 83_009,
            "Rust 侧 GUN_INSTANCE_INDEX 已变，但 WGSL 里的 `83009u` 是按旧值写死的 \
             ⇒ 改容量必须同时改 build.rs 两处字面量"
        );

        // 地形 identity 槽：Rust 侧没有同名常量（只在注释里提到），所以只钉 WGSL 两份副本同值。
        let terr = exprs.get("TERRAIN_INSTANCE_INDEX").cloned().unwrap_or_default();
        assert_eq!(
            terr.len(),
            2,
            "`TERRAIN_INSTANCE_INDEX` 应有 2 份 WGSL 声明，实际 {} 份",
            terr.len()
        );
        assert_eq!(
            values.get("TERRAIN_INSTANCE_INDEX").copied(),
            Some(65_536),
            "WGSL 的 `TERRAIN_INSTANCE_INDEX` 必须是 65536（shader 硬编码读该槽）"
        );
    }

    /// marker 区不得越界侵占 NPC 区（曾因 1024/3072 写错导致前 2048 个 NPC 盒体被当 marker）。
    #[test]
    fn marker_band_does_not_bleed_into_npc_band() {
        assert_eq!(NPC_SLOT_BASE - MARKER_SLOT_BASE, MAX_MARKER_INSTANCES);
        assert_eq!(EMISSIVE_SLOT_BASE - NPC_SLOT_BASE, MAX_NPC_INSTANCES * 3);
    }
}

// ============================================================
// 地形 LOD 单元测试
// ============================================================

#[cfg(test)]
mod terrain_lod_tests {
    use super::*;

    #[test]
    fn terrain_lod_level_selection_by_distance() {
        // 近距离 → 高级
        assert_eq!(terrain_lod_for_distance(0.0), TerrainLod::High);
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_HIGH_END - 1.0),
            TerrainLod::High
        );
        // 中距离 → 中级
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_HIGH_END),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_MED_END - 1.0),
            TerrainLod::Medium
        );
        // 远距离 → 低级
        assert_eq!(
            terrain_lod_for_distance(TERRAIN_LOD_MED_END),
            TerrainLod::Low
        );
        assert_eq!(terrain_lod_for_distance(f32::MAX), TerrainLod::Low);
    }

    #[test]
    fn terrain_lod_density_table() {
        // 🔴 2026-09-26：三档密度整体降到 128/64/32 格（间距 4/8/16m）—— 理由与实测见
        // `TERRAIN_LOD_CELLS` 的注释（玩家恒在地图中心 ⇒ 恒选最细一级 ⇒ 为平坦城市铺 131k 三角形）。
        // 高级 129²（128 格，间距 4.0）
        assert_eq!(TerrainLod::High.cells(), 128);
        assert_eq!(TerrainLod::High.verts(), 129);
        assert_eq!(TerrainLod::High.cell_size(), 4.0);
        assert_eq!(TerrainLod::High.index_count(), (128 * 128 * 6) as u32);
        // 中级 65²（64 格，间距 8.0）
        assert_eq!(TerrainLod::Medium.cells(), 64);
        assert_eq!(TerrainLod::Medium.verts(), 65);
        assert_eq!(TerrainLod::Medium.cell_size(), 8.0);
        // 低级 33²（32 格，间距 16.0）
        assert_eq!(TerrainLod::Low.cells(), 32);
        assert_eq!(TerrainLod::Low.verts(), 33);
        assert_eq!(TerrainLod::Low.cell_size(), 16.0);
        assert_eq!(TerrainLod::Low.index_count(), (32 * 32 * 6) as u32);
    }

    #[test]
    fn terrain_lod_blend_morphs_between_levels() {
        // 过渡带端点：blend 0→1，smoothstep 中点 = 0.5
        assert_eq!(terrain_lod_blend(0.0), (TerrainLod::High, 0.0));
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_MORPH_START),
            (TerrainLod::High, 0.0)
        );
        let mid = (TERRAIN_LOD_HIGH_MORPH_START + TERRAIN_LOD_HIGH_END) * 0.5;
        let (level, blend) = terrain_lod_blend(mid);
        assert_eq!(level, TerrainLod::High);
        assert!((blend - 0.5).abs() < 1e-4, "blend={}", blend);

        // 级别边界：High@blend→1 与 Medium@blend=0 几何重合，无 popping
        let (level, blend) = terrain_lod_blend(TERRAIN_LOD_HIGH_END - 0.5);
        assert_eq!(level, TerrainLod::High);
        assert!(blend > 0.99, "blend={}", blend);
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_END),
            (TerrainLod::Medium, 0.0)
        );
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_HIGH_END + 1.0),
            (TerrainLod::Medium, 0.0)
        );

        let (level, blend) = terrain_lod_blend(TERRAIN_LOD_MED_END - 0.5);
        assert_eq!(level, TerrainLod::Medium);
        assert!(blend > 0.99, "blend={}", blend);
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_MED_END),
            (TerrainLod::Low, 1.0)
        );
        assert_eq!(
            terrain_lod_blend(TERRAIN_LOD_MED_END + 1.0),
            (TerrainLod::Low, 1.0)
        );

        // 全距离扫描：级别只前进不后退，同级内 blend 单调不减
        let mut last_blend = -1.0f32;
        let mut last_level = 0usize;
        for i in 0..=1000 {
            let dist = i as f32 * 3.0;
            let (level, blend) = terrain_lod_blend(dist);
            let level_idx = level as usize;
            assert!(level_idx >= last_level, "级别回退 at dist={}", dist);
            if level_idx == last_level {
                assert!(blend + 1e-6 >= last_blend, "blend 回退 at dist={}", dist);
            }
            last_blend = blend;
            last_level = level_idx;
        }
    }

    #[test]
    fn terrain_coarse_height_interpolates_coarse_surface() {
        // 低级网格高度
        let low_cells = TerrainLod::Low.cells();
        let w = low_cells + 1;
        let cell = TerrainLod::Low.cell_size();
        let mut heights = Vec::with_capacity(w * w);
        for iz in 0..w {
            for ix in 0..w {
                let x = -TERRAIN_HALF + ix as f32 * cell;
                let z = -TERRAIN_HALF + iz as f32 * cell;
                heights.push(terrain_height(x, z));
            }
        }

        // 中级网格中与低级网格重合的顶点（ix/iz 均为偶数）：
        // 粗曲面插值必须精确等于该点 terrain_height
        let med_w = TerrainLod::Medium.verts();
        let med_cell = TerrainLod::Medium.cell_size();
        for iz in 0..med_w {
            for ix in 0..med_w {
                if ix % 2 != 0 || iz % 2 != 0 {
                    continue;
                }
                let x = -TERRAIN_HALF + ix as f32 * med_cell;
                let z = -TERRAIN_HALF + iz as f32 * med_cell;
                let interp = terrain_coarse_height(x, z, &heights, low_cells);
                let direct = terrain_height(x, z);
                assert!(
                    (interp - direct).abs() < 1e-4,
                    "({}, {}): interp={} direct={}",
                    x,
                    z,
                    interp,
                    direct
                );
            }
        }

        // 低级 cell 中心位于对角线上：插值 = 两对角角点高度均值（两三角形一致）
        let cx = 3usize;
        let cz = 5usize;
        let x = -TERRAIN_HALF + (cx as f32 + 0.5) * cell;
        let z = -TERRAIN_HALF + (cz as f32 + 0.5) * cell;
        let h00 = heights[cz * w + cx];
        let h11 = heights[(cz + 1) * w + cx + 1];
        let interp = terrain_coarse_height(x, z, &heights, low_cells);
        assert!((interp - (h00 + h11) * 0.5).abs() < 1e-5);
    }
}

// ============================================================
// 程序化地形高度单元测试
// ============================================================

#[cfg(test)]
mod terrain_height_tests {
    use super::*;

    #[test]
    fn terrain_height_deterministic_and_same_source() {
        // 同参数同输入同输出（单测/回放依赖）；terrain_height_at 与 terrain_height 同源
        for &(x, z) in &[
            (0.0, 0.0),
            (30.0, -30.0),
            (120.0, 80.0),
            (150.0, 10.0),
            (-200.0, 250.0),
            (255.0, -255.0),
        ] {
            assert_eq!(terrain_height(x, z), terrain_height(x, z), "({},{})", x, z);
            assert_eq!(
                terrain_height_at(x, z),
                terrain_height(x, z),
                "({},{})",
                x,
                z
            );
        }
    }

    #[test]
    fn terrain_flat_within_central_and_ring_zones() {
        // 中央 60×60（|x|≤30 且 |z|≤30）恒 y=0
        for &x in &[-30.0, -15.0, 0.0, 15.0, 30.0] {
            for &z in &[-30.0, 0.0, 30.0] {
                assert_eq!(terrain_height(x, z), 0.0, "central ({},{})", x, z);
            }
        }
        // 半径 ≤ 140m（覆盖障碍环带 58–130m 与两军接火区）恒 y=0
        for &(x, z) in &[
            (0.0, 140.0),
            (140.0, 0.0),
            (-140.0, 0.0),
            (0.0, -140.0),
            (90.0, 107.0),
            (-90.0, -107.0),
        ] {
            assert_eq!(terrain_height(x, z), 0.0, "ring ({},{})", x, z);
        }
    }

    #[test]
    fn terrain_hills_bounded_varied_and_gentle() {
        // 全图扫描（间距 2m，与 High LOD 网格同采样）：|高度| ≤ 15m、
        // 相邻点高度差 ≤ 0.6m（坡度 ≤ ~17°，平缓，LOD morph 不突兀）
        let mut max_h = 0.0f32;
        let mut ring_max = 0.0f32;
        for iz in 0..=255usize {
            for ix in 0..=255usize {
                let x = -TERRAIN_HALF + ix as f32 * 2.0;
                let z = -TERRAIN_HALF + iz as f32 * 2.0;
                let h = terrain_height(x, z);
                assert!(
                    h.abs() <= TERRAIN_HILL_AMPLITUDE + 1e-6,
                    "|h|={} 超限 at ({},{})",
                    h,
                    x,
                    z
                );
                max_h = max_h.max(h.abs());
                let r = (x * x + z * z).sqrt();
                if r >= 250.0 {
                    ring_max = ring_max.max(h.abs());
                }
                if ix < 255 {
                    let dx = terrain_height(x + 2.0, z) - h;
                    assert!(dx.abs() <= 0.6, "dx={} at ({},{})", dx, x, z);
                }
                if iz < 255 {
                    let dz = terrain_height(x, z + 2.0) - h;
                    assert!(dz.abs() <= 0.6, "dz={} at ({},{})", dz, x, z);
                }
            }
        }
        // 外围确实有起伏（防回退成全平）
        assert!(ring_max > 1.0, "外围丘陵应有起伏，实际 ring_max={}", ring_max);
        assert!(max_h > 1.0, "全图应有非零地形，实际 max_h={}", max_h);
    }
}

// ============================================================
// 画质预设单元测试
// ============================================================

#[cfg(test)]
mod quality_preset_tests {
    use super::*;

    #[test]
    fn quality_medium_matches_existing_constants() {
        // Medium 必须保持当前行为：阈值与现有常量完全一致
        let p = quality_params(QualityPreset::Medium);
        assert_eq!(p.terrain_lod_high_end, TERRAIN_LOD_HIGH_END);
        assert_eq!(p.terrain_lod_med_end, TERRAIN_LOD_MED_END);
        assert_eq!(p.terrain_lod_high_morph_start, TERRAIN_LOD_HIGH_MORPH_START);
        assert_eq!(p.terrain_lod_med_morph_start, TERRAIN_LOD_MED_MORPH_START);
        assert_eq!(p.instance_lod_distance, LOD_DISTANCE);
    }

    #[test]
    fn quality_low_medium_high_ordering() {
        // Low 阈值减小、High 阈值增大
        let low = quality_params(QualityPreset::Low);
        let med = quality_params(QualityPreset::Medium);
        let high = quality_params(QualityPreset::High);
        assert!(low.terrain_lod_high_end < med.terrain_lod_high_end);
        assert!(med.terrain_lod_high_end < high.terrain_lod_high_end);
        assert!(low.terrain_lod_med_end < med.terrain_lod_med_end);
        assert!(med.terrain_lod_med_end < high.terrain_lod_med_end);
        assert!(low.instance_lod_distance < med.instance_lod_distance);
        assert!(med.instance_lod_distance < high.instance_lod_distance);
        // morph 过渡带起点必须小于对应终点
        assert!(low.terrain_lod_high_morph_start < low.terrain_lod_high_end);
        assert!(low.terrain_lod_med_morph_start < low.terrain_lod_med_end);
        assert!(high.terrain_lod_high_morph_start < high.terrain_lod_high_end);
        assert!(high.terrain_lod_med_morph_start < high.terrain_lod_med_end);
    }

    #[test]
    fn quality_preset_default_and_label() {
        assert_eq!(QualityPreset::DEFAULT, QualityPreset::Medium);
        assert_eq!(QualityPreset::Low.label(), "低画质");
        assert_eq!(QualityPreset::Medium.label(), "中画质");
        assert_eq!(QualityPreset::High.label(), "高画质");
    }

    #[test]
    fn terrain_lod_switch_uses_quality_params() {
        // 同一距离下不同画质的 LOD 级别不同：距离 90 时 Low 已降级、Medium/High 仍高级
        let low = quality_params(QualityPreset::Low);
        let med = quality_params(QualityPreset::Medium);
        let high = quality_params(QualityPreset::High);
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, low),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, med),
            TerrainLod::High
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(90.0, high),
            TerrainLod::High
        );
        // 距离 150：Medium 已中级；High 高阈值 145，140 时仍高级、150 时降为中级
        assert_eq!(
            terrain_lod_for_distance_with_params(150.0, med),
            TerrainLod::Medium
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(140.0, high),
            TerrainLod::High
        );
        assert_eq!(
            terrain_lod_for_distance_with_params(150.0, high),
            TerrainLod::Medium
        );
    }
}

// ============================================================
// PNG 截图纯逻辑单元测试
// ============================================================

/// 水平面绕序回归守卫（2026-09-19）。
///
/// 本管线 `FrontFace::CLOCKWISE` + 着色器 Y 翻转：水平面**从上方可见 ⇔ (x,z) 有向面积 > 0**，
/// 以地面 quad 为参照。立方体顶/底面与 mesh 圆柱盖曾按相反约定绕序 ⇒ 顶面从上方恒被
/// 背面剔除，每个 marker 盒子实际是"顶面开口的盒子"：底面埋地的（喷泉池沿/水面、花坛、
/// 路缘石、碑座下两级）从开口露出地面，读作"坑"——追了两天的池子"坑"与柱头"管口"全是它。
/// 悬空盒子的"顶面"其实一直是透过开口看到的**底面**（平着色 + 法线翻向让它无从分辨）。
#[cfg(test)]
mod horizontal_winding_tests {
    use super::{Renderer, GROUND_INDICES, GROUND_VERTS, INDICES, VERTICES};

    /// 三角形在 (x,z) 平面的有向面积（正 = 与地面 quad 同绕序 = 从上方可见）。
    fn area_xz(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0])
    }

    #[test]
    fn ground_reference_and_cube_horizontal_faces_follow_it() {
        let g = |i: usize| GROUND_VERTS[GROUND_INDICES[i] as usize].pos;
        assert!(
            area_xz(g(0), g(1), g(2)) > 0.0,
            "地面 quad 自身绕序变了 —— 这条守卫失效，按新约定重写"
        );
        let v = |i: usize| VERTICES[INDICES[i] as usize].pos;
        for t in (24..30).step_by(3) {
            assert!(
                area_xz(v(t), v(t + 1), v(t + 2)) > 0.0,
                "立方体顶面 INDICES[{t}..] 与地面反绕 → 从上方被剔除（开口盒 bug 复发）"
            );
        }
        for t in (30..36).step_by(3) {
            assert!(
                area_xz(v(t), v(t + 1), v(t + 2)) < 0.0,
                "立方体底面 INDICES[{t}..] 从上方可见 → 盒子会被看穿"
            );
        }
    }

    #[test]
    fn cylinder_caps_follow_the_ground_winding() {
        let (verts, indices) = Renderer::cylinder_mesh_data();
        let pos = |i: u32| verts[i as usize].pos;
        // 每段 12 个索引：6 侧壁 + 3 上盖 + 3 下盖
        let segs = (indices.len() / 12) as u32;
        for i in 0..segs {
            let t = (i * 12 + 6) as usize;
            assert!(
                area_xz(pos(indices[t]), pos(indices[t + 1]), pos(indices[t + 2])) > 0.0,
                "圆柱上盖第 {i} 片从上方被剔除（柱头会读成管口）"
            );
            let b = (i * 12 + 9) as usize;
            assert!(
                area_xz(pos(indices[b]), pos(indices[b + 1]), pos(indices[b + 2])) < 0.0,
                "圆柱下盖第 {i} 片从上方可见（空心会被看穿）"
            );
        }
    }

    #[test]
    fn mesh_shader_horizontal_winding_matches_cpu() {
        let src = include_str!("../../../build.rs");
        for pat in [
            "vec3<u32>(16u, 18u, 17u), vec3<u32>(16u, 19u, 18u)",
            "vec3<u32>(20u, 22u, 21u), vec3<u32>(20u, 23u, 22u)",
            "vec3<u32>(48u, i + 24u, ((i + 1u) % 24u) + 24u)",
            "vec3<u32>(49u, ((i + 1u) % 24u), i)",
        ] {
            assert!(
                src.contains(pat),
                "mesh 着色的水平面绕序与 CPU 不一致，缺 `{pat}`（两条路径必须同约定）"
            );
        }
    }

    /// 🔴 两条管线的**几何模板必须逐值相同**，且每张表都要"真的被解析到"。
    ///
    /// 存在理由（2026-10-02）：`build.rs` 的 mesh 路径自带一整套 `CUBE_POS/CUBE_TRI/
    /// ICO_POS/SPH_POS/...`，与 CPU 侧 `VERTICES/INDICES`、`cylinder_mesh_data()`
    /// 是**两份独立维护的副本**；原有的 `mesh_shader_horizontal_winding_matches_cpu`
    /// 只做 `src.contains("vec3<u32>(16u, 18u, 17u)")` 这类**子串比对**——
    /// 既不比数值、也只覆盖水平面。改一侧忘改另一侧 ⇒ 回退路径与主路径画的不是同一个东西。
    ///
    /// ⚠ **本测试刻意不断言"绕序朝外"**：
    ///   立方体 4 个侧面原始法线朝外、顶/底两面朝内（`renderer.rs:14664` 自述
    ///   "曾因反绕被上方剔除"，后改用 `xz` 有向面积约定）⇒ 只看立方体，
    ///   "把叉积的 y 分量取负"很像是本引擎的真实规则。**但它对倾斜面无效**：
    ///   拿那条规则判二十面体会得到"8/20 朝内"，而一条**与渲染约定无关**的拓扑判据
    ///   （可定向闭合网格的每条无向边必须被两个三角形**反向**共享）证明
    ///   `ICO_TRI` / `SPH_TRI` 完全可定向且闭合 ⇒ 那个 8/20 是**我的模型错，不是几何错**（§37）。
    ///   ⇒ 在没有一个"差异局部于案发区"的 GPU 实验定死引擎真实前向规则之前，
    ///     任何朝外判据都是猜的；硬写出来只会得到一个要么常红、要么被后人删掉的假守卫。
    ///   另注：立方体**不能**用那条拓扑判据自查——它是"每面 4 个独立顶点"（24 顶点 / 6 面），
    ///     面与面之间没有公共索引边，跑拓扑判据必然得到 24 条"未闭合边"的**假红**。
    ///
    /// 自检（教训 27：判据必须能红）：每张表解析出的三元组数量**必须等于它自己声明的
    /// `array<vec3<T>, N>` 里的 N**；表被改名/解析器坏掉时直接红，
    /// 绝不"没测到就当通过"（§31.6 刚栽过一次）。
    #[test]
    fn procedural_geometry_templates_agree_across_paths() {
        /// 取出 `const NAME: array<vec3<TY>, N> = array<...>( vec3<TY>(a, b, c), ... );`
        /// 里的三元组，连同它声明的 N（用来证明"真的解析到了"）。
        fn table_of(src: &str, name: &str, ty: &str) -> (Vec<[f32; 3]>, usize) {
            let head = format!("const {}:", name);
            let at = src
                .find(&head)
                .unwrap_or_else(|| panic!("build.rs 里找不到表 `{}`（被改名或删了？）", name));
            let close = src[at..]
                .find("\n);")
                .unwrap_or_else(|| panic!("表 `{}` 找不到结尾", name));
            let body = &src[at..at + close];

            // 声明长度：`array<vec3<f32>, 24>` 里的那个 24（注意 `>` 在 `,` **之前**）
            let tag = format!("array<vec3<{}>, ", ty);
            let dpos = body
                .find(&tag)
                .unwrap_or_else(|| panic!("表 `{}` 的声明里没有 `{}`", name, tag));
            let declared: usize = body[dpos + tag.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or_else(|_| panic!("表 `{}` 声明的长度不是数字", name));

            let marker = format!("vec3<{}>(", ty);
            let mut out: Vec<[f32; 3]> = Vec::new();
            let mut pos = 0usize;
            while let Some(rel) = body[pos..].find(&marker) {
                let s = pos + rel + marker.len();
                let Some(re) = body[s..].find(')') else { break };
                let mut vals = [0f32; 3];
                let mut n = 0usize;
                for part in body[s..s + re].split(',') {
                    let compact: String = part.chars().filter(|c| !c.is_whitespace()).collect();
                    let token = compact.strip_suffix('u').unwrap_or(compact.as_str());
                    let v: f32 = token.parse().unwrap_or_else(|_| {
                        panic!("表 `{}` 有解析不了的三元组 `{:?}`", name, &body[s..s + re])
                    });
                    assert!(n < 3, "表 `{}` 的三元组超过 3 个分量", name);
                    vals[n] = v;
                    n += 1;
                }
                assert_eq!(n, 3, "表 `{}` 有不足 3 分量的三元组", name);
                out.push(vals);
                pos = s + re;
            }
            (out, declared)
        }

        /// 校验一张 (顶点, 索引) 表：索引必须落在顶点范围内、且三角形不退化；
        /// 返回检查过的三角形数（用来证明"真的检查了东西"，而不是解析器空转）。
        fn check_indices(verts: &[[f32; 3]], tris: &[[f32; 3]], label: &str) -> usize {
            for (i, t) in tris.iter().enumerate() {
                let (ia, ib, ic) = (t[0] as usize, t[1] as usize, t[2] as usize);
                assert!(
                    ia < verts.len() && ib < verts.len() && ic < verts.len(),
                    "{} 第 {} 个三角形索引越界：{:?}（顶点数 {}）\
                     ⇒ mesh 路径会读到**从未写过的 vertices 槽位**，画出来就是随机三角形",
                    label,
                    i,
                    t,
                    verts.len()
                );
                assert!(
                    ia != ib && ib != ic && ia != ic,
                    "{} 第 {} 个三角形退化（索引重复）：{:?}",
                    label,
                    i,
                    t
                );
            }
            tris.len()
        }

        let src = include_str!("../../../build.rs");
        let mut checked = 0usize;

        // ---- mesh 主管线的三张闭合模板 ----
        for (pos_name, tri_name, n_pos, n_tri) in [
            ("CUBE_POS", "CUBE_TRI", 24usize, 12usize),
            ("ICO_POS", "ICO_TRI", 12usize, 20usize),
            ("SPH_POS", "SPH_TRI", 42usize, 80usize),
        ] {
            let (verts, declared) = table_of(src, pos_name, "f32");
            assert_eq!(
                verts.len(),
                declared,
                "{} 声明 {} 个顶点，实际解析到 {} 个 ⇒ 解析器或表格式变了",
                pos_name,
                declared,
                verts.len()
            );
            assert_eq!(verts.len(), n_pos, "{} 顶点数应当是 {}", pos_name, n_pos);
            let (tris, declared_t) = table_of(src, tri_name, "u32");
            assert_eq!(
                tris.len(),
                declared_t,
                "{} 声明 {} 个三角形，实际解析到 {} 个",
                tri_name,
                declared_t,
                tris.len()
            );
            assert_eq!(tris.len(), n_tri, "{} 三角形数应当是 {}", tri_name, n_tri);
            checked += check_indices(&verts, &tris, pos_name);
        }

        // ---- CPU 侧立方体：直接用常量，不走文本解析（另一条独立路径）----
        let cpu_verts: Vec<[f32; 3]> = VERTICES.iter().map(|v| v.pos).collect();
        let cpu_tris: Vec<[f32; 3]> = INDICES
            .chunks(3)
            .map(|t| [t[0] as f32, t[1] as f32, t[2] as f32])
            .collect();
        checked += check_indices(&cpu_verts, &cpu_tris, "CPU VERTICES/INDICES");

        // ---- CPU 侧圆柱（含上下盖）：真实数据，不是复制公式 ----
        let (cyl_v, cyl_i) = Renderer::cylinder_mesh_data();
        let cyl_verts: Vec<[f32; 3]> = cyl_v.iter().map(|v| v.pos).collect();
        let cyl_tris: Vec<[f32; 3]> = cyl_i
            .chunks(3)
            .map(|t| [t[0] as f32, t[1] as f32, t[2] as f32])
            .collect();
        checked += check_indices(&cyl_verts, &cyl_tris, "CPU cylinder_mesh_data");

        // ---- 两条管线的立方体模板必须逐值相同，否则回退路径与主路径画的不是同一个东西 ----
        let (mesh_cube, _) = table_of(src, "CUBE_POS", "f32");
        for i in 0..24 {
            assert_eq!(
                mesh_cube[i], cpu_verts[i],
                "CUBE_POS[{}] {:?} 与 CPU VERTICES[{}] {:?} 不一致 \
                 ⇒ 两条管线的盒子不是同一个几何",
                i,
                mesh_cube[i],
                i,
                cpu_verts[i]
            );
        }
        let (mesh_cube_tri, _) = table_of(src, "CUBE_TRI", "u32");
        for i in 0..12 {
            assert_eq!(
                mesh_cube_tri[i], cpu_tris[i],
                "CUBE_TRI[{}] {:?} 与 CPU INDICES 第 {} 个三角形 {:?} 不一致",
                i,
                mesh_cube_tri[i],
                i,
                cpu_tris[i]
            );
        }

        assert!(
            checked >= 200,
            "本测试只检查了 {} 个三角形（应 >= 200）⇒ 解析器或表结构变了，\
             此时'全部通过'可能只是因为**什么都没测到**（§31.6 的教训）",
            checked
        );
    }
}

/// 并行剔除**段数上限**的判据（2026-09-22 复查新增）。
///
/// `cull_and_upload` 的两张前缀和表是栈上定长 `[u32; CULL_MAX_SEGMENTS]`，
/// 段数必须 ≤ 表长。旧写法是裸的 `pool.workers() + 1`，只有一句
/// `debug_assert!(nw <= 64)` 兜着 —— 而 **release 里 `debug_assert` 不存在**
/// ⇒ 64 个 worker 以上的机器（64C/128T 起）每帧 `index out of bounds`。
#[cfg(test)]
mod cull_segment_tests {
    use super::{cull_segment_count, CULL_MAX_SEGMENTS};

    #[test]
    fn segment_count_is_workers_plus_one_within_limit() {
        // 本机这一类（16C 级）与旧公式逐值一致 ⇒ 改动是空操作
        assert_eq!(cull_segment_count(0), 1, "0 个 worker 也要有 1 段（调用线程自己跑）");
        assert_eq!(cull_segment_count(3), 4);
        assert_eq!(cull_segment_count(31), 32);
        // 边界：刚好填满表
        assert_eq!(cull_segment_count(CULL_MAX_SEGMENTS - 1), CULL_MAX_SEGMENTS);
        // 🔴 这一条是"修之前会红"的判据：旧公式在这里给 65 / 1001
        assert_eq!(cull_segment_count(CULL_MAX_SEGMENTS), CULL_MAX_SEGMENTS);
        assert_eq!(cull_segment_count(1000), CULL_MAX_SEGMENTS);
    }

    /// 上限本身：低于 64 只会让"每核一段"的机器少用几个核（慢一点，结果不变），
    /// 但不该被顺手调小 —— 顺带锁住它的量级。
    #[test]
    fn limit_covers_common_topologies() {
        assert!(
            CULL_MAX_SEGMENTS >= 64,
            "段数上限被调小了：常见 32C64T 拓扑会退化成段数不足（只是少并行，不会算错）"
        );
    }
}

