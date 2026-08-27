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
    // accessor 读取（componentType 感知：5126=f32 顶点/法线/UV，5125=u32 索引，5123=u16 索引；
    // 无 byteStride 简化——绝大多数导出器默认密集布局）
    fn read_acc(json: &crate::llm_cmd::Json, bin: &[u8], idx: usize, comps: usize, _ty: u8) -> Result<Vec<f32>, String> {
        let acc = json.get("accessors").and_then(|a| a.as_arr()).and_then(|a| a.get(idx)).ok_or("accessor 缺失")?;
        let count = acc.get("count").and_then(|c| c.as_f64()).unwrap_or(0.0) as usize;
        let ctype = acc.get("componentType").and_then(|c| c.as_f64()).unwrap_or(5126.0) as u32;
        let bv = acc.get("bufferView").and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let off = json.get("bufferViews").and_then(|b| b.as_arr()).and_then(|b| b.get(bv))
            .and_then(|b| b.get("byteOffset")).and_then(|b| b.as_f64()).unwrap_or(0.0) as usize;
        let step = match ctype {
            5125 => 4, // u32
            5123 => 2, // u16
            _ => 4,    // f32
        };
        let mut out = Vec::with_capacity(count * comps);
        for i in 0..count * comps {
            let b = off + i * step;
            if b + step > bin.len() {
                return Err("GLB accessor 越界".into());
            }
            let v = if ctype == 5125 {
                u32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]]) as f32
            } else if ctype == 5123 {
                u16::from_le_bytes([bin[b], bin[b + 1]]) as f32
            } else {
                f32::from_le_bytes([bin[b], bin[b + 1], bin[b + 2], bin[b + 3]])
            };
            out.push(v);
        }
        Ok(out)
    }
    let pos = read_acc(json, bin, prim.get("attributes").and_then(|a| a.get("POSITION")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 3, 2)?;
    let nrm = read_acc(json, bin, prim.get("attributes").and_then(|a| a.get("NORMAL")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 3, 2)?;
    let uv = read_acc(json, bin, prim.get("attributes").and_then(|a| a.get("TEXCOORD_0")).and_then(|p| p.as_f64()).unwrap_or(0.0) as usize, 2, 2)?;
    // 顶点色（Blender 烘焙 COLOR_0，通常 VEC3；有则优先于材质基色）
    let col = prim.get("attributes").and_then(|a| a.get("COLOR_0")).and_then(|p| p.as_f64());
    let colv = if let Some(ci) = col {
        // 按 accessor 的 type 分量数读取（VEC3=3 / VEC4=4）
        let ty = json
            .get("accessors")
            .and_then(|a| a.as_arr())
            .and_then(|a| a.get(ci as usize))
            .and_then(|a| a.get("type"))
            .and_then(|t| t.as_str())
            .map(|s| if s == "VEC4" { 4 } else { 3 })
            .unwrap_or(3);
        read_acc(json, bin, ci as usize, ty, 2).unwrap_or_default()
    } else {
        Vec::new()
    };
    let ind = read_acc(json, bin, prim.get("indices").and_then(|i| i.as_f64()).unwrap_or(0.0) as usize, 1, 2)?;
    let base = *vert_offset;
    for i in 0..pos.len() / 3 {
        let n = if i * 3 + 2 < nrm.len() { [nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2]] } else { [0.0, 1.0, 0.0] };
        let t = if i * 2 + 1 < uv.len() { [uv[i * 2], uv[i * 2 + 1]] } else { [0.0, 0.0] };
        // 逐顶点颜色：COLOR_0 烘焙色优先，其次材质基色
        let c = if (i + 1) * 4 <= colv.len() {
            [colv[i * 4], colv[i * 4 + 1], colv[i * 4 + 2]]
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
