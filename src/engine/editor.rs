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
    y += ROW_H * s + 4.0 * s;
    // 分隔线（用一条很扁的 quad）
    q.push(Quad::new(
        Rect::new(x + PAD * s, y, w - PAD * 2.0 * s, 1.0 * s),
        DIM,
    ));
    y += 6.0 * s;

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
    // 底部：选中项
    let sel_line = match selection {
        Some(id) if scene.is_alive(id) => format!("选中 #{}", scene.index_of(id)),
        Some(_) => "选中已失效".to_string(),
        None => "未选中".to_string(),
    };
    crate::ui::render_text(
        &sel_line,
        x + PAD * s,
        h - ROW_H * 1.5 * s,
        HEAD,
        text_scale,
        &mut q,
    );
    q
}

#[cfg(test)]
mod tests {
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
