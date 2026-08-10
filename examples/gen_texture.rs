//! 生成 256x256 测试贴图：纯中灰 128（供 init_texture 加载验证）
//!
//! 运行: cargo run --example gen_texture
//! 输出: assets/textures/test.png
//!
//! 约定（勿改回）：2026-08-08 曾用四色象限调试图，导致地面面劈裂，
//! 已统一为纯中灰 128；若需调试 UV 方向请新增专用贴图，勿改回彩色。

use image::{ImageBuffer, Rgba};

const SIZE: u32 = 256;
const GRAY: u8 = 128;

fn main() {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(SIZE, SIZE, |_, _| Rgba([GRAY, GRAY, GRAY, 255]));
    let dir = "assets/textures";
    std::fs::create_dir_all(dir).expect("创建 assets/textures 目录失败");
    let path = format!("{}/test.png", dir);
    img.save(&path).expect("保存 test.png 失败");
    println!("已生成测试贴图: {} ({}x{} 中灰 {})", path, SIZE, SIZE, GRAY);
}
