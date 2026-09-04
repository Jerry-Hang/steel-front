//! GLB 世界道具：命名网格集合 + 由网格实测包围盒推导的碰撞足迹。
//!
//! ## 为什么不再有 props.toml 清单
//! `assets.rs` 的模块头原本写着「摆放：assets/props.toml」。这里刻意不做清单：
//! 清单要重复记一遍尺寸和碰撞盒，而这两样**从已加载的网格本身就能量出来**。多写一份
//! 数据就多一条会脱同步的路——资产在 `tools/blender/gen_props.py` 里改个高度，清单就会
//! 悄悄说谎，而没人会去核对它。所以尺寸、底面原点、足迹一律实测。
//!
//! ## 摆放数据从哪来
//! 由 `engine::city` 的街区生成器产出 [`PropPlacement`] 列表（位置/朝向/缩放），
//! 与地图布局同源，不落盘。
//!
//! ## 碰撞
//! 物理与导航只认 AABB（见 `game::MapObstacle`）。旋转盒的轴对齐包围盒有精确闭式解
//! （半宽 = |cosθ|·hw + |sinθ|·hd），所以任意朝向都能给出**准确**而非保守的碰撞盒，
//! 不需要把 yaw 强行量化到 90°。

use crate::engine::assets::ImportedMesh;

/// 单个道具网格 + 实测包围盒（米，Y 向上，原点在底面中心）。
#[derive(Debug, Clone)]
pub struct PropMesh {
    pub name: String,
    pub verts: Vec<[f32; 11]>,
    pub indices: Vec<u32>,
    /// 包围盒最小角 (x, y, z)
    pub min: [f32; 3],
    /// 包围盒最大角 (x, y, z)
    pub max: [f32; 3],
}

impl PropMesh {
    fn from_imported(name: &str, m: &ImportedMesh) -> Self {
        let mut mesh = PropMesh {
            name: name.to_string(),
            verts: m.verts.clone(),
            indices: m.indices.clone(),
            min: [0.0; 3],
            max: [0.0; 3],
        };
        if m.verts.is_empty() {
            return mesh;
        }
        mesh.min = m.verts[0][..3].try_into().unwrap();
        mesh.max = mesh.min;
        for v in &m.verts {
            for k in 0..3 {
                mesh.min[k] = mesh.min[k].min(v[k]);
                mesh.max[k] = mesh.max[k].max(v[k]);
            }
        }
        mesh
    }

    /// 底面中心到几何中心的偏移（米，竖直方向）。贴地摆放时用得上。
    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }

    /// 未旋转时的水平半足迹 (half_x, half_z)。
    pub fn half_footprint(&self) -> (f32, f32) {
        (
            (self.max[0] - self.min[0]) * 0.5,
            (self.max[2] - self.min[2]) * 0.5,
        )
    }
}

/// 一套道具网格，按名字排序加载，保证多次运行顺序一致（实例槽位依赖下标）。
#[derive(Debug, Default, Clone)]
pub struct PropSet {
    pub meshes: Vec<PropMesh>,
}

impl PropSet {
    /// 扫描目录加载全部 `.glb`。名字取文件名（去扩展名）。排序后返回，使下标稳定。
    pub fn load_dir(dir: &str) -> Result<Self, String> {
        let mut found: Vec<(String, ImportedMesh)> = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("读取 {dir} 失败: {e}"))?;
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("glb") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unnamed")
                .to_string();
            let bytes = std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
            let m = crate::engine::assets::parse_glb(&bytes)
                .map_err(|err| format!("{} 解析失败: {err}", path.display()))?;
            if m.verts.is_empty() || m.indices.is_empty() {
                return Err(format!("{} 是空网格", path.display()));
            }
            found.push((stem, m));
        }
        // 稳定顺序：同名文件在不同文件系统上的枚举顺序不保证一致，而实例下标要用它
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let meshes = found.iter().map(|(n, m)| PropMesh::from_imported(n, m)).collect();
        Ok(PropSet { meshes })
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&PropMesh> {
        self.meshes.get(i)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.meshes.iter().position(|m| m.name == name)
    }
}

/// 一次道具摆放。`mesh` 是 [`PropSet`] 下标。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropPlacement {
    pub mesh: usize,
    pub x: f32,
    /// 抬升量（米，相对地形表面）。落地件为 0；堆叠件（第二层集装箱）与需要沿坡
    /// 面抬高的件用它。建模约定原点在底面，所以这里就是"底面离地多高"。
    pub y: f32,
    pub z: f32,
    /// 绕竖直轴旋转（弧度）
    pub yaw: f32,
    /// 等比缩放（1.0 = 建模时的真实米制尺寸）
    pub scale: f32,
    /// 是否进刚体表/导航网格。false = 只画不挡（装饰件），沿用 city.rs 的结构/装饰分表纪律。
    pub solid: bool,
}

impl PropPlacement {
    pub fn new(mesh: usize, x: f32, z: f32, yaw: f32, scale: f32, solid: bool) -> Self {
        PropPlacement { mesh, x, y: 0.0, z, yaw, scale, solid }
    }

    /// 带抬升量的摆放（堆叠、坡地）。
    pub fn at(mesh: usize, x: f32, y: f32, z: f32, yaw: f32, scale: f32, solid: bool) -> Self {
        PropPlacement { mesh, x, y, z, yaw, scale, solid }
    }

    /// 旋转后的精确轴对齐足迹半尺寸。闭式解，非保守放大。
    pub fn rotated_footprint(&self, set: &PropSet) -> Option<(f32, f32)> {
        let m = set.get(self.mesh)?;
        let (hx, hz) = m.half_footprint();
        let (hx, hz) = (hx * self.scale, hz * self.scale);
        let (s, c) = (self.yaw.sin().abs(), self.yaw.cos().abs());
        Some((c * hx + s * hz, s * hx + c * hz))
    }

    /// 该道具的 AABB 参数：(x, z, half_w, half_d, y_center, half_h)。
    pub fn aabb(&self, set: &PropSet) -> Option<(f32, f32, f32, f32, f32, f32)> {
        let m = set.get(self.mesh)?;
        let (hw, hd) = self.rotated_footprint(set)?;
        let half_h = m.height() * self.scale * 0.5;
        // 建模约定是原点在底面，所以底面标高 = 抬升量 + min.y·scale：
        // 允许刻意把件埋进地里一点（min.y<0）来避免与地形平面共面。
        let base = self.y + m.min[1] * self.scale;
        Some((self.x, self.z, hw, hd, base + half_h, half_h))
    }
}

/// 哪些道具应当阻挡子弹与 AI 视线。建筑/大树是硬障碍；小件按玩法需要可摧毁。
/// 集中一处，避免每个调用点各自判断一遍。
pub fn is_solid_prop(name: &str) -> bool {
    !(name.starts_with("rubble") || name.starts_with("bush") || name.starts_with("capture_flag"))
}

/// 全部摆放烘焙成**一份**静态网格的结果。
#[derive(Debug, Default, Clone)]
pub struct MergedGeometry {
    /// 与 `assets::ImportedMesh::verts` 同布局：pos(3) normal(3) uv(2) color(3)
    pub verts: Vec<[f32; 11]>,
    pub indices: Vec<u32>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl MergedGeometry {
    pub fn is_empty(&self) -> bool {
        self.verts.is_empty()
    }
}

/// 把摆放列表烘成一份静态几何。
///
/// ## 为什么在 CPU 上烘死，而不是走实例化
/// 实例化要新增一套实例 buffer、描述符集与管线分支；而道具总量只有几十万三角，
/// 一张静态 VBO + 一次 draw call 就能画完，复用现成的 pos/color/uv 管线即可。
/// 代价是失去逐实例视锥剔除——按当前体量不值得为它多养一条管线。
/// 若日后要恢复剔除，改的就是这个函数，接缝在这里。
///
/// ## 地形跟随
/// 位姿只在**摆放点**采样一次地高，不对顶点逐点抬升：逐点采样会把建筑的山墙
/// 和窗台剪成斜面，比"四脚略有悬空"难看得多。中央 60×60 本来就压平到 y=0，
/// 绝大多数楼与树因此完全贴地。
pub fn merge(
    set: &PropSet,
    placements: &[PropPlacement],
    ground: impl Fn(f32, f32) -> f32,
) -> MergedGeometry {
    let mut out = MergedGeometry::default();
    let total_v: usize = placements
        .iter()
        .filter_map(|p| set.get(p.mesh))
        .map(|m| m.verts.len())
        .sum();
    let total_i: usize = placements
        .iter()
        .filter_map(|p| set.get(p.mesh))
        .map(|m| m.indices.len())
        .sum();
    out.verts.reserve(total_v);
    out.indices.reserve(total_i);
    let mut first = true;
    for p in placements {
        let Some(mesh) = set.get(p.mesh) else { continue };
        let (sy, cy) = (p.yaw.sin(), p.yaw.cos());
        let gy = ground(p.x, p.z) + p.y;
        let base = out.verts.len() as u32;
        for v in &mesh.verts {
            // 绕 +Y 旋转：(x, z) → (x·cosθ + z·sinθ, -x·sinθ + z·cosθ)。
            // 这个**顶点变换本身**是纯旋转+等比缩放+平移，行列式为正，不改变绕序；
            // 下面对索引另有一次刻意交换，那是为了适配引擎的 CLOCKWISE 约定，与此无关。
            let vx = v[0] * p.scale;
            let vy = v[1] * p.scale;
            let vz = v[2] * p.scale;
            let px = p.x + vx * cy + vz * sy;
            let pz = p.z - vx * sy + vz * cy;
            let py = gy + vy;
            let nx = v[3] * cy + v[5] * sy;
            let nz = -v[3] * sy + v[5] * cy;
            let baked = [
                px, py, pz,
                nx, v[4], nz,
                v[6], v[7],
                v[8], v[9], v[10],
            ];
            if first {
                out.min = [px, py, pz];
                out.max = [px, py, pz];
                first = false;
            } else {
                for k in 0..3 {
                    let c = baked[k];
                    out.min[k] = out.min[k].min(c);
                    out.max[k] = out.max[k].max(c);
                }
            }
            out.verts.push(baked);
        }
        for tri in mesh.indices.chunks_exact(3) {
            // **绕序交换**：外部建模的三角形在这里统一翻一次面。
            // 依据是实机对照实验而非推理：主管线是 `cull BACK + front_face CLOCKWISE`，
            // GLB 道具在开剔除时**一个像素都不出现**（截图差分 0.14/255、36 格 0 显著变化），
            // 把 cull 改成 NONE 后所有立面立刻正确显示 —— 说明它的面被整体判成了背面。
            // 引擎自己的程序化网格在 `meshgen.rs` 生成时就按该约定做过索引交换，
            // `parse_glb` 没有；枪模之所以正常，是因为 main.rs 给它的轴修正里恰好带一次翻转。
            // 交换放在这里而不是上传处或着色器里，因为这正是"外部内容进入引擎绕序约定"的边界。
            out.indices.push(tri[0] + base);
            out.indices.push(tri[2] + base);
            out.indices.push(tri[1] + base);
        }
        // 长度不是 3 的倍数的尾部（理论上不该出现）原样搬走：宁可留错也不静默丢三角形
        for i in mesh.indices.chunks_exact(3).remainder() {
            out.indices.push(*i + base);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn kit() -> Option<PropSet> {
        PropSet::load_dir("assets/props").ok()
    }

    #[test]
    fn prop_set_loads_blender_kit_deterministically() {
        let Some(a) = kit() else { return };
        assert!(a.len() >= 10, "应至少加载到 10 件道具，实际 {}", a.len());
        let b = PropSet::load_dir("assets/props").unwrap();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.meshes.iter().zip(b.meshes.iter()) {
            assert_eq!(x.name, y.name, "两次加载顺序必须一致（实例下标依赖它）");
        }
        // 排序不变式
        let names: Vec<&str> = a.meshes.iter().map(|m| m.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "meshes 必须按名排序");
    }

    #[test]
    fn prop_meshes_follow_the_meter_and_base_origin_rules() {
        let Some(set) = kit() else { return };
        for m in &set.meshes {
            let h = m.height();
            assert!(h > 0.05 && h < 40.0, "{} 高度 {h} 不符合米制约定", m.name);
            assert!(
                m.min[1] > -1.0,
                "{} 原点不在底面（min.y={}）",
                m.name,
                m.min[1]
            );
            assert!(m.verts.len() > 24, "{} 顶点过少", m.name);
            let max_idx = *m.indices.iter().max().unwrap();
            assert!(max_idx < m.verts.len() as u32, "{} 索引越界", m.name);
        }
    }

    #[test]
    fn named_props_are_present() {
        let Some(set) = kit() else { return };
        for n in ["building_block", "tree_oak", "container_20ft", "panel_block"] {
            assert!(set.index_of(n).is_some(), "套件缺少 {n}");
        }
    }

    #[test]
    fn footprint_is_axis_aligned_at_zero_yaw() {
        let Some(set) = kit() else { return };
        let i = set.index_of("building_block").unwrap();
        let m = &set.meshes[i];
        let (hx, hz) = m.half_footprint();
        let p = PropPlacement::new(i, 10.0, -4.0, 0.0, 1.0, true);
        let (x, z, hw, hd, _, _) = p.aabb(&set).unwrap();
        assert_eq!((x, z), (10.0, -4.0));
        assert!((hw - hx).abs() < 1e-5 && (hd - hz).abs() < 1e-5);
    }

    #[test]
    fn footprint_swaps_axes_at_ninety_degrees() {
        let Some(set) = kit() else { return };
        let i = set.index_of("building_block").unwrap();
        let (hx, hz) = set.meshes[i].half_footprint();
        assert!(hx > hz, "该测试需要一个非正方形足迹");
        let p = PropPlacement::new(i, 0.0, 0.0, FRAC_PI_2, 1.0, true);
        let (_, _, hw, hd, _, _) = p.aabb(&set).unwrap();
        assert!((hw - hz).abs() < 1e-4, "90° 应交换轴: {hw} vs {hz}");
        assert!((hd - hx).abs() < 1e-4, "90° 应交换轴: {hd} vs {hx}");
    }

    #[test]
    fn footprint_at_forty_five_degrees_is_the_exact_rotated_aabb() {
        let Some(set) = kit() else { return };
        let i = set.index_of("building_block").unwrap();
        let (hx, hz) = set.meshes[i].half_footprint();
        let p = PropPlacement::new(i, 0.0, 0.0, std::f32::consts::FRAC_PI_4, 1.0, true);
        let (_, _, hw, hd, _, _) = p.aabb(&set).unwrap();
        // 45° 时两轴相等，且等于 (hx+hz)/√2
        let want = (hx + hz) / 2f32.sqrt();
        assert!((hw - want).abs() < 1e-4, "45° half_w 应为 {want}，实际 {hw}");
        assert!((hd - want).abs() < 1e-4, "45° half_d 应为 {want}，实际 {hd}");
    }

    #[test]
    fn scale_multiplies_the_footprint_and_lifts_the_centre() {
        let Some(set) = kit() else { return };
        let i = set.index_of("container_20ft").unwrap();
        let one = PropPlacement::new(i, 0.0, 0.0, 0.0, 1.0, true).aabb(&set).unwrap();
        let two = PropPlacement::new(i, 0.0, 0.0, 0.0, 2.0, true).aabb(&set).unwrap();
        // 元组不能用变量下标，摊成数组再逐分量比
        let (o, t): ([f32; 6], [f32; 6]) = (one.into(), two.into());
        for k in 2..6 {
            assert!(
                (t[k] - o[k] * 2.0).abs() < 1e-4,
                "分量 {k} 未随 scale 线性放大: {o:?} vs {t:?}"
            );
        }
    }

    #[test]
    fn solidity_classification_is_stable_and_excludes_debris() {
        assert!(is_solid_prop("building_block"));
        assert!(is_solid_prop("container_20ft"));
        assert!(is_solid_prop("wall_brick"));
        assert!(!is_solid_prop("rubble_pile"), "瓦砾不该挡视线");
        assert!(!is_solid_prop("capture_flag"), "旗面是纯装饰");
    }

    /// 手工造一个只有 3 个顶点的网格，绕序与索引都已知，用来精确核对烘焙结果。
    fn tri_set() -> PropSet {
        let verts = vec![
            [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.1, 0.2, 0.5, 0.25, 0.125],
            [0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 0.3, 0.4, 0.5, 0.25, 0.125],
            [0.0, 3.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.6, 0.5, 0.25, 0.125],
        ];
        PropSet {
            meshes: vec![PropMesh {
                name: "tri".into(),
                verts,
                indices: vec![0, 1, 2],
                min: [0.0, 0.0, 0.0],
                max: [1.0, 3.0, 2.0],
            }],
        }
    }

    #[test]
    fn merge_at_identity_reproduces_source_vertices() {
        let set = tri_set();
        let p = PropPlacement::new(0, 0.0, 0.0, 0.0, 1.0, false);
        let g = merge(&set, &[p], |_, _| 0.0);
        assert_eq!(g.verts.len(), 3);
        // 顶点位置逐位相同，但索引被刻意换了一次面（适配引擎 CLOCKWISE 约定）
        assert_eq!(g.indices, vec![0, 2, 1], "merge 必须交换三角形第二、三个索引");
        for (a, b) in g.verts.iter().zip(set.meshes[0].verts.iter()) {
            assert_eq!(a, b, "零位姿下烘焙结果必须与源网格逐位相同");
        }
    }

    #[test]
    fn merge_rotates_positions_and_normals_about_y() {
        let set = tri_set();
        let p = PropPlacement::new(0, 0.0, 0.0, FRAC_PI_2, 1.0, false);
        let g = merge(&set, &[p], |_, _| 0.0);
        // 源 (1,0,0) 绕 +Y 转 90° → (0,0,-1)
        let v = &g.verts[0];
        assert!(v[0].abs() < 1e-5, "x 应为 0，实际 {}", v[0]);
        assert!((v[2] + 1.0).abs() < 1e-5, "z 应为 -1，实际 {}", v[2]);
        assert!((v[1]).abs() < 1e-5, "y 不受旋转影响，实际 {}", v[1]);
        // 法线 (0,1,0) 保持竖直
        assert!((v[4] - 1.0).abs() < 1e-5, "竖直法线不应被 Y 轴旋转改变");
        // 源 (0,0,2) → (2,0,0)
        let w = &g.verts[1];
        assert!((w[0] - 2.0).abs() < 1e-5 && w[2].abs() < 1e-5, "转后应为 (2,0,0)，实际 {:?}", &w[..3]);
    }

    #[test]
    fn merge_applies_ground_then_placement_lift() {
        let set = tri_set();
        let p = PropPlacement::at(0, 5.0, 1.5, 7.0, 0.0, 1.0, false);
        let g = merge(&set, &[p], |x, z| {
            assert!((x - 5.0).abs() < 1e-5 && (z - 7.0).abs() < 1e-5, "地高应在摆放点采样");
            2.25
        });
        // 源顶点 y=0 的两个点应落在 2.25 + 1.5 = 3.75
        assert!((g.verts[0][1] - 3.75).abs() < 1e-5, "实际 {}", g.verts[0][1]);
        // y=3 的顶点再加上 3·scale
        assert!((g.verts[2][1] - 6.75).abs() < 1e-5, "实际 {}", g.verts[2][1]);
    }

    #[test]
    fn merge_rebases_indices_and_keeps_them_in_range() {
        let set = tri_set();
        let ps = vec![
            PropPlacement::new(0, 0.0, 0.0, 0.0, 1.0, false),
            PropPlacement::new(0, 10.0, 0.0, 0.0, 1.0, false),
            PropPlacement::new(0, 20.0, 0.0, 0.0, 1.0, false),
        ];
        let g = merge(&set, &ps, |_, _| 0.0);
        assert_eq!(g.verts.len(), 9);
        assert_eq!(g.indices.len(), 9);
        // 每件源索引 (0,1,2) 重基后再换面 → (b, b+2, b+1)
        assert_eq!(&g.indices[0..3], &[0, 2, 1], "第一件应重基到 0 并换面");
        assert_eq!(&g.indices[3..6], &[3, 5, 4], "第二件应重基到 3 并换面");
        assert_eq!(&g.indices[6..9], &[6, 8, 7], "第三件应重基到 6 并换面");
        assert!(g.indices.iter().all(|&i| (i as usize) < g.verts.len()));
    }

    #[test]
    fn merge_of_real_kit_is_bounded_and_non_empty() {
        let Some(set) = kit() else { return };
        let ps: Vec<PropPlacement> = (0..set.len())
            .map(|i| PropPlacement::new(i, i as f32 * 30.0, 0.0, 0.3, 1.0, false))
            .collect();
        let g = merge(&set, &ps, |_, _| 0.0);
        assert!(!g.is_empty());
        assert_eq!(g.verts.len(), set.meshes.iter().map(|m| m.verts.len()).sum::<usize>());
        assert_eq!(g.indices.len(), set.meshes.iter().map(|m| m.indices.len()).sum::<usize>());
        assert!(g.indices.iter().all(|&i| (i as usize) < g.verts.len()));
        // 全部套件摊在一条 30m 间隔的直线上，总跨度应当是有限且可预期的量级
        let span = g.max[0] - g.min[0];
        assert!(span > 100.0 && span < 2000.0, "合并后 X 跨度异常：{span}");
        assert!(g.max[1] < 40.0, "合并后高度异常：{}", g.max[1]);
    }
}
