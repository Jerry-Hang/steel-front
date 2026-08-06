//! 双模式相机模块
//!
//! - 轨道模式（默认）：围绕目标点旋转（左键拖拽）、推拉距离（滚轮）、平移目标点（WASD/QE）
//! - 飞行模式：WASD 相机本地系平移、QE 升降、右键拖拽转视角、滚轮沿视线前进/后退
//! - 惯性阻尼：平移/旋转速度指数衰减（v *= exp(-damping*dt)，damping≈8），松手后自然滑行
//! - 边界 clamp（飞行模式）：x/z ∈ [-600,600]、y ∈ [0.5,800]，撞边界速度清零对应分量
//!
//! 轨道模式旋转保持位控（左键拖拽直接改 yaw/pitch），以保证既有回归机位不变；
//! 惯性旋转作用于飞行模式右键拖拽。

use glam::{Mat4, Vec3};

/// 拖拽灵敏度（弧度/像素）
const MOUSE_SENSITIVITY: f32 = 0.003;
/// 滚轮缩放比例（每格，轨道模式）
const WHEEL_ZOOM_STEP: f32 = 0.15;
/// 俯仰角限制（防止翻转）
const PITCH_LIMIT: f32 = 89.0_f32.to_radians();
/// 推拉距离范围（轨道模式）
const MIN_DISTANCE: f32 = 1.5;
const MAX_DISTANCE: f32 = 500.0;
/// 惯性指数衰减系数（v *= exp(-DAMPING*dt)）
const DAMPING: f32 = 8.0;
/// 飞行边界：x/z 范围
const BOUND_XZ: f32 = 600.0;
/// 飞行边界：y 范围
const BOUND_Y_MIN: f32 = 0.5;
const BOUND_Y_MAX: f32 = 800.0;
/// 飞行基础速度（u/s，随高度缩放：speed = BASE * (1 + y * HEIGHT_SCALE)）
const FLIGHT_BASE_SPEED: f32 = 40.0;
const FLIGHT_HEIGHT_SCALE: f32 = 0.01;
/// 飞行模式滚轮每格沿视线前进距离（随高度缩放）
const FLIGHT_WHEEL_STEP: f32 = 12.0;

/// 相机模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    /// 轨道模式（默认）：围绕目标点旋转/缩放/平移
    Orbit,
    /// 飞行模式：自由飞行（RTS 手感）
    Flight,
}

/// 双模式相机
#[allow(dead_code)]
pub struct Camera {
    /// 当前模式
    pub mode: CameraMode,
    /// 目标点（轨道模式围绕其旋转）
    pub target: Vec3,
    /// 偏航角（绕 Y 轴，弧度）
    pub yaw: f32,
    /// 俯仰角（相对水平面，弧度）
    pub pitch: f32,
    /// 相机到目标点的距离（轨道模式）
    pub distance: f32,
    /// 视场角（弧度）
    pub fov: f32,
    /// 近裁剪面
    pub near_plane: f32,
    /// 远裁剪面
    pub far_plane: f32,
    /// 飞行模式位置
    flight_pos: Vec3,
    /// 平移速度（轨道=目标点速度，飞行=位置速度），指数衰减
    move_vel: Vec3,
    /// 旋转速度（yaw/pitch，弧度/秒），指数衰减（飞行模式右键拖拽）
    yaw_vel: f32,
    pitch_vel: f32,
    /// 本帧累计的旋转输入（像素，飞行模式右键拖拽）
    rotate_dx: f32,
    rotate_dy: f32,
    /// 旋转输入是否激活（飞行模式右键按住）
    rotate_active: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl Camera {
    /// 创建默认轨道相机
    ///
    /// 默认值：目标 (0,0,0)，yaw=0°，pitch=26.565°，距离 3.3541。
    /// 等价于旧默认相机位于 (0, 1.5, 3) 看向原点，无输入时画面一致。
    pub fn new() -> Self {
        Self {
            mode: CameraMode::Orbit,
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 26.565_f32.to_radians(),
            distance: 3.3541,
            fov: 70.0_f32.to_radians(), // 视野 70 度
            near_plane: 0.1,
            far_plane: 1500.0,
            flight_pos: Vec3::new(0.0, 1.5, 3.0),
            move_vel: Vec3::ZERO,
            yaw_vel: 0.0,
            pitch_vel: 0.0,
            rotate_dx: 0.0,
            rotate_dy: 0.0,
            rotate_active: false,
        }
    }

    /// 从目标指向相机的单位方向（yaw 绕 Y 轴，pitch 为相对水平面仰角）
    fn direction(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }

    /// 相机位置
    pub fn position(&self) -> Vec3 {
        match self.mode {
            CameraMode::Orbit => self.target + self.direction() * self.distance,
            CameraMode::Flight => self.flight_pos,
        }
    }

    /// 相机前方向（看向目标/视线方向）
    pub fn forward(&self) -> Vec3 {
        -self.direction()
    }

    /// 相机右方向（前 × 世界上方向，水平）
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize()
    }

    /// 相机上方向（右 × 前）
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize()
    }

    /// 鼠标左键拖拽轨道旋转（位控，仅轨道模式）
    ///
    /// `delta_x` / `delta_y` 为屏幕像素位移（屏幕 Y 向下），
    /// pitch 夹在 [-89°, 89°]。
    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw += delta_x * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - delta_y * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// 滚轮推拉距离（轨道模式），clamp 到 [1.5, 500]
    ///
    /// `scroll` > 0 表示向前滚（拉近）。
    pub fn zoom(&mut self, scroll: f32) {
        self.distance =
            (self.distance * (1.0 - WHEEL_ZOOM_STEP * scroll)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// 飞行模式右键拖拽累计旋转输入（像素）
    pub fn add_rotation_input(&mut self, delta_x: f32, delta_y: f32) {
        if self.mode != CameraMode::Flight {
            return;
        }
        self.rotate_dx += delta_x;
        self.rotate_dy += delta_y;
    }

    /// 设置旋转输入激活状态（飞行模式右键按住/松开）
    pub fn set_rotation_active(&mut self, active: bool) {
        self.rotate_active = active;
        if !active {
            self.rotate_dx = 0.0;
            self.rotate_dy = 0.0;
        }
    }

    /// 飞行模式滚轮：沿视线前进/后退（`scroll` > 0 = 前进）
    pub fn flight_wheel(&mut self, scroll: f32) {
        if self.mode != CameraMode::Flight {
            return;
        }
        let step = FLIGHT_WHEEL_STEP * (1.0 + self.flight_pos.y * FLIGHT_HEIGHT_SCALE) * scroll;
        self.flight_pos += self.forward() * step;
        self.enforce_flight_bounds();
    }

    /// 切换相机模式（Tab），保持位姿与朝向不变；清零全部速度
    pub fn toggle_mode(&mut self) -> CameraMode {
        match self.mode {
            CameraMode::Orbit => {
                self.flight_pos = self.position();
                self.mode = CameraMode::Flight;
            }
            CameraMode::Flight => {
                self.target = self.flight_pos - self.direction() * self.distance;
                self.mode = CameraMode::Orbit;
            }
        }
        self.move_vel = Vec3::ZERO;
        self.yaw_vel = 0.0;
        self.pitch_vel = 0.0;
        self.rotate_dx = 0.0;
        self.rotate_dy = 0.0;
        self.mode
    }

    /// 每帧更新相机（输入 → 带惯性的速度 → 积分位移/旋转 → 边界 clamp）
    pub fn update(&mut self, keys: &KeyState, delta_time: f32) {
        let dt = delta_time.max(1e-6);
        // 指数衰减：v_new = v_old * e^(-damping*dt) + input * (1 - e^(-damping*dt))
        let decay = (-DAMPING * dt).exp();
        let step = 1.0 - decay;

        match self.mode {
            CameraMode::Orbit => {
                // 平移目标点：现状公式保留（速度 = 0.4 * distance，u/s）
                let speed = 0.4 * self.distance;
                let forward = Vec3::new(self.forward().x, 0.0, self.forward().z).normalize_or_zero();
                let right = self.right();
                let mut dir = Vec3::ZERO;
                if keys.forward {
                    dir += forward * speed;
                }
                if keys.backward {
                    dir -= forward * speed;
                }
                if keys.right {
                    dir += right * speed;
                }
                if keys.left {
                    dir -= right * speed;
                }
                if keys.up {
                    dir += Vec3::Y * speed;
                }
                if keys.down {
                    dir -= Vec3::Y * speed;
                }
                self.move_vel = self.move_vel * decay + dir * step;
                self.target += self.move_vel * dt;
            }
            CameraMode::Flight => {
                // 平移：WASD 相机本地系（W/S 沿水平前向投影），QE 世界 Y 升降，
                // 速度随高度缩放：speed = FLIGHT_BASE_SPEED * (1 + y * FLIGHT_HEIGHT_SCALE)
                let speed = FLIGHT_BASE_SPEED * (1.0 + self.flight_pos.y * FLIGHT_HEIGHT_SCALE);
                let forward = Vec3::new(self.forward().x, 0.0, self.forward().z).normalize_or_zero();
                let right = self.right();
                let mut dir = Vec3::ZERO;
                if keys.forward {
                    dir += forward;
                }
                if keys.backward {
                    dir -= forward;
                }
                if keys.right {
                    dir += right;
                }
                if keys.left {
                    dir -= right;
                }
                if keys.up {
                    dir += Vec3::Y;
                }
                if keys.down {
                    dir -= Vec3::Y;
                }
                let input_vel = dir.normalize_or_zero() * speed;
                self.move_vel = self.move_vel * decay + input_vel * step;
                self.flight_pos += self.move_vel * dt;

                // 旋转：右键拖拽 → 目标角速度（指数逼近），松手后指数衰减滑行
                let (target_yaw_rate, target_pitch_rate) = if self.rotate_active {
                    (
                        self.rotate_dx * MOUSE_SENSITIVITY / dt,
                        -self.rotate_dy * MOUSE_SENSITIVITY / dt,
                    )
                } else {
                    (0.0, 0.0)
                };
                self.yaw_vel = self.yaw_vel * decay + target_yaw_rate * step;
                self.pitch_vel = self.pitch_vel * decay + target_pitch_rate * step;
                self.yaw += self.yaw_vel * dt;
                self.pitch = (self.pitch + self.pitch_vel * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
                self.rotate_dx = 0.0;
                self.rotate_dy = 0.0;

                // 边界 clamp + 撞边界速度清零
                self.enforce_flight_bounds();
            }
        }
    }

    /// 飞行边界：x/z ∈ [-600,600]，y ∈ [0.5,800]；撞边界速度清零对应分量
    fn enforce_flight_bounds(&mut self) {
        if self.flight_pos.x < -BOUND_XZ {
            self.flight_pos.x = -BOUND_XZ;
            self.move_vel.x = 0.0;
        }
        if self.flight_pos.x > BOUND_XZ {
            self.flight_pos.x = BOUND_XZ;
            self.move_vel.x = 0.0;
        }
        if self.flight_pos.z < -BOUND_XZ {
            self.flight_pos.z = -BOUND_XZ;
            self.move_vel.z = 0.0;
        }
        if self.flight_pos.z > BOUND_XZ {
            self.flight_pos.z = BOUND_XZ;
            self.move_vel.z = 0.0;
        }
        if self.flight_pos.y < BOUND_Y_MIN {
            self.flight_pos.y = BOUND_Y_MIN;
            self.move_vel.y = 0.0;
        }
        if self.flight_pos.y > BOUND_Y_MAX {
            self.flight_pos.y = BOUND_Y_MAX;
            self.move_vel.y = 0.0;
        }
    }

    /// 获取视图矩阵（从世界空间变换到相机空间）
    pub fn view_matrix(&self) -> Mat4 {
        match self.mode {
            CameraMode::Orbit => Mat4::look_at_rh(self.position(), self.target, Vec3::Y),
            CameraMode::Flight => {
                let pos = self.flight_pos;
                Mat4::look_at_rh(pos, pos + self.forward(), Vec3::Y)
            }
        }
    }

    /// 获取投影矩阵（从相机空间变换到裁剪空间）
    pub fn projection_matrix(&self, aspect_ratio: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov, aspect_ratio, self.near_plane, self.far_plane)
    }

    /// 当前轨道参数：(yaw, pitch, distance)
    pub fn orbit_params(&self) -> (f32, f32, f32) {
        (self.yaw, self.pitch, self.distance)
    }
}

/// 键盘按键状态
#[derive(Debug, Clone, Default)]
pub struct KeyState {
    /// W 键 - 向前移动
    pub forward: bool,
    /// S 键 - 向后移动
    pub backward: bool,
    /// A 键 - 向左移动
    pub left: bool,
    /// D 键 - 向右移动
    pub right: bool,
    /// E 键 - 上升
    pub up: bool,
    /// Q 键 - 下降
    pub down: bool,
}

impl KeyState {
    /// 创建新的按键状态
    pub fn new() -> Self {
        Self::default()
    }

    /// 重置所有按键状态
    pub fn reset(&mut self) {
        self.forward = false;
        self.backward = false;
        self.left = false;
        self.right = false;
        self.up = false;
        self.down = false;
    }
}
