//! 外部资产导入管线（2026-08-26 用户决策：取消全程序化限制，引入外部模型/贴图）
//! - 模型：glTF 2.0 GLB（JSON chunk + BIN chunk）——零第三方依赖手写解析
//! - 摆放：`assets/props/*.glb` 目录扫描（见 `props.rs::load_dir`）
//!
//! 2026-09-08：删除手写 OBJ 解析器与 Windows GDI+ 图片解码模块。前者从未被任何加载路径
//! 调用过（仓库内也没有 .obj 资产，建模管线是 Blender → GLB），后者被 `image` crate 取代
//! （`renderer.rs::init_texture`），且带一个 `static mut` 的 UB 隐患。两条都是纯死代码，
//! 需要时从 git 历史取回即可。

/// 导入网格（位置/法线/UV/顶点色，逐面平台化单位数据）
#[derive(Debug, Clone)]
pub struct ImportedMesh {
    /// 每顶点：pos(3) + normal(3) + uv(2) + 材质基色(3)（2026-08-27：多材质模型逐顶点保留）
    pub verts: Vec<[f32; 11]>, // pos(3) normal(3) uv(2) color(3)
    pub indices: Vec<u32>,
    /// 默认材质基色（模型无 COLOR_0 时的兜底）
    pub base_color: [f32; 3],
}

impl ImportedMesh {
    pub fn empty() -> Self {
        Self { verts: Vec::new(), indices: Vec::new(), base_color: [0.7, 0.7, 0.7] }
    }
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
    // accessor 读取（componentType 感知 + **byteStride 感知**）
    // 整型分量必须按 glTF 的 `normalized` 标志换算：Blender 5.x 导出的 COLOR_0 就是
    // VEC4 + UNSIGNED_SHORT + normalized，直接取原值会得到 0..65535 当 albedo 用。
    fn read_acc(json: &crate::llm_cmd::Json, bin: &[u8], idx: usize, comps: usize) -> Result<Vec<f32>, String> {
        let acc = json.get("accessors").and_then(|a| a.as_arr()).and_then(|a| a.get(idx)).ok_or("accessor 缺失")?;
        let count = acc.get("count").and_then(|c| c.as_f64()).unwrap_or(0.0) as usize;
        let ctype = acc.get("componentType").and_then(|c| c.as_f64()).unwrap_or(5126.0) as u32;
        let norm = acc.get("normalized").and_then(|b| b.as_bool()).unwrap_or(false);
        let bv = acc.get("bufferView").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let bview = json.get("bufferViews").and_then(|b| b.as_arr()).and_then(|b| b.get(bv));
        let off = bview
            .and_then(|b| b.get("byteOffset")).and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        // accessor 自己还能再偏一段：多个 accessor 挤在同一个 bufferView 里时，这是唯一的区分手段。
        // 以前不读它——ak12.glb 正是因此把 mesh0 的 NORMAL 读成了 POSITION、把 mesh1 读成了
        // mesh0 前 988 个顶点的副本。几何全错，却一句错误信息都没有。
        let off = off + acc.get("byteOffset").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        // 🔴 **`bufferViews[].byteStride`（2026-09-14 补，未结案 #21）** —— 与上面那段注释
        // 是**同一个 bug 的上一层**：`byteOffset` 修好之后，**交错缓冲**仍然会读错。
        //
        // 上面那个循环 `let b = off + i * step;` 走的是**密集**假设：每两个分量紧挨着。
        // 而 glTF 允许一个 bufferView 里把 POSITION/NORMAL/UV **交错**排布
        // （`byteStride` 就是"一个顶点占多少字节"），此时分量间距不是 `step` 而是
        // `stride`，且每个顶点还要按 `comps` 歇一段。**两种排布读出来的都是"合法浮点数"**
        // ⇒ 不崩、不报错、几何静静变成一团乱麻（`tools/glb_survey.py` 早就会报
        // "byteStride(交错缓冲,会读错几何)"，但那是离线工具，运行时没人拦）。
        //
        // 修法是**正确支持**而不是拒绝：`byteStride` 只在 bufferView 上出现，
        // 语义是"相邻两个**元素**（顶点）之间的字节数"；元素内部的分量仍按 `step` 连续。
        let stride = bview
            .and_then(|b| b.get("byteStride"))
            .and_then(|b| b.as_f64())
            .map(|v| v as usize)
            .filter(|&v| v > 0);
        // (bytes per component, divisor applied only when the spec says the value is normalized)
        let (step, div): (usize, f32) = match ctype {
            5120 => (1, if norm { 127.0 } else { 1.0 }),   // BYTE
            5121 => (1, if norm { 255.0 } else { 1.0 }),   // UNSIGNED_BYTE
            5122 => (2, if norm { 32767.0 } else { 1.0 }), // SHORT
            5123 => (2, if norm { 65535.0 } else { 1.0 }), // UNSIGNED_SHORT
            5125 => (4, 1.0),                              // UNSIGNED_INT (indices)
            5126 => (4, 1.0),                              // FLOAT
            // 🔴 2026-09-26：这里以前是 `_ => (4, 1.0)` —— **任何**未知 componentType
            // 都按 4 字节浮点读。与上面 byteOffset / byteStride 两处是同一个形状：
            // 4 字节整数当 f32 读出来**仍是合法浮点数**，于是坏资产不崩不报，
            // 几何静静变成一团乱麻。**读不了就明说读不了**（判据见
            // `glb_unknown_component_type_is_an_error_that_names_itself`）。
            other => {
                return Err(format!("GLB accessor 不支持的 componentType {other}"));
            }
        };
        let mut out = Vec::with_capacity(count * comps);
        // 交错时：元素间距 = `stride`，元素内分量间距 = `step`。
        // 密集时 `stride` 为 None ⇒ 退化成原来那句 `off + i * step`（逐位不变，
        // 所以**现有全部资产的行为不受影响**，这一点有测试锁着）。
        let elem_stride = stride.unwrap_or(step * comps);
        for i in 0..count * comps {
            let b = off + (i / comps) * elem_stride + (i % comps) * step;
            if b + step > bin.len() {
                return Err("GLB accessor 越界".into());
            }
            let v = match ctype {
                5125 => u32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]]) as f32,
                5123 => u16::from_le_bytes([bin[b], bin[b + 1]]) as f32,
                5122 => i16::from_le_bytes([bin[b], bin[b + 1]]) as f32,
                5121 => bin[b] as f32,
                5120 => bin[b] as i8 as f32,
                // 上面已把其余类型全部拒绝，这里只可能是 FLOAT（写 `_` 会让"漏了一种类型"
                // 重新变成静默路径）。
                5126 => f32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]]),
                other => unreachable!("componentType {other} 已在上面被拒"),
            };
            out.push(v / div);
        }
        Ok(out)
    }
    // 属性缺失时**必须返回空**，不能退回 accessor 0：旧写法 `unwrap_or(0.0)` 会把
    // accessor 0（通常是 POSITION）当成缺失的法线/UV/索引来读，于是"没有 UV 的网格"
    // 拿到的是"位置当 UV"，几何与着色全错却零报错。下游已按长度做了兜底
    // （法线缺 → (0,1,0)，UV 缺 → (0,0)），空向量才是安全值。
    // 🔴 2026-09-26：**accessor 读失败**与**属性缺失**是两件事，以前被 `unwrap_or_default()`
    // 合并成一件 —— 坏 accessor 退化成"空属性"，错误信息于是变成"缺少 POSITION"
    // （把人指向错的方向）；NORMAL/UV 更糟：它们本来就有"缺了就回退默认值"的兜底，
    // 一个读不了的 accessor 会伪装成"这个模型没导出法线"，纯平着色下看不出异常。
    // ⇒ **属性缺失走默认值，accessor 读不了就报错**（判据
    // `glb_broken_normal_accessor_is_not_silently_dropped`）。
    let attr = |name: &str, comps: usize| -> Result<Vec<f32>, String> {
        match prim.get("attributes").and_then(|a| a.get(name)).and_then(|p| p.as_f64()) {
            Some(i) => read_acc(json, bin, i as usize, comps)
                .map_err(|e| format!("GLB {name}: {e}")),
            None => Ok(Vec::new()),
        }
    };
    let pos = attr("POSITION", 3)?;
    let nrm = attr("NORMAL", 3)?;
    let uv = attr("TEXCOORD_0", 2)?;
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
        read_acc(json, bin, ci as usize, ty)
            .map_err(|e| format!("GLB COLOR_0: {e}"))?
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

    /// 交错缓冲（`bufferViews[].byteStride`）必须按 stride 跳顶点，不能按分量紧挨着读。
    ///
    /// 这是上一条测试的**上一层**：`accessor.byteOffset` 修好之后，"一个 bufferView 里
    /// POSITION/NORMAL/UV 交错排布"仍然会被读错，而且**读出来的每个数都是合法浮点数** ——
    /// 不崩、不报错、几何静静变成乱麻（`tools/glb_survey.py` 早就会报这件事，但它是离线工具）。
    ///
    /// 造法：一个顶点占 32 字节 = pos(12) + nrm(12) + uv(8)，三个顶点交错排布。
    #[test]
    fn glb_honours_buffer_view_byte_stride() {
        // 逐顶点交错：pos.xyz | nrm.xyz | uv.xy
        let verts: [[f32; 8]; 3] = [
            [0.0, 0.0, 0.0, /**/ 0.0, 0.0, 1.0, /**/ 0.25, 0.75],
            [1.0, 0.0, 0.0, /**/ 0.0, 1.0, 0.0, /**/ 0.50, 0.50],
            [0.0, 1.0, 0.0, /**/ 1.0, 0.0, 0.0, /**/ 0.10, 0.20],
        ];
        let mut bin: Vec<u8> = Vec::new();
        for v in verts {
            for f in v {
                bin.extend_from_slice(&f.to_le_bytes());
            }
        }
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":
{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}}]}],
"accessors":[
{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC3"},
{"bufferView":0,"byteOffset":12,"componentType":5126,"count":3,"type":"VEC3"},
{"bufferView":0,"byteOffset":24,"componentType":5126,"count":3,"type":"VEC2"}],
"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":96,"byteStride":32,"target":34962}]}"#;
        let m = parse_glb(&build_glb(json, &bin)).unwrap();
        assert_eq!(m.verts.len(), 3);
        // 逐顶点核对：位置/法线/UV 三者都要来自**同一个顶点**，而不是被 stride 串位
        for (i, want) in verts.iter().enumerate() {
            assert_eq!(&m.verts[i][..3], &want[0..3], "顶点 {i} 位置错");
            assert_eq!(&m.verts[i][3..6], &want[3..6], "顶点 {i} 法线错（stride 没生效？）");
            assert_eq!(&m.verts[i][6..8], &want[6..8], "顶点 {i} UV 错（stride 没生效？）");
        }
        // 若无 stride 支持，第 1 个顶点的法线会读到 pos[2] 附近的值而不是 (0,0,1)
        assert_ne!(&m.verts[0][3..6], &m.verts[1][..3], "法线读成了下一个顶点的位置");
    }

    /// 🔴 判据：**读不了的 `componentType` 必须报错，而且要说得出是它**。
    ///
    /// 旧写法是 `_ => (4, 1.0)`：任何未知类型（例如 5124 INT）都按 4 字节浮点读。
    /// 4 字节整数当 f32 读出来**仍然是合法浮点数** —— 与上面 `byteOffset` / `byteStride`
    /// 两处是同一个形状：不崩、不报错，几何静静变成一团乱麻。
    /// 更早一层还有 `attr()` 的 `unwrap_or_default()`：accessor 读失败会被吞成"空"，
    /// 于是错误信息变成"缺少 POSITION"（把人指向错误的方向）。
    #[test]
    fn glb_unknown_component_type_is_an_error_that_names_itself() {
        let bin: Vec<u8> = (0..36u8).collect();
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}}]}],"accessors":[{"bufferView":0,"byteOffset":0,"componentType":5124,"count":3,"type":"VEC3"},{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC3"},{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC2"}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962}]}"#;
        let err = parse_glb(&build_glb(json, &bin))
            .err()
            .expect("未知 componentType 必须被拒（否则是静默错几何）");
        assert!(
            err.contains("componentType"),
            "错误信息必须点出 componentType，实际: {err}"
        );
        assert!(
            !err.contains("缺少 POSITION"),
            "读不了的 accessor 不能被吞成\"属性缺失\"（那会把人指向错方向），实际: {err}"
        );
    }

    /// 同一形状的第二面：**NORMAL 的 accessor 读不了也不能静默回退**。
    ///
    /// 现在 `nrm` 短了会按 (0,1,0) 兜底（那是为"属性本来就没导出"设计的），
    /// 于是一个坏 accessor 会伪装成"这个模型没有法线"，纯平着色看不出异常。
    #[test]
    fn glb_broken_normal_accessor_is_not_silently_dropped() {
        let bin: Vec<u8> = (0..36u8).collect();
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1}}]}],"accessors":[{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC3"},{"bufferView":0,"byteOffset":0,"componentType":5124,"count":3,"type":"VEC3"}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962}]}"#;
        let err = parse_glb(&build_glb(json, &bin))
            .err()
            .expect("坏掉的 NORMAL accessor 必须被拒，不能静默当成\"没有法线\"");
        assert!(err.contains("NORMAL"), "错误信息要点出是哪个属性，实际: {err}");
    }
}

