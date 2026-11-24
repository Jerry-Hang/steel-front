//! GLB 骨骼（skin）解析与骨骼变换数学（引擎化 P4 第一步，见 `docs/ENGINE-PLAN.md`）。
//!
//! 现状：`assets.rs` 的 GLB 读取只取 `POSITION` / `NORMAL` / `COLOR_0` / UV / 索引，
//! **完全不认识 `skins`、`JOINTS_0`、`WEIGHTS_0`、`animations`** —— 实测本仓 **53 件 GLB
//! 一件骨骼都没有**（`skins=0`、`animations=0`），所以这条能力必须先把"读"和"算"建起来。
//!
//! 骨骼变换公式（glTF 2.0 规范）：
//! ```text
//! skinMatrix[j] = worldMatrix(joint[j]) * inverseBindMatrix[j]
//! p' = Σ_i w_i * (skinMatrix[j_i] * p)
//! ```
//! **绑定姿势（bind pose）下 `p' == p`** —— 这是本模块最重要的自检：
//! 关节世界矩阵恰好等于绑定时，`world * inverseBind = I`，顶点必须一动不动。
//! 这条不成立就说明矩阵相乘的顺序或者 inverseBind 的方向错了。
//!
//! 为什么单独成模块而不塞进 `assets.rs`：`assets.rs` 里那个 accessor 读取函数是**嵌套**在
//! `parse_map_mesh` 内部的（`read_acc`），拿不出来；而骨骼要读的类型（MAT4、VEC4 的 u8 关节号）
//! 与它服务的"位置/法线/UV"不同。这里写一份**只服务骨骼**的最小读取器，边界写在注释里。

use glam::{Mat4, Vec3};

/// 一件网格上的骨骼绑定数据（glTF `skin` + 每个顶点属性）。
#[derive(Debug, Clone, Default)]
pub struct Skin {
    /// 每个关节的**绑定姿势世界矩阵**（由节点层级算出）。
    pub joint_world: Vec<Mat4>,
    /// 每个关节的 `inverseBindMatrices`（glTF 提供）。
    pub inverse_bind: Vec<Mat4>,
    /// 每个顶点 4 个关节下标（已校验 < joint 数）。
    pub joints: Vec<[u16; 4]>,
    /// 每个顶点 4 个权重（已归一化）。
    pub weights: Vec<[f32; 4]>,
}

impl Skin {
    pub fn joint_count(&self) -> usize {
        self.joint_world.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.joints.len()
    }

    /// 绑定姿势下的骨骼变换矩阵（`world * inverseBind`）。
    pub fn matrices(&self) -> Vec<Mat4> {
        self.joint_world
            .iter()
            .zip(&self.inverse_bind)
            .map(|(w, ib)| *w * *ib)
            .collect()
    }

    /// 算一个顶点的目标位置（`mats` 来自 [`Skin::matrices`]）。
    pub fn skin_vertex(&self, i: usize, p: Vec3, mats: &[Mat4]) -> Vec3 {
        let Some(j) = self.joints.get(i) else {
            return p;
        };
        let Some(w) = self.weights.get(i) else {
            return p;
        };
        skin_position(p, *j, *w, mats)
    }
}

/// 单个顶点的骨骼变换：`Σ w_i * (M[j_i] * p)`。
///
/// 权重之和小于 1（例如只导出了一个关节的权重）时**按剩余比例补到原位置**，
/// 而不是把顶点缩向原点 —— 后者会让模型在缺权重的地方整个塌陷，
/// 而"没被任何骨骼影响的顶点就该待在原地"才是直觉正确的行为。
pub fn skin_position(p: Vec3, joints: [u16; 4], weights: [f32; 4], mats: &[Mat4]) -> Vec3 {
    let mut acc = Vec3::ZERO;
    let mut sum = 0.0f32;
    for k in 0..4 {
        let w = weights[k];
        if w <= 0.0 {
            continue;
        }
        let Some(m) = mats.get(joints[k] as usize) else {
            continue;
        };
        acc += m.transform_point3(p) * w;
        sum += w;
    }
    if sum <= 1e-6 {
        return p;
    }
    // 归一化：权重和 != 1 时按比例缩放（glTF 要求和为 1，但外部资产经常不满足）
    let normalized = acc / sum;
    if sum < 1.0 {
        // 剩余比例留在原位置
        normalized * sum + p * (1.0 - sum)
    } else {
        normalized
    }
}


/// 把权重**预先归一化**到和为 1（全零权重保持全零 = "这个顶点不跟任何骨骼动"）。
///
/// 🔴 **这是给 GPU 侧准备的契约**：着色器不会做归一化（那需要分支，而且会把
/// "权重和不为 1 的坏数据"的语义藏进着色器）。所以上 GPU 之前必须在 CPU 侧归一化，
/// 使 `Σw = 1` 成立；否则 GPU 与 CPU 会算出不同结果，而**两边的实现各自看都对**。
///
/// 归一化之后 [`skin_position`] 与"着色器的直白写法"
/// （`p' = Σ w_i · (M_i · p)`，不归一化）**逐位一致** —— 判据钉住这条。
pub fn normalized_weights(skin: &Skin) -> Vec<[f32; 4]> {
    skin.weights
        .iter()
        .map(|w| {
            let sum: f32 = w.iter().map(|x| x.max(0.0)).sum();
            if sum <= 1e-6 {
                return [0.0, 0.0, 0.0, 0.0];
            }
            let mut out = [0.0f32; 4];
            for k in 0..4 {
                out[k] = w[k].max(0.0) / sum;
            }
            out
        })
        .collect()
}

/// **着色器的算法**在这里的一份参照实现：`p' = Σ w_i · (M_i · p)`，不做任何归一化、
/// 不做"剩余比例留在原位"的补偿。它的唯一用途是给 GPU 侧当契约：
/// 归一化权重之后，两者必须一致。
pub fn shader_reference_skin(
    p: Vec3,
    joints: [u16; 4],
    weights: [f32; 4],
    mats: &[Mat4],
) -> Vec3 {
    let mut acc = Vec3::ZERO;
    for k in 0..4 {
        let w = weights[k];
        if w <= 0.0 {
            continue;
        }
        if let Some(m) = mats.get(joints[k] as usize) {
            acc += m.transform_point3(p) * w;
        }
    }
    acc
}

/// 关节矩阵缓冲：每个关节一个 16 个 f32（**列主序**，与 WGSL 的 `mat4x4<f32>` 一致）。
///
/// GPU 侧要的就是这一段连续内存（将来塞进 storage buffer / uniform 数组）。
/// 把"展平"这一步放在 CPU 并判据化，是因为着色器里读错 stride 是**静默**的：
/// 每个数都是合法浮点数，画面只是"骨骼姿势怪"。
pub fn joint_matrix_buffer(mats: &[Mat4]) -> Vec<f32> {
    let mut out = Vec::with_capacity(mats.len() * 16);
    for m in mats {
        out.extend_from_slice(&m.to_cols_array());
    }
    out
}

/// 绑定姿势自检：返回**所有顶点的最大位移**（应当 ≈ 0）。
///
/// 这条是判据的核心 —— 它同时检验了"矩阵顺序"与"inverseBind 方向"两件事，
/// 而这两件任一做错，画面上的表现都只是"模型姿势怪"，很难定位。
pub fn bind_pose_error(skin: &Skin, positions: &[Vec3]) -> f32 {
    let mats = skin.matrices();
    let mut worst = 0.0f32;
    for (i, p) in positions.iter().enumerate() {
        let q = skin.skin_vertex(i, *p, &mats);
        worst = worst.max((q - *p).length());
    }
    worst
}

/// 从 GLB 字节里读骨骼。没有 `skins` 时返回 `Ok(None)`（**不是错误**：绝大多数模型没有骨骼）。
///
/// 只读第一个 skin 与它的第一个骨骼变换网格 —— 多 skin 的资产等真遇到再扩，
/// 现在扩了也没法验证（本仓没有任何带骨骼的资产）。
pub fn parse_glb_skin(bytes: &[u8]) -> Result<Option<(Skin, Vec<Vec3>)>, String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("非 GLB 文件（缺少 glTF magic）".into());
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_end = 20usize
        .checked_add(json_len)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| format!("GLB JSON 块长度越界：头里写 {json_len} 字节"))?;
    let json_str = std::str::from_utf8(&bytes[20..json_end]).map_err(|_| "GLB JSON 非 UTF-8")?;
    let json = crate::llm_cmd::parse_json_fn(json_str)
        .map_err(|e| format!("GLB JSON 解析失败: {e}"))?;
    let bin_start = json_end;
    let bin: &[u8] = if bin_start + 8 <= bytes.len() {
        let blen =
            u32::from_le_bytes([bytes[bin_start], bytes[bin_start + 1], bytes[bin_start + 2], bytes[bin_start + 3]])
                as usize;
        &bytes[bin_start + 8..(bin_start + 8 + blen).min(bytes.len())]
    } else {
        &[]
    };

    let Some(skins) = json.get("skins").and_then(|s| s.as_arr()) else {
        return Ok(None);
    };
    let Some(skin0) = skins.first() else {
        return Ok(None);
    };
    let joints_idx: Vec<usize> = skin0
        .get("joints")
        .and_then(|j| j.as_arr())
        .ok_or("skin 缺 joints")?
        .iter()
        .map(|v| v.as_f64().unwrap_or(-1.0) as isize)
        .filter(|v| *v >= 0)
        .map(|v| v as usize)
        .collect();
    if joints_idx.is_empty() {
        return Err("skin.joints 为空".into());
    }
    let ib_acc = skin0
        .get("inverseBindMatrices")
        .and_then(|v| v.as_f64())
        .ok_or("skin 缺 inverseBindMatrices")? as usize;
    let ib_raw = read_acc_f32(&json, bin, ib_acc, 16)?;
    if ib_raw.len() < joints_idx.len() * 16 {
        return Err(format!(
            "inverseBindMatrices 元素不足：{} < {}",
            ib_raw.len() / 16,
            joints_idx.len()
        ));
    }
    let inverse_bind: Vec<Mat4> = (0..joints_idx.len())
        .map(|j| {
            let s = &ib_raw[j * 16..j * 16 + 16];
            // glTF 的矩阵是**列主序**，与 glam 的 `from_cols_array` 一致
            Mat4::from_cols_array(&[
                s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8], s[9], s[10], s[11], s[12],
                s[13], s[14], s[15],
            ])
        })
        .collect();
    let joint_world: Vec<Mat4> = joints_idx
        .iter()
        .map(|&n| node_world(&json, n))
        .collect();

    // 找第一个带 JOINTS_0 / WEIGHTS_0 的 primitive
    let meshes = json.get("meshes").and_then(|m| m.as_arr()).ok_or("缺 meshes")?;
    for mesh in meshes {
        let Some(prims) = mesh.get("primitives").and_then(|p| p.as_arr()) else {
            continue;
        };
        for prim in prims {
            let Some(attrs) = prim.get("attributes") else {
                continue;
            };
            let (Some(j_acc), Some(w_acc)) = (
                attrs.get("JOINTS_0").and_then(|v| v.as_f64()),
                attrs.get("WEIGHTS_0").and_then(|v| v.as_f64()),
            ) else {
                continue;
            };
            let pos_acc = match attrs.get("POSITION").and_then(|v| v.as_f64()) {
                Some(v) => v as usize,
                None => continue,
            };
            let j_raw = read_acc_f32(&json, bin, j_acc as usize, 4)?;
            let w_raw = read_acc_f32(&json, bin, w_acc as usize, 4)?;
            let p_raw = read_acc_f32(&json, bin, pos_acc, 3)?;
            let n = (j_raw.len() / 4).min(w_raw.len() / 4).min(p_raw.len() / 3);
            let mut joints = Vec::with_capacity(n);
            let mut weights = Vec::with_capacity(n);
            let mut positions = Vec::with_capacity(n);
            for i in 0..n {
                let j = &j_raw[i * 4..i * 4 + 4];
                joints.push([
                    j[0].max(0.0) as u16,
                    j[1].max(0.0) as u16,
                    j[2].max(0.0) as u16,
                    j[3].max(0.0) as u16,
                ]);
                let w = &w_raw[i * 4..i * 4 + 4];
                weights.push([w[0].max(0.0), w[1].max(0.0), w[2].max(0.0), w[3].max(0.0)]);
                positions.push(Vec3::new(
                    p_raw[i * 3],
                    p_raw[i * 3 + 1],
                    p_raw[i * 3 + 2],
                ));
            }
            let skin = Skin {
                joint_world,
                inverse_bind,
                joints,
                weights,
            };
            return Ok(Some((skin, positions)));
        }
    }
    Ok(None)
}

/// 节点的世界矩阵：沿 `children` 反查上一层节点，再把从根到它的 TRS（或 `matrix`）连乘。
fn node_world(json: &crate::llm_cmd::Json, node_idx: usize) -> Mat4 {
    // 建一次"子 → 上一层"的反查表；节点数都很小（骨骼几十个），线性查可接受。
    let nodes = json.get("nodes").and_then(|n| n.as_arr());
    let mut parent: Vec<Option<usize>> = vec![None; nodes.map(|n| n.len()).unwrap_or(0)];
    if let Some(ns) = nodes {
        for (pi, n) in ns.iter().enumerate() {
            if let Some(ch) = n.get("children").and_then(|c| c.as_arr()) {
                for c in ch {
                    if let Some(ci) = c.as_f64() {
                        let ci = ci as usize;
                        if ci < parent.len() {
                            parent[ci] = Some(pi);
                        }
                    }
                }
            }
        }
    }
    let mut chain = vec![node_idx];
    let mut cur = node_idx;
    let mut guard = 0;
    while let Some(p) = parent.get(cur).copied().flatten() {
        chain.push(p);
        cur = p;
        guard += 1;
        if guard > 4096 {
            break;
        }
    }
    let mut m = Mat4::IDENTITY;
    for &n in chain.iter().rev() {
        m *= node_local(json, n);
    }
    m
}

/// 单个节点的局部矩阵：优先 `matrix`（列主序 16 个数），否则 TRS。
pub(crate) fn node_local(json: &crate::llm_cmd::Json, idx: usize) -> Mat4 {
    let Some(node) = json
        .get("nodes")
        .and_then(|n| n.as_arr())
        .and_then(|n| n.get(idx))
    else {
        return Mat4::IDENTITY;
    };
    if let Some(m) = node.get("matrix").and_then(|m| m.as_arr()) {
        if m.len() >= 16 {
            let v: Vec<f32> = m.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            return Mat4::from_cols_array(&[
                v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10], v[11], v[12],
                v[13], v[14], v[15],
            ]);
        }
    }
    let t = node.get("translation").and_then(|x| x.as_arr());
    let r = node.get("rotation").and_then(|x| x.as_arr());
    let s = node.get("scale").and_then(|x| x.as_arr());
    let tv = t
        .map(|a| {
            Vec3::new(
                a.first().and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                a.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
            )
        })
        .unwrap_or(Vec3::ZERO);
    let rq = r
        .map(|a| {
            glam::Quat::from_xyzw(
                a.first().and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                a.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                a.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            )
        })
        .unwrap_or(glam::Quat::IDENTITY);
    let sv = s
        .map(|a| {
            Vec3::new(
                a.first().and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                a.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                a.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            )
        })
        .unwrap_or(Vec3::ONE);
    Mat4::from_scale_rotation_translation(sv, rq, tv)
}

/// 把 GLB 拆成 `(JSON, BIN)`。**探针命令**与解析器共用，避免两处各写一份切块逻辑。
pub fn split_glb(
    bytes: &[u8],
) -> Result<(crate::llm_cmd::Json, Vec<u8>), String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("非 GLB 文件（缺少 glTF magic）".into());
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_end = 20usize
        .checked_add(json_len)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| format!("GLB JSON 块长度越界：头里写 {json_len} 字节"))?;
    let json_str = std::str::from_utf8(&bytes[20..json_end]).map_err(|_| "GLB JSON 非 UTF-8")?;
    let json =
        crate::llm_cmd::parse_json_fn(json_str).map_err(|e| format!("GLB JSON 解析失败: {e}"))?;
    let bin: Vec<u8> = if json_end + 8 <= bytes.len() {
        let blen = u32::from_le_bytes([
            bytes[json_end],
            bytes[json_end + 1],
            bytes[json_end + 2],
            bytes[json_end + 3],
        ]) as usize;
        bytes[json_end + 8..(json_end + 8 + blen).min(bytes.len())].to_vec()
    } else {
        Vec::new()
    };
    Ok((json, bin))
}

/// 建"子 → 上一层"的反查表（每个 node 的上一层下标）。
pub(crate) fn node_parents(json: &crate::llm_cmd::Json) -> Vec<Option<usize>> {
    let nodes = json.get("nodes").and_then(|n| n.as_arr());
    let mut parent: Vec<Option<usize>> = vec![None; nodes.map(|n| n.len()).unwrap_or(0)];
    if let Some(ns) = nodes {
        for (pi, n) in ns.iter().enumerate() {
            if let Some(ch) = n.get("children").and_then(|c| c.as_arr()) {
                for c in ch {
                    if let Some(ci) = c.as_f64() {
                        let ci = ci as usize;
                        if ci < parent.len() {
                            parent[ci] = Some(pi);
                        }
                    }
                }
            }
        }
    }
    parent
}

/// 按给定的局部矩阵表重算所有节点的世界矩阵（`locals[i]` = 节点 i 的局部矩阵）。
pub(crate) fn worlds_from_locals(parents: &[Option<usize>], locals: &[Mat4]) -> Vec<Mat4> {
    let mut out = vec![Mat4::IDENTITY; locals.len()];
    for i in 0..locals.len() {
        // 沿链收集（深度有限，骨骼层级通常 < 32）
        let mut chain = vec![i];
        let mut cur = i;
        let mut guard = 0;
        while let Some(p) = parents.get(cur).copied().flatten() {
            chain.push(p);
            cur = p;
            guard += 1;
            if guard > 4096 {
                break;
            }
        }
        let mut m = Mat4::IDENTITY;
        for &n in chain.iter().rev() {
            m *= *locals.get(n).unwrap_or(&Mat4::IDENTITY);
        }
        out[i] = m;
    }
    out
}

/// 只服务骨骼的最小 accessor 读取器：支持 FLOAT 与整型（`normalized` 感知）+ `byteStride`。
///
/// 与 `assets.rs` 里那个嵌套的 `read_acc` 是两份实现 —— 有意为之：
/// 那个函数嵌在 `parse_map_mesh` 内部拿不出来，而骨骼要读 MAT4 与 u8 关节号。
/// 边界写在这里，等 `assets.rs` 哪天把那函数提到模块级，这份就可以删掉。
pub(crate) fn read_acc_f32(
    json: &crate::llm_cmd::Json,
    bin: &[u8],
    idx: usize,
    comps: usize,
) -> Result<Vec<f32>, String> {
    let acc = json
        .get("accessors")
        .and_then(|a| a.as_arr())
        .and_then(|a| a.get(idx))
        .ok_or("accessor 缺失")?;
    let count = acc.get("count").and_then(|c| c.as_f64()).ok_or("accessor 缺 count")? as usize;
    let ctype = acc
        .get("componentType")
        .and_then(|c| c.as_f64())
        .ok_or("accessor 缺 componentType")? as u32;
    let normalized = acc.get("normalized").and_then(|b| b.as_bool()).unwrap_or(false);
    let bv = acc
        .get("bufferView")
        .and_then(|b| b.as_f64())
        .ok_or("accessor 缺 bufferView")? as usize;
    let bview = json
        .get("bufferViews")
        .and_then(|b| b.as_arr())
        .and_then(|b| b.get(bv))
        .ok_or("bufferView 缺失")?;
    let bv_off = bview
        .get("byteOffset")
        .and_then(|b| b.as_f64())
        .unwrap_or(0.0) as usize;
    let acc_off = acc
        .get("byteOffset")
        .and_then(|b| b.as_f64())
        .unwrap_or(0.0) as usize;
    let base = bv_off + acc_off;
    let elem_bytes = match ctype {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        other => return Err(format!("未知 componentType {other}")),
    };
    let stride = bview
        .get("byteStride")
        .and_then(|b| b.as_f64())
        .map(|v| v as usize)
        .filter(|v| *v > 0)
        .unwrap_or(elem_bytes * comps);
    let mut out = Vec::with_capacity(count * comps);
    for i in 0..count {
        for c in 0..comps {
            let off = base + i * stride + c * elem_bytes;
            if off + elem_bytes > bin.len() {
                return Err(format!("accessor 读越界：off={off} len={}", bin.len()));
            }
            let v = match ctype {
                5126 => f32::from_le_bytes([bin[off], bin[off + 1], bin[off + 2], bin[off + 3]]),
                5125 => u32::from_le_bytes([bin[off], bin[off + 1], bin[off + 2], bin[off + 3]]) as f32,
                5123 => {
                    let raw = u16::from_le_bytes([bin[off], bin[off + 1]]) as f32;
                    if normalized {
                        raw / 65535.0
                    } else {
                        raw
                    }
                }
                5122 => {
                    let raw = i16::from_le_bytes([bin[off], bin[off + 1]]) as f32;
                    if normalized {
                        (raw / 32767.0).max(-1.0)
                    } else {
                        raw
                    }
                }
                5121 => {
                    let raw = bin[off] as f32;
                    if normalized {
                        raw / 255.0
                    } else {
                        raw
                    }
                }
                5120 => {
                    let raw = bin[off] as i8 as f32;
                    if normalized {
                        (raw / 127.0).max(-1.0)
                    } else {
                        raw
                    }
                }
                _ => 0.0,
            };
            out.push(v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小的 **带骨骼** GLB：一根由两段关节组成的竖杆（3 个顶点）。
    ///
    /// 顶点与权重：
    /// - v0 = (0,0,0) 关节 0 权重 1
    /// - v1 = (0,1,0) 关节 1 权重 1
    /// - v2 = (0,0.5,0) 关节 0/1 各 0.5
    ///
    /// 绑定姿势：关节 0 在原点（IB=I），关节 1 在 (0,1,0)（IB=平移 (0,-1,0)）。
    fn make_glb(joint1_y: f32) -> Vec<u8> {
        make_glb_ex(joint1_y, true)
    }

    /// `with_skin = false` 时**整段 `skins` 都不写**（不是把键名改掉）。
    fn make_glb_ex(joint1_y: f32, with_skin: bool) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        // 0: positions (3 * vec3 f32)
        for v in [[0.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.5, 0.0]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        let pos_len = bin.len(); // 36
        // 36: joints (3 * vec4 u8)
        for j in [[0u8, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0]] {
            bin.extend_from_slice(&j);
        }
        let joint_len = bin.len(); // 48
        // 48: weights (3 * vec4 f32)
        for w in [[1.0f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]] {
            for c in w {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        let w_len = bin.len(); // 96
        // 96: inverseBind 两个 mat4（列主序）
        let ib0 = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let ib1 = [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, -1.0, 0.0, 1.0]];
        for m in [ib0, ib1] {
            for col in m {
                for c in col {
                    bin.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        let ib_len = bin.len(); // 224
        let _ = (pos_len, joint_len, w_len);

        let skins = if with_skin {
            "\"skins\": [{\"joints\": [1, 2], \"inverseBindMatrices\": 3}],\n  "
        } else {
            ""
        };
        let json = format!(
            r#"{{
  "asset": {{"version": "2.0"}},
  "buffers": [{{"byteLength": {ib_len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 12}},
    {{"buffer": 0, "byteOffset": 48, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 96, "byteLength": 128}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4"}},
    {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"}},
    {{"bufferView": 3, "componentType": 5126, "count": 2, "type": "MAT4"}}
  ],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}}}}]}}],
  "nodes": [
    {{"mesh": 0}},
    {{"translation": [0.0, 0.0, 0.0]}},
    {{"translation": [0.0, {joint1_y}, 0.0]}}
  ],
  {skins}"scenes": [{{"nodes": [0, 1, 2]}}]
}}"#
        );
        // 组装 GLB
        let mut json_bytes = json.into_bytes();
        while json_bytes.len() % 4 != 0 {
            json_bytes.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }


    /// 🔴 **GPU 契约**：权重归一化之后，`skin_position`（带补偿的实现）与
    /// `shader_reference_skin`（着色器的直白写法）必须**逐位一致**。
    ///
    /// 这条决定了将来着色器里能不能省掉归一化分支。用真实 Blender 资产（50/50 权重，
    /// 和恰好为 1）与一个**故意不归一**的合成 skin（0.3 + 0.3）各测一次。
    ///
    /// 反向验证：把 `normalized_weights` 改成"原样返回"⇒ 第二条断言红（0.6 的和会少 40% 位移）。
    #[test]
    fn normalized_weights_make_the_shader_formula_exact() {
        // ① 真实资产：权重本来就和为 1
        let glb = include_bytes!("../../tools/fixtures/skinned_bar.glb");
        let (skin, positions) = parse_glb_skin(glb).expect("解析").expect("有骨骼");
        let mats = skin.matrices();
        let nw = normalized_weights(&skin);
        for (i, p) in positions.iter().enumerate() {
            let a = skin.skin_vertex(i, *p, &mats);
            let b = shader_reference_skin(*p, skin.joints[i], nw[i], &mats);
            assert!(
                (a - b).length() < 1e-6,
                "v{i} 两种写法必须一致：CPU {a:?} vs 着色器式 {b:?}"
            );
        }

        // ② 合成 skin：权重和 = 0.6（不归一），必须靠归一化救回来
        let bad = Skin {
            joint_world: vec![Mat4::IDENTITY, Mat4::from_translation(glam::Vec3::new(0.0, 10.0, 0.0))],
            inverse_bind: vec![Mat4::IDENTITY, Mat4::IDENTITY],
            joints: vec![[1, 1, 1, 1]],
            weights: vec![[0.3, 0.3, 0.0, 0.0]],
        };
        let p = glam::Vec3::ZERO;
        let bad_mats = bad.matrices();
        let nw_bad = normalized_weights(&bad);
        // 🔴 契约是"**两边都喂归一化后的权重**"。
        // 注意不能用 `skin_vertex`：它内部读的是**原始权重** [0.3,0.3]，
        // 而 `skin_position` 有一条"剩余比例留在原位置"的补偿（和 < 1 时按比例混回原始位置），
        // 于是原始权重下得到 6m（0.6 × 10）、归一化权重下得到 10m —— 两者都对，语义不同。
        // 我第一版断言写成"归一化后应 10m"却调用了 `skin_vertex`，测出来是 6，正好暴露这点。
        let cpu_norm = skin_position(p, bad.joints[0], nw_bad[0], &bad_mats);
        let gpu = shader_reference_skin(p, bad.joints[0], nw_bad[0], &bad_mats);
        assert!(
            (cpu_norm.y - 10.0).abs() < 1e-4,
            "归一化后应整段位移 10m，实际 {}",
            cpu_norm.y
        );
        assert!(
            (cpu_norm - gpu).length() < 1e-4,
            "归一化后两写法必须一致：CPU {cpu_norm:?} vs 着色器式 {gpu:?}"
        );
        // 原始权重走 `skin_position` 的补偿路径 ⇒ 6m（与上面那条语义不同，但都是对的）
        let cpu_raw = bad.skin_vertex(0, p, &bad_mats);
        assert!(
            (cpu_raw.y - 6.0).abs() < 1e-4,
            "原始权重下补偿路径应给 6m，实际 {}",
            cpu_raw.y
        );
        // 用**未归一**的权重照着色器写法算 ⇒ 只有 6m，差 4m（这就是为什么要先归一化）
        let raw = shader_reference_skin(p, bad.joints[0], bad.weights[0], &bad_mats);
        assert!(
            (raw.y - 6.0).abs() < 1e-4,
            "未归一化时着色器式写法应只走 6m（0.6 × 10），实际 {}",
            raw.y
        );
    }

    /// 关节矩阵缓冲必须是**列主序、每关节 16 个 f32**（WGSL 的 `mat4x4<f32>` 约定）。
    /// 反向验证：改成 `to_cols_array_2d` 的行优先展平 ⇒ 平移分量会跑到错误位置，红。
    #[test]
    fn joint_matrix_buffer_is_column_major_and_16_per_joint() {
        // ① 纯平移：列主序下平移在 12..15
        let t = Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
        let buf = joint_matrix_buffer(&[t]);
        assert_eq!(buf.len(), 16, "每关节 16 个 f32");
        assert_eq!(&buf[12..15], &[1.0, 2.0, 3.0], "平移必须落在 12..15");
        assert_eq!(buf[15], 1.0);

        // ② 纯旋转：**必须用非对称矩阵**，否则行优先/列主序展开的字节完全相同（判据恒真）。
        // 我第一版用纯平移，注入"行优先展开"居然没红。
        // 绕 Y：R = [[c,0,s],[0,1,0],[-s,0,c]]，列主序 ⇒ col0 = (c,0,-s,0) ⇒ buf[2] = -s，
        // col2 = (s,0,c,0) ⇒ buf[8] = +s。
        let r = Mat4::from_rotation_y(0.7);
        let rb = joint_matrix_buffer(&[r]);
        let sn = 0.7f32.sin();
        assert!((rb[2] - (-sn)).abs() < 1e-6, "buf[2] 应是 -sinθ：{}", rb[2]);
        assert!((rb[8] - sn).abs() < 1e-6, "buf[8] 应是 +sinθ：{}", rb[8]);
    }

    /// 🔴 **绑定姿势自检**：关节在绑定位时，骨骼变换结果必须与原始顶点一致。
    ///
    /// 这条同时检验两件事：矩阵相乘的顺序（`world * inverseBind`）、
    /// 以及 inverseBind 的方向（该是"世界 → 绑定"）。任一做错，位移都会是 1m 量级。
    ///
    /// 反向验证：把 `matrices()` 里的 `*w * *ib` 写成 `*ib * *w` ⇒ 顶点整体偏离 1m，红。
    #[test]
    fn glb_skin_bind_pose_roundtrips_without_moving_vertices() {
        let glb = make_glb(1.0);
        let (skin, positions) = parse_glb_skin(&glb).expect("解析").expect("应有骨骼");
        assert_eq!(skin.joint_count(), 2, "关节数");
        assert_eq!(skin.vertex_count(), 3, "骨骼变换顶点数");
        let err = bind_pose_error(&skin, &positions);
        assert!(err < 1e-5, "绑定姿势下顶点不许动，最大位移 {err}");
    }

    /// 🔴 **动一根骨头，只有受它影响的顶点动**，且位移量与权重成正比。
    ///
    /// 关节 1 从 y=1 挪到 y=2 ⇒ 骨骼变换矩阵变成"向上平移 1m"：
    /// v1（权重 1）该 +1，v2（权重 0.5）该 +0.5，v0（权重 0）该不动。
    #[test]
    fn glb_skin_moves_only_the_weighted_vertices() {
        let glb = make_glb(2.0); // 关节 1 的世界位置改成 (0,2,0)，inverseBind 仍是 (0,-1,0)
        let (skin, positions) = parse_glb_skin(&glb).expect("解析").expect("应有骨骼");
        let mats = skin.matrices();
        let d0 = skin.skin_vertex(0, positions[0], &mats) - positions[0];
        let d1 = skin.skin_vertex(1, positions[1], &mats) - positions[1];
        let d2 = skin.skin_vertex(2, positions[2], &mats) - positions[2];
        assert!(d0.length() < 1e-5, "v0 权重在关节 0，不该动：{d0:?}");
        assert!((d1.y - 1.0).abs() < 1e-5, "v1 该 +1m，实际 {d1:?}");
        assert!((d2.y - 0.5).abs() < 1e-5, "v2 权重 0.5 该 +0.5m，实际 {d2:?}");
    }

    /// 🔴 **乘序在旋转下才有区别** —— 纯平移的夹具测不出顺序（平移可交换）。
    ///
    /// 本会话第三条恒真判据：用平移夹具时，把 `world * inverseBind` 写成
    /// `inverseBind * world` **仍然全绿**（T(a)·T(b) == T(b)·T(a)）。真被测的是这条：
    /// 关节 1 绑定姿势在 (0,1,0)（IB = 平移 (0,-1,0)），摆姿势时**绕 Z 转 90°**：
    /// - 正确顺序 `W·IB`：先把顶点挪回绑定位（y-1），再旋转+平移 ⇒ v=(0,1,0) 留在原地；
    /// - 写反 `IB·W`：先旋转平移 ⇒ (-1,1,0)，再 y-1 ⇒ **(-1,0,0)**，位移 1.41m。
    #[test]
    fn skin_matrix_order_matters_under_rotation() {
        let bind_world = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let posed_world = bind_world * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let skin = Skin {
            joint_world: vec![Mat4::IDENTITY, posed_world],
            inverse_bind: vec![Mat4::IDENTITY, bind_world.inverse()],
            joints: vec![[0, 0, 0, 0], [1, 0, 0, 0]],
            weights: vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        };
        let p = Vec3::new(0.0, 1.0, 0.0);
        let mats = skin.matrices();
        let q = skin.skin_vertex(1, p, &mats);
        // 关节绕自己的原点转 ⇒ 顶点相对关节的位置随转，但这里顶点正好在关节原点上，
        // 所以只受平移影响，仍应停在 (0,1,0)。
        assert!(
            (q - p).length() < 1e-5,
            "顶点在关节原点，转多少都不该动：期望 {p:?}，实际 {q:?}"
        );
    }


    /// 🔴 **真实 Blender 导出**的骨骼资产（`tools/fixtures/skinned_bar.glb`）。
    ///
    /// 用 `include_bytes!` 而不是读盘：仓库的架构判据禁止生产代码直接 `std::fs`，
    /// 而测试文件里读盘又只能证明"文件在"，证明不了"解析对"。
    /// 3.7KB 编进测试二进制，任何时候都能跑。
    ///
    /// 这个夹具比手工拼的字节多了三样东西：`skins.joints` 顺序是 `[1, 0]`（不是自然的
    /// 升序）、索引用 u16、动画有 6 条通道 —— 都是合成夹具覆盖不到的。
    ///
    /// 期望值不是"看着像"，而是**独立的 Python 实现**逐顶点算出来的：
    /// v0/v1 不动，v2 从 (0,2,0) 到 (-0.5,1.5,0)，位移 0.7070。
    #[test]
    fn real_blender_skinned_asset_parses_and_binds() {
        let glb = include_bytes!("../../tools/fixtures/skinned_bar.glb");
        let (skin, positions) = parse_glb_skin(glb)
            .expect("解析真实资产")
            .expect("应该有骨骼");
        assert_eq!(skin.joint_count(), 2, "Blender 导出应有 2 个关节");
        assert_eq!(skin.vertex_count(), 3);
        let err = bind_pose_error(&skin, &positions);
        assert!(err < 1e-4, "绑定姿势下顶点不许动，最大位移 {err}");
    }

    /// 🔴 真实资产的**动画驱动**：采样到末帧后，50/50 权重的顶点位移必须 ≈ 0.7070。
    ///
    /// 手算：b1 绕 Z 转 90°、轴心 (0,1,0)；v2=(0,2,0) 转后 (-1,1,0)，
    /// 与未旋转的 (0,2,0) 各取一半 ⇒ (-0.5,1.5,0)，位移 |(0.5,0.5,0)| = 0.7071。
    ///
    /// 反向验证：把 `posed_positions` 里的 `w * ib` 写成 `ib * w`（本夹具的 IB 是纯平移、
    /// 旋转不可交换）⇒ 位移偏离，红。
    #[test]
    fn real_blender_asset_animation_moves_the_weighted_vertex() {
        let glb = include_bytes!("../../tools/fixtures/skinned_bar.glb");
        let (json, _bin) = split_glb(glb).expect("拆块");
        let (skin, positions) = parse_glb_skin(glb).expect("解析").expect("有骨骼");
        let channels = crate::engine::anim::parse_glb_animations(glb).expect("动画");
        assert_eq!(channels.len(), 6, "Blender 导出应有 6 条通道（两个节点 × TRS）");
        let joints: Vec<usize> = json
            .get("skins")
            .and_then(|s| s.as_arr())
            .and_then(|s| s.first())
            .and_then(|s| s.get("joints"))
            .and_then(|j| j.as_arr())
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as usize).collect())
            .unwrap_or_default();
        let worlds = crate::engine::anim::sample_pose_worlds(&json, 0, &channels, 1.042);
        let posed = crate::engine::anim::posed_positions(&skin, &joints, &positions, &worlds);
        let mut worst = 0.0f32;
        let mut at = 0;
        for (i, (a, b)) in positions.iter().zip(&posed).enumerate() {
            let d = (*b - *a).length();
            if d > worst {
                worst = d;
                at = i;
            }
        }
        assert_eq!(at, 2, "位移最大的应是 50/50 权重的 v2，实际 v{at}");
        assert!(
            (worst - 0.7071).abs() < 5e-3,
            "v2 位移应约 0.7071（独立 Python 实现算得 0.7070），实际 {worst}"
        );
        // 另外两个顶点在旋转轴上，必须不动
        assert!((posed[0] - positions[0]).length() < 1e-4);
        assert!((posed[1] - positions[1]).length() < 1e-4);
    }
    /// 没有骨骼的 GLB 必须返回 `Ok(None)`（不是错误）—— 本仓现有 53 件资产全是这种。
    #[test]
    fn glb_without_skins_is_not_an_error() {
        // 整段不写 `skins`（**不能**拿二进制当字符串替换：会把 GLB 改成非法数据，
        // 第一次就是这么写的，报的是"JSON 块长度越界"——一个与被测行为无关的错误）
        let glb = make_glb_ex(1.0, false);
        assert!(
            parse_glb_skin(&glb).expect("不该报错").is_none(),
            "没有骨骼的模型必须返回 None"
        );
    }
}
