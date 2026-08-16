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
        for j in 0..seg {
            for i in 0..seg {
                let a = base + j * (seg + 1) + i;
                let b = a + 1;
                let c0 = a + seg + 1;
                let d0 = c0 + 1;
                mesh.indices.extend_from_slice(&[a, b, d0, a, d0, c0]);
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
        // 截面平面的两个基向量（从盒子中心指向该边的外侧方向 + 沿轴方向）
        let outward = match axis {
            0 => glam::Vec3::new(0.0, a.signum(), b.signum()).normalize_or_zero(),
            1 => glam::Vec3::new(a.signum(), 0.0, b.signum()).normalize_or_zero(),
            _ => glam::Vec3::new(a.signum(), b.signum(), 0.0).normalize_or_zero(),
        };
        let along = match axis {
            0 => glam::Vec3::X,
            1 => glam::Vec3::Y,
            _ => glam::Vec3::Z,
        };
        let t2 = along;
        let base = mesh.verts.len() as u32;
        for j in 0..=seg {
            let v = std::f32::consts::FRAC_PI_2 * j as f32 / seg as f32;
            let (sv, cv) = v.sin_cos();
            for i in 0..=seg {
                let u = i as f32 / seg as f32;
                let along_pt = p0.lerp(p1, u);
                let n = (outward * cv + t2 * sv).normalize_or_zero();
                // 位置 = 沿轴插值点 + 截面法向偏移（投影到截面平面）
                let off = match axis {
                    0 => glam::Vec3::new(0.0, n.y, n.z),
                    1 => glam::Vec3::new(n.x, 0.0, n.z),
                    _ => glam::Vec3::new(n.x, n.y, 0.0),
                };
                let pos = along_pt + off * r;
                mesh.verts.push(GVertex {
                    pos: pos.to_array(),
                    normal: off.normalize_or_zero().to_array(),
                    uv: [u, v / std::f32::consts::FRAC_PI_2],
                    color: [1.0, 1.0, 1.0],
                });
            }
        }
        for j in 0..seg {
            for i in 0..seg {
                let a0 = base + j * (seg + 1) + i;
                let b0 = a0 + 1;
                let c0 = a0 + seg + 1;
                let d0 = c0 + 1;
                mesh.indices.extend_from_slice(&[a0, c0, d0, a0, d0, b0]);
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
            mesh.verts.push(GVertex {
                pos: p.to_array(),
                normal: n.to_array(),
                uv: [if i % 2 == 0 { 0.0 } else { 1.0 }, if i / 2 == 0 { 0.0 } else { 1.0 }],
                color: [1.0, 1.0, 1.0],
            });
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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
            let n = tangent.cross(slant).normalize_or_zero();
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
                if n.y > 0.0 {
                    mesh.indices.extend_from_slice(&[base, a, b]);
                } else {
                    mesh.indices.extend_from_slice(&[base, b, a]);
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

/// 单位球体（UV 经纬，法线 = 位置）；预留 NPC 头部等使用
#[allow(dead_code)]
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
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    mesh
}
