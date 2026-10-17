//! 窗口管理模块
//!
//! 使用 winit 0.30 创建和管理游戏窗口。提供：
//! - 窗口创建常量（标题、尺寸）

/// 窗口标题
pub const WINDOW_TITLE: &str = "Steel Front - Vulkan";
/// 窗口宽度（回退尺寸；实际默认按主显示器宽高比，**见 `lib.rs` 里的窗口创建处**）
///
/// ⚠️ 2026-10-17 自查修正：原注释写「见 main.rs」，而 `main.rs` 只有 6 行、只管入口，
/// 真正的窗口创建与尺寸决策在 `lib.rs`（`.with_title(window::WINDOW_TITLE)` 那一段）。
/// 这两个常量当前的消费者是 `game.rs` → `session.rs::HudState::new`（HUD 的初始视口）。
pub const WINDOW_WIDTH: u32 = 1280;
/// 窗口高度（回退尺寸）
pub const WINDOW_HEIGHT: u32 = 720;
