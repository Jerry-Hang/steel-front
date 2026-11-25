//! GLB 内嵌贴图导入（CPU 侧）。
//!
//! 现状（2026-11-25 起）：引擎的材质只有顶点色 + 程序化贴图，**完全不读 GLB 的
//! `images` / `textures` / `materials[].pbrMetallicRoughness.baseColorTexture`**。
//! 本模块先把"能读出来、能解码、能验证"做完整（纯 CPU，不碰渲染管线），
//! 上屏是下一步的事 —— 因为共享 mesh 着色器是承重结构（铁律 A），要单独立项。
//!
//! 数据流：
//! ```text
//! GLB bytes ──► json.images[i] { bufferView, mimeType }
//!                    │
//!                    ▼  BIN[bufferView.byteOffset .. +byteLength]
//!              PNG/JPEG bytes ──► image 解码 ──► Rgba8
//! ```
//!
//! 🔴 **只用 `bufferView` 那条路**：GLB 规范里 `images[i].uri` 也可以指向外部文件/`data:` URI，
//! 但外部文件在 Android 上不可用（`AAssetManager` 语义不同），`data:` URI 会把 base64
//! 塞进 JSON；两者都不属于"GLB 内嵌贴图"这条确定的路径，所以**显式返回 Err 说明**，
//! 而不是静默返回 None（静默 = 上层以为"这个模型没贴图"）。

use crate::engine::assets::glb_chunks;
use crate::llm_cmd::Json;

/// 一张导入的贴图。
///
/// `Debug` 手写：`rgba8` 可能是几十万字节，派生实现会把整张图打进 panic 消息。
#[derive(Clone)]
pub struct ImportedTexture {
    /// 资源内的名字（`images[i].name` 或 `image_<i>`）。
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// RGBA8、线性排列（不做 sRGB→linear：转换是渲染侧的事，这里保持"文件里是什么"）。
    pub rgba8: Vec<u8>,
}

impl std::fmt::Debug for ImportedTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImportedTexture {{ name: {:?}, {}x{}, {} 字节 }}",
            self.name,
            self.width,
            self.height,
            self.rgba8.len()
        )
    }
}

impl ImportedTexture {
    /// 取一个像素的 RGBA（越界返回 None）。判据用它做逐像素比对。
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba8.get(i..i + 4).map(|s| [s[0], s[1], s[2], s[3]])
    }
}

/// 解析 GLB 里**全部**内嵌贴图（按 `images` 顺序）。
///
/// 没有 `images` 时返回空 `Vec`（这是"真的没贴图"，与"解析失败"不同）。
pub fn parse_glb_textures(bytes: &[u8]) -> Result<Vec<ImportedTexture>, String> {
    let (json, bin) = glb_chunks(bytes)?;
    let images = match json.get("images") {
        Some(v) => v.as_arr().ok_or("images 不是数组")?,
        None => return Ok(Vec::new()),
    };

    let mut out = Vec::with_capacity(images.len());
    for (i, img) in images.iter().enumerate() {
        let name = img
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("image_{i}"));
        // 明确拒绝非内嵌路径，并说明确为什么
        if img.get("bufferView").is_none() {
            let uri = img.get("uri").and_then(|v| v.as_str()).unwrap_or("<无>");
            return Err(format!(
                "贴图 {name} 不是内嵌的（uri={uri}）：本引擎只支持 GLB 内嵌贴图，\
                 外部文件在 Android 上不可用"
            ));
        }
        let views = json
            .get("bufferViews")
            .and_then(|v| v.as_arr())
            .ok_or("有 images 但没有 bufferViews")?;
        let bv_idx = img
            .get("bufferView")
            .and_then(|v| v.as_f64().map(|v| v as u64))
            .ok_or_else(|| format!("贴图 {name} 的 bufferView 不是整数"))?
            as usize;
        let bv = views
            .get(bv_idx)
            .ok_or_else(|| format!("贴图 {name} 引用了不存在的 bufferView {bv_idx}"))?;

        // 🔴 `byteOffset` 在 bufferView 里是**可选**的（缺省 0）；`byteLength` 必填。
        let off = bv.get("byteOffset").and_then(|v| v.as_f64().map(|v| v as u64)).unwrap_or(0) as usize;
        let len = bv
            .get("byteLength")
            .and_then(|v| v.as_f64().map(|v| v as u64))
            .ok_or_else(|| format!("bufferView {bv_idx} 缺 byteLength"))? as usize;
        let end = off
            .checked_add(len)
            .ok_or_else(|| format!("bufferView {bv_idx} 的偏移+长度溢出"))?;
        let raw = bin.get(off..end).ok_or_else(|| {
            format!("bufferView {bv_idx} 越界（要 {off}..{end}，BIN 只有 {} 字节）", bin.len())
        })?;

        let decoded = image::load_from_memory(raw)
            .map_err(|e| format!("贴图 {name} 解码失败：{e}"))?
            .to_rgba8();
        let (w, h) = (decoded.width(), decoded.height());
        // 零尺寸图会让上采样/UV 计算除零，直接拒绝
        if w == 0 || h == 0 {
            return Err(format!("贴图 {name} 尺寸为 0"));
        }
        out.push(ImportedTexture {
            name,
            width: w,
            height: h,
            rgba8: decoded.into_raw(),
        });
    }
    Ok(out)
}

/// 某个材质引用的**基色贴图下标**（`None` = 该材质不用贴图）。
///
/// 与 `parse_glb_textures` 分开：解析贴图（重活）与"谁用了它"（轻活）是两件事，
/// 上层可以只查后者而不解码任何图。
pub fn base_color_texture_of(json: &Json, mat: usize) -> Option<usize> {
    json.get("materials")?
        .as_arr()?
        .get(mat)?
        .get("pbrMetallicRoughness")?
        .get("baseColorTexture")?
        .get("index")?
        .as_f64().map(|v| v as u64)
        .map(|v| v as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小 GLB：JSON chunk + BIN chunk（BIN 里塞一张 PNG）。
    ///
    /// **不依赖任何外部资产**：图是代码里现画的 2×2 PNG，所以这条判据在任何机器上都成立。
    fn glb_with_png(json_extra: &str, png: &[u8]) -> Vec<u8> {
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{}}}],"images":[{{"bufferView":0,"mimeType":"image/png"}}]{}}}"#,
            png.len(),
            json_extra
        );
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + json.len() + 8 + png.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // JSON
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes()); // BIN
        out.extend_from_slice(png);
        out
    }

    fn png_2x2() -> Vec<u8> {
        // 用 image crate 编码，避免手搓 PNG 头
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 255, 0, 128]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .expect("编码 PNG");
        buf.into_inner()
    }

    /// 🔴 **贴图必须逐像素与原图一致**（这是"导入"这个词的全部含义）。
    ///
    /// 只测"解析成功"是不够的：解码路径写错（通道顺序、行序、预乘 alpha）
    /// 都会"解析成功"而像素全错，而肉眼在小图上未必看得出来。
    ///
    /// 反向验证：把 `to_rgba8()` 换成 `to_rgb8()` 并用 3 通道拼 4 ⇒ 第二、四像素的
    /// alpha 会变成 255（原图是 128），第一条断言红。
    #[test]
    fn embedded_png_decodes_pixel_for_pixel() {
        let tex = parse_glb_textures(&glb_with_png("", &png_2x2())).expect("应解析出 1 张图");
        assert_eq!(tex.len(), 1, "应有 1 张内嵌贴图");
        let t = &tex[0];
        assert_eq!((t.width, t.height), (2, 2), "尺寸必须是 2×2");
        assert_eq!(t.rgba8.len(), 2 * 2 * 4, "RGBA8 应有 16 字节");
        // 逐像素比对：PNG 无损，必须**逐字节相等**
        assert_eq!(t.pixel(0, 0), Some([255, 0, 0, 255]), "左上 = 不透明红");
        assert_eq!(t.pixel(1, 0), Some([0, 255, 0, 255]), "右上 = 不透明绿");
        assert_eq!(t.pixel(0, 1), Some([0, 0, 255, 255]), "左下 = 不透明蓝");
        assert_eq!(t.pixel(1, 1), Some([255, 255, 0, 128]), "右下 = 半透明黄（alpha 必须保留 128）");
        // 越界返回 None，不 panic
        assert_eq!(t.pixel(2, 0), None);
        assert_eq!(t.pixel(0, 2), None);
    }

    /// 没有 `images` 的 GLB ⇒ **空列表**（"真的没贴图"），不是错误。
    ///
    /// 反向验证：把 `None => return Ok(Vec::new())` 改成返回 Err ⇒ 本判据红。
    #[test]
    fn glb_without_images_yields_empty_list_not_error() {
        let mut out = Vec::new();
        let json = r#"{"asset":{"version":"2.0"}}"#;
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + json.len() + 8) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        let tex = parse_glb_textures(&out).expect("无图不是错误");
        assert!(tex.is_empty(), "无 images 应得到空列表");
    }

    /// 🔴 **外部 uri 必须显式报错并说明原因**，不许静默当成"没贴图"。
    ///
    /// 理由：静默返回空会让上层以为"这个模型本来就没有贴图"，
    /// 而真实情况是"有人把贴图放在 GLB 外面"—— 这类静默失败最难查。
    ///
    /// 反向验证：把那个 `return Err(...)` 改成 `continue` ⇒ 本判据红。
    #[test]
    fn external_texture_uri_is_an_explicit_error() {
        let json = r#"{"asset":{"version":"2.0"},"images":[{"uri":"barrel_diffuse.png"}]}"#;
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + json.len() + 8) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        let err = parse_glb_textures(&out).expect_err("外部 uri 必须报错");
        assert!(err.contains("barrel_diffuse.png"), "错误里要点出是哪个文件：{err}");
        assert!(err.contains("内嵌"), "错误要说明只支持内嵌：{err}");
    }

    /// bufferView 越界必须报 Err，不是 panic。
    ///
    /// 反向验证：把 `bin.get(off..end).ok_or_else(...)` 换成 `&bin[off..end]` ⇒
    /// 本判据变成 panic 而不是 Err（测试仍"红"，但红的方式不同 —— 两者都要避免）。
    #[test]
    fn buffer_view_out_of_range_is_an_error() {
        let json = r#"{"asset":{"version":"2.0"},"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":999999}],"images":[{"bufferView":0,"mimeType":"image/png"}]}"#;
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + json.len() + 8 + 8) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        let err = parse_glb_textures(&out).expect_err("越界必须报错");
        assert!(err.contains("越界"), "错误要说明越界：{err}");
    }
}
