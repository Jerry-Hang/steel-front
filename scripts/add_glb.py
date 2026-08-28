# -*- coding: utf-8 -*-
import io
p = 'src/engine/assets.rs'
s = io.open(p, encoding='utf-8').read()
add = '''

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
    // 取第一个 mesh 的 primitives[0]（静态静态模型简化：多基元后续合并/多材质后续扩展）
    let prim = json
        .get("meshes")
        .and_then(|m| m.as_arr())
        .and_then(|m| m.first())
        .and_then(|m| m.get("primitives"))
        .and_then(|p| p.as_arr())
        .and_then(|p| p.first())
        .ok_or("GLB 无 mesh/primitives".to_string())?;
    let base_color = prim
        .get("material")
        .and_then(|mi| mi.as_f64())
        .and_then(|mi| json.get("materials"))
        .and_then(|m| m.as_arr())
        .and_then(|m| m.get(mi as usize))
        .and_then(|m| m.get("pbrMetallicRoughness"))
        .and_then(|m| m.get("baseColorFactor"))
        .and_then(|m| m.as_arr())
        .map(|c| [c[0].as_f64().unwrap_or(0.7) as f32, c[1].as_f64().unwrap_or(0.7) as f32, c[2].as_f64().unwrap_or(0.7) as f32])
        .unwrap_or([0.7, 0.7, 0.7]);
    // accessor 读取（float32 / u32 indices，无 stride 简化——绝大多数导出器默认密集布局）
    fn read_acc(json: &crate::llm_cmd::Json, bin: &[u8], idx: usize, comps: usize, ty: u8) -> Result<Vec<f32>, String> {
        let acc = json.get("accessors").and_then(|a| a.as_arr()).and_then(|a| a.get(idx)).ok_or("accessor 缺失")?;
        let count = acc.get("count").and_then(|c| c.as_f64()).unwrap_or(0.0) as usize;
        let bv = acc.get("bufferView").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let off = json.get("bufferViews").and_then(|b| b.as_arr()).and_then(|b| b.get(bv))
            .and_then(|b| b.get("byteOffset")).and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let mut out = Vec::with_capacity(count * comps);
        for i in 0..count * comps {
            let b = off + i * if ty == 2 { 4 } else { 4 };
            if b + 4 > bin.len() {
                return Err("GLB accessor 越界".into());
            }
            let v = if ty == 2 {
                f32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]])
            } else {
                f32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]])
            };
            out.push(v);
        }
        Ok(out)
    }
    let pos = read_acc(&json, bin, prim.get("attributes").and_then(|a| a.get("POSITION")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 3, 2)?;
    let nrm = read_acc(&json, bin, prim.get("attributes").and_then(|a| a.get("NORMAL")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 3, 2)?;
    let uv = read_acc(&json, bin, prim.get("attributes").and_then(|a| a.get("TEXCOORD_0")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 2, 2)?;
    let ind = read_acc(&json, bin, prim.get("indices").and_then(|i| i.as_f64()).unwrap_or(0.0) as usize, 1, 2)?;
    let mut verts = Vec::with_capacity(pos.len() / 3);
    for i in 0..pos.len() / 3 {
        let n = if i * 3 + 2 < nrm.len() { [nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2]] } else { [0.0, 1.0, 0.0] };
        let t = if i * 2 + 1 < uv.len() { [uv[i * 2], uv[i * 2 + 1]] } else { [0.0, 0.0] };
        verts.push([pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2], n[0], n[1], n[2], t[0], t[1]]);
    }
    Ok(ImportedMesh { verts, indices: ind.iter().map(|v| *v as u32).collect(), base_color })
}
'''
s += add
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('glb added')
