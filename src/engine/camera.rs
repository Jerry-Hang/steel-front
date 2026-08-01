//! FPS 相机模块
//!
//! 实现了第一人称射击游戏的相机控制：
//! - WASD 前后左右移动
//! - 鼠标控制视角（偏航角/俯仰角）
//! - 使用 glam 库进行向量/矩阵计算

use glam::{Mat4, Vec3};

/// FPS 相机灵敏度常量
const MOUSE_SENSITIVITY: f32 = 0.002; // 鼠标灵敏度
const MOVE_SPEED: f32 = 5.0; // 移动速度（单位/秒）
const PITCH_LIMIT: f32 = 89.0_f32.to_radians(); // 俯仰角限制（防止翻转）

/// FPS 相机
#[allow(dead_code)]
pub struct Camera {
    /// 相机位置（世界坐标）
    pub position: Vec3,
    /// 偏航角（左右旋转，弧度）
    pub yaw: f32,
    /// 俯仰角（上下旋转，弧度）
    pub pitch: f32,
    /// 前方向向量
    pub forward: Vec3,
    /// 右方向向量
    pub right: Vec3,
    /// 上方向向量
    pub up: Vec3,
    /// 视场角（弧度）
    pub fov: f32,
    /// 近裁剪面
    pub near_plane: f32,
    /// 远裁剪面
    pub far_plane: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl Camera {
    /// 创建默认 FPS 相机
    ///
    /// 初始位置在 (0.0, 5.0, 10.0)，看向 -Z 方向（标准 OpenGL/Vulkan 坐标系）
    pub fn new() -> Self {
        let mut camera = Self {
            position: Vec3::new(0.0, 5.0, 10.0), // 初始位置
            yaw: -90.0_f32.to_radians(),         // 朝 -Z 方向
            pitch: 0.0,                          // 水平
            forward: Vec3::Z,                    // 暂存，update_vectors 会修正
            right: Vec3::X,
            up: Vec3::Y,
            fov: 70.0_f32.to_radians(), // 视野 70 度
            near_plane: 0.1,
            far_plane: 1000.0,
        };
        camera.update_vectors();
        camera
    }

    /// 根据偏航角/俯仰角更新方向向量
    fn update_vectors(&mut self) {
        // 计算前方向量（球坐标转直角坐标）
        let cos_pitch = self.pitch.cos();
        self.forward = Vec3::new(
            self.yaw.cos() * cos_pitch,
            self.pitch.sin(),
            self.yaw.sin() * cos_pitch,
        )
        .normalize();

        // 世界坐标系上方向始终为 Y 轴
        let world_up = Vec3::Y;

        // 计算右方向量（前方向 × 世界上方向）
        self.right = self.forward.cross(world_up).normalize();

        // 计算真正的上方向量（右方向 × 前方向）
        self.up = self.right.cross(self.forward).normalize();
    }

    /// 处理鼠标移动，更新视角
    ///
    /// * `delta_x` - 鼠标在 X 轴上的移动量（像素）
    /// * `delta_y` - 鼠标在 Y 轴上的移动量（像素）
    pub fn process_mouse(&mut self, delta_x: f32, delta_y: f32) {
        // 水平移动影响偏航角
        self.yaw += delta_x * MOUSE_SENSITIVITY;
        // 垂直移动影响俯仰角（反向，因为屏幕 Y 轴朝下）
        self.pitch -= delta_y * MOUSE_SENSITIVITY;

        // 限制俯仰角，防止翻转
        self.pitch = self.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);

        // 更新方向向量
        self.update_vectors();
    }

    /// 处理键盘移动
    ///
    /// * `keys` - 当前按下的按键位掩码
    /// * `delta_time` - 帧时间间隔（秒）
    pub fn process_keyboard(&mut self, keys: &KeyState, delta_time: f32) {
        let velocity = MOVE_SPEED * delta_time;

        // WASD 移动（在世界坐标系 XZ 平面上移动，忽略俯仰）
        let forward_flat = Vec3::new(self.forward.x, 0.0, self.forward.z).normalize();
        let right_flat = Vec3::new(self.right.x, 0.0, self.right.z).normalize();

        if keys.forward {
            self.position += forward_flat * velocity;
        }
        if keys.backward {
            self.position -= forward_flat * velocity;
        }
        if keys.left {
            self.position -= right_flat * velocity;
        }
        if keys.right {
            self.position += right_flat * velocity;
        }
    }

    /// 获取视图矩阵（从世界空间变换到相机空间）
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(
            self.position,                // 相机位置
            self.position + self.forward, // 观察目标点
            Vec3::Y,                      // 世界上方向
        )
    }

    /// 获取投影矩阵（从相机空间变换到裁剪空间）
    ///
    /// * `aspect_ratio` - 窗口宽高比
    pub fn projection_matrix(&self, aspect_ratio: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov, aspect_ratio, self.near_plane, self.far_plane)
    }

    /// 获取视图 × 投影矩阵（用于 Vulkan 的 push constant 或 uniform buffer）
    pub fn view_projection_matrix(&self, aspect_ratio: f32) -> Mat4 {
        self.projection_matrix(aspect_ratio) * self.view_matrix()
    }

    /// 获取 Vulkan 用的 view×proj 矩阵（列主序 [[f32;4];4]）
    pub fn view_proj_for_vulkan(&self, aspect_ratio: f32) -> [[f32; 4]; 4] {
        let mut vp = self.view_projection_matrix(aspect_ratio);
        vp.y_axis = -vp.y_axis;
        vp.to_cols_array_2d()
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
    }
}
