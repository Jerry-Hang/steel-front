//! 编辑器壳第一步：场景树面板（引擎化 P3b，见 `docs/ENGINE-PLAN.md`）。
//!
//! 用**仓库自己的** immediate-mode quad + 点阵文本管线（`ui.rs`），不引入 egui ——
//! 不新增第三方依赖是硬红线，而 HUD 这条路径已经能画框、画字、支持缩放，够做第一版面板。
//!
//! 这一版只做"能看见场景里有什么、哪一个是选中的"：
//! - 右侧栏列出所有实体（id / 上下级 id / 世界坐标）；
//! - 选中项整行高亮，底部显示选中句柄；
//! - 行数超出面板高度时显示剩余条数（**不滚动**，滚动等鼠标输入接线之后再加）。
//!
//! 刻意不做的事：不做拖拽重排、不做多选、不做右键菜单 —— 那些要先把鼠标命中接线，
//! 而这一版的目标是"编辑器态能看清场景"，看得见才说得上能改。

use crate::engine::scene::{EntityId, Scene};
use crate::ui::{Color, Quad, Rect};

/// 面板宽度（设计像素，乘 `ui_scale` 后生效）。
const PANEL_W: f32 = 320.0;
/// 面板内边距。
const PAD: f32 = 10.0;
/// 行高。
const ROW_H: f32 = 18.0;
/// 每行最多显示多少个实体（超出显示剩余条数）。
const MAX_ROWS: usize = 16;

/// 面板配色（与 HUD 的深色半透明一致，选中用青色靠拢本仓 HUD 强调色）。
const BG: Color = Color::new(0.05, 0.06, 0.09, 0.82);
const ROW_SEL: Color = Color::new(0.10, 0.30, 0.38, 0.95);
const HEAD: Color = Color::new(0.55, 0.85, 1.0, 1.0);
const BODY: Color = Color::new(0.78, 0.80, 0.84, 1.0);
const DIM: Color = Color::new(0.55, 0.57, 0.62, 1.0);

/// 生成场景树面板的 quad。返回的列表直接 append 到 HUD quad 里。
///
/// `ui_scale` 与 HUD 同源（`HudState::ui_scale()`），保证 720p/1600p 下比例一致。
pub fn panel_quads(
    scene: &Scene,
    selection: Option<EntityId>,
    screen_w: f32,
    screen_h: f32,
    ui_scale: f32,
) -> Vec<Quad> {
    let mut q = Vec::new();
    let s = ui_scale;
    let w = PANEL_W * s;
    let x = (screen_w - w).max(0.0);
    let h = screen_h;
    q.push(Quad::new(Rect::new(x, 0.0, w, h), BG));
    // 顶部标题
    let mut y = PAD * s;
    let text_scale = 2.0 * s;
    crate::ui::render_text("场景树", x + PAD * s, y, HEAD, text_scale, &mut q);
    y += ROW_H * s + 2.0 * s;
    crate::ui::render_text(
        &format!("实体 {}", scene.len()),
        x + PAD * s,
        y,
        DIM,
        text_scale,
        &mut q,
    );
    // 分隔线（用一条很扁的 quad）
    q.push(Quad::new(
        Rect::new(x + PAD * s, y + ROW_H * s + 4.0 * s, w - PAD * 2.0 * s, 1.0 * s),
        DIM,
    ));
    // 🔴 行起点来自 `tree_rows_top`（与命中判定同一份加法）
    y = tree_rows_top(s);

    let total = scene.len();
    for (shown, (id, local)) in scene.iter_alive().enumerate() {
        if shown >= MAX_ROWS {
            break;
        }
        let is_sel = selection == Some(id);
        if is_sel {
            q.push(Quad::new(
                Rect::new(x + 2.0 * s, y - 3.0 * s, w - 4.0 * s, ROW_H * s),
                ROW_SEL,
            ));
        }
        let t = local.translation;
        // 行首 id，接着世界坐标（世界坐标由调用方保证是"能读"的两位小数）
        let line = format!(
            "#{} {:.1} {:.1} {:.1}",
            scene.index_of(id),
            t.x,
            t.y,
            t.z
        );
        crate::ui::render_text(
            &line,
            x + PAD * s,
            y,
            if is_sel { Color::WHITE } else { BODY },
            text_scale,
            &mut q,
        );
        y += ROW_H * s;
    }
    if total > MAX_ROWS {
        crate::ui::render_text(
            &format!("还有 {} 个", total - MAX_ROWS),
            x + PAD * s,
            y,
            DIM,
            text_scale,
            &mut q,
        );
    }
    // 底部：检视块（选中实体的坐标 / 层级 / 可见性）
    //
    // 位置从"距底 4 行"起向上排：检视块的行数固定为 4（见 `inspector_lines`），
    // 所以这里可以写死偏移，不需要先量高度。
    let lines = inspector_lines(scene, selection);
    let base_y = h - ROW_H * 1.5 * s - (lines.len() as f32 - 1.0) * ROW_H * s;
    for (i, line) in lines.iter().enumerate() {
        crate::ui::render_text(
            line,
            x + PAD * s,
            base_y + i as f32 * ROW_H * s,
            if i == 0 { HEAD } else { BODY },
            text_scale,
            &mut q,
        );
    }
    q
}



/// 场景树里**第一行**的从上往下的起点（标题 + 实体数 + 分隔线之后）。
///
/// 与 `panel_quads` 里的排版必须一致 —— 所以两边都调这个函数，而不是各写一遍加法。
pub fn tree_rows_top(ui_scale: f32) -> f32 {
    let s = ui_scale;
    // 标题 1 行 + 实体数 1 行 + 分隔线
    PAD * s + ROW_H * s + 2.0 * s + ROW_H * s + 4.0 * s + 6.0 * s
}

/// 点在场景树上命中**第几个实体**（按 `iter_alive` 的顺序；超出 `MAX_ROWS` 或点在面板外返回 `None`）。
///
/// 用途与资产面板同一个道理：布局与命中必须共用一份，否则"点 A 选中 B"。
pub fn tree_row_at(
    screen_w: f32,
    screen_h: f32,
    rows: usize,
    ui_scale: f32,
    px: f32,
    py: f32,
) -> Option<usize> {
    let s = ui_scale;
    let w = PANEL_W * s;
    let x = (screen_w - w).max(0.0);
    if px < x || px > x + w || py < 0.0 || py > screen_h {
        return None;
    }
    let top = tree_rows_top(s);
    if py < top {
        return None;
    }
    let row = ((py - top) / (ROW_H * s)).floor();
    if row < 0.0 {
        return None;
    }
    let row = row as usize;
    if row >= rows.min(MAX_ROWS) {
        return None;
    }
    Some(row)
}

/// 资产面板的矩形（纯函数：绘制与命中判定**共用这一份布局**）。
///
/// 分开写的后果很具体：改了绘制里的一个偏移，命中判定还按旧偏移算 ⇒
/// "点上去没反应"或"点 A 选中 B"，而且看起来像鼠标事件没接上。
pub fn asset_panel_rect(screen_w: f32, screen_h: f32, line_count: usize, ui_scale: f32) -> Rect {
    let s = ui_scale;
    let w = PANEL_W * s;
    let x = (screen_w - w * 2.0 - PAD * s).max(0.0);
    let h = line_count as f32 * ROW_H * s + PAD * 2.0 * s;
    let y0 = (screen_h - h - PAD * s).max(0.0);
    Rect::new(x, y0, w, h)
}

/// 点击落在资产面板的**第几条资产**上（0 = 首条；点表头或面板外返回 `None`）。
///
/// `line_count` 是 `panel_lines()` 的长度（含表头）；第 0 行是表头 ⇒ 不算资产。
pub fn asset_row_at(
    screen_w: f32,
    screen_h: f32,
    line_count: usize,
    ui_scale: f32,
    px: f32,
    py: f32,
) -> Option<usize> {
    if line_count < 2 {
        // 只有表头 = 没有资产可点
        return None;
    }
    let r = asset_panel_rect(screen_w, screen_h, line_count, ui_scale);
    if px < r.x || px > r.x + r.w || py < r.y || py > r.y + r.h {
        return None;
    }
    let s = ui_scale;
    let row = ((py - r.y - PAD * s) / (ROW_H * s)).floor();
    if row < 0.0 {
        return None;
    }
    let row = row as usize;
    // 第 0 行是表头
    if row == 0 || row >= line_count {
        return None;
    }
    Some(row - 1)
}

/// 把资产列表的文字画到面板**左侧下部**（与右侧场景树分开，两块互不遮挡）。
///
/// 传 `lines` 而不是 `&AssetBrowser`：绘制层不该知道列表内部结构，
/// 这样判据测 `panel_lines`、绘制只负责摆位置，两边不会互相绑死。
pub fn append_asset_lines(
    lines: Vec<String>,
    screen_w: f32,
    screen_h: f32,
    ui_scale: f32,
    q: &mut Vec<Quad>,
) {
    if lines.is_empty() {
        return;
    }
    let s = ui_scale;
    let text_scale = 2.0 * s;
    // 🔴 布局来自 `asset_panel_rect`（与命中判定同一份）
    let r = asset_panel_rect(screen_w, screen_h, lines.len(), ui_scale);
    let (x, y0) = (r.x, r.y);
    // 半透明底：压在场景上要能读清字
    q.push(Quad::new(r, BG));
    for (i, line) in lines.iter().enumerate() {
        crate::ui::render_text(
            line,
            x + PAD * s,
            y0 + PAD * s + i as f32 * ROW_H * s,
            if i == 0 { HEAD } else { BODY },
            text_scale,
            q,
        );
    }
}

/// 像素坐标 → NDC。**winit 的 Y 向下，NDC 的 Y 向上**，所以 y 要翻。
///
/// 这条单独抽出来是因为它是最容易写反的一步，而写反之后"上下颠倒着拖"看起来像
/// gizmo 选轴错了 —— 两者很难区分，所以给它一条自己的判据。
pub fn ndc_from_pixel(x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (0.0, 0.0);
    }
    (2.0 * x / w - 1.0, 1.0 - 2.0 * y / h)
}

/// 编辑器正在进行的拖拽（左键按住某个把手）。
///
/// 存的是**起拖时刻**的 `TranslateDrag`（起点参数 + 起点平移）与目标实体；
/// 每次鼠标移动都用它从头算新位置，而不是累加增量 —— 累加会把每帧的数值偏差累积起来。
#[derive(Debug, Clone, Copy)]
pub struct EditorDrag {
    pub entity: crate::engine::scene::EntityId,
    /// 这一拖在做的事。**两种模式共用同一个字段名**，因为它们在编辑器里是同一个手势，
    /// 只是 `gizmo_mode` 不同。
    pub kind: DragKind,
}

/// 一次拖动的具体形态。
#[derive(Debug, Clone, Copy)]
pub enum DragKind {
    /// 沿轴平移（起点参数 + 起点平移）。
    Translate(crate::engine::gizmo::TranslateDrag),
    /// 绕 Y 旋转：记住起点光标 NDC 与起点四元数，之后从起点重算（与平移同一纪律）。
    RotateY {
        start_ndc: (f32, f32),
        start_rotation: glam::Quat,
    },
    /// 沿轴等比缩放：记住起点缩放与"这次拖的是哪根轴"，之后从起点重算。
    Scale {
        drag: crate::engine::gizmo::TranslateDrag,
        start_scale: glam::Vec3,
    },
}

/// 检视面板要显示的行（**纯函数**，与绘制分离）。
///
/// 分成"取数据"和"画"两件事，是因为"面板上显示的数字对不对"只能在这一层判：
/// quad 画出来之后就没法断言内容了（测试里没法读像素里的字）。
/// 所以判据测这个函数，绘制只负责把字符串摆上去。
///
/// 行序固定：`#id` / `pos x y z` / `parent` / `visible` —— 变化只改内容不改行数，
/// 面板高度才稳定。
/// 编辑态按键能触发的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Tab：循环 gizmo 模式。
    CycleGizmo,
    /// Backspace：撤销。
    Undo,
    /// F5：重做。
    Redo,
    /// Delete：删除选中实体（撤销栈兜底）。
    Delete,
}

/// 按键种类（不直接依赖 winit：引擎层不认识 `KeyCode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Tab,
    Backspace,
    F5,
    Delete,
    Other,
}

/// **编辑态按键路由**：纯函数，四个输入一个输出。
///
/// 存在的理由（本仓教训：键盘在 headless 下按不了）：
/// 把"按键 → 动作"这段逻辑单独判掉，剩下"事件循环有没有把按键喂进来"才是必须真机验证的。
/// 没有这条，整块按键逻辑只能靠读代码，而"读了就等于做了"是这一晚反复出现的自我说服。
///
/// 返回 `None` 表示**不拦截**：交回给系统键处理（例如 `Tab` 在玩法态要切相机）。
pub fn key_action(
    mode: crate::engine::mode::Mode,
    key: KeyKind,
    pressed: bool,
) -> Option<KeyAction> {
    // 只认"按下"，不认"抬起"：否则一次按键会触发两次。
    if !pressed || mode != crate::engine::mode::Mode::Edit {
        return None;
    }
    match key {
        KeyKind::Tab => Some(KeyAction::CycleGizmo),
        KeyKind::Backspace => Some(KeyAction::Undo),
        KeyKind::F5 => Some(KeyAction::Redo),
        KeyKind::Delete => Some(KeyAction::Delete),
        KeyKind::Other => None,
    }
}

pub fn inspector_lines(scene: &Scene, selection: Option<EntityId>) -> Vec<String> {
    let Some(id) = selection else {
        return vec!["未选中".to_string()];
    };
    let Ok(local) = scene.local(id) else {
        return vec!["选中已失效".to_string()];
    };
    let t = local.translation;
    let parent = match scene.parent(id) {
        Ok(Some(p)) => format!("#{}", scene.index_of(p)),
        Ok(None) => "-".to_string(),
        Err(_) => "?".to_string(),
    };
    vec![
        format!("#{}", scene.index_of(id)),
        format!("pos {:.3} {:.3} {:.3}", t.x, t.y, t.z),
        format!("parent {parent}"),
        format!(
            "visible {}",
            if scene.is_visible(id) { "on" } else { "off" }
        ),
        // 🔴 只读要**写在面板上**：这类实体拖不动、删不掉，
        // 不标出来用户只会以为编辑器坏了。
        if scene.is_readonly(id) { "只读镜像" } else { "可编辑" }.to_string(),
    ]
}

/// 左键按下时的**全部决策**：是否点在实体上？落在哪根把手上？能否起拖？
///
/// 抽成纯函数（不碰 GameApp）有两个理由：
/// 1. 鼠标路径与 AI 接口调的是同一份 —— 不会出现"鼠标能拖、接口拖出来不一样"；
/// 2. 它现在**能被判据覆盖**。之前这段逻辑长在窗口事件里，只能靠真鼠标手测。
///
/// 返回 `(选中项, 拖拽)`：点空白 ⇒ `(None, None)`；点中实体但没落在把手上 ⇒ `(Some, None)`
/// —— 后者是"只选中、不拖"，与 Unity 里点模型体只选中是同一个行为。
pub fn begin_drag(
    scene: &Scene,
    view: &crate::engine::gizmo::ViewInfo,
    ndc: (f32, f32),
    pick_radius: f32,
    mode: crate::engine::gizmo::Mode,
) -> (
    Option<EntityId>,
    Option<EditorDrag>,
) {
    let Some((id, _t)) = view.pick(scene, ndc.0, ndc.1, pick_radius) else {
        return (None, None);
    };
    let Ok(local) = scene.local(id) else {
        return (Some(id), None);
    };
    // 旋转模式：只要点在物体上就开始转（不需要"抓住某根轴"——
    // 绕 Y 的旋转把手在屏幕上是绕中心的一个圆环，点环上任何位置都等价于记录起始角度）。
    if mode == crate::engine::gizmo::Mode::Rotate {
        return (
            Some(id),
            Some(EditorDrag {
                entity: id,
                kind: DragKind::RotateY {
                    start_ndc: ndc,
                    start_rotation: local.rotation,
                },
            }),
        );
    }
    let Some(axis) = crate::engine::gizmo::pick_axis_at_screen(local.translation, view, ndc.0, ndc.1)
    else {
        return (Some(id), None);
    };
    // 缩放与平移**共用同一套抓把手**（都要抓住某根轴），只是拖出来的东西不同。
    if mode == crate::engine::gizmo::Mode::Scale {
        let (o, d) = crate::engine::gizmo::screen_ray(
            view.eye,
            view.forward,
            view.right,
            view.up,
            view.half_fov_y,
            view.aspect,
            ndc.0,
            ndc.1,
        );
        let Some(drag) = crate::engine::gizmo::TranslateDrag::begin(axis, local.translation, o, d)
        else {
            return (Some(id), None);
        };
        return (
            Some(id),
            Some(EditorDrag {
                entity: id,
                kind: DragKind::Scale {
                    drag,
                    start_scale: local.scale,
                },
            }),
        );
    }
    let (o, d) = crate::engine::gizmo::screen_ray(
        view.eye,
        view.forward,
        view.right,
        view.up,
        view.half_fov_y,
        view.aspect,
        ndc.0,
        ndc.1,
    );
    match crate::engine::gizmo::TranslateDrag::begin(axis, local.translation, o, d) {
        Some(drag) => (
            Some(id),
            Some(EditorDrag {
                entity: id,
                kind: DragKind::Translate(drag),
            }),
        ),
        None => (Some(id), None),
    }
}

/// 拖动中的一步：算出新平移并写回场景。返回是否真的写了。
///
/// 写回时保留旋转与缩放（只改平移）—— gizmo 现在只有平移轴，若把 rot/scale 一起覆盖，
/// 用户在这个阶段做的旋转会在下一次拖动时静默丢失。
pub fn update_drag(
    scene: &mut Scene,
    view: &crate::engine::gizmo::ViewInfo,
    ed: &EditorDrag,
    ndc: (f32, f32),
) -> bool {
    let Ok(local) = scene.local(ed.entity) else {
        return false;
    };
    match ed.kind {
        DragKind::Translate(drag) => {
            let (o, d) = crate::engine::gizmo::screen_ray(
                view.eye,
                view.forward,
                view.right,
                view.up,
                view.half_fov_y,
                view.aspect,
                ndc.0,
                ndc.1,
            );
            let Some(moved) = drag.drag(o, d) else {
                return false;
            };
            scene
                .set_local(
                    ed.entity,
                    crate::engine::scene::Transform {
                        translation: moved,
                        ..local
                    },
                )
                .is_ok()
        }
        DragKind::Scale { drag, start_scale } => {
            let (o, dd) = crate::engine::gizmo::screen_ray(
                view.eye,
                view.forward,
                view.right,
                view.up,
                view.half_fov_y,
                view.aspect,
                ndc.0,
                ndc.1,
            );
            let Some(moved) = drag.drag(o, dd) else {
                return false;
            };
            // 沿轴的位移（米）→ 倍率。用 `|moved - start|` 在轴上的投影，
            // 而不是 `|moved|`：物体不在原点时后者会把离原点的距离也算进去。
            // `TranslateDrag::drag` 返回的是**新平移**，沿轴位移 = 新平移在轴上减起点。
            let delta = (moved - drag.start_translation()).dot(drag.axis().dir());
            let f = crate::engine::gizmo::scale_factor_from_drag(delta);
            scene
                .set_local(
                    ed.entity,
                    crate::engine::scene::Transform {
                        scale: start_scale * f,
                        ..local
                    },
                )
                .is_ok()
        }
        DragKind::RotateY {
            start_ndc,
            start_rotation,
        } => {
            let Some(d) =
                crate::engine::gizmo::rotate_delta_y(local.translation, view, start_ndc, ndc)
            else {
                return false;
            };
            scene
                .set_local(
                    ed.entity,
                    crate::engine::scene::Transform {
                        rotation: glam::Quat::from_rotation_y(d) * start_rotation,
                        ..local
                    },
                )
                .is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    /// 🔴 **场景树命中判定**：第 k 行返回 k；标题区/面板外/超出最大行数返回 None。
    ///
    /// 反向验证：把 `tree_rows_top(s)` 换成 `0.0`（忘了留标题区）⇒ 点标题会选中第 0 行，红。
    #[test]
    fn tree_row_hit_test_skips_the_header() {
        let (sw, sh, s) = (1280.0f32, 720.0f32, 1.0f32);
        let rows = 3usize;
        let w = PANEL_W * s;
        let x = (sw - w).max(0.0);
        let cx = x + w * 0.5;
        let top = tree_rows_top(s);
        // 标题区（行起点之上）不许命中
        assert_eq!(tree_row_at(sw, sh, rows, s, cx, top - 1.0), None);
        // 逐行命中
        for row in 0..rows {
            let cy = top + row as f32 * ROW_H * s + 2.0;
            assert_eq!(
                tree_row_at(sw, sh, rows, s, cx, cy),
                Some(row),
                "第 {row} 行应命中自己"
            );
        }
        // 超出行数（面板空白处）不许命中
        let beyond = top + rows as f32 * ROW_H * s + 2.0;
        assert_eq!(tree_row_at(sw, sh, rows, s, cx, beyond), None);
        // 面板左侧（场景区域）不许命中
        assert_eq!(tree_row_at(sw, sh, rows, s, x - 10.0, top + 2.0), None);
    }



    /// 🔴 **资产面板命中判定**：点第 k 条资产要返回 k，点表头/面板外要返回 None。
    ///
    /// 反向验证：把 `Some(row - 1)` 写成 `Some(row)`（忘了扣表头）⇒ 点第一条会返回 1，红。
    #[test]
    fn asset_row_hit_test_skips_the_header() {
        let (sw, sh, s) = (1280.0f32, 720.0f32, 1.0f32);
        // 表头 + 3 条资产 = 4 行
        let lines = 4usize;
        let r = asset_panel_rect(sw, sh, lines, s);
        // 面板必须落在屏幕内
        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= sw && r.y + r.h <= sh, "{r:?}");
        // 逐行扫一遍：每一行中点都必须命中对应的资产下标
        for row in 0..lines {
            let cy = r.y + PAD * s + row as f32 * ROW_H * s + ROW_H * s * 0.5;
            let got = asset_row_at(sw, sh, lines, s, r.x + r.w * 0.5, cy);
            if row == 0 {
                assert_eq!(got, None, "点表头不该选中资产");
            } else {
                assert_eq!(got, Some(row - 1), "第 {row} 行应是资产 {}", row - 1);
            }
        }
        // 面板外
        assert_eq!(asset_row_at(sw, sh, lines, s, 1.0, 1.0), None);
        // 只有表头（没有资产）
        assert_eq!(asset_row_at(sw, sh, 1, s, 100.0, 100.0), None);
    }




    use super::*;
    use crate::engine::scene::Transform;
    use glam::Vec3;

    fn scene_with(n: usize) -> Scene {
        let mut s = Scene::new();
        for i in 0..n {
            s.spawn(None, Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)));
        }
        s
    }

    /// 🔴 检视面板显示的四个字段必须与场景**逐字对应**：
    /// 改了 Transform，面板文字必须跟着变（不是"大概显示了一下"）。
    ///
    /// 反向验证：把 `pos {:.3}` 写成固定字符串 ⇒ 改坐标后文字不变，红。
    #[test]
    fn inspector_lines_reflect_the_actual_transform() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let e = s.spawn(
            Some(root),
            Transform::from_translation(Vec3::new(1.5, -2.0, 3.25)),
        );
        let lines = inspector_lines(&s, Some(e));
        assert_eq!(lines.len(), 5, "行数固定为 5，面板高度才稳定：{lines:?}");
        assert_eq!(lines[0], format!("#{}", s.index_of(e)));
        assert_eq!(lines[1], "pos 1.500 -2.000 3.250");
        assert_eq!(lines[2], format!("parent #{}", s.index_of(root)));
        assert_eq!(lines[3], "visible on");
        assert_eq!(lines[4], "可编辑", "默认实体应标为可编辑");
        // 改坐标 ⇒ 文字必须跟着变
        s.set_local(e, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)))
            .unwrap();
        assert_eq!(inspector_lines(&s, Some(e))[1], "pos 0.000 0.000 0.000");
        // 关掉可见性 ⇒ 文字跟着变
        s.set_visible(e, false).unwrap();
        assert_eq!(inspector_lines(&s, Some(e))[3], "visible off");
    }

    /// 没选中 / 句柄失效时必须给**明确的文字**，不能空着（空面板看不出是没选中还是坏了）。
    #[test]
    fn inspector_lines_are_explicit_when_nothing_is_selected() {
        let mut s = Scene::new();
        assert_eq!(inspector_lines(&s, None), vec!["未选中".to_string()]);
        let e = s.spawn(None, Transform::IDENTITY);
        s.despawn(e).unwrap();
        assert_eq!(inspector_lines(&s, Some(e)), vec!["选中已失效".to_string()]);
    }




    /// 🔴 **编辑态按键路由**：Tab/Backspace/F5 只在编辑态、且只在"按下"时生效。
    ///
    /// 反向验证：把 `if !pressed || mode != Edit` 里的 `!pressed` 去掉 ⇒ 抬起也会触发，红。
    #[test]
    fn key_actions_are_edit_only_and_press_only() {
        use crate::engine::mode::Mode;
        // 编辑态 + 按下 ⇒ 有动作
        assert_eq!(
            key_action(Mode::Edit, KeyKind::Tab, true),
            Some(KeyAction::CycleGizmo)
        );
        assert_eq!(
            key_action(Mode::Edit, KeyKind::Backspace, true),
            Some(KeyAction::Undo)
        );
        assert_eq!(
            key_action(Mode::Edit, KeyKind::F5, true),
            Some(KeyAction::Redo)
        );
        // 抬起 ⇒ 不触发（否则一次按键触发两次）
        assert_eq!(key_action(Mode::Edit, KeyKind::Tab, false), None);
        // 玩法态 ⇒ 完全不拦截（Tab 要留给相机切换）
        for k in [KeyKind::Tab, KeyKind::Backspace, KeyKind::F5] {
            assert_eq!(key_action(Mode::Play, k, true), None, "玩法态不许拦 {k:?}");
        }
        // 其它键 ⇒ 放行
        assert_eq!(key_action(Mode::Edit, KeyKind::Other, true), None);
    }

    /// 🔴 **缩放模式全链路**：抓把手拖 1m ⇒ 缩放翻倍、平移与旋转都不许变。
    ///
    /// 反向验证：把 `scale: start_scale * f` 写成 `scale: start_scale` ⇒ 仍是 1.0，红。
    #[test]
    fn scale_mode_doubles_after_dragging_one_handle() {
        let view = crate::engine::gizmo::ViewInfo {
            eye: Vec3::new(0.0, 0.0, 5.0),
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let mut s = Scene::new();
        let id = s.spawn(None, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
        // 🔴 必须先**点中实体**才会抓把手（`pick_entity` 的半径 0.9m），
        // 所以抓点取把手**中点**（0.5m 处，在半径内），而不是末端 1m 处 —— 后者点不到实体。
        // 我第一版就取在 1m 处，`begin_drag` 直接返回 None。
        let grip = Vec3::new(0.5, 0.0, 0.0);
        let (gx, gy) = crate::engine::gizmo::project_to_ndc(&view, grip).expect("投影");
        let (sel, drag) = begin_drag(&s, &view, (gx, gy), 0.9, crate::engine::gizmo::Mode::Scale);
        assert_eq!(sel, Some(id));
        let drag = drag.expect("抓中把手就该起拖");
        // 沿 +X 再拖 1m（0.5 → 1.5）：倍率应变 2.0
        let (tx, ty) = crate::engine::gizmo::project_to_ndc(&view, Vec3::new(1.5, 0.0, 0.0))
            .expect("投影");
        assert!(update_drag(&mut s, &view, &drag, (tx, ty)), "必须写回");
        let after = s.local(id).unwrap();
        assert!(
            (after.scale.x - 2.0).abs() < 1e-2,
            "拖一个把手长度应翻倍，实际 {}",
            after.scale.x
        );
        assert!(
            (after.translation - Vec3::ZERO).length() < 1e-6,
            "缩放不许动物体位置：{:?}",
            after.translation
        );
        assert!(
            (after.rotation - glam::Quat::IDENTITY).length() < 1e-6,
            "缩放不许动旋转"
        );
    }

    /// 🔴 **旋转模式的全链路**：按下 → 拖动 → 写回，只改 rotation 不改 translation。
    ///
    /// 反向验证：把 `DragKind::RotateY` 分支写成 `* start_rotation`（忽略增量）⇒ 角度不变，红。
    #[test]
    fn rotate_mode_turns_the_entity_without_moving_it() {
        let view = crate::engine::gizmo::ViewInfo {
            eye: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let mut s = Scene::new();
        let id = s.spawn(
            None,
            Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)),
        );
        let (cx, cy) = crate::engine::gizmo::project_to_ndc(&view, Vec3::new(0.0, 0.0, -3.0))
            .expect("投影");
        // 从中心右侧拖到中心上方
        let from = (cx + 0.2, cy);
        let to = (cx, cy + 0.2);
        let (sel, drag) = begin_drag(&s, &view, from, 0.9, crate::engine::gizmo::Mode::Rotate);
        assert_eq!(sel, Some(id));
        let drag = drag.expect("旋转模式点中物体就该起拖");
        assert!(update_drag(&mut s, &view, &drag, to), "必须写回");
        let after = s.local(id).unwrap();
        let yaw = after.rotation.to_euler(glam::EulerRot::YXZ).0;
        assert!(
            (yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "应转 90°，实际 {}°",
            yaw.to_degrees()
        );
        assert!(
            (after.translation - Vec3::new(0.0, 0.0, -3.0)).length() < 1e-6,
            "旋转不许动物体位置：{:?}",
            after.translation
        );
    }

    /// 🔴 **按下 → 拖动 → 写回** 三段全链路（纯函数层，与鼠标走同一份代码）。
    ///
    /// 相机在原点朝 -Z，实体在 (0,0,-3)；抓住 Y 把手往上拖 0.2 NDC ⇒ 只有 Y 增加。
    ///
    /// 反向验证：把 `update_drag` 里的 `..local` 去掉（只写 translation、丢掉 rot/scale）
    /// ⇒ 第二条断言（旋转必须保住）红。
    #[test]
    fn begin_and_update_drag_move_the_entity_along_the_handle() {
        let view = crate::engine::gizmo::ViewInfo {
            eye: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let mut s = Scene::new();
        let id = s.spawn(
            None,
            Transform {
                translation: Vec3::new(0.0, 0.0, -3.0),
                rotation: glam::Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
                scale: Vec3::splat(2.0),
            },
        );
        // 光标落在 Y 把手（0..1m）中点
        let from = crate::engine::gizmo::project_to_ndc(
            &view,
            Vec3::new(0.0, 0.0, -3.0) + Vec3::Y * 0.5,
        )
        .unwrap();
        let (sel, drag) = begin_drag(&s, &view, from, 0.9, crate::engine::gizmo::Mode::Translate);
        assert_eq!(sel, Some(id), "应选中该实体");
        let drag = drag.expect("落在把手上必须能起拖");
        // 往上拖
        let to = (from.0, from.1 + 0.2);
        assert!(update_drag(&mut s, &view, &drag, to), "拖动必须写回");
        let after = s.local(id).unwrap();
        assert!(after.translation.y > 0.05, "Y 必须增加：{:?}", after.translation);
        assert!(
            (after.translation.x).abs() < 1e-4 && (after.translation.z + 3.0).abs() < 1e-4,
            "X/Z 不许动：{:?}",
            after.translation
        );
        assert!(
            (after.scale.x - 2.0).abs() < 1e-6 && (after.rotation.w - 0.8).abs() < 1e-6,
            "旋转与缩放必须保住：{:?} / {:?}",
            after.rotation,
            after.scale
        );
    }

    /// 点空白必须**清空选中**且不起拖（否则编辑器里物体再也放不下）。
    #[test]
    fn begin_drag_on_empty_space_clears_selection() {
        let view = crate::engine::gizmo::ViewInfo {
            eye: Vec3::ZERO,
            forward: Vec3::new(0.0, 0.0, -1.0),
            right: Vec3::X,
            up: Vec3::Y,
            half_fov_y: std::f32::consts::FRAC_PI_4,
            aspect: 1.0,
        };
        let mut s = Scene::new();
        s.spawn(None, Transform::from_translation(Vec3::new(0.0, 0.0, -3.0)));
        let (sel, drag) = begin_drag(&s, &view, (0.9, 0.9), 0.9, crate::engine::gizmo::Mode::Translate);
        assert_eq!(sel, None, "点空白不该选中任何东西");
        assert!(drag.is_none(), "点空白不该起拖");
    }

    /// 🔴 像素 → NDC 的映射（含 Y 翻转）。四角与中心钉死。
    ///
    /// 反向验证：去掉 `1.0 -`（不翻 Y）⇒ 左上角得 (-1,-1)，红。
    #[test]
    fn ndc_from_pixel_flips_y_and_maps_corners() {
        assert_eq!(ndc_from_pixel(0.0, 0.0, 800.0, 600.0), (-1.0, 1.0));
        assert_eq!(ndc_from_pixel(800.0, 600.0, 800.0, 600.0), (1.0, -1.0));
        assert_eq!(ndc_from_pixel(400.0, 300.0, 800.0, 600.0), (0.0, 0.0));
        // 零尺寸不许除出 inf
        assert_eq!(ndc_from_pixel(10.0, 10.0, 0.0, 0.0), (0.0, 0.0));
    }

    /// 🔴 面板必须**画出选中行**，且只画一行高亮。
    ///
    /// 反向验证：把 `if is_sel` 改成 `if false` ⇒ 高亮 quad 消失，这条红。
    #[test]
    fn editor_panel_highlights_exactly_one_selected_row() {
        let s = scene_with(3);
        let ids: Vec<_> = s.iter_alive().map(|(id, _)| id).collect();
        let q = panel_quads(&s, Some(ids[1]), 1280.0, 720.0, 1.0);
        let sel_rows = q
            .iter()
            .filter(|x| x.color.r == ROW_SEL.r && x.color.g == ROW_SEL.g && x.color.b == ROW_SEL.b)
            .count();
        assert_eq!(sel_rows, 1, "选中行必须且只能高亮一行，实际 {sel_rows}");
    }

    /// 实体的世界坐标必须真的出现在面板里 —— 面板"看得见场景"的最低要求。
    ///
    /// 做法：把 `render_text` 产生的 quad 数量作为代理指标（文本每字符一个 quad），
    /// 比较 1 个实体与 3 个实体的 quad 数：必须随实体数增长。
    /// 反向验证：把行循环改成只画第一行 ⇒ 两者相等，这条红。
    #[test]
    fn editor_panel_draws_a_row_per_entity() {
        let a = panel_quads(&scene_with(1), None, 1280.0, 720.0, 1.0).len();
        let b = panel_quads(&scene_with(3), None, 1280.0, 720.0, 1.0).len();
        assert!(b > a, "3 个实体的面板应比 1 个的多出文本 quad（{a} vs {b}）");
    }

    /// 超出上限时不许无限增长（HUD quad 容量是有限的），且要给出"还有 N 个"。
    #[test]
    fn editor_panel_is_bounded_and_reports_the_remainder() {
        let a = panel_quads(&scene_with(MAX_ROWS), None, 1280.0, 720.0, 1.0).len();
        let b = panel_quads(&scene_with(MAX_ROWS + 5), None, 1280.0, 720.0, 1.0).len();
        assert!(
            b <= a + 64,
            "超出 MAX_ROWS 后只应多出一行提示，实际从 {a} 涨到 {b}"
        );
    }
}
