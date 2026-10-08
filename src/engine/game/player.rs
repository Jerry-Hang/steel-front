// 由 src/engine/game.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `game` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Game {
    /// 设置面板：进入"等待按键绑定"（Enter 触发，绑定当前选中的键位动作）
    pub(crate) fn begin_rebind(&mut self) {
        if let Some(action) = self.hud.selected_action() {
            self.hud.begin_rebind(action);
        }
    }
    /// 设置面板：完成绑定（非 ESC 按键触发），绑定后持久化配置
    pub(crate) fn complete_rebind(&mut self, code: u32) {
        if self.hud.complete_rebind(code).is_some() {
            crate::config::save(&self.current_config());
        }
    }
    /// 设置面板：取消绑定（ESC 触发）
    pub(crate) fn cancel_rebind(&mut self) {
        self.hud.cancel_rebind();
    }
    /// 设置面板是否正在等待按键绑定（main.rs 据此拦截按键）
    pub(crate) fn rebinding_active(&self) -> bool {
        self.hud.rebinding_action().is_some()
    }
    /// 当前可持久化配置（键位 + 音量 + 灵敏度）
    pub(crate) fn current_config(&self) -> crate::config::GameConfig {
        crate::config::GameConfig {
            volume: self.hud.volume,
            music_volume: self.hud.music_volume,
            sensitivity: self.hud.sensitivity,
            bindings: self.hud.key_bindings,
            resolution: self.hud.resolution(),
            resolution_explicit: true, // 保存时总是写 resolution 行，加载后视为显式选择
            pt_enable: self.hud.pt_enable,
            pt_exposure: self.hud.pt_exposure,
            quality: self.hud.quality_index as u32,
        }
    }
    /// 取走本帧开火累计的后坐力（pitch/yaw 弧度），由 main.rs 施加到相机
    pub(crate) fn drain_kick(&mut self) -> (f32, f32) {
        let kick = self.pending_kick;
        self.pending_kick = (0.0, 0.0);
        kick
    }
    /// 玩家脚底位置（世界坐标）
    pub(crate) fn player_pos(&self) -> glam::Vec3 {
        glam::Vec3::new(
            self.player_body.pos.x,
            self.player_body.pos.y,
            self.player_body.pos.z,
        )
    }
    /// 玩家眼睛位置（脚底 + 身高），main.rs 每帧同步给第一人称相机
    pub(crate) fn player_eye(&self) -> glam::Vec3 {
        glam::Vec3::new(
            self.player_body.pos.x,
            self.player_body.pos.y + self.stance.eye_height(),
            self.player_body.pos.z,
        )
    }
    /// 转发 WASD 按键状态（FPS 玩家移动；仅 Playing + 第一人称生效）
    /// C 键：站立 ↔ 下蹲（卧倒时按下先回到站立）。日志落在姿态真正变化时。
    pub(crate) fn toggle_crouch(&mut self) {
        let next = if self.stance == Stance::Crouching {
            Stance::Standing
        } else {
            Stance::Crouching
        };
        self.stance = next;
        log::info!("stance: {:?} eye={:.2}m", next, next.eye_height());
    }
    /// Z 键：站立 ↔ 卧倒（下蹲时按下直接转卧倒）。
    pub(crate) fn toggle_prone(&mut self) {
        let next = if self.stance == Stance::Prone {
            Stance::Standing
        } else {
            Stance::Prone
        };
        self.stance = next;
        log::info!("stance: {:?} eye={:.2}m", next, next.eye_height());
    }
    /// 冲刺键（Shift）按住状态
    pub(crate) fn set_sprint(&mut self, on: bool) {
        self.sprint_held = on;
    }
    /// 此刻是否真的在冲刺：按住 Shift **且** 正在前进、非后退、站立、未开镜、在地面。
    /// HUD/视场角可以据此变化；速度倍率在 `move_first_person` 里消费。
    pub(crate) fn sprinting(&self) -> bool {
        self.sprint_held
            && self.move_forward
            && !self.move_backward
            && self.stance == Stance::Standing
            && !self.hud.ads
            && self.jump_vel == 0.0
    }
    pub(crate) fn set_movement(&mut self, forward: bool, backward: bool, left: bool, right: bool) {
        self.move_forward = forward;
        self.move_backward = backward;
        self.move_left = left;
        self.move_right = right;
    }
    /// 本帧命中点列表（main.rs 生成命中火花后清空）
    pub(crate) fn take_hit_points(&mut self) -> Vec<[f32; 3]> {
        std::mem::take(&mut self.hit_points)
    }
    /// 本帧命中伤害值（与命中点一一对应；HUD 伤害飘字）
    pub(crate) fn take_hit_damages(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.hit_damages)
    }
    /// 弹着标记（弹孔）当前集合；`main.rs` 每帧读它生成渲染实例。
    pub(crate) fn impact_marks(&self) -> &[ImpactMark] {
        &self.impact_marks
    }
    /// 追加一枚弹着标记（`RV3D_NO_DECALS=1` 整体关闭，供同机位 A/B 当对照组）。
    pub(crate) fn push_impact_mark(&mut self, pos: [f32; 3], normal: [f32; 3]) {
        // 环境变量只解析一次：本函数虽然只在命中的那几帧被调用，
        // 但 `env::var` 带锁 + 扫环境表，没有必要每次付（与 WorldMarker::for_obstacle 同一处理）。
        static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DISABLED.get_or_init(|| std::env::var("RV3D_NO_DECALS").is_ok()) {
            return;
        }
        if self.impact_marks.len() >= IMPACT_MARK_MAX {
            self.impact_marks.remove(0);
        }
        self.impact_marks.push(ImpactMark {
            pos,
            normal,
            age: 0.0,
        });
    }
    /// 设置面板开关（ESC 切换）：开/关都播提示音
    pub(crate) fn toggle_settings(&mut self) {
        let was_open = self.hud.settings_open;
        self.hud.toggle_settings();
        if was_open {
            // 关闭设置面板：取消进行中的键位绑定，并持久化键位/音量/灵敏度
            self.hud.cancel_rebind();
            crate::config::save(&self.current_config());
        }
        let src = AudioSource::new(self.player_eye(), 1.0);
        self.sfx.play(
            &mut self.audio.mixer_mut(),
            SfxKind::UiBlip,
            src,
            Channel::Sfx,
            false,
        );
    }
    /// 设置面板是否打开（main.rs 据此拦截游戏输入/释放光标）
    pub(crate) fn settings_open(&self) -> bool {
        self.hud.settings_open
    }
    /// 循环切换设置面板选中项（音量/灵敏度/7 个键位动作）
    pub(crate) fn cycle_settings(&mut self) {
        self.hud.cycle_settings_selection();
    }
    /// 按当前选中项调整设置（滚轮 delta）
    pub(crate) fn adjust_settings(&mut self, delta: f32) {
        if self.hud.settings_selection == 0 {
            self.hud.adjust_volume(delta);
        } else if self.hud.settings_selection == 1 {
            self.hud.adjust_sensitivity(delta);
        } else if self.hud.settings_selection == 2 {
            self.hud.adjust_music_volume(delta);
        }
        // selection >= 3 是分辨率/画质/键位行，滚轮不做调整（Enter 进入绑定）
    }
    /// 灵敏度（0..=1）→ 相机 rad/px（0.0005..=0.0025，默认 0.5 → 0.0015）
    pub(crate) fn sensitivity_rads(&self) -> f32 {
        0.0005 + self.hud.sensitivity * 0.002
    }
    /// 第一人称玩家移动：WASD 相对相机朝向，与演示刚体碰撞推回，y 每帧贴地形
    /// 玩家跳跃请求（main.rs 按 Jump 绑定置位；落地后清除）
    pub(crate) fn jump_requested(&mut self, jump: bool) {
        self.jump_pressed = jump;
    }
    pub(crate) fn move_first_person(&mut self, camera: &Camera, dt: f32) {
        let fwd = glam::Vec3::new(camera.forward().x, 0.0, camera.forward().z).normalize_or_zero();
        let right = camera.right();
        let mut dx = 0.0f32;
        let mut dz = 0.0f32;
        if self.move_forward {
            dx += fwd.x;
            dz += fwd.z;
        }
        if self.move_backward {
            dx -= fwd.x;
            dz -= fwd.z;
        }
        if self.move_right {
            dx += right.x;
            dz += right.z;
        }
        if self.move_left {
            dx -= right.x;
            dz -= right.z;
        }
        let len = (dx * dx + dz * dz).sqrt();
        // 🔴 2026-09-13：**冲刺起跳保留水平动量**（用户要求："在奔跑的时候，跳的时候会有
        // 向前的力，会直接跟过去一样越过去"）。
        //
        // 原实现是纯"位置式"移动：每帧只按 `速度 × dt` 沿**当前输入方向**挪一下，
        // 不保存任何水平速度。于是起跳那一瞬间水平速度就没了 —— 而且空中还要再乘
        // `air_factor = 0.5`，结果是"原地直上直下"，冲刺跳跃完全过不去障碍。
        //
        // 现在：起跳瞬间把**当时的有效水平速度**存进 `jump_hvel`，空中把它按 `AIR_DRAG`
        // 衰减着继续推进（叠加在削弱后的空中控制之上），落地清零。
        // 这样冲刺跳能靠惯性跃过缺口，普通走跳几乎看不出来（速度本来就小）。
        if len > 1e-4 {
            // 空中控制衰减：跳跃中水平移动减半（真实物理——空中无法急转弯）
            let air_factor = if self.jump_vel != 0.0 { 0.5 } else { 1.0 };
            // 开镜减速：举枪瞄准移动 -35%（ADS 重量感；hud.ads 由 main.rs 每帧同步）
            let ads_factor = if self.hud.ads { 0.65 } else { 1.0 };
            let sprint_mul = if self.sprinting() { SPRINT_MUL } else { 1.0 };
            let step = (PLAYER_SPEED
                * self.stance.speed_mul()
                * sprint_mul
                * ads_factor
                * air_factor
                * dt)
                .min(0.5);
            // 惯性位移：与输入方向无关，纯粹把起跳时带走的速度延续下去
            let (ix, iz) = (self.jump_hvel.x * dt, self.jump_hvel.z * dt);
            let (mx, mz) = self
                .player_body
                .try_move(&self.world, dx / len * step + ix, dz / len * step + iz);
            let moved = (mx * mx + mz * mz).sqrt();
            if moved > 0.01 && self.time - self.footstep_timer >= FOOTSTEP_INTERVAL {
                self.footstep_timer = self.time;
                // 程序化脚步：短促宽带噪声，交替强弱（0.8 / 1.0）确定性变化
                let step_scale = 0.8 + 0.2 * ((self.time * 2.0) as u32 % 2) as f32;
                let pos = self.player_pos();
                self.audio.synth_mut().play_footstep(pos, step_scale);
            }
        } else if self.jump_hvel.length_squared() > 1e-6 {
            // 无输入但空中有惯性：仍然往前滑（松手也越得过去）
            let (ix, iz) = (self.jump_hvel.x * dt, self.jump_hvel.z * dt);
            let _ = self.player_body.try_move(&self.world, ix, iz);
        }
        // 惯性衰减（空中每帧），落地时由下面的贴地分支清零
        if self.jump_vel != 0.0 {
            let k = (1.0 - AIR_DRAG * dt).max(0.0);
            self.jump_hvel.x *= k;
            self.jump_hvel.z *= k;
        }
        // 垂直运动：跳跃（Jump 键按下且在地面 → 初速）+ 重力 + 落地贴地
        //
        // 🔴 2026-09-13：站立面 = **地形高度 与 障碍物顶面 的较大者**。
        // 配套改动在 `physics.rs`：`push_out_of_aabb` 现在带 Y 判据（站在盒子上方不再被
        // 水平推开），所以必须有东西接住玩家 —— 否则会直接穿进盒子里掉下去。
        // 这条接线同时给了"能站到集装箱/掩体/矮墙上"这个此前完全不存在的能力。
        let terrain = terrain_height_at(self.player_body.pos.x, self.player_body.pos.z);
        let support = self.player_body.support_height(&self.world, PLAYER_STEP_UP);
        let ground = terrain.max(support);
        let on_ground = self.player_body.pos.y <= ground + 0.05 && self.jump_vel <= 0.0;
        // 卧倒不能起跳（蹲姿可以）
        if self.jump_pressed && on_ground && self.stance != Stance::Prone {
            self.jump_vel = JUMP_SPEED;
            // 🔴 起飞瞬间把**当前有效水平速度**带进惯性（见上面 jump_hvel 的注释）。
            // 输入方向归一化：斜着冲刺不会比直着冲刺飞得更远。
            if len > 1e-4 {
                let sprint_mul = if self.sprinting() { SPRINT_MUL } else { 1.0 };
                let ads_factor = if self.hud.ads { 0.65 } else { 1.0 };
                let v = PLAYER_SPEED * self.stance.speed_mul() * sprint_mul * ads_factor;
                self.jump_hvel = glam::Vec3::new(dx / len * v, 0.0, dz / len * v);
            } else {
                self.jump_hvel = glam::Vec3::ZERO; // 原地起跳：没有惯性
            }
            let jpos = self.player_pos();
            self.audio.synth_mut().play_footstep(jpos, 1.0);
        }
        if self.jump_vel != 0.0 {
            self.jump_vel -= GRAVITY * dt;
            self.player_body.pos.y += self.jump_vel * dt;
            if self.player_body.pos.y <= ground {
                self.player_body.pos.y = ground;
                self.jump_vel = 0.0;
                self.jump_hvel = glam::Vec3::ZERO; // 落地：惯性结束
            }
        } else {
            self.player_body.pos.y = ground;
            self.jump_hvel = glam::Vec3::ZERO;
        }
        self.player_body.grounded = self.jump_vel <= 0.0;
        // 跳跃结束后清除请求（避免长按连续起跳；重置时也清）
        if on_ground && self.jump_pressed {
            self.jump_pressed = false;
        }
    }
}
