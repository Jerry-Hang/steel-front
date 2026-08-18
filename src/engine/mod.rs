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
//! - `simd`: 爆炸/冲击波等特效浮点计算的 SIMD 选路与加速比测量
//! - `game`: 运行时中枢（模块接线）

pub mod ai;
pub mod camera;
#[cfg(windows)]
pub mod font_cjk;
pub mod cpu;
pub mod game;
pub mod gpu_caps;
pub mod guns;
pub mod lighting;
pub mod map;
pub mod meshgen;
pub mod objective;
pub mod physics;
pub mod procedural;
pub mod renderer;
pub mod simd;
pub mod weapons;
pub mod weapon_data;
pub mod window;
