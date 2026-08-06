//! 生成 256x256 测试贴图：四色象限 + 角标（供 init_texture 加载验证 UV）
//!
//! 运行: cargo run --example gen_texture
//! 输出: assets/textures/test.png

use image::{ImageBuffer, Rgba};

const SIZE: u32 = 256;

fn main() {
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(SIZE, SIZE);

    // 四色象限（image 行序：第 0 行为图片顶部，与 Vulkan UV 原点一致，无需翻转）
    // 左上=红, 右上=绿, 左下=蓝, 右下=黄
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (r, g, b): (u8, u8, u8) = match (x < SIZE / 2, y < SIZE / 2) {
                (true, true) => (255, 0, 0),     // 左上
                (false, true) => (0, 255, 0),    // 右上
                (true, false) => (0, 0, 255),    // 左下
                (false, false) => (255, 255, 0), // 右下
            };
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // 白色圆点角标：居中于 UV (0.5, 0.75)，即当前三角形的顶点处，
    // 便于肉眼确认方向与 UV 映射正确
    let (cx, cy) = (128i32, 192i32); // (0.5 * SIZE, 0.75 * SIZE)
    let radius = 12i32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && px < SIZE as i32 && py < SIZE as i32 {
                    img.put_pixel(px as u32, py as u32, Rgba([255u8, 255, 255, 255]));
                }
            }
        }
    }

    // 图片左上角 4x4 白点：在图片查看器里确认行序方向
    // （当前三角形 UV 范围约 [0.25,0.75]，这个角点不会被采样）
    for y in 1..5 {
        for x in 1..5 {
            img.put_pixel(x, y, Rgba([255u8, 255, 255, 255]));
        }
    }

    let dir = "assets/textures";
    std::fs::create_dir_all(dir).expect("创建 assets/textures 目录失败");
    let path = format!("{}/test.png", dir);
    img.save(&path).expect("保存 test.png 失败");
    println!("已生成测试贴图: {} ({}x{})", path, SIZE, SIZE);
}
