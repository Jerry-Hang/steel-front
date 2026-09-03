//! 外部资产导入管线（2026-08-26 用户决策：取消全程序化限制，引入外部模型/贴图）
//! - 模型：OBJ（文本） / glTF 2.0 GLB（JSON chunk + BIN chunk）——零第三方依赖手写解析
//! - 贴图：PNG/JPEG 经 Windows WIC（系统组件）解码为 RGBA8（零第三方库）
//! - 摆放：assets/props.toml（复用 map.rs 的 TOML 解析器，可选）

use glam::{Mat4, Vec3};

/// 导入网格（位置/法线/UV/顶点色，逐面平台化单位数据）
#[derive(Debug, Clone)]
pub struct ImportedMesh {
    /// 每顶点：pos(3) + normal(3) + uv(2) + 材质基色(3)（2026-08-27：多材质模型逐顶点保留）
    pub verts: Vec<[f32; 11]>, // pos(3) normal(3) uv(2) color(3)
    pub indices: Vec<u32>,
    /// 默认材质基色（OBJ 模型用）
    pub base_color: [f32; 3],
}

impl ImportedMesh {
    pub fn empty() -> Self {
        Self { verts: Vec::new(), indices: Vec::new(), base_color: [0.7, 0.7, 0.7] }
    }
}

/// 解析 OBJ（仅 v/vt/vn/f 子集；f 支持 3-4 顶点面并三角化）
pub fn parse_obj(text: &str) -> Result<ImportedMesh, String> {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut uv: Vec<[f32; 2]> = Vec::new();
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    let mut verts: Vec<[f32; 11]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let mut remap: std::collections::HashMap<(i32, i32, i32), u32> = std::collections::HashMap::new();
    fn push_face(
        remap: &mut std::collections::HashMap<(i32, i32, i32), u32>,
        pos: &[[f32; 3]],
        uv: &[[f32; 2]],
        nrm: &[[f32; 3]],
        fields: &[&str],
        verts: &mut Vec<[f32; 11]>,
        indices: &mut Vec<u32>,
    ) -> Result<(), String> {
        let mut corner: Vec<u32> = Vec::new();
        for f in fields {
            let parts: Vec<&str> = f.split('/').collect();
            let vi: i32 = parts[0].parse().map_err(|_| format!("OBJ 顶点索引非法: {f}"))?;
            let (ti, ni) = match parts.len() {
                1 => (0, 0),
                2 => (parts[1].parse().unwrap_or(0), 0),
                _ => (parts[1].parse().unwrap_or(0), parts[2].parse().unwrap_or(0)),
            };
            let key = (vi, ti, ni);
            let idx = *remap.entry(key).or_insert_with(|| {
                let p = if vi > 0 {
                    pos[(vi - 1) as usize]
                } else {
                    pos[(pos.len() as i32 + vi) as usize]
                };
                let t = if ti > 0 {
                    uv[(ti - 1) as usize]
                } else {
                    [0.0, 0.0]
                };
                let n = if ni > 0 {
                    nrm[(ni - 1) as usize]
                } else {
                    [0.0, 1.0, 0.0]
                };
                verts.push([p[0], p[1], p[2], n[0], n[1], n[2], t[0], t[1], 0.7, 0.7, 0.7]);
                (verts.len() - 1) as u32
            });
            corner.push(idx);
        }
        // 三角化（fan）
        for k in 1..corner.len().saturating_sub(1) {
            indices.push(corner[0]);
            indices.push(corner[k]);
            indices.push(corner[k + 1]);
        }
        Ok(())
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let kw = it.next().unwrap_or("");
        let rest: Vec<&str> = it.collect();
        match kw {
            "v" => {
                if rest.len() >= 3 {
                    pos.push([
                        rest[0].parse().map_err(|_| "OBJ v 非法")?,
                        rest[1].parse().map_err(|_| "OBJ v 非法")?,
                        rest[2].parse().map_err(|_| "OBJ v 非法")?,
                    ]);
                }
            }
            "vt" => {
                if rest.len() >= 2 {
                    uv.push([
                        rest[0].parse().map_err(|_| "OBJ vt 非法")?,
                        rest[1].parse().map_err(|_| "OBJ vt 非法")?,
                    ]);
                }
            }
            "vn" => {
                if rest.len() >= 3 {
                    nrm.push([
                        rest[0].parse().map_err(|_| "OBJ vn 非法")?,
                        rest[1].parse().map_err(|_| "OBJ vn 非法")?,
                        rest[2].parse().map_err(|_| "OBJ vn 非法")?,
                    ]);
                }
            }
            "f" => {
                push_face(&mut remap, &pos, &uv, &nrm, &rest, &mut verts, &mut indices)?;
            }
            _ => {}
        }
    }
    let _ = &nrm;
    Ok(ImportedMesh { verts, indices, base_color: [0.7, 0.7, 0.7] })
}

// ---------------------------------------------------------------------------
// GLB（glTF 2.0 二进制）：JSON chunk（用 llm_cmd 迷你 JSON 解析器，零依赖）+ BIN chunk
// ---------------------------------------------------------------------------
pub fn parse_glb(bytes: &[u8]) -> Result<ImportedMesh, String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("非 GLB 文件（缺少 glTF magic）".into());
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_str = std::str::from_utf8(&bytes[20..20 + json_len]).map_err(|_| "GLB JSON 非 UTF-8")?;
    let json = crate::llm_cmd::parse_json_fn(json_str).map_err(|e| format!("GLB JSON 解析失败: {e}"))?;
    let bin_start = 20 + json_len;
    let bin = if bin_start + 8 <= bytes.len() {
        let blen = u32::from_le_bytes([bytes[bin_start], bytes[bin_start + 1], bytes[bin_start + 2], bytes[bin_start + 3]]) as usize;
        &bytes[bin_start + 8..(bin_start + 8 + blen).min(bytes.len())]
    } else {
        &[]
    };
    // 遍历全部 mesh 的 primitives（多材质合并：一个 GLB = 一个 ImportedMesh）
    let meshes = json
        .get("meshes")
        .and_then(|m| m.as_arr())
        .ok_or("GLB 无 mesh".to_string())?;
    let mut out = ImportedMesh::empty();
    let mut vert_offset = 0u32;
    for m in meshes {
        let prims = m
            .get("primitives")
            .and_then(|p| p.as_arr())
            .ok_or("primitive 缺失".to_string())?;
        for prim in prims {
            let base_color = match prim.get("material").and_then(|v| v.as_f64()) {
                Some(mi) => json
                    .get("materials")
                    .and_then(|v| v.as_arr())
                    .and_then(|v| v.get(mi as usize))
                    .and_then(|v| v.get("pbrMetallicRoughness"))
                    .and_then(|v| v.get("baseColorFactor"))
                    .and_then(|v| v.as_arr())
                    .map(|c| [c[0].as_f64().unwrap_or(0.7) as f32, c[1].as_f64().unwrap_or(0.7) as f32, c[2].as_f64().unwrap_or(0.7) as f32])
                    .unwrap_or([0.7, 0.7, 0.7]),
                None => [0.7, 0.7, 0.7],
            };
            append_prim(&json, bin, prim, base_color, &mut out, &mut vert_offset)?;
        }
    }
    Ok(out)
}

/// 追加单个 primitive（accessor 密集布局；顶点色=材质 baseColor；顶点/索引偏移拼接）
fn append_prim(
    json: &crate::llm_cmd::Json,
    bin: &[u8],
    prim: &crate::llm_cmd::Json,
    base_color: [f32; 3],
    out: &mut ImportedMesh,
    vert_offset: &mut u32,
) -> Result<(), String> {
    let _ = prim.get("material");
    // accessor 读取（componentType 感知；无 byteStride 简化——绝大多数导出器默认密集布局）
    // 整型分量必须按 glTF 的 `normalized` 标志换算：Blender 5.x 导出的 COLOR_0 就是
    // VEC4 + UNSIGNED_SHORT + normalized，直接取原值会得到 0..65535 当 albedo 用。
    fn read_acc(json: &crate::llm_cmd::Json, bin: &[u8], idx: usize, comps: usize) -> Result<Vec<f32>, String> {
        let acc = json.get("accessors").and_then(|a| a.as_arr()).and_then(|a| a.get(idx)).ok_or("accessor 缺失")?;
        let count = acc.get("count").and_then(|c| c.as_f64()).unwrap_or(0.0) as usize;
        let ctype = acc.get("componentType").and_then(|c| c.as_f64()).unwrap_or(5126.0) as u32;
        let norm = acc.get("normalized").and_then(|b| b.as_bool()).unwrap_or(false);
        let bv = acc.get("bufferView").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let off = json.get("bufferViews").and_then(|b| b.as_arr()).and_then(|b| b.get(bv))
            .and_then(|b| b.get("byteOffset")).and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        // accessor 自己还能再偏一段：多个 accessor 挤在同一个 bufferView 里时，这是唯一的区分手段。
        // 以前不读它——ak12.glb 正是因此把 mesh0 的 NORMAL 读成了 POSITION、把 mesh1 读成了
        // mesh0 前 988 个顶点的副本。几何全错，却一句错误信息都没有。
        let off = off + acc.get("byteOffset").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        // (bytes per component, divisor applied only when the spec says the value is normalized)
        let (step, div): (usize, f32) = match ctype {
            5120 => (1, if norm { 127.0 } else { 1.0 }),   // BYTE
            5121 => (1, if norm { 255.0 } else { 1.0 }),   // UNSIGNED_BYTE
            5122 => (2, if norm { 32767.0 } else { 1.0 }), // SHORT
            5123 => (2, if norm { 65535.0 } else { 1.0 }), // UNSIGNED_SHORT
            5125 => (4, 1.0),                              // UNSIGNED_INT (indices)
            _ => (4, 1.0),                                 // FLOAT
        };
        let mut out = Vec::with_capacity(count * comps);
        for i in 0..count * comps {
            let b = off + i * step;
            if b + step > bin.len() {
                return Err("GLB accessor 越界".into());
            }
            let v = match ctype {
                5125 => u32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]]) as f32,
                5123 => u16::from_le_bytes([bin[b], bin[b + 1]]) as f32,
                5122 => i16::from_le_bytes([bin[b], bin[b + 1]]) as f32,
                5121 => bin[b] as f32,
                5120 => bin[b] as i8 as f32,
                _ => f32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]]),
            };
            out.push(v / div);
        }
        Ok(out)
    }
    // 属性缺失时**必须返回空**，不能退回 accessor 0：旧写法 `unwrap_or(0.0)` 会把
    // accessor 0（通常是 POSITION）当成缺失的法线/UV/索引来读，于是"没有 UV 的网格"
    // 拿到的是"位置当 UV"，几何与着色全错却零报错。下游已按长度做了兜底
    // （法线缺 → (0,1,0)，UV 缺 → (0,0)），空向量才是安全值。
    let attr = |name: &str, comps: usize| -> Vec<f32> {
        match prim.get("attributes").and_then(|a| a.get(name)).and_then(|p| p.as_f64()) {
            Some(i) => read_acc(json, bin, i as usize, comps).unwrap_or_default(),
            None => Vec::new(),
        }
    };
    let pos = attr("POSITION", 3);
    let nrm = attr("NORMAL", 3);
    let uv = attr("TEXCOORD_0", 2);
    // 顶点色（Blender 烘焙 COLOR_0；有则优先于材质基色）。分量数按 accessor 的 type 取，
    // 后面也必须按同一个数寻址——写死 4 会让 VEC3 颜色逐顶点错位，越界后静默退回基色。
    let col = prim.get("attributes").and_then(|a| a.get("COLOR_0")).and_then(|p| p.as_f64());
    let mut col_stride = 0usize;
    let colv = if let Some(ci) = col {
        let ty = json
            .get("accessors")
            .and_then(|a| a.as_arr())
            .and_then(|a| a.get(ci as usize))
            .and_then(|a| a.get("type"))
            .and_then(|t| t.as_str())
            .map(|s| if s == "VEC4" { 4 } else { 3 })
            .unwrap_or(3);
        col_stride = ty;
        read_acc(json, bin, ci as usize, ty).unwrap_or_default()
    } else {
        Vec::new()
    };
    if pos.is_empty() {
        return Err("GLB primitive 缺少 POSITION".into());
    }
    // 无索引图元（glTF 允许）按顺序生成索引，而不是去读 accessor 0
    let ind = match prim.get("indices").and_then(|i| i.as_f64()) {
        Some(i) => read_acc(json, bin, i as usize, 1)?,
        None => (0..pos.len() / 3).map(|i| i as f32).collect(),
    };
    let base = *vert_offset;
    for i in 0..pos.len() / 3 {
        let n = if i * 3 + 2 < nrm.len() { [nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2]] } else { [0.0, 1.0, 0.0] };
        let t = if i * 2 + 1 < uv.len() { [uv[i * 2], uv[i * 2 + 1]] } else { [0.0, 0.0] };
        // 逐顶点颜色：COLOR_0 烘焙色优先，其次材质基色
        let c = if col_stride >= 3 && (i + 1) * col_stride <= colv.len() {
            [colv[i * col_stride], colv[i * col_stride + 1], colv[i * col_stride + 2]]
        } else {
            base_color
        };
        out.verts.push([
            pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2],
            n[0], n[1], n[2],
            t[0], t[1],
            c[0], c[1], c[2],
        ]);
    }
    for v in &ind {
        out.indices.push((*v as u32) + base);
    }
    out.base_color = base_color;
    *vert_offset += (pos.len() / 3) as u32;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obj_parse_cube() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        let m = parse_obj(obj).unwrap();
        assert_eq!(m.verts.len(), 4);
        assert_eq!(m.indices.len(), 6, "四边形应三角化为 2 个三角形");
    }
    #[test]
    fn obj_parse_tri() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nf 1//1 2//1 3//1\n";
        let m = parse_obj(obj).unwrap();
        assert_eq!(m.verts.len(), 3);
        assert_eq!(m.indices, vec![0, 1, 2]);
    }
    #[test]
    fn glb_baked_color_and_index() {
        let p = "assets/guns/ak12_baked.glb";
        if std::path::Path::new(p).exists() {
            let m = parse_glb(&std::fs::read(p).unwrap()).unwrap();
            assert!(m.verts[0][8] < 0.5, "首网格顶点色应深色, 实际 {:?}", &m.verts[0][8..11]);
            let max_idx = m.indices.iter().take(1000).copied().max().unwrap_or(0);
            assert!(max_idx < m.verts.len() as u32, "首索引越界 {}", max_idx);
        }
    }

    #[test]
    fn glb_parses_real_ak12() {
        let p = "assets/guns/ak12_baked.glb";
        if std::path::Path::new(p).exists() {
            let bytes = std::fs::read(p).unwrap();
            let m = parse_glb(&bytes).unwrap();
            assert!(m.verts.len() > 1000);
            // 顶点色应为烘焙的深色（0.05-0.35），不是白色
            let c0 = &m.verts[0][8..11];
            assert!(
                c0[0] < 0.5 && c0[1] < 0.5 && c0[2] < 0.5,
                "烘焙顶点色应深色，实际 {:?}",
                c0
            );
        }
    }

    #[test]
    fn glb_parses_real_ak12_orig() {
        let p = "assets/guns/ak12.glb";
        if std::path::Path::new(p).exists() {
            let bytes = std::fs::read(p).unwrap();
            let m = parse_glb(&bytes).unwrap();
            assert!(m.verts.len() > 1000, "AK12 顶点应上千，实际 {}", m.verts.len());
            assert!(m.indices.len() >= m.verts.len());
        }
    }

    /// 拼一个最小合法 GLB（JSON chunk + BIN chunk），供解析器做无磁盘依赖的回归测试
    fn build_glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut jb = json.as_bytes().to_vec();
        while jb.len() % 4 != 0 {
            jb.push(b' ');
        }
        let mut bb = bin.to_vec();
        while bb.len() % 4 != 0 {
            bb.push(0);
        }
        let total = 12 + 8 + jb.len() + 8 + bb.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&[0x67, 0x6C, 0x54, 0x46]); // "glTF"
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(jb.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
        out.extend_from_slice(&jb);
        out.extend_from_slice(&(bb.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
        out.extend_from_slice(&bb);
        out
    }

    /// 3 顶点三角形，POSITION/NORMAL/UV 固定，只有 COLOR_0 的 type 与 componentType 变
    fn glb_bin(colour: &[u8]) -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for f in v {
                b.extend_from_slice(&f.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for f in [0f32, 0.0, 1.0] {
                b.extend_from_slice(&f.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for f in [0.25f32, 0.75] {
                b.extend_from_slice(&f.to_le_bytes());
            }
        }
        b.extend_from_slice(colour);
        for i in 0..3u32 {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b
    }

    fn glb_json(colour_accessor: &str, colour_off: usize, colour_len: usize,
                idx_off: usize) -> String {
        // 单行：仓库自带的极简 JSON 解析器对结构位置上的换行不宽容，测试夹具没必要踩这个
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
            \"nodes\":[{{\"mesh\":0}}],\"meshes\":[{{\"primitives\":[{{\"attributes\":\
            {{\"POSITION\":0,\"NORMAL\":1,\"TEXCOORD_0\":2,\"COLOR_0\":3}},\"indices\":4}}]}}],\
            \"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},\
            {{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},\
            {{\"bufferView\":2,\"componentType\":5126,\"count\":3,\"type\":\"VEC2\"}},\
            {colour_accessor},\
            {{\"bufferView\":4,\"componentType\":5125,\"count\":3,\"type\":\"SCALAR\"}}],\
            \"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36,\"target\":34962}},\
            {{\"buffer\":0,\"byteOffset\":36,\"byteLength\":36,\"target\":34962}},\
            {{\"buffer\":0,\"byteOffset\":72,\"byteLength\":24,\"target\":34962}},\
            {{\"buffer\":0,\"byteOffset\":{colour_off},\"byteLength\":{colour_len},\"target\":34962}},\
            {{\"buffer\":0,\"byteOffset\":{idx_off},\"byteLength\":12,\"target\":34963}}]}}"
        )
    }

    /// Blender 5.x 导出的 COLOR_0 是 VEC4 + UNSIGNED_SHORT + normalized：必须除以 65535，
    /// 否则 albedo 拿到 0..65535 的原值。
    #[test]
    fn glb_color_u16_normalized_is_scaled() {
        let mut col = Vec::new();
        for c in [[4u16, 2u16, 1u16, 3u16], [8, 4, 2, 1], [16, 8, 4, 2]] {
            for v in c {
                col.extend_from_slice(&v.to_le_bytes());
            }
        }
        let json = glb_json(
            r#"{"bufferView":3,"componentType":5123,"count":3,"type":"VEC4","normalized":true}"#,
            96, 24, 120,
        );
        let m = parse_glb(&build_glb(&json, &glb_bin(&col))).unwrap();
        assert_eq!(m.verts.len(), 3);
        let want = [[4u32, 2, 1], [8, 4, 2], [16, 8, 4]];
        for (vi, v) in m.verts.iter().enumerate() {
            for k in 0..3 {
                let got = v[8 + k];
                let exp = want[vi][k] as f32 / 65535.0;
                assert!((got - exp).abs() < 1e-4,
                    "顶点 {vi} 通道 {k} 应归一化为 {exp}，实际 {got}");
            }
        }
    }

    /// VEC3 颜色必须按 stride 3 寻址：旧实现写死 4，第三个顶点会越界并静默退回材质基色。
    #[test]
    fn glb_color_vec3_addresses_by_three() {
        let mut col = Vec::new();
        for c in [[0.5f32, 0.25, 0.125], [0.75, 0.5, 0.25], [1.0, 0.0625, 0.5]] {
            for v in c {
                col.extend_from_slice(&v.to_le_bytes());
            }
        }
        let json = glb_json(
            r#"{"bufferView":3,"componentType":5126,"count":3,"type":"VEC3"}"#,
            96, 36, 132,
        );
        let m = parse_glb(&build_glb(&json, &glb_bin(&col))).unwrap();
        let last = &m.verts[2][8..11];
        assert!((last[0] - 1.0).abs() < 1e-6 && (last[1] - 0.0625).abs() < 1e-6
                && (last[2] - 0.5).abs() < 1e-6,
            "VEC3 第三个顶点色应按 stride 3 读到，实际 {last:?}");
    }

    /// 真实资产回归：Blender headless 导出的整套 props 必须能被本解析器吃下，且
    /// 顶点色落在 0..=1（u16 归一化没生效时会直接溢出到几万）、原点在底面（y 不为负太多）。
    #[test]
    fn glb_prop_kit_loads_with_valid_range() {
        let dir = std::path::Path::new("assets/props");
        if !dir.is_dir() {
            return; // 资产未生成时不失败（与仓库既有 glb_* 测试一致的容错风格）
        }
        let mut checked = 0usize;
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("glb"))
            .collect();
        entries.sort();
        for path in entries {
            let bytes = std::fs::read(&path).unwrap();
            let m = parse_glb(&bytes)
                .unwrap_or_else(|e| panic!("{} 解析失败: {e}", path.display()));
            assert!(m.verts.len() > 24, "{} 顶点过少 {}", path.display(), m.verts.len());
            assert_eq!(m.indices.len() % 3, 0, "{} 索引不是三角形", path.display());
            let max_idx = *m.indices.iter().max().unwrap_or(&0);
            assert!(max_idx < m.verts.len() as u32,
                "{} 索引越界 {max_idx} >= {}", path.display(), m.verts.len());
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            let mut clo = f32::MAX;
            let mut chi = f32::MIN;
            for v in &m.verts {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
                for k in 0..3 {
                    clo = clo.min(v[8 + k]);
                    chi = chi.max(v[8 + k]);
                }
            }
            assert!(clo >= 0.0 && chi <= 1.001,
                "{} 顶点色超出 0..=1（[{clo}, {chi}]）——normalized 分量没换算",
                path.display());
            assert!(chi > clo, "{} 顶点色全同值，COLOR_0 可能根本没读到", path.display());
            assert!(lo[1] > -1.0,
                "{} 原点不在底面（min.y={}）", path.display(), lo[1]);
            assert!(hi[1] < 40.0,
                "{} 高度异常（max.y={}），单位应为米", path.display(), hi[1]);
            checked += 1;
        }
        assert!(checked > 0, "assets/props 下没有 GLB");
    }

    /// 多个 accessor 挤在同一个 bufferView 里时，必须按 `accessor.byteOffset` 分开读。
    /// ak12.glb 的 NORMAL 之所以被读成 POSITION，就是这个字段根本没被读。
    #[test]
    fn glb_honours_accessor_byte_offset_within_shared_buffer_view() {
        // 一个 bufferView 装下 POSITION(36B) + NORMAL(36B)
        let mut bin: Vec<u8> = Vec::new();
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for f in v {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for f in [0f32, 0.0, 1.0] {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for f in [0.25f32, 0.75] {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        for i in 0..3u32 {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":
{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3}]}],
"accessors":[
{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC3"},
{"bufferView":0,"byteOffset":36,"componentType":5126,"count":3,"type":"VEC3"},
{"bufferView":0,"byteOffset":72,"componentType":5126,"count":3,"type":"VEC2"},
{"bufferView":0,"byteOffset":96,"componentType":5125,"count":3,"type":"SCALAR"}],
"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":108,"target":34962}]}"#;
        let m = parse_glb(&build_glb(json, &bin)).unwrap();
        assert_eq!(m.verts.len(), 3);
        // pos[0] = (0,0,0)，nrm[0] 必须是 (0,0,1) 而不是位置副本
        assert_eq!(&m.verts[0][..3], &[0.0, 0.0, 0.0]);
        assert_eq!(&m.verts[0][3..6], &[0.0, 0.0, 1.0], "NORMAL 被读成了 POSITION");
        assert_eq!(&m.verts[2][3..6], &[0.0, 0.0, 1.0]);
        assert_eq!(&m.verts[0][6..8], &[0.25, 0.75], "UV 偏移没算对");
    }

    /// 缺 NORMAL / TEXCOORD_0 / indices 时不得别名到 accessor 0（那会把位置当法线）。
    #[test]
    fn glb_missing_attributes_do_not_alias_accessor_zero() {
        let mut bin: Vec<u8> = Vec::new();
        for v in [[1f32, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]] {
            for f in v {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962}]}"#;
        let m = parse_glb(&build_glb(json, &bin)).unwrap();
        assert_eq!(m.verts.len(), 3);
        // 法线缺失 → 安全默认 (0,1,0)，绝不能是位置 (1,2,3)
        assert_eq!(&m.verts[0][3..6], &[0.0, 1.0, 0.0], "缺 NORMAL 时别名到了 accessor 0");
        assert_eq!(&m.verts[0][6..8], &[0.0, 0.0], "缺 UV 时别名到了 accessor 0");
        // 无索引图元应按顺序生成索引，而不是把位置当索引读
        assert_eq!(m.indices, vec![0, 1, 2], "缺 indices 时应生成顺序索引");
    }

    #[cfg(windows)]
    #[test]
    fn png_decode_rgba() {
        // 用仓库既有截图做解码验证
        for path in ["screenshots/view.png", "C:\\Users\\Jerry-Huang\\Pictures\\Screenshots\\屏幕截图 2026-08-25 213754.png"] {
            if std::path::Path::new(path).exists() {
                let (rgba, w, h) = gdi_img::load_rgba(path).unwrap();
                assert!(w > 0 && h > 0);
                assert_eq!(rgba.len(), (w * h * 4) as usize);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PNG/JPEG 解码：Windows GDI+（系统组件，零外部库）→ RGBA8
// 复用 launcher wallpaper.rs 的 GdiplusStartup 模式；LockBits 取 32bpp ARGB
// ---------------------------------------------------------------------------
#[cfg(windows)]
pub mod gdi_img {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    type GpStatus = i32;
    #[repr(C)]
    struct Startup { ver: u32, cb: *mut c_void, bg: i32, codecs: i32 }
    #[link(name = "gdiplus")]
    extern "system" {
        fn GdiplusStartup(t: *mut usize, i: *const Startup, o: *mut c_void) -> GpStatus;
        fn GdipLoadImageFromFile(f: *const u16, img: *mut *mut c_void) -> GpStatus;
        fn GdipGetImageWidth(img: *mut c_void, w: *mut u32) -> GpStatus;
        fn GdipGetImageHeight(img: *mut c_void, h: *mut u32) -> GpStatus;
        fn GdipBitmapLockBits(bmp: *mut c_void, rect: *mut c_void, flags: u32, fmt: u32, locked: *mut c_void) -> GpStatus;
        fn GdipBitmapUnlockBits(bmp: *mut c_void, locked: *mut c_void) -> GpStatus;
        fn GdipDisposeImage(img: *mut c_void) -> GpStatus;
        fn GdiplusShutdown(t: usize);
    }

    static mut TOKEN: usize = 0;
    fn startup() {
        unsafe {
            if TOKEN == 0 {
                let i = Startup { ver: 1, cb: std::ptr::null_mut(), bg: 0, codecs: 0 };
                GdiplusStartup(&mut TOKEN, &i, std::ptr::null_mut());
            }
        }
    }

    /// 解码图片文件 → RGBA8（32bpp ARGB 内存序 = R,G,B,A）
    pub fn load_rgba(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
        startup();
        let wide: Vec<u16> = std::path::Path::new(path).as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut img: *mut c_void = std::ptr::null_mut();
            let st = GdipLoadImageFromFile(wide.as_ptr(), &mut img);
            if st != 0 || img.is_null() {
                return Err(format!("图片加载失败（GDI+ 状态 {st}）: {path}"));
            }
            let (mut w, mut h) = (0u32, 0u32);
            let s3 = GdipGetImageWidth(img, &mut w);
            let s4 = GdipGetImageHeight(img, &mut h);
            // PixelFormat32bppARGB = 0x0026200A；LockUserInputBuffer = 0x00000001
            #[repr(C)]
            #[derive(Default)]
            struct Rect { x: i32, y: i32, w: i32, h: i32 }
            let rect = Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
            // GDI+ BitmapData 真实布局：Width(4) + Height(4) + Stride(4) + PixelFormat(4) + Scan0(8) + Reserved(4)
            #[repr(C)]
            #[derive(Default)]
            struct Locked {
                #[allow(dead_code)]
                width: u32,
                #[allow(dead_code)]
                height: u32,
                stride: i32,
                #[allow(dead_code)]
                pixel_format: u32,
                data: *mut u8,
                #[allow(dead_code)]
                reserved: u32,
            }
            let mut locked: Locked = std::mem::zeroed();
            let st = GdipBitmapLockBits(img, &rect as *const _ as *mut c_void, 0x00000001u32, 0x0026200Au32, &mut locked as *mut _ as *mut c_void);
            if st != 0 || locked.data.is_null() {
                GdipDisposeImage(img);
                return Err(format!("图片锁定失败（GDI+ 状态 {st}）"));
            }
            // stride 可能为负（GDI+ bottom-up）：绝对化 + BGR 序采样
            let stride = locked.stride.unsigned_abs() as usize;
            let row0 = std::slice::from_raw_parts(locked.data, (w as usize).min(8) * 4);
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(locked.data.add(y * stride), (w as usize) * 4);
                for p in row.chunks_exact(4) {
                    out.extend_from_slice(&[p[2], p[1], p[0], p[3]]); // BGRA→RGBA
                }
            }
            GdipBitmapUnlockBits(img, &mut locked as *mut _ as *mut c_void);
            GdipDisposeImage(img);
            Ok((out, w, h))
        }
    }
}
