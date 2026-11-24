//! SceneView：渲染器唯一该看的**只读快照**（引擎化 P1b，见 `docs/ENGINE-PLAN.md`）。
//!
//! 为什么要有这一层：今天 `app_render` 直接问 `game` 要 marker，渲染器又反过来引用
//! `engine::game` 的类型（那一行已经消掉了，但"渲染器认识玩法"的形态还在）。
//! SceneView 把关系正过来：**场景是数据源，渲染器只消费快照**。
//!
//! 快照一旦生成就与场景断开：渲染线程/后续阶段改动场景不会回头影响这一帧的画面 ——
//! 这正是编辑器"播放时改场景不该撕裂当前帧"所需要的性质。
//!
//! 现在快照里只放"可见实体 + 世界矩阵 + 包围盒 + 色调"。等 P4 有了网格组件再扩字段，
//! 但**结构不变**：新增能力 = 往 `SceneDrawItem` 加字段，而不是让渲染器回头去读 Scene。

use crate::engine::scene::Scene;
use glam::{Mat4, Vec3};

/// 单个可绘制实体。**渲染器只认这个**，不再认识 `EntityId` 与 `Transform`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDrawItem {
    pub model: Mat4,
    /// 世界空间半尺寸（米）。给剔除与调试用；画出来的尺寸由 `model` 决定。
    pub half_extent: Vec3,
    pub tint: [f32; 4],
}

/// 一帧的只读快照。
#[derive(Debug, Default, Clone)]
pub struct SceneView {
    pub items: Vec<SceneDrawItem>,
}

impl SceneView {
    /// 从场景抽取。**只取 `visible` 的实体**：世界根与玩家是场景结构的一部分，
    /// 但不是"要画出来的东西"（玩家由第一人称相机表示）。
    pub fn extract(scene: &Scene) -> Self {
        let mut items = Vec::new();
        for (id, local) in scene.iter_alive() {
            if !scene.is_visible(id) {
                continue;
            }
            // 🔴 半尺寸**必须烘进模型矩阵**：marker 模板是单位立方体（半幅 1），
            // 直接把世界矩阵喂进去等于"1 单位见方"，看起来是一块压扁的板
            // （实测第一版就是这样：截图里是个扁盒子）。
            let world = scene.world_matrix(id).unwrap_or(local.matrix());
            items.push(SceneDrawItem {
                model: world * glam::Mat4::from_scale(DEFAULT_HALF_EXTENT),
                half_extent: DEFAULT_HALF_EXTENT,
                tint: EDITOR_TINT,
            });
        }
        SceneView { items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 编辑器里新实体的默认半尺寸（米）与色调。
///
/// 半尺寸 0.5 = 边长 1m 的立方体，和 Unity 新建 Cube 一致。
/// 色调用青色系，与场景里既有道具（灰/砖红/绿）区分得开，一眼能认出是编辑器放的。
pub const DEFAULT_HALF_EXTENT: Vec3 = Vec3::new(0.5, 0.5, 0.5);
/// 色调用 `Shape::Authored` 标签：它让片元**跳过四条程序化表面效果**
/// （窗带 / 玻璃分格+菲涅耳 / 树冠噪声 / 混凝土皮肤）。
///
/// 用 `Legacy` 的后果实测过：编辑器放的方盒子被当成建筑立面，贴上玻璃与窗带，
/// 看起来像一块脏玻璃板 —— 而编辑器里的默认物体应该是一眼看清形状的纯色盒。
pub const EDITOR_TINT: [f32; 4] = [0.35, 0.85, 1.0, crate::engine::geom::Shape::Authored.tag()];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::scene::Transform;

    /// 🔴 只有 `visible` 的实体进快照 —— 世界根与玩家必须被排除。
    ///
    /// 反向验证：把 `if !scene.is_visible(id) { continue; }` 删掉 ⇒ 这条红
    /// （而且播放态每帧会多画出两个盒子，`png_diff` 立刻不为 0）。
    #[test]
    fn scene_view_skips_invisible_entities() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let player = s.spawn(Some(root), Transform::IDENTITY);
        s.set_visible(root, false).unwrap();
        s.set_visible(player, false).unwrap();
        assert!(SceneView::extract(&s).is_empty(), "根与玩家都不该进快照");

        let e = s.spawn(None, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let v = SceneView::extract(&s);
        assert_eq!(v.len(), 1, "可见实体必须进快照");
        let m = v.items[0].model;
        assert_eq!(m.w_axis.truncate(), Vec3::new(1.0, 2.0, 3.0));
        let _ = e;
    }

    /// 快照必须带**世界矩阵**（上一层有位移时不能只给局部矩阵）。
    #[test]
    fn scene_view_uses_world_matrix_not_local() {
        let mut s = Scene::new();
        let parent = s.spawn(None, Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        let child = s.spawn(Some(parent), Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
        let v = SceneView::extract(&s);
        // 上层与下层都可见 ⇒ 两个 item；取子（第二个）
        assert_eq!(v.len(), 2);
        let child_item = v.items.iter().find(|it| {
            it.model.w_axis.truncate() == Vec3::new(11.0, 0.0, 0.0)
        });
        assert!(child_item.is_some(), "子实体必须是世界坐标 (11,0,0)：{:?}", v.items);
        let _ = child;
    }
}
