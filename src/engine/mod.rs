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
pub mod ai_command;
pub mod assets;
pub mod camera;
pub mod city;
// 🔴 2026-09-26：这一行以前是 `#[cfg(windows)]`，而 `font_cjk.rs` **无条件** `use` 它的
// `CJK_GLYPHS` ⇒ `cargo check --target aarch64-unknown-linux-gnu` 直接
// `E0432: unresolved import crate::engine::cjk_glyphs`（而本仓铁律 E 要求跑这条交叉验证）。
// 表本身只是一份数据（docstring 也写着"跨平台无依赖"），没有理由按平台门控 ⇒ 去掉 cfg，
// 让代码与文档一致。判据 = 那条交叉验证 0 error / 0 warning。
pub mod cjk_glyphs;
pub mod font_cjk;
pub mod cpu;
pub mod game;
pub mod geom;
pub mod gpu_caps;
pub mod guns;
pub mod lighting;
pub mod map;
pub mod meshgen;
pub mod objective;
pub mod physics;
pub mod procedural;
pub mod props;
pub mod ray_tracer;
pub mod renderer;
pub mod simd;
pub mod weapons;
pub mod weapon_data;
pub mod window;
