//! 窗口管理模块
//!
//! 使用 winit 0.30 创建和管理游戏窗口。提供：
//! - 窗口创建常量（标题、尺寸）
//! - FPS 鼠标锁定/解锁辅助函数

use winit::window::{CursorGrabMode, Window};

/// 窗口标题
pub const WINDOW_TITLE: &str = "Steel Front - 钢铁前线";
/// 窗口宽度
pub const WINDOW_WIDTH: u32 = 1280;
/// 窗口高度
pub const WINDOW_HEIGHT: u32 = 720;

/// 锁定鼠标为 FPS 模式（锁定到窗口内，隐藏指针）
///
/// 优先尝试 Locked 模式（提供原始相对运动），
/// 如果不可用则降级为 Confined 模式。
pub fn lock_cursor(window: &Window) {
    if window
        .set_cursor_grab(CursorGrabMode::Locked)
        .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
        .is_ok()
    {
        window.set_cursor_visible(false);
    }
}

/// 解锁鼠标，恢复普通模式
pub fn unlock_cursor(window: &Window) {
    let _ = window.set_cursor_grab(CursorGrabMode::None);
    window.set_cursor_visible(true);
}
