//! HUD 覆盖层模块（立即模式，纯数据驱动，自绘 quad）
//!
//! - `HudState`：HUD 状态（血量/弹药/FPS/小地图占位），由游戏逻辑每帧更新
//! - `HudElement`：纯数据模型，描述一个绘制图元（`Quad` / `Bar` / `Text`）
//! - `layout()`：纯函数，把 `HudState` 展开为渲染 quad 列表（位置/尺寸/颜色）
//! - `layout_elements()`：纯函数，输出元素级数据模型（供测试/调试/自定义拍平）
//! - 内置 5x7 位图字体（ASCII 0x20..=0x7E），`render_text()` 把字符串展开为小 quad
//!   列表，自绘文本，无需任何外部字体或依赖
//! - `handle_event()`：输入透传接口，UI 层一律返回"不消费输入"，绝不拦截游戏输入
//!
//! 本模块仅使用 `std`，不引入外部依赖；如将来需要新依赖，在文件头部按
//! `// DEP: crate = version` 声明。
//! 尚未接入 main.rs 主循环，整体允许 dead_code 警告。

#![allow(dead_code)]

/// 2D 颜色（RGBA，分量范围 0.0..=1.0）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);
    pub const GREEN: Color = Color::new(0.2, 0.9, 0.3, 1.0);
    pub const YELLOW: Color = Color::new(0.95, 0.8, 0.2, 1.0);
    pub const RED: Color = Color::new(0.9, 0.2, 0.2, 1.0);
    pub const CYAN: Color = Color::new(0.2, 0.8, 0.9, 1.0);
    pub const ORANGE: Color = Color::new(0.95, 0.6, 0.2, 1.0);
}

/// 2D 矩形（屏幕坐标，原点左上角，x 向右 / y 向下，单位为像素）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// 一个可渲染的四边形（位置/尺寸/颜色）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub rect: Rect,
    pub color: Color,
}

impl Quad {
    pub const fn new(rect: Rect, color: Color) -> Self {
        Self { rect, color }
    }
}

/// HUD 元素（纯数据模型）
#[derive(Debug, Clone, PartialEq)]
pub enum HudElement {
    /// 实心矩形
    Quad(Quad),
    /// 条形（血条/弹药条等）：`back` 为底色，`fill` 为按 `ratio` 缩放宽度的前景色
    Bar { back: Quad, fill: Quad, ratio: f32 },
    /// 文本：左上角 `(x, y)`，`scale` 为像素缩放（1.0 = 5x7 原始大小）
    Text { text: String, x: f32, y: f32, color: Color, scale: f32 },
}

impl HudElement {
    /// 把元素拍平为渲染 quad 列表：
    /// - `Quad` → 自身
    /// - `Bar` → 背景 + 按比例宽度的填充，共 2 个 quad
    /// - `Text` → 由 5x7 位图字体展开为若干小 quad
    pub fn to_quads(&self, out: &mut Vec<Quad>) {
        match self {
            HudElement::Quad(q) => out.push(*q),
            HudElement::Bar { back, fill, ratio } => {
                out.push(*back);
                let ratio = ratio.clamp(0.0, 1.0);
                let mut fill_rect = fill.rect;
                fill_rect.w = fill.rect.w * ratio;
                out.push(Quad::new(fill_rect, fill.color));
            }
            HudElement::Text { text, x, y, color, scale } => {
                render_text(text, *x, *y, *color, *scale, out);
            }
        }
    }
}

/// 当前画面（由游戏状态机驱动，HUD 只做纯布局）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HudScreen {
    /// 游戏中：血条/弹药/分数/波次/准星
    #[default]
    Game,
    /// 开始菜单：标题 + 操作提示
    Start,
    /// 死亡结算：分数 + 重开提示
    GameOver,
    /// 设置面板：音量/灵敏度/键位
    Settings,
}

/// 分辨率选项（设置面板 RESOLUTION 行循环切换；索引与 config.rs 持久化的 resolution 对齐）
/// 含 16:10 档位 1280x800：16:10 显示器首次运行默认用它（见 main.rs 默认分辨率选择）
pub const RESOLUTIONS: [(u32, u32); 5] = [
    (1280, 720),
    (1280, 800),
    (1600, 900),
    (1920, 1080),
    (2560, 1600),
];
/// 分辨率显示名（设置面板/日志用，ASCII 大写，位图字体可用）
pub const RESOLUTION_LABELS: [&str; 5] =
    ["1280x720", "1280x800", "1600x900", "1920x1080", "2560x1600"];
/// 画质选项（0=LOW / 1=MEDIUM / 2=HIGH；索引 = config.rs 持久化的 quality）
pub const QUALITY_LABELS: [&str; 3] = ["LOW", "MEDIUM", "HIGH"];

/// 键位绑定动作（枚举顺序 = 设置面板键位行顺序，`selected_action()` 索引映射依赖此顺序）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingAction {
    /// 前进
    Forward,
    /// 后退
    Backward,
    /// 左移
    Left,
    /// 右移
    Right,
    /// 换弹
    Reload,
    /// 开火
    Fire,
    /// 跳跃
    Jump,
    /// 菜单/设置
    Menu,
}

/// 键位配置（纯数据，u32 物理键码 = winit 0.30 `KeyCode` 枚举序号，非 USB HID 码）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyBindings {
    /// 前进键
    pub move_forward: u32,
    /// 后退键
    pub move_backward: u32,
    /// 左移键
    pub move_left: u32,
    /// 右移键
    pub move_right: u32,
    /// 换弹键
    pub reload: u32,
    /// 菜单/设置键
    pub menu: u32,
    /// 开火键（默认 0 = 无键盘开火，用鼠标左键）
    pub fire: u32,
    /// 跳跃键
    pub jump: u32,
}

impl KeyBindings {
    /// 默认键位：W/S/A/D 移动、R 换弹、MENU 键设置、SPACE 开火。
    ///
    /// 键码是 winit 0.30 `KeyCode` 枚举序号（KeyW=41/KeyS=37/KeyA=19/KeyD=22/
    /// KeyR=36/ContextMenu=54/Space=62），不是 USB HID 码；由测试
    /// `winit_keycode_indices_match_table` 锁死，winit 升级导致序号漂移会立刻暴露。
    pub fn defaults() -> Self {
        Self {
            move_forward: 41,  // KeyW
            move_backward: 37, // KeyS
            move_left: 19,     // KeyA
            move_right: 22,    // KeyD
            reload: 36,        // KeyR
            menu: 54,          // ContextMenu（物理菜单键）
            fire: 0,           // 无键盘开火（鼠标左键开火；2026-08-15 Space 改为跳跃）
            jump: 62,          // Space（跳跃）
        }
    }

    /// 动作的默认键码（W/S/A/D 移动、R 换弹、MENU 键设置、SPACE 开火）
    pub fn default_code(action: BindingAction) -> u32 {
        match action {
            BindingAction::Forward => 41,  // KeyW
            BindingAction::Backward => 37, // KeyS
            BindingAction::Left => 19,     // KeyA
            BindingAction::Right => 22,    // KeyD
            BindingAction::Reload => 36,   // KeyR
            BindingAction::Fire => 0,      // 无（鼠标左键开火）
            BindingAction::Jump => 62,     // Space（跳跃）
            BindingAction::Menu => 54,     // ContextMenu
        }
    }

    /// 查询指定动作当前绑定的键码
    pub fn code_for(&self, action: BindingAction) -> u32 {
        match action {
            BindingAction::Forward => self.move_forward,
            BindingAction::Backward => self.move_backward,
            BindingAction::Left => self.move_left,
            BindingAction::Right => self.move_right,
            BindingAction::Reload => self.reload,
            BindingAction::Fire => self.fire,
            BindingAction::Jump => self.jump,
            BindingAction::Menu => self.menu,
        }
    }

    /// 按 Forward→Menu 顺序返回第一个绑定该键码的动作（互斥后通常唯一，顺序仅兜底）
    pub fn action_for(&self, code: u32) -> Option<BindingAction> {
        [
            BindingAction::Forward,
            BindingAction::Backward,
            BindingAction::Left,
            BindingAction::Right,
            BindingAction::Reload,
            BindingAction::Fire,
            BindingAction::Jump,
            BindingAction::Menu,
        ]
        .into_iter()
        .find(|&a| self.code_for(a) == code)
    }

    /// 动作显示名（设置面板/按键提示用，ASCII 大写）
    pub fn action_name(action: BindingAction) -> &'static str {
        match action {
            BindingAction::Forward => "FORWARD",
            BindingAction::Backward => "BACKWARD",
            BindingAction::Left => "LEFT",
            BindingAction::Right => "RIGHT",
            BindingAction::Reload => "RELOAD",
            BindingAction::Fire => "FIRE",
            BindingAction::Jump => "JUMP",
            BindingAction::Menu => "MENU",
        }
    }

    /// 把动作绑定到新键码；同时把其它所有"当前键码 == code"的动作重置回各自默认键码。
    ///
    /// 互斥原因：同一键码同时绑定两个动作时，按下该键会产生歧义行为，
    /// 因此优先保证新绑定生效，冲突动作一律退回默认键位。
    pub fn bind(&mut self, action: BindingAction, code: u32) {
        let slot = match action {
            BindingAction::Forward => &mut self.move_forward,
            BindingAction::Backward => &mut self.move_backward,
            BindingAction::Left => &mut self.move_left,
            BindingAction::Right => &mut self.move_right,
            BindingAction::Reload => &mut self.reload,
            BindingAction::Fire => &mut self.fire,
            BindingAction::Jump => &mut self.jump,
            BindingAction::Menu => &mut self.menu,
        };
        *slot = code;
        // 冲突动作复位：按 Forward→Menu 顺序检查其它动作，凡当前键码 == code 者
        // 重置回各自的默认键码，避免两个动作共用同一键。
        for other in [
            BindingAction::Forward,
            BindingAction::Backward,
            BindingAction::Left,
            BindingAction::Right,
            BindingAction::Reload,
            BindingAction::Fire,
            BindingAction::Menu,
        ] {
            if other != action && self.code_for(other) == code {
                match other {
                    BindingAction::Forward => self.move_forward = Self::default_code(other),
                    BindingAction::Backward => self.move_backward = Self::default_code(other),
                    BindingAction::Left => self.move_left = Self::default_code(other),
                    BindingAction::Right => self.move_right = Self::default_code(other),
                    BindingAction::Reload => self.reload = Self::default_code(other),
                    BindingAction::Fire => self.fire = Self::default_code(other),
                    BindingAction::Jump => self.jump = Self::default_code(other),
                    BindingAction::Menu => self.menu = Self::default_code(other),
                }
            }
        }
    }

    /// 常见键码 → 显示名（供设置面板文本）；其余回退 `KEY#<code>`
    pub fn label(code: u32) -> String {
        match code {
            // 序号 = winit 0.30 KeyCode 枚举隐式值（0,1,2,...），见 keyboard.rs
            0 => "GRAVE".to_string(),
            1 => "BACKSLASH".to_string(),
            2 => "LBRACKET".to_string(),
            3 => "RBRACKET".to_string(),
            4 => "COMMA".to_string(),
            5..=14 => char::from(b'0' + (code - 5) as u8).to_string(),
            15 => "EQUALS".to_string(),
            16 => "INTL-BS".to_string(),
            19..=44 => char::from(b'A' + (code - 19) as u8).to_string(),
            45 => "MINUS".to_string(),
            46 => "PERIOD".to_string(),
            47 => "APOSTROPHE".to_string(),
            48 => "SEMICOLON".to_string(),
            49 => "SLASH".to_string(),
            50 => "LALT".to_string(),
            51 => "RALT".to_string(),
            52 => "BACKSPACE".to_string(),
            53 => "CAPSLOCK".to_string(),
            54 => "MENU".to_string(),
            55 => "LCTRL".to_string(),
            56 => "RCTRL".to_string(),
            57 => "ENTER".to_string(),
            58 => "LSUPER".to_string(),
            59 => "RSUPER".to_string(),
            60 => "LSHIFT".to_string(),
            61 => "RSHIFT".to_string(),
            62 => "SPACE".to_string(),
            63 => "TAB".to_string(),
            72 => "DELETE".to_string(),
            73 => "END".to_string(),
            74 => "HELP".to_string(),
            75 => "HOME".to_string(),
            76 => "INSERT".to_string(),
            77 => "PAGEDOWN".to_string(),
            78 => "PAGEUP".to_string(),
            79 => "DOWN".to_string(),
            80 => "LEFT".to_string(),
            81 => "RIGHT".to_string(),
            82 => "UP".to_string(),
            83 => "NUMLOCK".to_string(),
            84..=93 => format!("NUM{}", code - 84),
            94 => "NUM+".to_string(),
            99 => "NUM.".to_string(),
            100 => "NUM/".to_string(),
            101 => "NUMENTER".to_string(),
            109 => "NUM*".to_string(),
            113 => "NUM-".to_string(),
            114 => "ESC".to_string(),
            117 => "PRINTSCREEN".to_string(),
            118 => "SCROLLLOCK".to_string(),
            119 => "PAUSE".to_string(),
            159..=170 => format!("F{}", code - 158),
            other => format!("KEY#{}", other),
        }
    }

    /// 保留系统键（设置面板导航/截图/垂直移动/补给），不可作为可重绑定键位
    pub fn is_reserved(code: u32) -> bool {
        // KeyE=23 / KeyN=32 / KeyQ=35 / Enter=57 / Tab=63 / Escape=114 / F12=170
        matches!(code, 23 | 32 | 35 | 57 | 63 | 114 | 170)
    }
}

/// HUD 状态（每帧由游戏逻辑更新，`layout()` 读取它生成绘制命令）
#[derive(Debug, Clone, PartialEq)]
pub struct HudState {
    /// 屏幕宽度（像素）
    pub screen_w: f32,
    /// 屏幕高度（像素）
    pub screen_h: f32,
    /// 当前血量
    pub health: f32,
    /// 血量上限
    pub max_health: f32,
    /// 当前弹药数
    pub ammo: u32,
    /// 弹匣容量
    pub max_ammo: u32,
    /// 备弹（弹匣外，随换弹补充）
    pub reserve: u32,
    /// 最近一帧 FPS（用于 FPS 文本显示）
    pub fps: f32,
    /// 小地图占位是否显示
    pub minimap_visible: bool,
    /// 当前得分
    pub score: u64,
    /// 当前波次
    pub wave: u32,
    /// 波间倒计时（秒，>0 时顶部显示"下一波"）
    pub countdown: f32,
    /// survive 总波数（0 = 非 survive 规则；game.rs 每帧同步，HUD 显示 "WAVE x/N"）
    pub survive_waves: u32,
    /// 当前画面（游戏/菜单/结算）
    pub screen: HudScreen,
    /// 设置面板是否打开（由 `toggle_settings()` 切换，接入主循环后用于拦截游戏输入）
    pub settings_open: bool,
    /// 音量（0..=1，默认 0.8）
    pub volume: f32,
    /// 音乐音量（0..=1，默认 0.6；独立于总音量，作用于 Mixer 的 Music 通道）
    pub music_volume: f32,
    /// 鼠标灵敏度（0..=1，默认 0.5）
    pub sensitivity: f32,
    /// 分辨率索引（0..=3，对应 RESOLUTIONS；默认按显示器宽高比取 0=1280x720 或 1=1280x800）
    pub resolution_index: u8,
    /// 画质索引（0..=2，对应 QUALITY_LABELS；默认 1 = MEDIUM）
    pub quality_index: u8,
    /// 键位配置（默认 WASD + R + ESC + SPACE）
    pub key_bindings: KeyBindings,
    /// 设置面板当前选中项（0=音量 / 1=灵敏度 / 2=音乐 / 3=分辨率 / 4=画质 / 5..=11=键位，Tab 循环）
    pub settings_selection: u8,
    /// 当前武器名（HUD 弹药行显示，如 "M1 Rifle" / "Thompson SMG"）
    pub weapon_name: String,
    /// 是否处于切枪计时（切换期间禁用开火/换弹）
    pub switching: bool,
    /// 手榴弹库存（HUD 显示；0..=2，G 投掷、N 补给）
    pub grenades: u32,
    /// 命中标记剩余显示时间（秒，>0 时准星外圈闪一下）
    pub hit_marker_timer: f32,
    /// 是否正在换弹（由游戏逻辑写入）
    pub reloading: bool,
    /// 换弹进度（0..=1）
    pub reload_progress: f32,
    /// 当前关卡（默认 1，由 game.rs 每帧同步）
    pub level: u32,
    /// 正在等待重新绑定的动作（None = 无；Some = 设置面板等待按键）
    pub rebinding: Option<BindingAction>,
    /// 退出确认：ESC 首次按下置位（HUD 提示再按一次退出），任意其它键取消
    pub confirm_quit: bool,
    /// 任务目标进度（已歼灭/目标；0/0 = 未启用），game.rs 每帧同步
    pub objective: (u32, u32),
    /// 占领据点状态（id/归属/进度 0..=1），game.rs 每帧同步；空 = 无据点
    pub capture_points: Vec<(String, Option<crate::engine::ai::Team>, f32)>,
    /// 胜利横幅（任务目标达成时置位；重开/升关/新一轮时由 game.rs 清除）
    pub victory_banner: Option<String>,
    /// 累计运行时间（秒，由 `tick(dt)` 累加，驱动开始菜单闪烁）
    pub elapsed: f32,
    /// 击杀提示（右上角 feed，最多保留 4 条；每帧由 `tick_kill_feed(dt)` 老化）
    pub kill_feed: Vec<KillFeedEntry>,
    /// ESC 菜单是否打开（毛玻璃菜单：退出游戏 / 设置）
    pub esc_menu_open: bool,
    /// ESC 菜单当前选中项（0=退出游戏 1=设置）
    pub esc_menu_selection: u8,
    /// 开镜瞄准中（main.rs 每帧同步；准星收窄、枪模居中提示）
    pub ads: bool,
    /// 小地图单位快照：(x, z, 阵营 0=红方 1=蓝方)；每帧由 game 同步
    pub mm_units: Vec<(f32, f32, u8)>,
    /// 小地图障碍快照：(x, z, half_w, half_d, 种类 0=墙 1=栅栏 2=树 3=建筑 4=废墟)
    pub mm_obstacles: Vec<(f32, f32, f32, f32, u8)>,
    /// 小地图玩家世界位置 (x, z)
    pub mm_player: [f32; 2],
    /// 玩家朝向（弧度，小地图旋转使"前方朝上"）
    pub mm_yaw: f32,
}

/// 击杀提示条目（战地风格右上角 feed）
#[derive(Debug, Clone, PartialEq)]
pub struct KillFeedEntry {
    /// 显示文本（如 "YOU KILLED RED #12" / "RED KILLED BLUE" / "YOU WERE KILLED"）
    pub text: String,
    /// 已存留秒数（超过 KILL_FEED_DURATION 移除）
    pub age: f32,
}

/// 血条文字缩放
const TEXT_SCALE: f32 = 1.4;

/// 命中标记闪烁时长（秒）
const HIT_MARKER_DURATION: f32 = 0.15;

/// 击杀提示存留时长（秒）
const KILL_FEED_DURATION: f32 = 6.0;

impl HudState {
    /// 创建 HUD 状态（默认满血满弹、FPS 0、显示小地图占位）
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        Self {
            screen_w,
            screen_h,
            health: 100.0,
            max_health: 100.0,
            ammo: 30,
            max_ammo: 30,
            reserve: 120,
            fps: 0.0,
            minimap_visible: true,
            score: 0,
            wave: 1,
            countdown: 0.0,
            survive_waves: 0,
            screen: HudScreen::Game,
            settings_open: false,
            volume: 0.8,
            music_volume: 0.6,
            sensitivity: 0.5,
            resolution_index: 0,
            quality_index: 1,
            key_bindings: KeyBindings::defaults(),
            settings_selection: 0,
            weapon_name: "M1 Rifle".to_string(),
            switching: false,
            grenades: 2,
            hit_marker_timer: 0.0,
            reloading: false,
            reload_progress: 0.0,
            level: 1,
            rebinding: None,
            confirm_quit: false,
            objective: (0, 0),
            capture_points: Vec::new(),
            victory_banner: None,
            elapsed: 0.0,
            kill_feed: Vec::new(),
            esc_menu_open: false,
            esc_menu_selection: 0,
            ads: false,
            mm_units: Vec::new(),
            mm_obstacles: Vec::new(),
            mm_player: [0.0, 0.0],
            mm_yaw: 0.0,
        }
    }

    /// 血量比例（clamp 到 0..=1；`max_health <= 0` 视为 0）
    pub fn health_ratio(&self) -> f32 {
        if self.max_health <= 0.0 {
            0.0
        } else {
            (self.health / self.max_health).clamp(0.0, 1.0)
        }
    }

    /// 弹药比例（clamp 到 0..=1；`max_ammo == 0` 视为 0）
    pub fn ammo_ratio(&self) -> f32 {
        if self.max_ammo == 0 {
            0.0
        } else {
            (self.ammo as f32 / self.max_ammo as f32).clamp(0.0, 1.0)
        }
    }

    /// HUD 分辨率缩放系数：以 1280x800 为设计基准，按实际高度等比放大
    /// （2560x1600 → 2.0，字体/面板/血条等比放大，避免高分辨率下文字过细）
    pub fn ui_scale(&self) -> f32 {
        (self.screen_h / 800.0).clamp(1.0, 3.0)
    }

    /// 纯布局函数：按当前画面展开为元素列表（出口统一按分辨率缩放）。
    pub fn layout_elements(&self) -> Vec<HudElement> {
        let mut elems = match self.screen {
            HudScreen::Game => self.game_elements(),
            HudScreen::Start => self.start_menu_elements(),
            HudScreen::GameOver => self.game_over_elements(),
            HudScreen::Settings => self.settings_elements(),
        };
        // ESC 毛玻璃菜单：半透明全屏遮罩 + 两个选项（退出游戏 / 设置），覆盖在任何画面之上
        if self.esc_menu_open {
            self.esc_menu_elements(&mut elems);
        }
        // 统一分辨率缩放：坐标/尺寸/字号全部乘 ui_scale（布局按 1280x800 设计）
        let s = self.ui_scale();
        for e in elems.iter_mut() {
            match e {
                HudElement::Quad(q) => {
                    q.rect.x *= s;
                    q.rect.y *= s;
                    q.rect.w *= s;
                    q.rect.h *= s;
                }
                HudElement::Bar { back, fill, ratio } => {
                    let _ = ratio;
                    back.rect.x *= s;
                    back.rect.y *= s;
                    back.rect.w *= s;
                    back.rect.h *= s;
                    fill.rect.x *= s;
                    fill.rect.y *= s;
                    fill.rect.w *= s;
                    fill.rect.h *= s;
                }
                HudElement::Text { x, y, scale, .. } => {
                    *x *= s;
                    *y *= s;
                    *scale *= s;
                }
            }
        }
        elems
    }

    /// ESC 毛玻璃菜单：全屏半透明暗色遮罩（毛玻璃观感）+ 居中面板 + 两个选项
    /// 选中项高亮（Tab 切换 / Enter 确认 / ESC 关闭，由 main.rs 键位处理驱动）
    fn esc_menu_elements(&self, elems: &mut Vec<HudElement>) {
        // 设计基准 1280x800：坐标先用设计空间计算，layout_elements 出口统一乘 ui_scale
        // （不能直接用 screen_w/h —— 那样会双重缩放，血条/准星被推出屏幕）
        let s = self.ui_scale();
        let w = self.screen_w / s;
        let h = self.screen_h / s;
        // 全屏半透明遮罩（模拟毛玻璃暗化背景）
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, 0.0, w, h),
            Color::new(0.02, 0.03, 0.05, 0.55),
        )));
        // 居中面板
        let pw = 380.0;
        let ph = 240.0;
        let px = (w - pw) * 0.5;
        let py = (h - ph) * 0.5;
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(px, py, pw, ph),
            Color::new(0.06, 0.09, 0.12, 0.85),
        )));
        // 标题
        let title = "PAUSED";
        elems.push(HudElement::Text {
            text: title.to_string(),
            x: w * 0.5 - text_width(title, 1.6) * 0.5,
            y: py + 30.0,
            color: Color::CYAN,
            scale: 1.6,
        });
        // 两个选项：0=退出游戏 1=设置（选中项黄底高亮；中文走 8x8 点阵）
        let options = ["退出游戏", "设置"];
        for (i, label) in options.iter().enumerate() {
            let oy = py + 90.0 + i as f32 * 56.0;
            let selected = (i as u8) == self.esc_menu_selection;
            if selected {
                elems.push(HudElement::Quad(Quad::new(
                    Rect::new(px + 60.0, oy - 6.0, pw - 120.0, 34.0),
                    Color::new(0.35, 0.45, 0.55, 0.65),
                )));
            }
            elems.push(HudElement::Text {
                text: label.to_string(),
                x: w * 0.5 - text_width(label, 1.2) * 0.5,
                y: oy,
                color: if selected { Color::YELLOW } else { Color::WHITE },
                scale: 1.2,
            });
        }
        // 操作提示
        let hint = "TAB 切换  |  ENTER 确认  |  ESC 关闭";
        elems.push(HudElement::Text {
            text: hint.to_string(),
            x: w * 0.5 - text_width(hint, 0.7) * 0.5,
            y: py + ph - 26.0,
            color: Color::new(0.6, 0.65, 0.7, 0.9),
            scale: 0.7,
        });
    }

    /// 游戏画面元素：血条/弹药/FPS/小地图 + 分数/波次/倒计时/准星
    ///
    /// 布局规则（左上角原点，像素坐标）：
    /// - 左下角：血条（宽度约屏幕 30%，上限 360px）+ 文字 `HP x/y`
    /// - 血条右侧：弹药条 + 文字 `AMMO x/y`
    /// - 左上角：FPS 文本
    /// - 右上角：小地图占位（半透明底 + 边框 + 中心玩家十字标记）
    fn game_elements(&self) -> Vec<HudElement> {
        let mut elems = Vec::new();
        // 设计基准 1280x800：坐标先用设计空间计算，layout_elements 出口统一乘 ui_scale
        // （不能直接用 screen_w/h —— 那样会双重缩放，血条/准星被推出屏幕）
        let s = self.ui_scale();
        let w = self.screen_w / s;
        let h = self.screen_h / s;
        let margin = 24.0;

        // ---- 血条（左下角）----
        let bar_w = (w * 0.30).min(360.0);
        let bar_h = 22.0;
        let back = Quad::new(
            Rect::new(margin, h - margin - bar_h, bar_w, bar_h),
            Color::new(0.08, 0.08, 0.10, 0.75),
        );
        let fill = Quad::new(
            Rect::new(margin + 3.0, h - margin - bar_h + 3.0, bar_w - 6.0, bar_h - 6.0),
            health_color(self.health_ratio()),
        );
        elems.push(HudElement::Bar {
            back,
            fill,
            ratio: self.health_ratio(),
        });
        elems.push(HudElement::Text {
            text: format!("HP {:.0}/{:.0}", self.health, self.max_health),
            x: margin + 6.0,
            y: h - margin - bar_h + (bar_h - 7.0 * TEXT_SCALE) * 0.5,
            color: Color::WHITE,
            scale: TEXT_SCALE,
        });

        // ---- 弹药条（血条右侧）----
        let ammo_w = bar_w * 0.45;
        let ammo_x = margin + bar_w + 16.0;
        let ammo_back = Quad::new(
            Rect::new(ammo_x, h - margin - bar_h, ammo_w, bar_h),
            Color::new(0.08, 0.08, 0.10, 0.75),
        );
        let ammo_fill = Quad::new(
            Rect::new(ammo_x + 3.0, h - margin - bar_h + 3.0, ammo_w - 6.0, bar_h - 6.0),
            Color::ORANGE,
        );
        elems.push(HudElement::Bar {
            back: ammo_back,
            fill: ammo_fill,
            ratio: self.ammo_ratio(),
        });
        elems.push(HudElement::Text {
            text: format!(
                "{}  AMMO {}/{} +{}  |  GRENADES {}",
                self.weapon_name, self.ammo, self.max_ammo, self.reserve, self.grenades
            ),
            x: ammo_x + 6.0,
            y: h - margin - bar_h + (bar_h - 7.0 * TEXT_SCALE) * 0.5,
            color: Color::WHITE,
            scale: TEXT_SCALE,
        });
        // 切枪计时提示（切换期间禁止开火/换弹）
        if self.switching {
            elems.push(HudElement::Text {
                text: "SWITCHING...".to_string(),
                x: ammo_x + 6.0,
                y: h - margin - bar_h + 26.0,
                color: Color::YELLOW,
                scale: 0.8,
            });
        }

        // ---- 换弹指示（弹药条下方，黄字 + 进度条）----
        if self.reloading {
            let reload_txt = "RELOADING";
            elems.push(HudElement::Text {
                text: reload_txt.to_string(),
                x: ammo_x + (ammo_w - text_width(reload_txt, 1.0)) * 0.5,
                y: h - margin + 2.0,
                color: Color::YELLOW,
                scale: 1.0,
            });
            let pbar_w = ammo_w * 0.7;
            let pbar_h = 5.0;
            let pbar_x = ammo_x + (ammo_w - pbar_w) * 0.5;
            let pbar_y = h - margin + 13.0;
            elems.push(HudElement::Bar {
                back: Quad::new(
                    Rect::new(pbar_x, pbar_y, pbar_w, pbar_h),
                    Color::new(0.10, 0.10, 0.12, 0.85),
                ),
                fill: Quad::new(
                    Rect::new(pbar_x + 1.0, pbar_y + 1.0, pbar_w - 2.0, pbar_h - 2.0),
                    Color::YELLOW,
                ),
                ratio: self.reload_progress.clamp(0.0, 1.0),
            });
        }

        // ---- 占领据点（顶部中央，波次下方）：归属色 + 进度条 ----
        if !self.capture_points.is_empty() {
            let center_x = w * 0.5;
            let pts = &self.capture_points;
            let label_w = 70.0;
            let gap = 16.0;
            let total_w = pts.len() as f32 * label_w + (pts.len().saturating_sub(1)) as f32 * gap;
            let start_x = center_x - total_w * 0.5;
            let y = margin + 42.0;
            for (i, (id, owner, progress)) in pts.iter().enumerate() {
                let x = start_x + i as f32 * (label_w + gap);
                let bar_h = 10.0;
                let back = Quad::new(
                    Rect::new(x, y, label_w, bar_h),
                    Color::new(0.08, 0.08, 0.10, 0.75),
                );
                let fill_color = match owner {
                    Some(crate::engine::ai::Team::Blue) => Color::new(0.08, 0.55, 0.98, 1.0),
                    Some(crate::engine::ai::Team::Red) => Color::new(0.95, 0.12, 0.08, 1.0),
                    None => Color::new(0.45, 0.45, 0.45, 1.0),
                };
                let fill = Quad::new(
                    Rect::new(x + 1.0, y + 1.0, label_w - 2.0, bar_h - 2.0),
                    fill_color,
                );
                elems.push(HudElement::Bar {
                    back,
                    fill,
                    ratio: progress.clamp(0.0, 1.0),
                });
                let owner_txt = match owner {
                    Some(crate::engine::ai::Team::Blue) => "BLUE",
                    Some(crate::engine::ai::Team::Red) => "RED",
                    None => "NONE",
                };
                let id_txt = format!("{}: {}", id, owner_txt);
                let id_w = text_width(&id_txt, 1.0);
                elems.push(HudElement::Text {
                    text: id_txt,
                    x: x + (label_w - id_w) * 0.5,
                    y: y + bar_h + 2.0,
                    color: Color::WHITE,
                    scale: 1.0,
                });
            }
        }

        // ---- FPS（左上角）----
        elems.push(HudElement::Text {
            text: format!("FPS {:.0}", self.fps),
            x: margin,
            y: margin,
            color: Color::CYAN,
            scale: 2.0,
        });



        // ---- 波次/分数（顶部中央）----
        let center_x = w * 0.5;
        let top = margin;
        // survive 规则显示 "WAVE x/N"，普通模式 "WAVE x"
        let wave_txt = if self.survive_waves > 0 {
            format!("WAVE {}/{}", self.wave, self.survive_waves)
        } else {
            format!("WAVE {}", self.wave)
        };
        let wave_x = center_x - text_width(&wave_txt, 1.8) * 0.5;
        elems.push(HudElement::Text {
            text: wave_txt,
            x: wave_x,
            y: top,
            color: Color::WHITE,
            scale: 1.8,
        });
        let score_txt = format!("SCORE {}", self.score);
        let score_x = center_x - text_width(&score_txt, 1.4) * 0.5;
        elems.push(HudElement::Text {
            text: score_txt,
            x: score_x,
            y: top + 20.0,
            color: Color::YELLOW,
            scale: 1.4,
        });
        // ---- 关卡（level 由 game.rs 每帧同步）----
        let level_txt = format!("LEVEL {}", self.level);
        let level_x = center_x - text_width(&level_txt, 1.2) * 0.5;
        elems.push(HudElement::Text {
            text: level_txt,
            x: level_x,
            y: top + 38.0,
            color: Color::WHITE,
            scale: 1.2,
        });
        // ---- 波间倒计时 ----
        if self.countdown > 0.0 {
            let next_txt = format!("WAVE {} IN {:.0}", self.wave + 1, self.countdown.ceil());
            let next_x = center_x - text_width(&next_txt, 1.6) * 0.5;
            elems.push(HudElement::Text {
                text: next_txt,
                x: next_x,
                y: top + 58.0,
                color: Color::CYAN,
                scale: 1.6,
            });
        }
        // ---- 任务目标进度（顶部，波次/倒计时下方）----
        if self.objective.1 > 0 {
            let obj_txt = format!("OBJECTIVE 歼灭敌人 {}/{}", self.objective.0, self.objective.1);
            let obj_x = center_x - text_width(&obj_txt, 1.2) * 0.5;
            elems.push(HudElement::Text {
                text: obj_txt,
                x: obj_x,
                y: top + 78.0,
                color: Color::WHITE,
                scale: 1.2,
            });
        }
        // ---- 胜利横幅（任务目标达成，居中）----
        if let Some(banner) = &self.victory_banner {
            let bw = text_width(banner, 2.4);
            elems.push(HudElement::Text {
                text: banner.clone(),
                x: center_x - bw * 0.5,
                y: h * 0.35,
                color: Color::GREEN,
                scale: 2.4,
            });
        }
        // ---- 准星（屏幕中心）：腰射 = 扩散十字；开镜 = 极小中心瞄准点 ----
        // 2026-08-19：开镜隐藏十字，但保留 3px 中心红点——弹道=屏幕中心，
        // 机瞄为视觉参考；若完全无瞄准点，玩家凭机瞄瞄准会与弹道偏差（打不中）。
        let cx = w * 0.5;
        let cy = h * 0.5;
        if self.ads {
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 1.5, cy - 1.5, 3.0, 3.0),
                Color::new(1.0, 0.2, 0.2, 0.9),
            )));
        } else if !self.ads {
            // 腰射：扩散十字（半长 8px）
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 8.0, cy - 1.5, 16.0, 3.0),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 1.5, cy - 8.0, 3.0, 16.0),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 1.5, cy - 1.5, 3.0, 3.0),
                Color::RED,
            )));
        }

        // ---- 命中标记（准星外圈四个短臂，闪一下）----
        if self.hit_marker_timer > 0.0 {
            let arm_len = 7.0;
            let thick = 3.0;
            let radius = 14.0;
            let hit = Color::RED;
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - radius - arm_len, cy - thick * 0.5, arm_len, thick),
                hit,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx + radius, cy - thick * 0.5, arm_len, thick),
                hit,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - thick * 0.5, cy - radius - arm_len, thick, arm_len),
                hit,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - thick * 0.5, cy + radius, thick, arm_len),
                hit,
            )));
        }

        // ---- 小地图（右上角，玩家朝上旋转；实时显示地形/障碍/红蓝单位）----
        if self.minimap_visible {
            let size = 200.0;
            let mm_x = w - margin - size;
            let mm_y = margin;
            let border = 2.0;
            // 边框
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y - border, size + border * 2.0, border),
                Color::new(0.85, 0.9, 0.95, 0.9),
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y + size, size + border * 2.0, border),
                Color::new(0.85, 0.9, 0.95, 0.9),
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y, border, size),
                Color::new(0.85, 0.9, 0.95, 0.9),
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x + size, mm_y, border, size),
                Color::new(0.85, 0.9, 0.95, 0.9),
            )));
            // 底色（深色半透明）
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x, mm_y, size, size),
                Color::new(0.10, 0.14, 0.17, 0.82),
            )));
            let cx = mm_x + size * 0.5;
            let cy = mm_y + size * 0.5;
            // 旋转投影：世界范围 500m（±250，地图边界）→ size px
            // 前方（水平）= (-sin yaw, -cos yaw)（yaw=0 朝 -Z），右 = (cos yaw, -sin yaw)
            // 世界偏移 (dx, dz) → 屏幕偏移 (sx, sy)：前方朝屏幕上方（-y）
            let world = 500.0;
            let scale = size / world;
            let (sin_y, cos_y) = (self.mm_yaw.sin(), self.mm_yaw.cos());
            let proj = |dx: f32, dz: f32| -> (f32, f32) {
                (
                    (dx * cos_y - dz * sin_y) * scale,
                    (dx * sin_y + dz * cos_y) * scale,
                )
            };
            let inside = |sx: f32, sy: f32| {
                sx.abs() < size * 0.5 - 4.0 && sy.abs() < size * 0.5 - 4.0
            };
            // 地形高度灰度（16×16 网格，31.25m/格）：格中心投影 + 轴对齐方块近似
            let grid = 16u32;
            let cell = size / grid as f32;
            for gi in 0..grid {
                for gj in 0..grid {
                    let gx = -250.0 + (gi as f32 + 0.5) * (500.0 / grid as f32);
                    let gz = -250.0 + (gj as f32 + 0.5) * (500.0 / grid as f32);
                    let (sx, sy) = proj(gx - self.mm_player[0], gz - self.mm_player[1]);
                    if !inside(sx, sy) {
                        continue;
                    }
                    let h = crate::engine::renderer::terrain_height_at(gx, gz);
                    // 高度 -5..35m → 亮度 0.14..0.52（低处深、高处亮）
                    let k = ((h + 5.0) / 40.0).clamp(0.0, 1.0);
                    let lum = 0.14 + 0.38 * k;
                    let gsz = cell + 0.7; // 少量重叠补偿旋转间隙
                    elems.push(HudElement::Quad(Quad::new(
                        Rect::new(cx + sx - gsz * 0.5, cy + sy - gsz * 0.5, gsz, gsz),
                        Color::new(lum, lum * 1.03, lum * 1.08, 0.9),
                    )));
                }
            }
            // 障碍物：按种类配色的小矩形（中心投影 + 轴对齐近似）
            let obs_colors: [[f32; 4]; 6] = [
                [0.72, 0.72, 0.78, 0.95], // 0 墙（灰白）
                [0.55, 0.50, 0.60, 0.95], // 1 大块（紫灰）
                [0.62, 0.45, 0.30, 0.95], // 2 栅栏（棕）
                [0.25, 0.55, 0.25, 0.95], // 3 树（绿）
                [0.45, 0.48, 0.55, 0.95], // 4 建筑（深灰蓝）
                [0.60, 0.45, 0.32, 0.95], // 5 废墟（褐）
            ];
            for &(ox, oz, hw, hd, kind) in &self.mm_obstacles {
                let (sx, sy) = proj(ox - self.mm_player[0], oz - self.mm_player[1]);
                if !inside(sx, sy) {
                    continue;
                }
                let bw = (hw * 2.0 * scale).clamp(3.0, 14.0);
                let bd = (hd * 2.0 * scale).clamp(3.0, 14.0);
                let c = obs_colors[(kind as usize).min(5)];
                elems.push(HudElement::Quad(Quad::new(
                    Rect::new(cx + sx - bw * 0.5, cy + sy - bd * 0.5, bw, bd),
                    Color::new(c[0], c[1], c[2], c[3]),
                )));
            }
            // 单位：红方/蓝方小点
            let unit_colors = [
                [0.98, 0.16, 0.10, 0.92], // 红方
                [0.10, 0.42, 0.98, 0.92], // 蓝方
            ];
            let dot = 3.6;
            for &(ux, uz, team) in &self.mm_units {
                let (sx, sy) = proj(ux - self.mm_player[0], uz - self.mm_player[1]);
                if !inside(sx, sy) {
                    continue;
                }
                let c = unit_colors[(team as usize).min(1)];
                elems.push(HudElement::Quad(Quad::new(
                    Rect::new(cx + sx - dot * 0.5, cy + sy - dot * 0.5, dot, dot),
                    Color::new(c[0], c[1], c[2], c[3]),
                )));
            }
            // 中心玩家十字 + 比例尺
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 9.0, cy - 1.6, 18.0, 3.2),
                Color::CYAN,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 1.6, cy - 9.0, 3.2, 18.0),
                Color::CYAN,
            )));
            elems.push(HudElement::Text {
                text: "500m".to_string(),
                x: mm_x + 6.0,
                y: mm_y + size - 15.0,
                color: Color::new(0.9, 0.95, 1.0, 0.75),
                scale: 0.7,
            });
        }

        // 击杀提示（右上角 feed，战地风格：最新在上，最多 4 条，6 秒消退）
        let feed_x = w - 320.0;
        let mut feed_y = 70.0;
        for e in &self.kill_feed {
            let alpha = (1.0 - (e.age / KILL_FEED_DURATION).clamp(0.0, 1.0)).clamp(0.25, 1.0);
            let color = Color::new(1.0, 0.95, 0.85, alpha);
            elems.push(HudElement::Text {
                text: e.text.clone(),
                x: feed_x - text_width(&e.text, 0.8) * 0.5,
                y: feed_y,
                color,
                scale: 0.8,
            });
            feed_y += 16.0;
        }

        elems
    }

    /// 开始菜单：暗色遮罩 + 标题装饰条 + 操作提示 + 版本行
    fn start_menu_elements(&self) -> Vec<HudElement> {
        let mut elems = Vec::new();
        // 设计基准 1280x800：坐标先用设计空间计算，layout_elements 出口统一乘 ui_scale
        // （不能直接用 screen_w/h —— 那样会双重缩放，血条/准星被推出屏幕）
        let s = self.ui_scale();
        let w = self.screen_w / s;
        let h = self.screen_h / s;
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, 0.0, w, h),
            Color::new(0.0, 0.0, 0.0, 0.72),
        )));
        // 标题上下各一条全宽细条（高约 3px，CYAN 半透明 0.5），作为装饰
        let title_y = h * 0.30;
        let title_h = 7.0 * 4.0; // 5x7 字形 7 行 * scale 4.0
        let bar_h = 3.0;
        let bar_color = Color::new(0.2, 0.8, 0.9, 0.5); // CYAN 半透明
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, title_y - bar_h - 10.0, w, bar_h),
            bar_color,
        )));
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, title_y + title_h + 10.0, w, bar_h),
            bar_color,
        )));
        let title = "STEEL FRONT";
        elems.push(HudElement::Text {
            text: title.to_string(),
            x: w * 0.5 - text_width(title, 4.0) * 0.5,
            y: title_y,
            color: Color::WHITE,
            scale: 4.0,
        });
        let sub = "A RUST + VULKAN FPS TECH DEMO";
        elems.push(HudElement::Text {
            text: sub.to_string(),
            x: w * 0.5 - text_width(sub, 1.2) * 0.5,
            y: h * 0.30 + 44.0,
            color: Color::CYAN,
            scale: 1.2,
        });
        // 标题区域下方的操作提示
        let ops = "WASD MOVE / MOUSE AIM / LMB FIRE / R RELOAD / TAB CAMERA / MENU KEY: SETTINGS";
        elems.push(HudElement::Text {
            text: ops.to_string(),
            x: w * 0.5 - text_width(ops, 1.0) * 0.5,
            y: h * 0.30 + 68.0,
            color: Color::new(0.8, 0.8, 0.8, 1.0),
            scale: 1.0,
        });
        // "PRESS ANY KEY TO START" 闪烁：由 tick(dt) 累加的 elapsed 驱动，
        // 每 0.5 秒在 1.0（亮）与 0.35（暗）之间切换
        let hint = "PRESS ANY KEY TO START";
        let blink_alpha = if (self.elapsed * 2.0) % 2.0 < 1.0 { 1.0 } else { 0.35 };
        elems.push(HudElement::Text {
            text: hint.to_string(),
            x: w * 0.5 - text_width(hint, 2.0) * 0.5,
            y: h * 0.55,
            color: Color::new(0.95, 0.8, 0.2, blink_alpha),
            scale: 2.0,
        });
        let ctrl1 = "WASD MOVE   SPACE / LMB FIRE   TAB CAMERA";
        let ctrl2 = "R / ENTER RESTART (GAME OVER)   ESC QUIT (PRESS AGAIN)";
        let gray = Color::new(0.6, 0.6, 0.6, 1.0);
        elems.push(HudElement::Text {
            text: ctrl1.to_string(),
            x: w * 0.5 - text_width(ctrl1, 1.0) * 0.5,
            y: h * 0.62,
            color: gray,
            scale: 1.0,
        });
        elems.push(HudElement::Text {
            text: ctrl2.to_string(),
            x: w * 0.5 - text_width(ctrl2, 1.0) * 0.5,
            y: h * 0.62 + 16.0,
            color: gray,
            scale: 1.0,
        });
        // 底部版本行（ASCII）
        let version = "V0.3 WAVE-DEFENSE TECH DEMO";
        elems.push(HudElement::Text {
            text: version.to_string(),
            x: w * 0.5 - text_width(version, 1.0) * 0.5,
            y: h * 0.90,
            color: gray,
            scale: 1.0,
        });
        elems
    }

    /// 死亡结算：暗色遮罩 + 分数/波次 + 重开提示
    fn game_over_elements(&self) -> Vec<HudElement> {
        let mut elems = Vec::new();
        // 设计基准 1280x800：坐标先用设计空间计算，layout_elements 出口统一乘 ui_scale
        // （不能直接用 screen_w/h —— 那样会双重缩放，血条/准星被推出屏幕）
        let s = self.ui_scale();
        let w = self.screen_w / s;
        let h = self.screen_h / s;
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, 0.0, w, h),
            Color::new(0.08, 0.08, 0.10, 0.72),
        )));
        let title = "GAME OVER";
        elems.push(HudElement::Text {
            text: title.to_string(),
            x: w * 0.5 - text_width(title, 4.0) * 0.5,
            y: h * 0.30,
            color: Color::RED,
            scale: 4.0,
        });
        let score_txt = format!("SCORE {}", self.score);
        let score_x = w * 0.5 - text_width(&score_txt, 2.0) * 0.5;
        elems.push(HudElement::Text {
            text: score_txt,
            x: score_x,
            y: h * 0.45,
            color: Color::WHITE,
            scale: 2.0,
        });
        let wave_txt = format!("WAVE {}", self.wave);
        let wave_x = w * 0.5 - text_width(&wave_txt, 1.6) * 0.5;
        elems.push(HudElement::Text {
            text: wave_txt,
            x: wave_x,
            y: h * 0.45 + 30.0,
            color: Color::CYAN,
            scale: 1.6,
        });
        let hint = "PRESS R / ENTER TO RESTART";
        elems.push(HudElement::Text {
            text: hint.to_string(),
            x: w * 0.5 - text_width(hint, 1.8) * 0.5,
            y: h * 0.62,
            color: Color::YELLOW,
            scale: 1.8,
        });

        elems
    }

    /// 设置面板：半透明底 + 标题 + 音量/灵敏度条 + 分辨率/画质行 + 键位列表 + 操作提示
    ///
    /// 布局规则（左上角原点，像素坐标）：
    /// - 全屏半透明遮罩
    /// - 顶部中央 `SETTINGS` 标题
    /// - 中部两行：`VOLUME` / `SENSITIVITY` 标签 + 右侧按比例填充的条
    /// - 中部两行：`RESOLUTION` / `QUALITY` 标签 + 右侧当前值文本（Enter 循环切换）
    /// - 中下部键位列表（动作名 + `KeyBindings::label` 键名）
    /// - 底部提示（字体仅 ASCII，意图为：ESC: 返回 / 滚轮: 调整）
    fn settings_elements(&self) -> Vec<HudElement> {
        let mut elems = Vec::new();
        // 设计基准 1280x800：坐标先用设计空间计算，layout_elements 出口统一乘 ui_scale
        // （不能直接用 screen_w/h —— 那样会双重缩放，血条/准星被推出屏幕）
        let s = self.ui_scale();
        let w = self.screen_w / s;
        let h = self.screen_h / s;
        // 半透明底
        elems.push(HudElement::Quad(Quad::new(
            Rect::new(0.0, 0.0, w, h),
            Color::new(0.0, 0.0, 0.0, 0.60),
        )));
        // 标题
        let title = "设置";
        elems.push(HudElement::Text {
            text: title.to_string(),
            x: w * 0.5 - text_width(title, 3.0) * 0.5,
            y: h * 0.14,
            color: Color::WHITE,
            scale: 3.0,
        });
        // 正在等待按键（rebinding 非 None）：显示 PRESS KEY FOR <NAME> (ESC CANCEL) 提示行
        if let Some(action) = self.rebinding {
            let prompt = format!(
                "PRESS KEY FOR {} (ESC CANCEL)",
                KeyBindings::action_name(action)
            );
            let prompt_x = w * 0.5 - text_width(&prompt, 1.2) * 0.5;
            elems.push(HudElement::Text {
                text: prompt,
                x: prompt_x,
                y: h * 0.14 + 38.0,
                color: Color::YELLOW,
                scale: 1.2,
            });
        }
        // 音量 / 灵敏度条
        let bar_w = (w * 0.32).min(320.0);
        let bar_h = 20.0;
        let label_w = 160.0;
        let row_h = 34.0;
        let start_y = h * 0.28;
        let left = w * 0.5 - (label_w + bar_w + 16.0) * 0.5;
        let rows = [
            ("音量", self.volume, Color::CYAN),
            ("灵敏度", self.sensitivity, Color::ORANGE),
            ("音乐", self.music_volume, Color::GREEN),
        ];
        for (i, (name, ratio, color)) in rows.iter().enumerate() {
            let y = start_y + i as f32 * row_h;
            let selected = (i as u8) == self.settings_selection;
            elems.push(HudElement::Text {
                text: if selected {
                    format!("> {}", name)
                } else {
                    name.to_string()
                },
                x: left,
                y: y + (bar_h - 7.0) * 0.5,
                color: if selected { Color::YELLOW } else { Color::WHITE },
                scale: 1.0,
            });
            let back = Quad::new(
                Rect::new(left + label_w, y, bar_w, bar_h),
                Color::new(0.10, 0.10, 0.12, 0.85),
            );
            let fill = Quad::new(
                Rect::new(left + label_w + 3.0, y + 3.0, bar_w - 6.0, bar_h - 6.0),
                *color,
            );
            elems.push(HudElement::Bar {
                back,
                fill,
                ratio: *ratio,
            });
            let pct = format!("{:.0}%", ratio * 100.0);
            elems.push(HudElement::Text {
                text: pct,
                x: left + label_w + bar_w + 10.0,
                y: y + (bar_h - 7.0) * 0.5,
                color: Color::new(0.7, 0.7, 0.7, 1.0),
                scale: 1.0,
            });
        }
        // 分辨率 / 画质行（右侧显示当前值，Enter 循环切换，与键位行同一套高亮交互）
        let display_rows = [
            ("分辨率", RESOLUTION_LABELS[self.resolution_index as usize]),
            ("画质", QUALITY_LABELS[self.quality_index as usize]),
        ];
        for (i, (name, value)) in display_rows.iter().enumerate() {
            let row = 3 + i as u8; // 0=音量 1=灵敏度 2=音乐 3=分辨率 4=画质
            let y = start_y + row as f32 * row_h;
            let selected = row == self.settings_selection;
            elems.push(HudElement::Text {
                text: if selected {
                    format!("> {}", name)
                } else {
                    name.to_string()
                },
                x: left,
                y: y + (bar_h - 7.0) * 0.5,
                color: if selected { Color::YELLOW } else { Color::WHITE },
                scale: 1.0,
            });
            elems.push(HudElement::Text {
                text: value.to_string(),
                x: left + label_w,
                y: y + (bar_h - 7.0) * 0.5,
                color: Color::YELLOW,
                scale: 1.0,
            });
        }
        // 键位列表（顺序 = BindingAction 枚举顺序，与 selected_action() 索引映射一致）
        let keys = [
            ("前进", self.key_bindings.move_forward),
            ("后退", self.key_bindings.move_backward),
            ("左移", self.key_bindings.move_left),
            ("右移", self.key_bindings.move_right),
            ("换弹", self.key_bindings.reload),
            ("开火", self.key_bindings.fire),
            ("跳跃", self.key_bindings.jump),
            ("菜单", self.key_bindings.menu),
        ];
        let key_start_y = start_y + 5.0 * row_h + 24.0;
        for (i, (name, code)) in keys.iter().enumerate() {
            let y = key_start_y + i as f32 * 18.0;
            let selected = (5 + i as u8) == self.settings_selection;
            elems.push(HudElement::Text {
                text: if selected {
                    format!("> {}", name)
                } else {
                    name.to_string()
                },
                x: left + 40.0,
                y,
                color: if selected {
                    Color::YELLOW
                } else {
                    Color::new(0.75, 0.75, 0.75, 1.0)
                },
                scale: 1.0,
            });
            elems.push(HudElement::Text {
                text: KeyBindings::label(*code),
                x: left + label_w + 40.0,
                y,
                color: Color::YELLOW,
                scale: 1.0,
            });
        }
        // 底部提示
        let hint = "ESC 返回  |  TAB 选择  |  滚轮 调整  |  ENTER 切换";
        elems.push(HudElement::Text {
            text: hint.to_string(),
            x: w * 0.5 - text_width(hint, 1.2) * 0.5,
            y: h * 0.88,
            color: Color::new(0.6, 0.6, 0.6, 1.0),
            scale: 1.2,
        });
        elems
    }

    /// 纯布局函数：输出渲染可直接消费的 quad 列表（位置/尺寸/颜色）。
    ///
    /// 等价于 `layout_elements()` 后逐个 `to_quads()` 拍平。
    pub fn layout(&self) -> Vec<Quad> {
        let mut out = Vec::new();
        for element in self.layout_elements() {
            element.to_quads(&mut out);
        }
        out
    }

    /// 输入事件透传：HUD 目前是纯显示层，不消费任何输入。
    ///
    /// 始终返回 `false`（不消费），调用方（主循环）应继续把事件交给游戏输入逻辑。
    pub fn handle_event(&mut self, _event: &UiEvent) -> bool {
        let _ = _event;
        false
    }

    /// 切换设置面板开关
    pub fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
    }

    /// 调整音量（`delta` 为增量，结果 clamp 到 0..=1）
    pub fn adjust_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
    }

    /// 调整音乐音量（`delta` 为增量，结果 clamp 到 0..=1；独立于总音量）
    pub fn adjust_music_volume(&mut self, delta: f32) {
        self.music_volume = (self.music_volume + delta).clamp(0.0, 1.0);
    }

    /// 调整灵敏度（`delta` 为增量，结果 clamp 到 0..=1）
    pub fn adjust_sensitivity(&mut self, delta: f32) {
        self.sensitivity = (self.sensitivity + delta).clamp(0.0, 1.0);
    }

    /// 循环切换分辨率索引（0=1280x720 → 1=1280x800 → 2=1600x900 → 3=1920x1080 → 0）
    pub fn cycle_resolution(&mut self) {
        self.resolution_index = (self.resolution_index + 1) % RESOLUTIONS.len() as u8;
    }

    /// 当前分辨率 (宽, 高)（与 config.rs 持久化的 resolution 对齐）
    pub fn resolution(&self) -> (u32, u32) {
        RESOLUTIONS[self.resolution_index as usize]
    }

    /// 窗口尺寸变化时同步 HUD 布局基准（16:10 等非 16:9 分辨率下保证 HUD 不错位）
    pub fn set_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
    }

    /// 循环切换画质索引（0=LOW → 1=MEDIUM → 2=HIGH → 0，与 QUALITY_LABELS 对齐）
    pub fn cycle_quality(&mut self) {
        self.quality_index = (self.quality_index + 1) % QUALITY_LABELS.len() as u8;
    }

    /// 开始等待重新绑定指定动作（设置面板进入"等待按键"状态）
    ///
    /// 预留：尚未接入 main.rs，由其在设置面板按 ENTER 时接线调用。
    pub fn begin_rebind(&mut self, action: BindingAction) {
        self.rebinding = Some(action);
    }

    /// 当前正在等待重新绑定的动作（无则 None）
    ///
    /// 预留：尚未接入 main.rs，由其在等待按键时读取。
    pub fn rebinding_action(&self) -> Option<BindingAction> {
        self.rebinding
    }

    /// 完成重新绑定：若正在等待按键，则把 `code` 绑定到该动作并返回该动作；否则 None
    ///
    /// 预留：尚未接入 main.rs，由其在收到键码后调用。
    pub fn complete_rebind(&mut self, code: u32) -> Option<BindingAction> {
        let action = self.rebinding?;
        self.key_bindings.bind(action, code);
        self.rebinding = None;
        Some(action)
    }

    /// 取消正在进行的重新绑定（ESC 取消）
    ///
    /// 预留：尚未接入 main.rs，由其在等待按键收到 ESC 时调用。
    pub fn cancel_rebind(&mut self) {
        self.rebinding = None;
    }

    /// 循环切换设置面板选中项（12 项：0=音量 / 1=灵敏度 / 2=音乐 / 3=分辨率 / 4=画质 / 5..=11=7 个键位动作，
    /// 顺序与 `BindingAction` 及设置面板键位行一致）
    pub fn cycle_settings_selection(&mut self) {
        self.settings_selection = (self.settings_selection + 1) % 13;
    }

    /// 当前选中项（0=音量 / 1=灵敏度 / 2=音乐 / 3=分辨率 / 4=画质 / 5..=11=键位动作）
    pub fn settings_selection(&self) -> u8 {
        self.settings_selection
    }

    /// 当前选中项对应的键位动作：settings_selection 在 5..=12 时返回
    /// 第 (selection-5) 个动作（与设置面板键位行顺序一致），否则 None
    ///
    /// 预留：尚未接入 main.rs，由其在设置面板按 ENTER 时决定重绑定哪个动作。
    pub fn selected_action(&self) -> Option<BindingAction> {
        const ACTIONS: [BindingAction; 8] = [
            BindingAction::Forward,
            BindingAction::Backward,
            BindingAction::Left,
            BindingAction::Right,
            BindingAction::Reload,
            BindingAction::Fire,
            BindingAction::Jump,
            BindingAction::Menu,
        ];
        (5..=12)
            .contains(&self.settings_selection)
            .then(|| ACTIONS[(self.settings_selection - 5) as usize])
    }

    /// 显示命中标记（准星外圈闪一下）
    pub fn show_hit_marker(&mut self) {
        self.hit_marker_timer = HIT_MARKER_DURATION;
    }

    /// 每帧推进计时器（`hit_marker_timer` 递减到 0；`elapsed` 累加驱动开始菜单闪烁）
    pub fn tick(&mut self, dt: f32) {
        self.hit_marker_timer = (self.hit_marker_timer - dt).max(0.0);
        self.elapsed += dt;
        // 击杀提示老化：超过时长移除（保留最近 4 条）
        for e in self.kill_feed.iter_mut() {
            e.age += dt;
        }
        self.kill_feed.retain(|e| e.age < KILL_FEED_DURATION);
        while self.kill_feed.len() > 4 {
            self.kill_feed.remove(0);
        }
    }

    /// 追加击杀提示（最新在上：渲染从顶部向下排，超出 4 条挤掉最旧）
    pub fn push_kill(&mut self, text: impl Into<String>) {
        self.kill_feed.insert(0, KillFeedEntry { text: text.into(), age: 0.0 });
        while self.kill_feed.len() > 4 {
            self.kill_feed.pop();
        }
    }
}

/// UI 层输入事件的最小描述（与 winit 类型解耦，保持模块纯 std、可独立单测）。
///
/// 接入时可在主循环把 winit 事件转换为 `UiEvent` 后调用 `HudState::handle_event`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEvent {
    /// 键盘按键（`code` 为物理键码，`pressed` 为按下/抬起）
    Key { code: u32, pressed: bool },
    /// 鼠标按键
    MouseButton { left: bool, right: bool, pressed: bool },
    /// 鼠标移动（窗口坐标）
    CursorMoved { x: f64, y: f64 },
    /// 滚轮（行增量）
    Scroll { dy: f32 },
    /// 窗口焦点变化
    FocusChanged { focused: bool },
}

/// 根据血量比例返回血条颜色：绿 → 黄 → 红
pub fn health_color(ratio: f32) -> Color {
    let ratio = ratio.clamp(0.0, 1.0);
    if ratio > 0.5 {
        Color::GREEN
    } else if ratio > 0.25 {
        Color::YELLOW
    } else {
        Color::RED
    }
}

/// 字符网格宽度（列数）
pub const FONT_COLS: usize = 5;
/// 字符网格高度（行数）
pub const FONT_ROWS: usize = 7;
/// 字符间距（像素，未缩放）
pub const FONT_SPACING: f32 = 1.0;

/// 内置 5x7 位图字体：ASCII 0x20..=0x7E，共 95 个字符。
///
/// 每个字符 5 列字节，每字节位 0..=6 对应行 0..=6（位 0 为顶部像素）。
#[rustfmt::skip]
pub const FONT5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // 32 ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // 33 '!'
    [0x00, 0x07, 0x00, 0x07, 0x00], // 34 '"'
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // 35 '#'
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // 36 '$'
    [0x23, 0x13, 0x08, 0x64, 0x62], // 37 '%'
    [0x36, 0x49, 0x55, 0x22, 0x50], // 38 '&'
    [0x00, 0x05, 0x03, 0x00, 0x00], // 39 '''
    [0x00, 0x1C, 0x22, 0x41, 0x00], // 40 '('
    [0x00, 0x41, 0x22, 0x1C, 0x00], // 41 ')'
    [0x14, 0x08, 0x3E, 0x08, 0x14], // 42 '*'
    [0x08, 0x08, 0x3E, 0x08, 0x08], // 43 '+'
    [0x00, 0x50, 0x30, 0x00, 0x00], // 44 ','
    [0x08, 0x08, 0x08, 0x08, 0x08], // 45 '-'
    [0x00, 0x60, 0x60, 0x00, 0x00], // 46 '.'
    [0x20, 0x10, 0x08, 0x04, 0x02], // 47 '/'
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 48 '0'
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 49 '1'
    [0x42, 0x61, 0x51, 0x49, 0x46], // 50 '2'
    [0x21, 0x41, 0x45, 0x4B, 0x31], // 51 '3'
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 52 '4'
    [0x27, 0x45, 0x45, 0x45, 0x39], // 53 '5'
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 54 '6'
    [0x01, 0x71, 0x09, 0x05, 0x03], // 55 '7'
    [0x36, 0x49, 0x49, 0x49, 0x36], // 56 '8'
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 57 '9'
    [0x00, 0x36, 0x36, 0x00, 0x00], // 58 ':'
    [0x00, 0x56, 0x36, 0x00, 0x00], // 59 ';'
    [0x08, 0x14, 0x22, 0x41, 0x00], // 60 '<'
    [0x14, 0x14, 0x14, 0x14, 0x14], // 61 '='
    [0x00, 0x41, 0x22, 0x14, 0x08], // 62 '>'
    [0x02, 0x01, 0x51, 0x09, 0x06], // 63 '?'
    [0x32, 0x49, 0x79, 0x41, 0x3E], // 64 '@'
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // 65 'A'
    [0x7F, 0x49, 0x49, 0x49, 0x36], // 66 'B'
    [0x3E, 0x41, 0x41, 0x41, 0x22], // 67 'C'
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // 68 'D'
    [0x7F, 0x49, 0x49, 0x49, 0x41], // 69 'E'
    [0x7F, 0x09, 0x09, 0x09, 0x01], // 70 'F'
    [0x3E, 0x41, 0x49, 0x49, 0x7A], // 71 'G'
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // 72 'H'
    [0x00, 0x41, 0x7F, 0x41, 0x00], // 73 'I'
    [0x20, 0x40, 0x41, 0x3F, 0x01], // 74 'J'
    [0x7F, 0x08, 0x14, 0x22, 0x41], // 75 'K'
    [0x7F, 0x40, 0x40, 0x40, 0x40], // 76 'L'
    [0x7F, 0x02, 0x0C, 0x02, 0x7F], // 77 'M'
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // 78 'N'
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // 79 'O'
    [0x7F, 0x09, 0x09, 0x09, 0x06], // 80 'P'
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // 81 'Q'
    [0x7F, 0x09, 0x19, 0x29, 0x46], // 82 'R'
    [0x46, 0x49, 0x49, 0x49, 0x31], // 83 'S'
    [0x01, 0x01, 0x7F, 0x01, 0x01], // 84 'T'
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // 85 'U'
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // 86 'V'
    [0x3F, 0x40, 0x38, 0x40, 0x3F], // 87 'W'
    [0x63, 0x14, 0x08, 0x14, 0x63], // 88 'X'
    [0x07, 0x08, 0x70, 0x08, 0x07], // 89 'Y'
    [0x61, 0x51, 0x49, 0x45, 0x43], // 90 'Z'
    [0x00, 0x7F, 0x41, 0x41, 0x00], // 91 '['
    [0x02, 0x04, 0x08, 0x10, 0x20], // 92 '\\'
    [0x00, 0x41, 0x41, 0x7F, 0x00], // 93 ']'
    [0x04, 0x02, 0x01, 0x02, 0x04], // 94 '^'
    [0x40, 0x40, 0x40, 0x40, 0x40], // 95 '_'
    [0x00, 0x01, 0x02, 0x04, 0x00], // 96 '`'
    [0x20, 0x54, 0x54, 0x54, 0x78], // 97 'a'
    [0x7F, 0x48, 0x44, 0x44, 0x38], // 98 'b'
    [0x38, 0x44, 0x44, 0x44, 0x20], // 99 'c'
    [0x38, 0x44, 0x44, 0x48, 0x7F], // 100 'd'
    [0x38, 0x54, 0x54, 0x54, 0x18], // 101 'e'
    [0x08, 0x7E, 0x09, 0x01, 0x02], // 102 'f'
    [0x0C, 0x52, 0x52, 0x52, 0x3E], // 103 'g'
    [0x7F, 0x08, 0x04, 0x04, 0x78], // 104 'h'
    [0x00, 0x44, 0x7D, 0x40, 0x00], // 105 'i'
    [0x20, 0x40, 0x44, 0x3D, 0x00], // 106 'j'
    [0x7F, 0x10, 0x28, 0x44, 0x00], // 107 'k'
    [0x00, 0x41, 0x7F, 0x40, 0x00], // 108 'l'
    [0x7C, 0x04, 0x18, 0x04, 0x78], // 109 'm'
    [0x7C, 0x08, 0x04, 0x04, 0x78], // 110 'n'
    [0x38, 0x44, 0x44, 0x44, 0x38], // 111 'o'
    [0x7C, 0x14, 0x14, 0x14, 0x08], // 112 'p'
    [0x08, 0x14, 0x14, 0x18, 0x7C], // 113 'q'
    [0x7C, 0x08, 0x04, 0x04, 0x08], // 114 'r'
    [0x48, 0x54, 0x54, 0x54, 0x20], // 115 's'
    [0x04, 0x3F, 0x44, 0x40, 0x20], // 116 't'
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // 117 'u'
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // 118 'v'
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // 119 'w'
    [0x44, 0x28, 0x10, 0x28, 0x44], // 120 'x'
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // 121 'y'
    [0x44, 0x64, 0x54, 0x4C, 0x44], // 122 'z'
    [0x00, 0x08, 0x36, 0x41, 0x00], // 123 '{'
    [0x00, 0x00, 0x7F, 0x00, 0x00], // 124 '|'
    [0x00, 0x41, 0x36, 0x08, 0x00], // 125 '}'
    [0x08, 0x08, 0x2A, 0x1C, 0x08], // 126 '~'
];

/// 查询字符字形；非 ASCII 可打印字符一律回退到 `?`
pub fn glyph(ch: char) -> [u8; 5] {
    let code = ch as u32;
    if (0x20..=0x7E).contains(&code) {
        FONT5X7[(code - 0x20) as usize]
    } else {
        FONT5X7[('?' as u32 - 0x20) as usize]
    }
}

/// 中文字形查询（8x8 点阵，行主序每行 1 字节，bit7=左侧）：
/// 经 engine::font_cjk（Windows GDI 光栅化，零依赖）按需生成并缓存；
/// 非 Windows 或字体缺失返回 None → 渲染回退为 `?`（不 panic、不方块）。
pub fn glyph_cjk(ch: char) -> Option<[u8; 8]> {
    #[cfg(windows)]
    {
        crate::engine::font_cjk::glyph(ch)
    }
    #[cfg(not(windows))]
    {
        let _ = ch;
        None
    }
}

/// 是否中文字符（统一表意/扩展区/全角标点等完整范围，见 font_cjk::is_cjk_char）
pub fn is_cjk(ch: char) -> bool {
    #[cfg(windows)]
    {
        crate::engine::font_cjk::is_cjk_char(ch)
    }
    #[cfg(not(windows))]
    {
        let cp = ch as u32;
        (0x4E00..=0x9FFF).contains(&cp) || (0x3000..=0x303F).contains(&cp)
    }
}

/// 计算字符串的渲染宽度（像素，含字距）。ASCII 每字 FONT_COLS 列，
/// CJK 每字 16 列（16x16 字形），混排按字符分别累加。
pub fn text_width(text: &str, scale: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut w = 0.0f32;
    for ch in text.chars() {
        // CJK：8 格 × 1.12 缩放 + 额外 1 列字距（与 render_text 一致）
        let cols = if is_cjk(ch) { 8.0 * 1.12 + 1.0 } else { FONT_COLS as f32 };
        w += (cols + FONT_SPACING) * scale;
    }
    w - FONT_SPACING * scale
}

/// 把字符串按位图字体展开为小 quad 列表（自绘文本，无外部依赖）。
///
/// ASCII 走内置 5x7 字体；中文（CJK）走 engine::font_cjk 的 8x8 点阵（Windows GDI
/// 生成，位宽 8 行主序）；字符之间留 `FONT_SPACING` 像素间距。
pub fn render_text(text: &str, x: f32, y: f32, color: Color, scale: f32, out: &mut Vec<Quad>) {
    let mut cx = x;
    for ch in text.chars() {
        if is_cjk(ch) {
            // 8x8 预烘焙点阵（Fusion Pixel 8px 手工点阵）。
            // 微调（2026-08-20）：Fusion 8px 字形内容约占 6.5/8 格，视觉略小于英文，
            // 渲染 scale ×1.12 补齐尺寸；y 偏移 -0.5×scale 让内容中线与英文对齐。
            let cs = scale * 1.12;
            let yoff = y - scale * 0.5;
            if let Some(rows) = glyph_cjk(ch) {
                for (row, byte) in rows.iter().enumerate() {
                    for col in 0..8 {
                        if (byte >> (7 - col)) & 1 == 1 {
                            out.push(Quad::new(
                                Rect::new(
                                    cx + col as f32 * cs,
                                    yoff + row as f32 * cs,
                                    cs,
                                    cs,
                                ),
                                color,
                            ));
                        }
                    }
                }
            } else {
                // 字体缺失：回退 '?'（ASCII 路径）
                let cols = glyph('?');
                for (col, byte) in cols.iter().enumerate() {
                    for row in 0..FONT_ROWS {
                        if (byte >> row) & 1 == 1 {
                            out.push(Quad::new(
                                Rect::new(cx + col as f32 * scale, y + row as f32 * scale, scale, scale),
                                color,
                            ));
                        }
                    }
                }
            }
            // 中文字距比英文多 1 逻辑像素：汉字笔画密，同字距会显得粘连
            cx += (8.0 * 1.12 + FONT_SPACING + 1.0) * scale;
        } else {
            let cols = glyph(ch);
            for (col, byte) in cols.iter().enumerate() {
                for row in 0..FONT_ROWS {
                    if (byte >> row) & 1 == 1 {
                        out.push(Quad::new(
                            Rect::new(cx + col as f32 * scale, y + row as f32 * scale, scale, scale),
                            color,
                        ));
                    }
                }
            }
            cx += (FONT_COLS as f32 + FONT_SPACING) * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_bar(elems: &[HudElement]) -> Option<(&Quad, &Quad, f32)> {
        elems.iter().find_map(|e| match e {
            HudElement::Bar { back, fill, ratio } => Some((back, fill, *ratio)),
            _ => None,
        })
    }

    fn find_text<'a>(elems: &'a [HudElement], needle: &str) -> Option<&'a str> {
        elems.iter().find_map(|e| match e {
            HudElement::Text { text, .. } if text.starts_with(needle) => Some(text.as_str()),
            _ => None,
        })
    }

    #[test]
    fn layout_health_bar_ratio() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.health = 75.0;
        hud.max_health = 100.0;
        let elems = hud.layout_elements();
        let (back, fill, ratio) = find_bar(&elems).expect("布局应包含血条 Bar");
        assert!((ratio - 0.75).abs() < 1e-5, "血条比例应为 0.75");
        assert!(
            (fill.rect.w - back.rect.w + 6.0).abs() < 1e-4,
            "填充应内缩 3px 边框"
        );
        assert!(back.rect.x >= 0.0 && back.rect.y + back.rect.h <= hud.screen_h, "血条应在左下角");
        // 拍平后填充宽度 = 内缩宽度 * 比例
        let mut out = Vec::new();
        HudElement::Bar { back: *back, fill: *fill, ratio }
            .to_quads(&mut out);
        assert_eq!(out.len(), 2, "Bar 应展开为背景 + 填充两个 quad");
        assert!((out[1].rect.w - fill.rect.w * ratio).abs() < 1e-4);
    }

    #[test]
    fn layout_health_ratio_clamped() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.health = 200.0;
        assert_eq!(hud.health_ratio(), 1.0, "超上限应 clamp 到 1");
        hud.health = -5.0;
        assert_eq!(hud.health_ratio(), 0.0, "负数应 clamp 到 0");
        hud.max_health = 0.0;
        assert_eq!(hud.health_ratio(), 0.0, "上限为 0 应视为空");
    }

    #[test]
    fn layout_ammo_bar_and_text() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.ammo = 10;
        hud.max_ammo = 40;
        let elems = hud.layout_elements();
        let bars: Vec<f32> = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Bar { ratio, .. } => Some(*ratio),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 2, "应包含血条和弹药条两个 Bar");
        assert!((bars[1] - 0.25).abs() < 1e-5, "弹药比例应为 10/40 = 0.25");
        let ammo_text = find_text(&elems, "M1 Rifle").expect("应有武器名+AMMO 文本");
        assert!(ammo_text.contains("10/40"), "武器弹药文本应含弹药数字");
        assert!(ammo_text.contains("AMMO"), "应含 AMMO 标记");
        assert!(ammo_text.contains("GRENADES"), "应含手榴弹计数");
    }

    #[test]
    fn layout_has_fps_text() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.fps = 142.5;
        let elems = hud.layout_elements();
        let fps_text = find_text(&elems, "FPS").expect("应有 FPS 文本");
        assert!(fps_text.contains("142"), "FPS 文本应显示帧率: {}", fps_text);
    }

    #[test]
    fn layout_minimap_placeholder() {
        let hud = HudState::new(1280.0, 720.0);
        let elems = hud.layout_elements();
        let quad_count = elems
            .iter()
            .filter(|e| matches!(e, HudElement::Quad(_)))
            .count();
        assert!(quad_count >= 5, "小地图占位应含底色+边框+玩家标记，实际 {}", quad_count);
        // 全部 quad 都应落在屏幕内
        for e in &elems {
            if let HudElement::Quad(q) = e {
                assert!(q.rect.x >= 0.0 && q.rect.y >= 0.0, "quad 应在屏幕内");
                assert!(q.rect.x + q.rect.w <= hud.screen_w + 2.0, "quad 不应超出右边界");
                assert!(q.rect.y + q.rect.h <= hud.screen_h + 2.0, "quad 不应超出下边界");
            }
        }

        let mut hidden = hud.clone();
        hidden.minimap_visible = false;
        let elems2 = hidden.layout_elements();
        assert!(
            !elems2.iter().any(|e| matches!(
                e,
                HudElement::Quad(q) if q.rect.x >= hud.screen_w * 0.5 && q.rect.y < hud.screen_h * 0.5
            )),
            "隐藏小地图后右上角不应有占位 quad"
        );
    }

    /// 小地图单位投影：玩家朝 -Z（yaw=0）时，正前方 100m 红点在中心上方 40px，
    /// 正右方 100m 蓝点在中心右方 40px（scale = 200px / 500m = 0.4）
    #[test]
    fn minimap_unit_projection() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.mm_player = [0.0, 0.0];
        hud.mm_yaw = 0.0;
        hud.mm_units = vec![(0.0, -100.0, 0), (100.0, 0.0, 1), (0.0, 100.0, 0)];
        let elems = hud.layout_elements();
        let quads: Vec<Rect> = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Quad(q) => Some(q.rect),
                _ => None,
            })
            .collect();
        let center = |r: &Rect| (r.x + r.w * 0.5, r.y + r.h * 0.5);
        // 前方红点（3.6px 点，中心应在 (1156, 124-40=84)）
        let front = quads
            .iter()
            .find(|r| (center(r).1 - 84.0).abs() < 2.0)
            .expect("应有正前方红点");
        assert!((center(front).0 - 1156.0).abs() < 2.0, "前方点 x 应在中心");
        // 右方蓝点（中心 (1196, 124)）
        let right = quads
            .iter()
            .find(|r| (center(r).0 - 1196.0).abs() < 2.0 && (center(r).1 - 124.0).abs() < 2.0)
            .expect("应有正右方蓝点");
        assert_eq!(right.w, 3.6);
        // 后方红点（中心 (1156, 164)）
        assert!(
            quads
                .iter()
                .any(|r| (center(r).0 - 1156.0).abs() < 2.0 && (center(r).1 - 164.0).abs() < 2.0),
            "应有正后方红点"
        );
    }

    /// 小地图旋转：玩家转向 90°（yaw=π/2）后，世界正前方（-Z）的单位应出现在屏幕右侧
    #[test]
    fn minimap_rotates_with_player_yaw() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.mm_player = [0.0, 0.0];
        hud.mm_yaw = std::f32::consts::FRAC_PI_2;
        hud.mm_units = vec![(0.0, -100.0, 0)];
        let elems = hud.layout_elements();
        let center = |r: &Rect| (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let hit = elems.iter().find_map(|e| match e {
            HudElement::Quad(q) if (q.rect.w - 3.6).abs() < 0.1 => Some(center(&q.rect)),
            _ => None,
        });
        let (x, y) = hit.expect("yaw=90° 后前方单位应仍在图上");
        assert!((x - 1196.0).abs() < 2.0, "转向 90° 后单位应到屏幕右方，x={}", x);
        assert!((y - 124.0).abs() < 2.0, "y 应保持中心行，y={}", y);
    }

    /// 障碍种类全部渲染且投影在图上（按投影位置精确定位，排除地形格/单位点）
    #[test]
    fn minimap_obstacles_render_all_kinds() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.mm_player = [0.0, 0.0];
        hud.mm_yaw = 0.0;
        hud.mm_obstacles = vec![
            (50.0, -50.0, 5.0, 5.0, 0), // 墙
            (-50.0, -50.0, 5.0, 5.0, 1), // 大块
            (50.0, 50.0, 5.0, 5.0, 2),   // 栅栏
            (-50.0, 50.0, 5.0, 5.0, 3),  // 树
            (0.0, -120.0, 6.0, 6.0, 4),  // 建筑
            (120.0, 0.0, 6.0, 6.0, 5),   // 废墟
        ];
        let elems = hud.layout_elements();
        let quads: Vec<Rect> = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Quad(q) => Some(q.rect),
                _ => None,
            })
            .collect();
        let center = |r: &Rect| (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let find_at = |x: f32, y: f32| {
            quads.iter().any(|r| {
                let (cx, cy) = center(r);
                (cx - x).abs() < 2.0 && (cy - y).abs() < 2.0 && r.w <= 6.0 && r.h <= 6.0
            })
        };
        // 投影（scale=0.4）：(50,-50)→(+20,-20) 屏幕 (1176,104)；(-50,-50)→(1136,104)；
        // (50,50)→(1176,144)；(-50,50)→(1136,144)；(0,-120)→(1156,76)；(120,0)→(1204,124)
        for (name, x, y) in [
            ("墙", 1176.0, 104.0),
            ("大块", 1136.0, 104.0),
            ("栅栏", 1176.0, 144.0),
            ("树", 1136.0, 144.0),
            ("建筑", 1156.0, 76.0),
            ("废墟", 1204.0, 124.0),
        ] {
            assert!(find_at(x, y), "{} 障碍应投影到 ({}, {})", name, x, y);
        }
    }

    #[test]
    fn layout_outputs_positive_quads() {
        let hud = HudState::new(1280.0, 720.0);
        let quads = hud.layout();
        assert!(!quads.is_empty(), "layout 应输出 quad 列表");
        for q in &quads {
            assert!(q.rect.w > 0.0 && q.rect.h > 0.0, "所有 quad 应有正尺寸");
        }
    }

    #[test]
    fn bar_element_expands_to_two_quads() {
        let bar = HudElement::Bar {
            back: Quad::new(Rect::new(0.0, 0.0, 100.0, 20.0), Color::BLACK),
            fill: Quad::new(Rect::new(3.0, 3.0, 94.0, 14.0), Color::GREEN),
            ratio: 0.5,
        };
        let mut out = Vec::new();
        bar.to_quads(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], bar_back_quad());
        assert!((out[1].rect.w - 94.0 * 0.5).abs() < 1e-4);
    }

    fn bar_back_quad() -> Quad {
        Quad::new(Rect::new(0.0, 0.0, 100.0, 20.0), Color::BLACK)
    }

    #[test]
    fn render_text_empty_has_no_quads() {
        let mut out = Vec::new();
        render_text("", 0.0, 0.0, Color::WHITE, 1.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn render_text_space_is_blank() {
        let mut out = Vec::new();
        render_text(" ", 0.0, 0.0, Color::WHITE, 1.0, &mut out);
        assert!(out.is_empty(), "空格字形应为全空");
    }

    #[test]
    fn render_text_a_produces_glyph_pixels() {
        let mut out = Vec::new();
        render_text("A", 10.0, 20.0, Color::GREEN, 1.0, &mut out);
        let expected = FONT5X7['A' as usize - 0x20]
            .iter()
            .map(|b| b.count_ones())
            .sum::<u32>();
        assert_eq!(out.len() as u32, expected, "quad 数应等于字形置位像素数");
        for q in &out {
            assert!(q.rect.x >= 10.0 && q.rect.x < 10.0 + FONT_COLS as f32, "像素应落在网格内");
            assert!(q.rect.y >= 20.0 && q.rect.y < 20.0 + FONT_ROWS as f32, "像素应落在网格内");
            assert_eq!(q.color, Color::GREEN, "文本 quad 颜色应透传");
        }
    }

    #[test]
    fn render_text_advances_cursor_per_char() {
        let mut out = Vec::new();
        render_text("!!", 0.0, 0.0, Color::WHITE, 2.0, &mut out);
        // '!' 字形置位 6 像素，两个字符共 12 个 quad
        assert_eq!(out.len(), 12);
        let mut xs: Vec<f32> = out.iter().map(|q| q.rect.x).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let advance = (FONT_COLS as f32 + FONT_SPACING) * 2.0;
        assert!((xs[6] - xs[0] - advance).abs() < 1e-5, "第二个字符应前进一个字符宽度");
    }

    #[test]
    fn text_width_matches_char_advance() {
        assert_eq!(text_width("", 1.0), 0.0);
        assert_eq!(text_width("A", 1.0), 5.0);
        assert_eq!(text_width("AB", 1.0), 11.0);
        assert_eq!(text_width("AB", 2.0), 22.0);
    }

    #[test]
    fn glyph_falls_back_to_question_mark() {
        assert_eq!(glyph('中'), glyph('?'), "非 ASCII 应回退到 '?'");
        assert_eq!(glyph('\u{0}'), glyph('?'));
        assert_eq!(glyph('A')[0], 0x7E, "'A' 首列字形应与字体表一致");
    }

    #[test]
    fn font_table_covers_printable_ascii() {
        assert_eq!(FONT5X7.len(), 95);
        for code in 0x21..=0x7E {
            let lit = FONT5X7[code - 0x20]
                .iter()
                .map(|b| b.count_ones())
                .sum::<u32>();
            assert!(lit > 0, "ASCII {} ('{}') 字形不能为空", code, code as u8 as char);
        }
    }

    #[test]
    fn health_color_thresholds() {
        assert_eq!(health_color(0.9), Color::GREEN);
        assert_eq!(health_color(0.4), Color::YELLOW);
        assert_eq!(health_color(0.1), Color::RED);
        assert_eq!(health_color(-1.0), Color::RED, "负值应 clamp");
        assert_eq!(health_color(2.0), Color::GREEN, "超值应 clamp");
    }

    #[test]
    fn key_bindings_defaults_and_labels() {
        let kb = KeyBindings::defaults();
        assert_eq!(kb.move_forward, 41, "W");
        assert_eq!(kb.move_backward, 37, "S");
        assert_eq!(kb.move_left, 19, "A");
        assert_eq!(kb.move_right, 22, "D");
        assert_eq!(kb.reload, 36, "R");
        assert_eq!(kb.menu, 54, "MENU");
        assert_eq!(kb.fire, 0, "FIRE 无键盘默认（鼠标左键开火）");
        assert_eq!(kb.jump, 62, "JUMP=SPACE");
        // label 映射
        assert_eq!(KeyBindings::label(41), "W");
        assert_eq!(KeyBindings::label(37), "S");
        assert_eq!(KeyBindings::label(19), "A");
        assert_eq!(KeyBindings::label(22), "D");
        assert_eq!(KeyBindings::label(36), "R");
        assert_eq!(KeyBindings::label(114), "ESC");
        assert_eq!(KeyBindings::label(62), "SPACE");
        assert_eq!(KeyBindings::label(6), "1");
        assert_eq!(KeyBindings::label(159), "F1");
        assert_eq!(KeyBindings::label(60), "LSHIFT");
        assert_eq!(KeyBindings::label(999), "KEY#999", "未知键码应回退");
    }

    #[test]
    fn winit_keycode_indices_match_table() {
        // winit 0.30 `KeyCode` 是无显式判别值的枚举（隐式 0,1,2,...），键位表按此序号填写；
        // 若 winit 升级导致序号漂移，此测试立刻失败，防止再出现"W 开设置"类错位。
        use winit::keyboard::KeyCode;
        assert_eq!(KeyCode::KeyA as u32, 19);
        assert_eq!(KeyCode::KeyD as u32, 22);
        assert_eq!(KeyCode::KeyE as u32, 23);
        assert_eq!(KeyCode::KeyN as u32, 32);
        assert_eq!(KeyCode::KeyQ as u32, 35);
        assert_eq!(KeyCode::KeyR as u32, 36);
        assert_eq!(KeyCode::KeyS as u32, 37);
        assert_eq!(KeyCode::KeyW as u32, 41);
        assert_eq!(KeyCode::Space as u32, 62);
        assert_eq!(KeyCode::ContextMenu as u32, 54);
        assert_eq!(KeyCode::Escape as u32, 114);
        assert_eq!(KeyCode::Enter as u32, 57);
        assert_eq!(KeyCode::Tab as u32, 63);
        assert_eq!(KeyCode::F12 as u32, 170);
        assert!(KeyBindings::is_reserved(114), "ESC 应保留");
        assert!(KeyBindings::is_reserved(63), "TAB 应保留");
        assert!(!KeyBindings::is_reserved(41), "W 不应是保留键");
    }

    #[test]
    fn settings_toggle_and_elements() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert!(!hud.settings_open, "初始应关闭设置面板");
        hud.toggle_settings();
        assert!(hud.settings_open, "toggle 后应打开");
        hud.toggle_settings();
        assert!(!hud.settings_open, "再 toggle 应关闭");

        hud.screen = HudScreen::Settings;
        let elems = hud.settings_elements();
        assert!(!elems.is_empty(), "设置面板元素不应为空");
        assert!(
            find_text(&elems, "设置").is_some(),
            "应有 SETTINGS 标题"
        );
        let bars: Vec<f32> = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Bar { ratio, .. } => Some(*ratio),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 3, "应有音量+灵敏度+音乐三个条");
        assert!((bars[0] - 0.8).abs() < 1e-5, "音量条应反映默认音量");
        assert!((bars[1] - 0.5).abs() < 1e-5, "灵敏度条应反映默认灵敏度");
        assert!((bars[2] - 0.6).abs() < 1e-5, "音乐条应反映默认音乐音量");
        // 键位列表含默认键名
        let texts: String = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(texts.contains("W"), "键位列表应含 FORWARD 键名 W: {}", texts);
        assert!(
            texts.contains("SPACE"),
            "键位列表应含 JUMP 键名 SPACE（2026-08-15 起 Space=跳跃，开火=鼠标左键）: {}",
            texts
        );
        // layout_elements 在 Settings 画面应走 settings_elements
        let via_layout = hud.layout_elements();
        assert!(
            find_text(&via_layout, "设置").is_some(),
            "Settings 画面应输出设置面板"
        );
    }

    #[test]
    fn adjust_volume_sensitivity_clamps() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert!((hud.volume - 0.8).abs() < 1e-5);
        assert!((hud.sensitivity - 0.5).abs() < 1e-5);
        hud.adjust_volume(0.3);
        assert!((hud.volume - 1.0).abs() < 1e-5, "音量上限 1.0");
        hud.adjust_volume(-2.0);
        assert!((hud.volume - 0.0).abs() < 1e-5, "音量下限 0.0");
        hud.adjust_sensitivity(1.0);
        assert!((hud.sensitivity - 1.0).abs() < 1e-5, "灵敏度上限 1.0");
        hud.adjust_sensitivity(-5.0);
        assert!((hud.sensitivity - 0.0).abs() < 1e-5, "灵敏度下限 0.0");
        hud.adjust_sensitivity(0.2);
        assert!((hud.sensitivity - 0.2).abs() < 1e-5, "正常增量应生效");
    }

    #[test]
    fn hit_marker_shows_and_decays() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.hit_marker_timer, 0.0);
        let base = hud.layout_elements().len();
        hud.show_hit_marker();
        assert!(hud.hit_marker_timer > 0.0, "show_hit_marker 应启动计时");
        let with_marker = hud.layout_elements().len();
        assert!(
            with_marker > base,
            "命中标记应增加元素（{} -> {}）",
            base,
            with_marker
        );
        hud.tick(0.05);
        assert!(hud.hit_marker_timer > 0.0, "部分衰减后仍应 > 0");
        hud.tick(10.0);
        assert_eq!(hud.hit_marker_timer, 0.0, "tick 后应衰减到 0");
        assert_eq!(
            hud.layout_elements().len(),
            base,
            "计时归零后应恢复原元素数"
        );
    }

    #[test]
    fn reloading_indicator_shown() {
        let mut hud = HudState::new(1280.0, 720.0);
        let base = hud.layout_elements();
        assert!(
            find_text(&base, "RELOADING").is_none(),
            "非换弹时不应有 RELOADING 文本"
        );
        hud.reloading = true;
        hud.reload_progress = 0.5;
        let elems = hud.layout_elements();
        assert!(
            find_text(&elems, "RELOADING").is_some(),
            "换弹时应显示 RELOADING 文本"
        );
        let bars: Vec<f32> = elems
            .iter()
            .filter_map(|e| match e {
                HudElement::Bar { ratio, .. } => Some(*ratio),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 3, "换弹时应有 血条+弹药条+换弹进度 三个 Bar");
        assert!((bars[2] - 0.5).abs() < 1e-5, "换弹进度比例应透传");
    }

    #[test]
    fn binding_action_codes_names_and_lookup() {
        assert_eq!(KeyBindings::default_code(BindingAction::Forward), 41);
        assert_eq!(KeyBindings::default_code(BindingAction::Backward), 37);
        assert_eq!(KeyBindings::default_code(BindingAction::Left), 19);
        assert_eq!(KeyBindings::default_code(BindingAction::Right), 22);
        assert_eq!(KeyBindings::default_code(BindingAction::Reload), 36);
        assert_eq!(KeyBindings::default_code(BindingAction::Menu), 54);
        assert_eq!(KeyBindings::default_code(BindingAction::Fire), 0, "FIRE 无键盘默认");
        assert_eq!(KeyBindings::default_code(BindingAction::Jump), 62, "JUMP=SPACE");
        assert_eq!(KeyBindings::action_name(BindingAction::Forward), "FORWARD");
        assert_eq!(KeyBindings::action_name(BindingAction::Backward), "BACKWARD");
        assert_eq!(KeyBindings::action_name(BindingAction::Left), "LEFT");
        assert_eq!(KeyBindings::action_name(BindingAction::Right), "RIGHT");
        assert_eq!(KeyBindings::action_name(BindingAction::Reload), "RELOAD");
        assert_eq!(KeyBindings::action_name(BindingAction::Fire), "FIRE");
        assert_eq!(KeyBindings::action_name(BindingAction::Menu), "MENU");
        let kb = KeyBindings::defaults();
        assert_eq!(kb.code_for(BindingAction::Forward), 41);
        assert_eq!(kb.code_for(BindingAction::Menu), 54);
        assert_eq!(kb.action_for(41), Some(BindingAction::Forward));
        assert_eq!(kb.action_for(37), Some(BindingAction::Backward));
        assert_eq!(kb.action_for(62), Some(BindingAction::Jump));
        assert_eq!(kb.action_for(54), Some(BindingAction::Menu));
        assert_eq!(kb.action_for(999), None, "未绑定键码应返回 None");
    }

    #[test]
    fn bind_is_mutually_exclusive_with_default_reset() {
        let mut kb = KeyBindings::defaults();
        // 把 BACKWARD 绑到 W(41)（当前 FORWARD 的键码）→ FORWARD 应复位回默认 W
        kb.bind(BindingAction::Backward, 41);
        assert_eq!(kb.code_for(BindingAction::Backward), 41, "BACKWARD 应占用 W");
        assert_eq!(kb.code_for(BindingAction::Forward), 41, "FORWARD 应复位回默认 41");
        assert_eq!(
            kb.action_for(41),
            Some(BindingAction::Forward),
            "冲突键码应按 Forward→Menu 顺序归属默认动作"
        );
        // 把 FORWARD 绑到 SPACE(62)（当前 JUMP 的键码）→ JUMP 应复位回默认 SPACE
        kb.bind(BindingAction::Forward, 62);
        assert_eq!(kb.code_for(BindingAction::Forward), 62);
        assert_eq!(kb.code_for(BindingAction::Jump), 62, "JUMP 应复位回默认 62");
        // 未冲突动作不受影响
        assert_eq!(kb.code_for(BindingAction::Right), 22, "RIGHT 应保持 D");
        assert_eq!(kb.code_for(BindingAction::Reload), 36, "RELOAD 应保持 R");
    }

    #[test]
    fn rebind_begin_complete_cancel_flow() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.rebinding, None);
        assert_eq!(hud.rebinding_action(), None);
        assert_eq!(hud.complete_rebind(30), None, "无 rebinding 时完成应返回 None");
        // begin → complete
        hud.begin_rebind(BindingAction::Fire);
        assert_eq!(hud.rebinding_action(), Some(BindingAction::Fire));
        assert_eq!(hud.complete_rebind(30), Some(BindingAction::Fire), "完成应返回动作");
        assert_eq!(hud.rebinding, None, "完成后应清除 rebinding");
        assert_eq!(hud.key_bindings.code_for(BindingAction::Fire), 30, "FIRE 应改为 1");
        // begin → cancel
        hud.begin_rebind(BindingAction::Menu);
        hud.cancel_rebind();
        assert_eq!(hud.rebinding, None, "取消后应清除 rebinding");
        assert_eq!(hud.key_bindings.code_for(BindingAction::Menu), 54, "取消不应改键");
        // 再次 begin 且 complete 时冲突键复位
        hud.begin_rebind(BindingAction::Forward);
        assert_eq!(hud.complete_rebind(54), Some(BindingAction::Forward));
        assert_eq!(hud.key_bindings.code_for(BindingAction::Forward), 54);
        assert_eq!(
            hud.key_bindings.code_for(BindingAction::Menu),
            54,
            "MENU 应复位回默认 ContextMenu"
        );
    }

    #[test]
    fn start_menu_hint_blinks_by_elapsed() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Start;
        let hint_alpha = |hud: &HudState| {
            hud.layout_elements()
                .iter()
                .find_map(|e| match e {
                    HudElement::Text { text, color, .. }
                        if text == "PRESS ANY KEY TO START" =>
                    {
                        Some(color.a)
                    }
                    _ => None,
                })
                .expect("开始菜单应有提示行")
        };
        hud.elapsed = 0.0; // (0.0*2)%2.0 = 0.0 < 1.0 → 亮
        let bright = hint_alpha(&hud);
        hud.elapsed = 0.6; // (0.6*2)%2.0 = 1.2 >= 1.0 → 暗
        let dim = hint_alpha(&hud);
        assert!((bright - 1.0).abs() < 1e-5, "前半周期应全亮，实际 {}", bright);
        assert!((dim - 0.35).abs() < 1e-5, "后半周期应 0.35，实际 {}", dim);
        assert!((bright - dim).abs() > 0.5, "两个时刻 alpha 应不同");
        // tick(dt) 驱动 elapsed 累加（闪烁时钟来源）
        hud.elapsed = 0.0;
        hud.tick(0.5);
        assert!((hud.elapsed - 0.5).abs() < 1e-5, "tick 应累加 elapsed");
    }

    #[test]
    fn selected_action_maps_settings_selection() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.selected_action(), None, "0=音量，非键位动作");
        hud.settings_selection = 1;
        assert_eq!(hud.selected_action(), None, "1=灵敏度，非键位动作");
        hud.settings_selection = 2;
        assert_eq!(hud.selected_action(), None, "2=音乐，非键位动作");
        hud.settings_selection = 3;
        assert_eq!(hud.selected_action(), None, "3=分辨率，非键位动作");
        hud.settings_selection = 4;
        assert_eq!(hud.selected_action(), None, "4=画质，非键位动作");
        let expected = [
            BindingAction::Forward,
            BindingAction::Backward,
            BindingAction::Left,
            BindingAction::Right,
            BindingAction::Reload,
            BindingAction::Fire,
            BindingAction::Jump,
            BindingAction::Menu,
        ];
        for (i, action) in expected.iter().enumerate() {
            hud.settings_selection = 5 + i as u8;
            assert_eq!(
                hud.selected_action(),
                Some(*action),
                "选中 5+{} 应对应 {:?}",
                i,
                action
            );
        }
        hud.settings_selection = 13;
        assert_eq!(hud.selected_action(), None, "越界应返回 None");
    }

    #[test]
    fn cycle_settings_selection_wraps_13_rows() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.settings_selection(), 0);
        for expected in [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0] {
            hud.cycle_settings_selection();
            assert_eq!(hud.settings_selection(), expected);
        }
    }

    #[test]
    fn level_defaults_to_one() {
        let hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.level, 1, "默认关卡应为 1");
    }

    #[test]
    fn game_hud_shows_level() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.level = 3;
        let elems = hud.layout_elements();
        let level_text = find_text(&elems, "LEVEL").expect("游戏 HUD 应显示 LEVEL 行");
        assert!(level_text.contains("3"), "LEVEL 文本应含关卡: {}", level_text);
    }

    #[test]
    fn settings_key_rows_selectable() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Settings;
        hud.settings_selection = 7; // 5+2 → LEFT(左移)
        let elems = hud.settings_elements();
        assert!(find_text(&elems, "> 左移").is_some(), "选中键位行应有 '> ' 前缀");
        assert!(
            find_text(&elems, "> 前进").is_none(),
            "未选中键位行不应有前缀"
        );
        let selected_color = elems.iter().find_map(|e| match e {
            HudElement::Text { text, color, .. } if text == "> 左移" => Some(*color),
            _ => None,
        });
        assert_eq!(selected_color, Some(Color::YELLOW), "选中行应高亮为 YELLOW");
    }

    #[test]
    fn settings_shows_rebind_prompt() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Settings;
        hud.begin_rebind(BindingAction::Fire);
        let elems = hud.settings_elements();
        assert!(
            find_text(&elems, "PRESS KEY FOR FIRE (ESC CANCEL)").is_some(),
            "等待按键时应显示 PRESS KEY FOR FIRE (ESC CANCEL)"
        );
        // 无 rebinding 时不应显示按键提示
        hud.cancel_rebind();
        let elems = hud.settings_elements();
        assert!(
            find_text(&elems, "PRESS KEY FOR").is_none(),
            "无 rebinding 时不应显示按键提示"
        );
    }

    #[test]
    fn settings_hint_lists_rebind_controls() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Settings;
        let elems = hud.settings_elements();
        assert!(
            find_text(&elems, "ESC 返回").is_some(),
            "底部提示应说明调整入口"
        );
    }

    #[test]
    fn settings_defaults_resolution_and_quality() {
        let hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.resolution_index, 0, "默认分辨率索引应为 0");
        assert_eq!(hud.resolution(), (1280, 720), "默认分辨率应为 1280x720");
        assert_eq!(hud.quality_index, 1, "默认画质应为 MEDIUM");
    }

    #[test]
    fn resolution_cycles_five_options() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.resolution(), (1280, 720));
        hud.cycle_resolution();
        assert_eq!(hud.resolution(), (1280, 800), "1280x720 → 1280x800（16:10 档）");
        hud.cycle_resolution();
        assert_eq!(hud.resolution(), (1600, 900), "1280x800 → 1600x900");
        hud.cycle_resolution();
        assert_eq!(hud.resolution(), (1920, 1080), "1600x900 → 1920x1080");
        hud.cycle_resolution();
        assert_eq!(hud.resolution(), (2560, 1600), "1920x1080 → 2560x1600（2.5K 原生）");
        hud.cycle_resolution();
        assert_eq!(hud.resolution(), (1280, 720), "循环应回到首项");
    }

    #[test]
    fn quality_cycles_three_options() {
        let mut hud = HudState::new(1280.0, 720.0);
        assert_eq!(hud.quality_index, 1, "默认 MEDIUM");
        hud.cycle_quality();
        assert_eq!(hud.quality_index, 2, "MEDIUM → HIGH");
        hud.cycle_quality();
        assert_eq!(hud.quality_index, 0, "HIGH → LOW");
        hud.cycle_quality();
        assert_eq!(hud.quality_index, 1, "LOW → MEDIUM");
    }

    #[test]
    fn settings_shows_resolution_and_quality_rows() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Settings;
        hud.settings_selection = 3; // RESOLUTION 行（0=音量 1=灵敏度 2=音乐 3=分辨率 4=画质）
        let elems = hud.settings_elements();
        assert!(
            find_text(&elems, "> 分辨率").is_some(),
            "选中分辨率行应有 '> ' 前缀"
        );
        assert!(
            find_text(&elems, "1280x720").is_some(),
            "分辨率行应显示当前值"
        );
        assert!(
            find_text(&elems, "MEDIUM").is_some(),
            "画质行应显示当前值 MEDIUM"
        );
        // 切到 QUALITY 行：高亮跟随，分辨率行前缀消失
        hud.settings_selection = 4;
        let elems = hud.settings_elements();
        assert!(
            find_text(&elems, "> 画质").is_some(),
            "选中画质行应有 '> ' 前缀"
        );
        assert!(
            find_text(&elems, "> 分辨率").is_none(),
            "未选中分辨率行不应有前缀"
        );
    }

    #[test]
    fn start_menu_has_controls_hint() {
        let mut hud = HudState::new(1280.0, 720.0);
        hud.screen = HudScreen::Start;
        let elems = hud.layout_elements();
        let ops = "WASD MOVE / MOUSE AIM / LMB FIRE / R RELOAD / TAB CAMERA / MENU KEY: SETTINGS";
        assert!(
            elems.iter().any(|e| matches!(
                e,
                HudElement::Text { text, .. } if text.as_str() == ops
            )),
            "开始菜单标题下方应有操作提示行"
        );
        assert!(
            find_text(&elems, "STEEL FRONT").is_some(),
            "标题文本结构应保留"
        );
    }

    #[test]
    fn handle_event_never_consumes_input() {
        let mut hud = HudState::new(1280.0, 720.0);
        let events = [
            UiEvent::Key { code: 30, pressed: true },
            UiEvent::Key { code: 30, pressed: false },
            UiEvent::MouseButton { left: true, right: false, pressed: true },
            UiEvent::CursorMoved { x: 100.0, y: 200.0 },
            UiEvent::Scroll { dy: -1.0 },
            UiEvent::FocusChanged { focused: false },
        ];
        for e in &events {
            assert!(!hud.handle_event(e), "UI 层不得消费输入: {:?}", e);
        }
    }
}
