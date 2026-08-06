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
    /// 最近一帧 FPS（用于 FPS 文本显示）
    pub fps: f32,
    /// 小地图占位是否显示
    pub minimap_visible: bool,
}

/// 血条文字缩放
const TEXT_SCALE: f32 = 1.4;

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
            fps: 0.0,
            minimap_visible: true,
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

    /// 纯布局函数：把 HUD 状态展开为元素列表。
    ///
    /// 布局规则（左上角原点，像素坐标）：
    /// - 左下角：血条（宽度约屏幕 30%，上限 360px）+ 文字 `HP x/y`
    /// - 血条右侧：弹药条 + 文字 `AMMO x/y`
    /// - 左上角：FPS 文本
    /// - 右上角：小地图占位（半透明底 + 边框 + 中心玩家十字标记）
    pub fn layout_elements(&self) -> Vec<HudElement> {
        let mut elems = Vec::new();
        let w = self.screen_w;
        let h = self.screen_h;
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
            text: format!("AMMO {}/{}", self.ammo, self.max_ammo),
            x: ammo_x + 6.0,
            y: h - margin - bar_h + (bar_h - 7.0 * TEXT_SCALE) * 0.5,
            color: Color::WHITE,
            scale: TEXT_SCALE,
        });

        // ---- FPS（左上角）----
        elems.push(HudElement::Text {
            text: format!("FPS {:.0}", self.fps),
            x: margin,
            y: margin,
            color: Color::CYAN,
            scale: 2.0,
        });

        // ---- 小地图占位（右上角）----
        if self.minimap_visible {
            let size = 180.0;
            let mm_x = w - margin - size;
            let mm_y = margin;
            let border = 2.0;
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y - border, size + border * 2.0, border),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y + size, size + border * 2.0, border),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x - border, mm_y, border, size),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x + size, mm_y, border, size),
                Color::WHITE,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(mm_x, mm_y, size, size),
                Color::new(0.12, 0.16, 0.18, 0.65),
            )));
            // 中心玩家十字标记（占位）
            let cx = mm_x + size * 0.5;
            let cy = mm_y + size * 0.5;
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 10.0, cy - 2.0, 20.0, 4.0),
                Color::CYAN,
            )));
            elems.push(HudElement::Quad(Quad::new(
                Rect::new(cx - 2.0, cy - 10.0, 4.0, 20.0),
                Color::CYAN,
            )));
        }

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

/// 计算字符串的渲染宽度（像素，含字距）
pub fn text_width(text: &str, scale: f32) -> f32 {
    let chars = text.chars().count();
    if chars == 0 {
        0.0
    } else {
        chars as f32 * (FONT_COLS as f32 + FONT_SPACING) * scale - FONT_SPACING * scale
    }
}

/// 把字符串按 5x7 位图字体展开为小 quad 列表（自绘文本，无外部依赖）。
///
/// 每个置位像素生成一个 `scale x scale` 的 quad；字符之间留 `FONT_SPACING` 像素间距。
pub fn render_text(text: &str, x: f32, y: f32, color: Color, scale: f32, out: &mut Vec<Quad>) {
    let mut cx = x;
    for ch in text.chars() {
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
        let ammo_text = find_text(&elems, "AMMO").expect("应有 AMMO 文本");
        assert!(ammo_text.contains("10/40"), "AMMO 文本应含弹药数字");
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
            !elems2.iter().any(|e| matches!(e, HudElement::Quad(_))),
            "隐藏小地图后不应有占位 quad"
        );
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
