//! 钢铁前线 (Steel Front) 引擎模块
//!
//! 引擎核心子模块：
//! - `ai`: AI 寻路与 NPC 状态机
//! - `window`: 窗口管理（winit）
//! - `renderer`: Vulkan 渲染器（ash）
//! - `camera`: FPS 相机控制
//! - `lighting`: 光照与阴影
//! - `physics`: 物理碰撞系统
//! - `weapons`: 武器系统
//! - `game`: 运行时中枢（模块接线）

pub mod ai;
pub mod camera;
pub mod cpu;
pub mod game;
pub mod gpu_caps;
pub mod lighting;
pub mod physics;
pub mod renderer;
pub mod weapons;
pub mod window;
