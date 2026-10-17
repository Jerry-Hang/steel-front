//! 程序化网格图元生成器（零外部资产路线）
//!
//! 枪械/NPC 等高模由数学函数生成：圆角盒（beveled box）、锥台/圆柱、球体，
//! 每个顶点带真实法线（供烘焙光照/逐顶点明暗），绕序按"从外侧看 CCW"生成，
//! 与主管线 FrontFace::CLOCKWISE + shader Y 翻转的约定配合（绘制时若背面被
//! 剔除，翻转索引序即可）。
//!
//! 2026-08-16：第一人称枪模从"9 个立方体积木"升级为程序化圆角高模。

/// 顶点：位置 + 法线 + UV + 颜色（颜色由合并方用材质色 × 烘焙光照填充）
#[derive(Debug, Clone, Copy)]
pub struct GVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

/// 程序化网格：局部坐标顶点 + 三角形索引
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub verts: Vec<GVertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// 用矩阵变换全部顶点并追加到 out（索引整体偏移），用于多部件合并。
    /// `tint` 为材质色，`light_dir` 为烘焙光方向（世界空间，法线点积 ≥0 生效），
    /// 输出颜色 = tint × (ambient + diffuse×max(dot(n, L), 0))，让圆角网格有立体明暗。
    pub fn append_transformed(
        &self,
        out_verts: &mut Vec<GVertex>,
        out_indices: &mut Vec<u32>,
        m: glam::Mat4,
        tint: [f32; 3],
        light_dir: glam::Vec3,
        ambient: f32,
        diffuse: f32,
    ) {
        let base = out_verts.len() as u32;
        for v in &self.verts {
            let p = m.transform_point3(glam::Vec3::from(v.pos));
            let n = m
                .transform_vector3(glam::Vec3::from(v.normal))
                .normalize_or_zero();
            let ndl = n.dot(light_dir).max(0.0);
            let shade = ambient + diffuse * ndl;
            let c = [
                (tint[0] * shade).min(1.0),
                (tint[1] * shade).min(1.0),
                (tint[2] * shade).min(1.0),
            ];
            out_verts.push(GVertex { pos: p.to_array(), normal: n.to_array(), uv: v.uv, color: c });
        }
        // 绕序翻转（每 3 个一组交换）：主管线 front_face=CLOCKWISE + cull BACK，
        // 程序化网格按外侧 CCW 生成 → 屏幕空间判定为背面被剔除 → 整体取反适配
        for chunk in self.indices.chunks(3) {
            if chunk.len() == 3 {
                out_indices.push(base + chunk[0]);
                out_indices.push(base + chunk[2]);
                out_indices.push(base + chunk[1]);
            } else {
                for i in chunk {
                    out_indices.push(base + i);
                }
            }
        }
    }
}

/// 圆角盒：中心在原点，尺寸 (w, h, d)，圆角半径 r（< 各半尺寸）
pub fn beveled_box(w: f32, h: f32, d: f32, r: f32, seg: u32) -> Mesh {
    let hw = (w * 0.5 - r).max(0.001);
    let hh = (h * 0.5 - r).max(0.001);
    let hd = (d * 0.5 - r).max(0.001);
    let seg = seg.max(2);
    let mut mesh = Mesh::default();

    // 8 个角：以角心为球心的 1/8 球面片
    let corners = [
        glam::Vec3::new(hw, hh, hd), glam::Vec3::new(-hw, hh, hd),
        glam::Vec3::new(hw, -hh, hd), glam::Vec3::new(-hw, -hh, hd),
        glam::Vec3::new(hw, hh, -hd), glam::Vec3::new(-hw, hh, -hd),
        glam::Vec3::new(hw, -hh, -hd), glam::Vec3::new(-hw, -hh, -hd),
    ];
    for &c in &corners {
        let sx = if c.x > 0.0 { 1.0 } else { -1.0 };
        let sy = if c.y > 0.0 { 1.0 } else { -1.0 };
        let sz = if c.z > 0.0 { 1.0 } else { -1.0 };
        let base = mesh.verts.len() as u32;
        for j in 0..=seg {
            let v = std::f32::consts::FRAC_PI_2 * j as f32 / seg as f32;
            let (sv, cv) = v.sin_cos();
            for i in 0..=seg {
                let u = std::f32::consts::FRAC_PI_2 * i as f32 / seg as f32;
                let (su, cu) = u.sin_cos();
                let n = glam::Vec3::new(sx * su * cv, sy * sv, sz * cu * cv).normalize();
                let pos = c + n * r;
                mesh.verts.push(GVertex {
                    pos: pos.to_array(),
                    normal: n.to_array(),
                    uv: [i as f32 / seg as f32, j as f32 / seg as f32],
                color: [1.0, 1.0, 1.0],
                });
            }
        }
        // 🔴 **8 个角片里有一半是反绕的**（2026-10-17 实测：176 CCW / 172 CW 里的一半）。
        // 角片的参数化是 `n = (sx·su·cv, sy·sv, sz·cu·cv)`，三角形绕序的符号
        // 正比于 `sx·sy·sz` ⇒ **乘积为 -1 的 4 个角必须反绕**。
        // （`append_transformed` 只统一翻转一次，图元内部必须自身一致。）
        let flip = sx * sy * sz < 0.0;
        for j in 0..seg {
            for i in 0..seg {
                let a = base + j * (seg + 1) + i;
                let b = a + 1;
                let c0 = a + seg + 1;
                let d0 = c0 + 1;
                if flip {
                    mesh.indices.extend_from_slice(&[a, d0, b, a, c0, d0]);
                } else {
                    mesh.indices.extend_from_slice(&[a, b, d0, a, d0, c0]);
                }
            }
        }
    }

    // 12 条边：沿轴的 1/4 圆柱带（连接角片）
    let edges: [(u8, f32, f32); 12] = [
        (0, hh, hd), (0, hh, -hd), (0, -hh, hd), (0, -hh, -hd),
        (1, hw, hd), (1, hw, -hd), (1, -hw, hd), (1, -hw, -hd),
        (2, hw, hh), (2, hw, -hh), (2, -hw, hh), (2, -hw, -hh),
    ];
    for &(axis, a, b) in &edges {
        let (p0, p1): (glam::Vec3, glam::Vec3) = match axis {
            0 => (glam::Vec3::new(-hw, a, b), glam::Vec3::new(hw, a, b)),
            1 => (glam::Vec3::new(a, -hh, b), glam::Vec3::new(a, hh, b)),
            _ => (glam::Vec3::new(a, b, -hd), glam::Vec3::new(a, b, hd)),
        };
        // 截面 1/4 圆：法向从相邻面1外法向扫到面2外法向（等距面，与角片无缝衔接）。
        // 2026-08-16 P0 修复：旧实现用"盒子中心→棱线对角方向"作截面基向量，
        // v=π/2 端顶点塌到棱线上 → 从表面伸出卷曲飘带面片。
        let (n1, n2) = match axis {
            0 => (
                glam::Vec3::new(0.0, a.signum(), 0.0),
                glam::Vec3::new(0.0, 0.0, b.signum()),
            ),
            1 => (
                glam::Vec3::new(a.signum(), 0.0, 0.0),
                glam::Vec3::new(0.0, 0.0, b.signum()),
            ),
            _ => (
                glam::Vec3::new(a.signum(), 0.0, 0.0),
                glam::Vec3::new(0.0, b.signum(), 0.0),
            ),
        };
        let base = mesh.verts.len() as u32;
        for j in 0..=seg {
            let v = std::f32::consts::FRAC_PI_2 * j as f32 / seg as f32;
            let (sv, cv) = v.sin_cos();
            let n = (n1 * cv + n2 * sv).normalize();
            for i in 0..=seg {
                let u = i as f32 / seg as f32;
                let pos = p0.lerp(p1, u) + n * r;
                mesh.verts.push(GVertex {
                    pos: pos.to_array(),
                    normal: n.to_array(),
                    uv: [u, v / std::f32::consts::FRAC_PI_2],
                    color: [1.0, 1.0, 1.0],
                });
            }
        }
        // 绕序与角片一致（外侧 CCW）
        // 🔴 **12 条棱边里有 6 条是反绕的**（2026-10-17 实测）。截面从 `n1` 扫到 `n2`，
        // 三角形绕序的符号对 `axis=0/2` 正比于 `-sign(a)·sign(b)`、对 `axis=1` 正比于
        // `+sign(a)·sign(b)` ⇒ 该乘积为负的那些棱边必须反绕。
        let sa = if a > 0.0 { 1.0f32 } else { -1.0 };
        let sb = if b > 0.0 { 1.0f32 } else { -1.0 };
        let flip = match axis {
            0 | 2 => sa * sb > 0.0,
            _ => sa * sb < 0.0,
        };
        for j in 0..seg {
            for i in 0..seg {
                let a0 = base + j * (seg + 1) + i;
                let b0 = a0 + 1;
                let c0 = a0 + seg + 1;
                let d0 = c0 + 1;
                if flip {
                    mesh.indices.extend_from_slice(&[a0, d0, b0, a0, c0, d0]);
                } else {
                    mesh.indices.extend_from_slice(&[a0, b0, d0, a0, d0, c0]);
                }
            }
        }
    }

    // 6 个面：平面片
    let faces: [(glam::Vec3, glam::Vec3, glam::Vec3, glam::Vec3, glam::Vec3); 6] = [
        (glam::Vec3::X, glam::Vec3::new(hw, -hh, -hd), glam::Vec3::new(hw, hh, -hd), glam::Vec3::new(hw, hh, hd), glam::Vec3::new(hw, -hh, hd)),
        (glam::Vec3::NEG_X, glam::Vec3::new(-hw, -hh, hd), glam::Vec3::new(-hw, hh, hd), glam::Vec3::new(-hw, hh, -hd), glam::Vec3::new(-hw, -hh, -hd)),
        (glam::Vec3::Y, glam::Vec3::new(-hw, hh, -hd), glam::Vec3::new(hw, hh, -hd), glam::Vec3::new(hw, hh, hd), glam::Vec3::new(-hw, hh, hd)),
        (glam::Vec3::NEG_Y, glam::Vec3::new(-hw, -hh, hd), glam::Vec3::new(hw, -hh, hd), glam::Vec3::new(hw, -hh, -hd), glam::Vec3::new(-hw, -hh, -hd)),
        (glam::Vec3::Z, glam::Vec3::new(-hw, -hh, hd), glam::Vec3::new(hw, -hh, hd), glam::Vec3::new(hw, hh, hd), glam::Vec3::new(-hw, hh, hd)),
        (glam::Vec3::NEG_Z, glam::Vec3::new(hw, -hh, -hd), glam::Vec3::new(-hw, -hh, -hd), glam::Vec3::new(-hw, hh, -hd), glam::Vec3::new(hw, hh, -hd)),
    ];
    for (n, p0, p1, p2, p3) in faces {
        let base = mesh.verts.len() as u32;
        for (i, p) in [p0, p1, p2, p3].iter().enumerate() {
            // 🔴 **必须 `+ n * r` 推到外表面**（2026-10-17 外部复核指出，实测确认）：
            // `hw/hh/hd` 是**内缩后的圆角球心坐标**（`w/2 - r`），角片（`c + n*r`）与
            // 棱边（`p0.lerp(..) + n*r`）都推到了 `±w/2`，唯独这里原来直接写 `p` ⇒
            // 6 个平面比圆角面**内缩 r**，整圈裂开一道宽 r 的缝（`ak12m` 护木 r≈2.4cm）。
            let outer = *p + n * r;
            mesh.verts.push(GVertex {
                pos: outer.to_array(),
                normal: n.to_array(),
                uv: [if i % 2 == 0 { 0.0 } else { 1.0 }, if i / 2 == 0 { 0.0 } else { 1.0 }],
                color: [1.0, 1.0, 1.0],
            });
        }
        // 🔴 **±Y 两个面要反绕**（2026-10-17 实测：`(p1-p0)×(p2-p0)` 指向盒内）：
        // 其余四面 `[0,1,2, 0,2,3]` 是外侧 CCW，±Y 必须 `[0,2,1, 0,3,2]`。
        // 判据 = `winding_tests::every_primitive_is_outward_ccw`。
        if n.y.abs() > 0.5 {
            mesh.indices.extend_from_slice(&[
                base, base + 2, base + 1,
                base, base + 3, base + 2,
            ]);
        } else {
            mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    mesh
}

/// 锥台（截锥）：底部半径 r0、顶部半径 r1、高 height，中心在原点沿 Y 轴。
/// caps=true 时带顶/底盖。侧面法线精确，绕序外侧 CCW。
pub fn frustum(r0: f32, r1: f32, height: f32, seg: u32, caps: bool) -> Mesh {
    let seg = seg.max(6);
    let mut mesh = Mesh::default();
    let h = height * 0.5;
    let ring0 = 0u32;
    let ring1 = (seg + 1) as u32;
    for (rr, y) in [(r0, -h), (r1, h)] {
        for i in 0..=seg {
            let t = std::f32::consts::TAU * i as f32 / seg as f32;
            let (s, c) = t.sin_cos();
            // 侧面法线：由母线方向与圆周方向叉乘
            let slant = glam::Vec3::new(c * (r1 - r0), height, s * (r1 - r0));
            let tangent = glam::Vec3::new(-s, 0.0, c);
            // 🔴 **顺序必须是 `slant × tangent`**（2026-10-17 自查发现，判据见文末
            // `normal_tests::cylinder_side_normals_point_outward`）：
            // 代入圆柱（`r1 == r0`）在 θ=0 处：`tangent=(0,0,1)`、`slant=(0,height,0)`，
            //   `tangent × slant` = `(-height, 0, 0)` ⇒ **朝内**（外法线应是 +X）；
            //   `slant × tangent` = `(+height, 0, 0)` ⇒ **朝外** ✅
            // 锥台时该式的 Y 分量 = `r0 - r1`：上窄下宽（r1<r0）得 +Y ⇒ 法线上倾，也对。
            // ⚠️ 朝内的后果是**静默的**：明暗在 `Mesh::append_transformed` 里按
            // `ndl = n·L` 烘进顶点色 ⇒ 全部圆柱/圆锥会被"从里往外"照亮，
            // 不崩、不报 VUID、不打日志。判据在修复前实测 dot=-1。
            let n = slant.cross(tangent).normalize_or_zero();
            mesh.verts.push(GVertex {
                pos: [c * rr, y, s * rr],
                normal: n.to_array(),
                uv: [i as f32 / seg as f32, if y < 0.0 { 0.0 } else { 1.0 }],
                color: [1.0, 1.0, 1.0],
            });
        }
    }
    for i in 0..seg {
        let a = ring0 + i;
        let b = a + 1;
        let c = ring1 + i;
        let d = c + 1;
        mesh.indices.extend_from_slice(&[a, c, d, a, d, b]);
    }
    if caps {
        for (y, r, n) in [(-h, r0, glam::Vec3::NEG_Y), (h, r1, glam::Vec3::Y)] {
            let base = mesh.verts.len() as u32;
            mesh.verts.push(GVertex { pos: [0.0, y, 0.0], normal: n.to_array(), uv: [0.5, 0.5], color: [1.0, 1.0, 1.0] });
            for i in 0..=seg {
                let t = std::f32::consts::TAU * i as f32 / seg as f32;
                let (s, c) = t.sin_cos();
                mesh.verts.push(GVertex { pos: [c * r, y, s * r], normal: n.to_array(), uv: [c * 0.5 + 0.5, s * 0.5 + 0.5], color: [1.0, 1.0, 1.0] });
            }
            for i in 0..seg {
                let a = base + 1 + i;
                let b = a + 1;
                // 🔴 **两个端盖原来都朝内**（2026-10-17 实测：顶盖 `[base,a,b]` 的几何法线
                // 是 `-Y`、底盖 `[base,b,a]` 是 `+Y`，都与各自的外法线相反）。
                // 侧面 `[a,c,d,...]` 实测是外侧 CCW（32 CCW / 0 CW），端盖必须与之同向：
                // 顶盖（外法线 +Y）用 `[base, b, a]`，底盖（外法线 -Y）用 `[base, a, b]`。
                if n.y > 0.0 {
                    mesh.indices.extend_from_slice(&[base, b, a]);
                } else {
                    mesh.indices.extend_from_slice(&[base, a, b]);
                }
            }
        }
    }
    mesh
}

/// 圆柱（frustum 特例）
pub fn cylinder(r: f32, height: f32, seg: u32) -> Mesh {
    frustum(r, r, height, seg, true)
}

/// 圆环弧段（torus arc）：环半径 R、管半径 r，沿环角度 [t0, t1]（弧度），
/// 环平面为 XY（θ 绕 Z），φ 绕管。用于扳机护圈等环形细节（2026-08-16 P2）。

pub fn torus_arc(ring_r: f32, tube_r: f32, t0: f32, t1: f32, seg_ring: u32, seg_tube: u32) -> Mesh {
    let seg_ring = seg_ring.max(6);
    let seg_tube = seg_tube.max(6);
    let mut mesh = Mesh::default();
    for j in 0..=seg_tube {
        let phi = std::f32::consts::TAU * j as f32 / seg_tube as f32;
        let (sp, cp) = phi.sin_cos();
        for i in 0..=seg_ring {
            let theta = t0 + (t1 - t0) * i as f32 / seg_ring as f32;
            let (st, ct) = theta.sin_cos();
            let n = glam::Vec3::new(cp * ct, cp * st, sp);
            let pos = glam::Vec3::new((ring_r + tube_r * cp) * ct, (ring_r + tube_r * cp) * st, tube_r * sp);
            mesh.verts.push(GVertex {
                pos: pos.to_array(),
                normal: n.to_array(),
                uv: [i as f32 / seg_ring as f32, j as f32 / seg_tube as f32],
                color: [1.0, 1.0, 1.0],
            });
        }
    }
    for j in 0..seg_tube {
        for i in 0..seg_ring {
            let a = j * (seg_ring + 1) + i;
            let b = a + 1;
            let c = a + seg_ring + 1;
            let d = c + 1;
            mesh.indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    mesh
}

/// 单位球体（UV 经纬，法线 = 位置）；预留 NPC 头部等使用

pub fn sphere(seg: u32, rings: u32) -> Mesh {
    let mut mesh = Mesh::default();
    for j in 0..=rings {
        let phi = std::f32::consts::PI * j as f32 / rings as f32;
        let (sp, cp) = phi.sin_cos();
        for i in 0..=seg {
            let theta = std::f32::consts::TAU * i as f32 / seg as f32;
            let (st, ct) = theta.sin_cos();
            let n = glam::Vec3::new(sp * ct, cp, sp * st);
            mesh.verts.push(GVertex {
                pos: n.to_array(),
                normal: n.to_array(),
                uv: [i as f32 / seg as f32, 1.0 - j as f32 / rings as f32],
                color: [1.0, 1.0, 1.0],
            });
        }
    }
    for j in 0..rings {
        for i in 0..seg {
            let a = j * (seg + 1) + i;
            let b = a + 1;
            let c = a + seg + 1;
            let d = c + 1;
            // 🔴 **整个球原来的绕序是反的**（2026-10-17 实测：352 个三角形全部 CW，
            // `(c-a)×(b-a)` 指向球心）。`append_transformed` 统一翻转后，
            // 反向的图元会只剩背面 ⇒ 球只渲染内壁。改成外侧 CCW：
            // `[a, b, c]` + `[b, d, c]`（原为 `[a, c, b]` + `[b, c, d]`）。
            // 判据 = `winding_tests::every_primitive_is_outward_ccw`。
            mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    mesh
}

#[cfg(test)]
mod normal_tests {
    use super::*;
    use glam::Vec3;

    /// 圆柱侧面法线必须**朝外**（与位置的水平分量同向）。
    ///
    /// 🔴 2026-10-17 自查发现：侧面法线是 `tangent.cross(slant)`，代入圆柱
    /// （`r1 == r0` ⇒ `slant` 的径向分量为 0）在 θ=0 处得
    /// `tangent = (0,0,1)`、`slant = (0, height, 0)` ⇒ 叉积 `(-height, 0, 0)`
    /// ⇒ **指向 -X**，而该处外法线应是 +X ⇒ **整个圆柱/圆锥的侧面法线朝内**。
    ///
    /// `Mesh::append_transformed` 用这些法线烘明暗（`ndl = n·L`），
    /// 朝内 ⇒ **所有圆柱（枪管、弹鼓、护木、柱子）的明暗整个翻面**，
    /// 而它不崩、不报 VUID、不产生任何日志 —— 典型的静默视觉缺陷。
    #[test]
    fn cylinder_side_normals_point_outward() {
        let m = cylinder(1.0, 2.0, 16);
        let mut checked = 0;
        for v in &m.verts {
            let n = Vec3::from(v.normal);
            // 按**法线**筛，不按位置筛：盖子的环边顶点与侧面顶点**位置相同**
            // （y=±1、半径 1），按位置筛会把 ±Y 的盖子法线也查进来。
            let radial_part = Vec3::new(n.x, 0.0, n.z).length();
            if radial_part < 0.5 {
                continue; // 盖子法线是纯 ±Y
            }
            let p = Vec3::from(v.pos);
            let radial = Vec3::new(p.x, 0.0, p.z).normalize();
            let dot = n.dot(radial);
            assert!(
                dot > 0.9,
                "侧面法线朝内：pos={:?} normal={:?} 外法线应≈{:?} dot={dot}",
                p.to_array(),
                n.to_array(),
                radial.to_array()
            );
            checked += 1;
        }
        // 教训 27：判据自己要说清「检查了几个」，否则扫 0 个也是绿的
        assert!(checked >= 16, "只检查到 {checked} 个侧面顶点，判据没测到东西");
    }

    /// 锥台的侧面法线同样必须朝外（`frustum` 的径向分量非零时也要对）。
    #[test]
    fn frustum_side_normals_point_outward() {
        let m = frustum(1.0, 0.5, 2.0, 16, false);
        let mut checked = 0;
        for v in &m.verts {
            let p = Vec3::from(v.pos);
            let radial_len = (p.x * p.x + p.z * p.z).sqrt();
            if radial_len > 0.4 {
                let n = Vec3::from(v.normal);
                // 侧面法线的径向分量必须为正（锥面法线有轴向分量，故阈值取 0.3）
                let outward = n.x * p.x + n.z * p.z;
                assert!(
                    outward > 0.3 * radial_len,
                    "锥台侧面法线朝内：pos={:?} normal={:?} 径向点积={outward}",
                    p.to_array(),
                    n.to_array()
                );
                checked += 1;
            }
        }
        assert!(checked >= 16, "只检查到 {checked} 个顶点，判据没测到东西");
    }
}

#[cfg(test)]
mod winding_tests {
    use super::*;
    use glam::Vec3;

    /// 统计一个图元里「几何法线 · 顶点法线」为正 / 为负的三角形数。
    fn winding_counts(m: &Mesh) -> (usize, usize) {
        let (mut ccw, mut cw) = (0, 0);
        for tri in m.indices.chunks(3) {
            if tri.len() != 3 {
                continue;
            }
            let v = |i: u32| &m.verts[i as usize];
            let p0 = Vec3::from(v(tri[0]).pos);
            let p1 = Vec3::from(v(tri[1]).pos);
            let p2 = Vec3::from(v(tri[2]).pos);
            let n = (Vec3::from(v(tri[0]).normal)
                + Vec3::from(v(tri[1]).normal)
                + Vec3::from(v(tri[2]).normal))
                / 3.0;
            let geo = (p1 - p0).cross(p2 - p0);
            if geo.length_squared() < 1e-16 {
                continue; // 退化三角形不参与判定
            }
            if geo.dot(n) > 0.0 {
                ccw += 1;
            } else {
                cw += 1;
            }
        }
        (ccw, cw)
    }

    /// 🔴 **每个图元的每个非退化三角形都必须「外侧 CCW」**（2026-10-17 外部复核指出，
    /// 实测确认并修复）。`Mesh::append_transformed` 对**全部**三角形统一翻转一次绕序
    /// 来适配主管线的 `front_face = CLOCKWISE + cull BACK` ⇒ **图元内部必须自身一致**；
    /// 混着就会有一半被背面剔除（实测修复前：`beveled_box` 176/172、`frustum` 端盖 32/32、
    /// `sphere` 0/352）。
    ///
    /// 这条判据的**反向验证**：把任一处索引顺序改回去 ⇒ 对应图元的 `cw` 立刻 > 0。
    #[test]
    fn every_primitive_is_outward_ccw() {
        let cases: [(&str, Mesh, usize); 5] = [
            ("beveled_box", beveled_box(1.0, 1.0, 1.0, 0.1, 3), 300),
            ("frustum+caps", frustum(1.0, 1.0, 2.0, 16, true), 60),
            ("frustum-nocaps", frustum(1.0, 0.5, 2.0, 16, false), 30),
            ("torus_arc", torus_arc(0.5, 0.1, 0.0, std::f32::consts::TAU, 16, 8), 250),
            ("sphere", sphere(16, 12), 340),
        ];
        for (name, mesh, min_tris) in cases {
            let (ccw, cw) = winding_counts(&mesh);
            assert!(
                ccw + cw >= min_tris,
                "{name}: 只统计到 {} 个三角形（期望 >= {min_tris}），判据没测到东西",
                ccw + cw
            );
            assert_eq!(
                cw, 0,
                "{name}: 有 {cw} 个三角形是 CW（朝内）—— 它们会被背面剔除。\
                 \n修复：把对应索引顺序反写（外侧 CCW）。详见 `meshgen` 里各图元的注释。"
            );
        }
    }

    /// 🔴 **`beveled_box` 的 6 个平面片必须在外表面 `±w/2`**，不能停在内缩核 `±hw`。
    ///
    /// `hw = w/2 - r` 是圆角球心坐标；角片与棱边都加了 `n * r` 推到 `±w/2`，
    /// 只有平面片原来漏了 ⇒ 整圈裂开宽 `r` 的缝（2026-10-17 修复）。
    #[test]
    fn beveled_box_faces_reach_the_outer_surface() {
        let (w, r) = (1.0f32, 0.1f32);
        let m = beveled_box(w, 1.0, 1.0, r, 3);
        // 平面片顶点必须真正到达 ±w/2（允许 1e-4 的浮点误差）
        let half = w / 2.0;
        let mut reached = 0;
        for v in &m.verts {
            if (v.pos[0] - half).abs() < 1e-4 {
                reached += 1;
            }
        }
        // seg=3 时 +X 面是 4x4 顶点；+X 面的所有顶点 x 都应等于 half
        assert!(
            reached >= 16,
            "只有 {reached} 个顶点到达 +X 外表面（期望 >= 16）——\
             平面片大概又漏了 `+ n * r`"
        );
        // 且没有任何顶点的 |x| 超过外表面（否则是别处写错了）
        for v in &m.verts {
            assert!(
                v.pos[0].abs() <= half + 1e-4,
                "顶点越出外表面：pos={:?} 半宽={half}",
                v.pos
            );
        }
    }
}
