//! 三模式相机模块
//!
//! - 第一人称模式（默认）：眼睛位置由游戏侧驱动，鼠标视角 + 后坐力衰减
//! - 轨道模式：围绕目标点旋转（左键拖拽）、推拉距离（滚轮）、平移目标点（WASD/QE）
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
pub(crate) const PITCH_LIMIT: f32 = 89.0_f32.to_radians();
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
    /// 第一人称模式（默认）：眼睛位置由游戏侧驱动，鼠标视角 + 后坐力
    FirstPerson,
    /// 轨道模式：围绕目标点旋转/缩放/平移
    Orbit,
    /// 飞行模式：自由飞行（RTS 手感）
    Flight,
}

/// 三模式相机
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
    /// 第一人称眼睛位置
    fp_pos: Vec3,
    /// 第一人称速度（预留：Wave2 玩家移动由游戏侧写入）
    fp_vel: Vec3,
    /// 视角后坐力累计（pitch/yaw，弧度），FirstPerson 更新时指数衰减
    recoil_pitch: f32,
    recoil_yaw: f32,
    /// 鼠标灵敏度（弧度/像素），look() 使用
    mouse_sens: f32,
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
    /// 创建默认第一人称相机
    ///
    /// 默认值：眼睛 (0,1.6,0)，yaw=0°、pitch=0°（平视 -Z），距离 3.3541（轨道/飞行切换沿用）。
    pub fn new() -> Self {
        Self {
            mode: CameraMode::FirstPerson,
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 3.3541,
            fov: 70.0_f32.to_radians(), // 视野 70 度
            near_plane: 0.1,
            far_plane: 1500.0,
            flight_pos: Vec3::new(0.0, 1.5, 3.0),
            fp_pos: Vec3::new(0.0, 1.6, 0.0),
            fp_vel: Vec3::ZERO,
            recoil_pitch: 0.0,
            recoil_yaw: 0.0,
            mouse_sens: MOUSE_SENSITIVITY,
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
            CameraMode::FirstPerson => self.fp_pos,
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
    /// 标准方向：鼠标下移（dy>0）→ pitch 增大 → 看向下方（pitch 正 = 低头看地）。
    /// pitch 夹在 [-89°, 89°]。
    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw += delta_x * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch + delta_y * MOUSE_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// 第一人称鼠标视角（供 FPS 主视角）：`delta_x` / `delta_y` 为屏幕像素位移，
    /// 标准方向：鼠标下移（dy>0）→ pitch 增大 → 看向下方（pitch 正 = 低头看地）；
    /// 鼠标右移（dx>0）→ yaw 减小 → 视角右转（forward.x = -sin(yaw)，yaw 减小 → +X 偏转）。
    /// pitch 夹在 [-89°, 89°]；灵敏度用 `mouse_sens`（默认 0.003）。
    /// 注：2026-08-15 Windows 原生真机修正——旧 `yaw += dx*sens` 使右移变左转
    /// （WSL2 输入捕获不可用从未暴露，冒烟闭环正反皆可收敛）。
    pub fn look(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw -= delta_x * self.mouse_sens;
        self.pitch = (self.pitch + delta_y * self.mouse_sens).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// 累计视角后坐力（弧度），update() 的 FirstPerson 分支以指数衰减施加到 yaw/pitch
    pub fn add_recoil(&mut self, pitch_kick: f32, yaw_kick: f32) {
        self.recoil_pitch += pitch_kick;
        self.recoil_yaw += yaw_kick;
    }

    /// 设置第一人称眼睛位置（玩家移动由游戏侧驱动）
    pub fn set_first_person_eye(&mut self, pos: Vec3) {
        self.fp_pos = pos;
    }

    /// 设置鼠标灵敏度（弧度/像素，设置面板用；clamp 到 [0.0005, 0.02]）
    pub fn set_mouse_sens(&mut self, sens: f32) {
        self.mouse_sens = sens.clamp(0.0005, 0.02);
    }

    /// 获取第一人称眼睛位置
    #[allow(dead_code)] // 预留：Wave2 集成 game.rs/main.rs 时启用
    pub fn first_person_eye(&self) -> Vec3 {
        self.fp_pos
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

    /// 切换相机模式（Tab）：Orbit → Flight → FirstPerson → Orbit，保持位姿与朝向不变；清零全部速度
    pub fn toggle_mode(&mut self) -> CameraMode {
        match self.mode {
            CameraMode::Orbit => {
                self.flight_pos = self.position();
                self.mode = CameraMode::Flight;
            }
            CameraMode::Flight => {
                // 切入第一人称：眼睛位置取当前相机位置，朝向沿用 yaw/pitch
                self.fp_pos = self.position();
                self.mode = CameraMode::FirstPerson;
            }
            CameraMode::FirstPerson => {
                // 切出第一人称：飞行位置取眼睛位置；轨道目标取视线前方 distance 处，
                // 保证切回后相机仍停留在原眼睛位置（位姿连续）
                self.flight_pos = self.fp_pos;
                self.target = self.fp_pos + self.forward() * self.distance;
                self.mode = CameraMode::Orbit;
            }
        }
        self.move_vel = Vec3::ZERO;
        self.yaw_vel = 0.0;
        self.pitch_vel = 0.0;
        self.rotate_dx = 0.0;
        self.rotate_dy = 0.0;
        self.recoil_pitch = 0.0;
        self.recoil_yaw = 0.0;
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
            CameraMode::FirstPerson => {
                // 不做位置移动（玩家移动由游戏侧负责），只做后坐力衰减与 pitch clamp。
                // 后坐力按角速度积分并指数衰减：残留 = exp(-8*0.35)≈6% < 10%（0.35s 内）。
                self.yaw += self.recoil_yaw * dt;
                // 后坐力方向：kick_pitch 为正 → 枪口上扬（pitch 减小 = 抬头）
                self.pitch -= self.recoil_pitch * dt;
                self.recoil_yaw *= decay;
                self.recoil_pitch *= decay;
                self.pitch = self.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
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
            CameraMode::FirstPerson => {
                let pos = self.fp_pos;
                Mat4::look_at_rh(pos, pos + self.forward(), Vec3::Y)
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// FirstPerson 默认位姿：眼睛 (0,1.6,0)、平视（yaw/pitch=0）、前向 -Z
    #[test]
    fn first_person_default_pose() {
        let cam = Camera::new();
        assert_eq!(cam.mode, CameraMode::FirstPerson);
        assert_eq!(cam.position(), Vec3::new(0.0, 1.6, 0.0));
        assert_eq!(cam.first_person_eye(), Vec3::new(0.0, 1.6, 0.0));
        assert_eq!(cam.yaw, 0.0);
        assert_eq!(cam.pitch, 0.0);
        assert!(
            (cam.forward() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6,
            "默认前向应为 -Z"
        );
    }

    /// FirstPerson view_matrix：look_at(eye, eye+forward, Y)，眼睛经视图变换后应位于原点
    #[test]
    fn first_person_view_matrix() {
        let cam = Camera::new();
        let eye = cam.position();
        let center = eye + cam.forward();
        let expect = Mat4::look_at_rh(eye, center, Vec3::Y);
        assert_eq!(cam.view_matrix(), expect);
        let view_eye = cam.view_matrix().transform_point3(eye);
        assert!(view_eye.length() < 1e-5, "眼睛在视图空间中应位于原点，实际 {:?}", view_eye);
    }

    /// set_first_person_eye：更新眼睛位置，position() 同步反映
    #[test]
    fn set_first_person_eye_updates_position() {
        let mut cam = Camera::new();
        cam.set_first_person_eye(Vec3::new(5.0, 3.0, 1.0));
        assert_eq!(cam.first_person_eye(), Vec3::new(5.0, 3.0, 1.0));
        assert_eq!(cam.position(), Vec3::new(5.0, 3.0, 1.0));
    }

    /// look：右移 dx 减小 yaw（视角右转），下移 dy（屏幕 Y 向下）增大 pitch（低头），
    /// 灵敏度取 mouse_sens
    #[test]
    fn look_updates_yaw_pitch() {
        let mut cam = Camera::new();
        cam.look(100.0, 50.0);
        assert!((cam.yaw + 100.0 * MOUSE_SENSITIVITY).abs() < 1e-6);
        assert!((cam.pitch - (50.0 * MOUSE_SENSITIVITY)).abs() < 1e-6);
    }

    /// look：pitch 夹在 [-89°, 89°]
    #[test]
    fn look_clamps_pitch() {
        let mut cam = Camera::new();
        cam.look(0.0, 1e6);
        assert_eq!(cam.pitch, PITCH_LIMIT);
        cam.look(0.0, -1e6);
        assert_eq!(cam.pitch, -PITCH_LIMIT);
    }

    /// 投影矩阵保持 y-up NDC（Y 轴缩放为正）：Vulkan 的 Y 翻转由 shader
    /// （triangle.vert.spv 的 gl_Position.y）完成，main.rs 不再翻转投影，
    /// 避免双重翻转导致画面上下颠倒。
    #[test]
    fn projection_keeps_y_up_for_shader_flip() {
        let cam = Camera::new();
        let p = cam.projection_matrix(16.0 / 9.0);
        assert!(p.y_axis.y > 0.0, "projection_matrix 应保持 y-up NDC");
    }

    /// add_recoil：0.35s 内后坐力残留 <10%，且确实转动了视角；FirstPerson 不做位置移动
    #[test]
    fn add_recoil_decays_within_035s() {
        let mut cam = Camera::new();
        let (kick_pitch, kick_yaw) = (0.3, 0.2);
        cam.add_recoil(kick_pitch, kick_yaw);
        let dt: f32 = 1.0 / 60.0;
        let steps = (0.35 / dt).round() as usize;
        for _ in 0..steps {
            cam.update(&KeyState::default(), dt);
        }
        assert!(
            cam.recoil_pitch.abs() < kick_pitch * 0.10,
            "0.35s 后 recoil_pitch 残留应 <10%，实际 {}",
            cam.recoil_pitch
        );
        assert!(
            cam.recoil_yaw.abs() < kick_yaw * 0.10,
            "0.35s 后 recoil_yaw 残留应 <10%，实际 {}",
            cam.recoil_yaw
        );
        assert!(cam.pitch.abs() > 0.0, "后坐力应改变 pitch");
        assert!(cam.yaw.abs() > 0.0, "后坐力应改变 yaw");
        assert_eq!(cam.position(), Vec3::new(0.0, 1.6, 0.0), "FirstPerson 不移动位置");
    }

    /// 超大后坐力：pitch 始终被 clamp 在 [-89°, 89°]
    #[test]
    fn recoil_pitch_is_clamped() {
        let mut cam = Camera::new();
        cam.add_recoil(20.0, 0.0);
        let dt: f32 = 1.0 / 60.0;
        for _ in 0..60 {
            cam.update(&KeyState::default(), dt);
        }
        assert!(cam.pitch <= PITCH_LIMIT + 1e-6);
        assert!(cam.pitch >= -PITCH_LIMIT - 1e-6);
    }

    /// toggle_mode 三态循环：FirstPerson → Orbit → Flight → FirstPerson，位姿连续
    #[test]
    fn toggle_mode_cycles_three_ways() {
        let mut cam = Camera::new();
        assert_eq!(cam.mode, CameraMode::FirstPerson);
        cam.set_first_person_eye(Vec3::new(10.0, 2.0, -5.0));
        cam.yaw = 0.5;
        cam.pitch = 0.2;

        // FirstPerson → Orbit：相机停在眼睛处，target 在视线前方 distance
        assert_eq!(cam.toggle_mode(), CameraMode::Orbit);
        assert!(
            (cam.position() - Vec3::new(10.0, 2.0, -5.0)).length() < 1e-5,
            "切到 Orbit 后位置应连续"
        );
        let expect_target = Vec3::new(10.0, 2.0, -5.0) + cam.forward() * cam.distance;
        assert!((cam.target - expect_target).length() < 1e-5);

        // Orbit → Flight：flight_pos = 当前 position
        assert_eq!(cam.toggle_mode(), CameraMode::Flight);
        assert!((cam.flight_pos - cam.position()).length() < 1e-6);
        assert!((cam.flight_pos - Vec3::new(10.0, 2.0, -5.0)).length() < 1e-5);

        // Flight → FirstPerson：fp_pos = 当前 position
        assert_eq!(cam.toggle_mode(), CameraMode::FirstPerson);
        assert!((cam.first_person_eye() - cam.position()).length() < 1e-6);
        assert!((cam.first_person_eye() - Vec3::new(10.0, 2.0, -5.0)).length() < 1e-5);
    }

    /// Orbit 既有行为：位姿公式与 orbit() 位控旋转保持不变
    #[test]
    fn orbit_pose_and_rotation_unchanged() {
        let mut cam = Camera::new();
        cam.mode = CameraMode::Orbit;
        cam.target = Vec3::ZERO;
        cam.yaw = 0.0;
        cam.pitch = 26.565_f32.to_radians();
        cam.distance = 3.3541;
        assert!(
            (cam.position() - Vec3::new(0.0, 1.5, 3.0)).length() < 1e-3,
            "默认轨道机位应等价 (0,1.5,3)"
        );
        let expect = Mat4::look_at_rh(cam.position(), cam.target, Vec3::Y);
        assert_eq!(cam.view_matrix(), expect);

        cam.orbit(100.0, 50.0);
        assert!((cam.yaw - 100.0 * MOUSE_SENSITIVITY).abs() < 1e-6);
        assert!(
            (cam.pitch - (26.565_f32.to_radians() + 50.0 * MOUSE_SENSITIVITY)).abs() < 1e-6
        );
    }

    /// Flight 既有行为：W 沿水平前向（-Z）移动，切到 FirstPerson 位置连续
    #[test]
    fn flight_moves_and_toggle_preserves_pose() {
        let mut cam = Camera::new();
        cam.mode = CameraMode::Flight;
        cam.flight_pos = Vec3::new(0.0, 10.0, 0.0);
        cam.yaw = 0.0;
        cam.pitch = 0.0;
        let before = cam.flight_pos;
        let keys = KeyState {
            forward: true,
            ..KeyState::default()
        };
        cam.update(&keys, 1.0 / 60.0);
        assert!(
            (cam.flight_pos - before).length() > 0.0,
            "W 应让飞行相机前进"
        );
        assert!(cam.flight_pos.z < before.z, "默认前向 -Z，应沿 -Z 前进");
        let pos = cam.position();
        cam.toggle_mode();
        assert_eq!(cam.mode, CameraMode::FirstPerson);
        assert!(
            (cam.first_person_eye() - pos).length() < 1e-5,
            "Flight → FirstPerson 位置应连续"
        );
    }
}
