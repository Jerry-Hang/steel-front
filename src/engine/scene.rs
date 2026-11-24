//! 场景层：实体 + 变换 + 上下层级 —— **引擎化的地基**（2026-11-24 立，见 `docs/ENGINE-PLAN.md`）。
//!
//! 为什么先做它：编辑器里"拖拽一个物体"、渲染器"画一个物体"、AI 接口"移动一个物体"，
//! 这三件事改的都是同一个东西 —— **一张 `Transform` 表**。三者共用这张表，才才算得上
//! "人在编辑器里编辑的同时，AI 通过接口操作同一个场景"。
//!
//! 设计约束（与仓库既有约定一致）：
//! - **零第三方依赖**（`glam` 是已有的直接依赖，允许）。
//! - **`EntityId` 带 generation**：删除后旧 id 必须失效，否则"删掉的实体又被改到"这类
//!   静默事故会一直存在（本仓对"静默错误"的红线见 `docs/pitfalls/lying-gates.md`）。
//! - **上下成环必须当场报错**：层级是树，不是图。宁可返回 `Err` 也不要静默截断。

use glam::{Mat4, Quat, Vec3};

/// 实体句柄：`index` 指向槽位，`generation` 区分"同一槽位被复用"的前后两代。
///
/// 为什么不用裸 `usize`：删除后槽位会被新实体复用，裸下标会让**旧句柄悄悄指向新实体**。
/// 这类错误不会崩、不会报，只会让"我明明删了它"变成"它跑到别处去了"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId {
    index: u32,
    generation: u32,
}

/// 局部变换（相对上层节点）。组合顺序固定为 **T ∘ R ∘ S**（先缩放、再旋转、最后平移），
/// 与 Unity/Unreal 的常规一致 —— 编辑器 gizmo 拖动时依赖这个顺序。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Transform {
            translation,
            ..Transform::IDENTITY
        }
    }

    /// 局部矩阵 = T · R · S。
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// 场景层可能返回的错误。全部是"调用方必须处理"的：没有静默兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneError {
    /// 句柄无效（已删除，或从未存在，或属于被复用的上一代）。
    NoSuchEntity(EntityId),
    /// 把节点挂到自己或自己的后代下 ⇒ 成环。
    WouldCycle,
}

struct Slot {
    generation: u32,
    alive: bool,
    parent: Option<EntityId>,
    children: Vec<EntityId>,
    local: Transform,
}

/// 场景：一张槽位表（实体表）。**只有变换与层级**，组件表在后续阶段加（见 P0/P1 计划）。
pub struct Scene {
    slots: Vec<Slot>,
    alive: usize,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        Scene {
            slots: Vec::new(),
            alive: 0,
        }
    }

    /// 存活实体数。
    pub fn len(&self) -> usize {
        self.alive
    }

    /// 遍历所有存活实体（槽位顺序）以及它们的局部变换。
    ///
    /// 序列化（`engine::scene_io`）与编辑器场景树都靠它 —— 遍历顺序稳定，
    /// 存出来的文件 diff 才可读（不是每次跑都换一个顺序）。
    pub fn iter_alive(&self) -> impl Iterator<Item = (EntityId, Transform)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.alive.then_some((
                EntityId {
                    index: i as u32,
                    generation: s.generation,
                },
                s.local,
            ))
        })
    }

    /// 槽位下标（**只给序列化/诊断用**：它是实现细节，不是身份，身份是 `EntityId`）。
    pub fn index_of(&self, id: EntityId) -> usize {
        id.index as usize
    }

    pub fn is_alive(&self, id: EntityId) -> bool {
        self.slots
            .get(id.index as usize)
            .is_some_and(|s| s.alive && s.generation == id.generation)
    }

    fn slot(&self, id: EntityId) -> Result<&Slot, SceneError> {
        match self.slots.get(id.index as usize) {
            Some(s) if s.alive && s.generation == id.generation => Ok(s),
            _ => Err(SceneError::NoSuchEntity(id)),
        }
    }

    fn slot_mut(&mut self, id: EntityId) -> Result<&mut Slot, SceneError> {
        match self.slots.get_mut(id.index as usize) {
            Some(s) if s.alive && s.generation == id.generation => Ok(s),
            _ => Err(SceneError::NoSuchEntity(id)),
        }
    }

    /// 新建实体。`parent` 必须是有效句柄。
    pub fn spawn(&mut self, parent: Option<EntityId>, local: Transform) -> EntityId {
        if let Some(p) = parent {
            // 上层句柄无效时不建半个实体出来：直接当作编程错误暴露（调用方应先 is_alive）。
            assert!(self.is_alive(p), "spawn: 上层句柄无效 {p:?}");
        }
        // 复用第一个空槽（编辑器里会反复增删，不留空洞）。
        let index = match self.slots.iter().position(|s| !s.alive) {
            Some(i) => i,
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    alive: false,
                    parent: None,
                    children: Vec::new(),
                    local: Transform::IDENTITY,
                });
                self.slots.len() - 1
            }
        };
        let slot = &mut self.slots[index];
        slot.alive = true;
        slot.parent = parent;
        slot.children.clear();
        slot.local = local;
        let id = EntityId {
            index: index as u32,
            generation: slot.generation,
        };
        if let Some(p) = parent {
            // 上面已断言存活，这里不会再失败。
            if let Ok(ps) = self.slot_mut(p) {
                ps.children.push(id);
            }
        }
        self.alive += 1;
        id
    }

    /// 删除实体**及其全部后代**（级联）。
    ///
    /// 级联是硬要求：编辑器里删一个上层节点却留下没人管的深层后代，下一帧就会看到"删不掉的残留"。
    pub fn despawn(&mut self, id: EntityId) -> Result<(), SceneError> {
        self.slot(id)?; // 校验句柄
        // 先收集整棵子树（显式栈，不递归 —— 层级可能很深）。
        let mut stack = vec![id];
        let mut doomed: Vec<EntityId> = Vec::new();
        while let Some(cur) = stack.pop() {
            if let Ok(s) = self.slot(cur) {
                stack.extend(s.children.iter().copied());
                doomed.push(cur);
            }
        }
        // 把自己从上层节点的 children 里摘掉（上层不受影响）
        if let Ok(parent) = self.parent(id) {
            if let Some(p) = parent {
                if let Ok(ps) = self.slot_mut(p) {
                    ps.children.retain(|c| *c != id);
                }
            }
        }
        for e in doomed {
            if let Some(s) = self.slots.get_mut(e.index as usize) {
                s.alive = false;
                s.parent = None;
                s.children.clear();
                // 代号自增 ⇒ 旧句柄永久失效（除非它恰好绕满 2^32 代）
                s.generation = s.generation.wrapping_add(1);
            }
            self.alive -= 1;
        }
        Ok(())
    }

    pub fn local(&self, id: EntityId) -> Result<Transform, SceneError> {
        Ok(self.slot(id)?.local)
    }

    pub fn set_local(&mut self, id: EntityId, local: Transform) -> Result<(), SceneError> {
        self.slot_mut(id)?.local = local;
        Ok(())
    }

    pub fn parent(&self, id: EntityId) -> Result<Option<EntityId>, SceneError> {
        Ok(self.slot(id)?.parent)
    }

    pub fn children(&self, id: EntityId) -> Result<&[EntityId], SceneError> {
        Ok(&self.slot(id)?.children)
    }

    /// 改上层节点。拒绝 `parent == child` 与"把更上层的节点挂到自己的后代下"（成环）。
    ///
    /// 关键：**局部变换保持不变**（不重新解算世界位置）。编辑器里改层级与改坐标是两件事，
    /// 混在一起会让"拖进另一个上层节点"时物体莫名位移。
    pub fn set_parent(&mut self, child: EntityId, parent: Option<EntityId>) -> Result<(), SceneError> {
        self.slot(child)?;
        if let Some(p) = parent {
            if p == child {
                return Err(SceneError::WouldCycle);
            }
            self.slot(p)?;
            // 从 p 往上走，若遇到 child ⇒ 成环
            let mut cur = Some(p);
            let mut guard = 0usize;
            while let Some(c) = cur {
                if c == child {
                    return Err(SceneError::WouldCycle);
                }
                cur = self.slot(c)?.parent;
                guard += 1;
                // 防御：已有数据结构里不该出现环；真出现了也不要死循环
                if guard > self.slots.len() + 1 {
                    return Err(SceneError::WouldCycle);
                }
            }
        }
        // 从旧上层节点的 children 里摘掉
        let old = self.slot(child)?.parent;
        if let Some(o) = old {
            if let Ok(os) = self.slot_mut(o) {
                os.children.retain(|c| *c != child);
            }
        }
        self.slot_mut(child)?.parent = parent;
        if let Some(p) = parent {
            if let Ok(ps) = self.slot_mut(p) {
                ps.children.push(child);
            }
        }
        Ok(())
    }

    /// 世界矩阵 = 根 → 该节点 的局部矩阵连乘。
    pub fn world_matrix(&self, id: EntityId) -> Result<Mat4, SceneError> {
        let mut chain: Vec<EntityId> = vec![id];
        let mut cur = self.slot(id)?.parent;
        let mut guard = 0usize;
        while let Some(p) = cur {
            chain.push(p);
            cur = self.slot(p)?.parent;
            guard += 1;
            if guard > self.slots.len() + 1 {
                return Err(SceneError::WouldCycle);
            }
        }
        let mut m = Mat4::IDENTITY;
        for e in chain.iter().rev() {
            m *= self.slot(*e)?.local.matrix();
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    /// 🔴 世界矩阵必须沿上层链组合：上层的**旋转会带着下层一起转**。
    ///
    /// 手算：上层 T=(2,0,0)、R=绕 Y 轴 +90°；子 local T=(0,0,1)（无旋转无缩放）。
    /// `rot_y(90°)·(0,0,1)` = (1,0,0) ⇒ 世界位置 = (1,0,0)+(2,0,0) = **(3,0,0)**。
    ///
    /// 反向验证：把 `world_matrix` 里的 `m *= local` 改成 `m = local`（只取本节点）⇒ 立刻红。
    #[test]
    fn scene_transform_composes_through_parents() {
        let mut s = Scene::new();
        let parent = s.spawn(
            None,
            Transform {
                translation: Vec3::new(2.0, 0.0, 0.0),
                rotation: Quat::from_rotation_y(FRAC_PI_2),
                scale: Vec3::ONE,
            },
        );
        let child = s.spawn(Some(parent), Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)));
        let wp = s.world_matrix(child).expect("child 存活").w_axis.truncate();
        assert!(
            approx(wp, Vec3::new(3.0, 0.0, 0.0)),
            "子节点的世界位置应为 (3,0,0)，实际 {wp:?}"
        );
        // 上层自身没有更上一层 ⇒ 世界矩阵 == 局部矩阵
        let pp = s.world_matrix(parent).expect("parent 存活").w_axis.truncate();
        assert!(approx(pp, Vec3::new(2.0, 0.0, 0.0)), "上层自身位置应为 (2,0,0)");
    }

    /// 上层的缩放也必须传给下层（否则编辑器里缩放手感不对）。
    /// 上层 S=(2,2,2)，子 local T=(1,0,0) ⇒ 世界位置 (2,0,0)。
    #[test]
    fn scene_scale_propagates_to_children() {
        let mut s = Scene::new();
        let p = s.spawn(
            None,
            Transform {
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(2.0),
            },
        );
        let c = s.spawn(Some(p), Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
        let wp = s.world_matrix(c).unwrap().w_axis.truncate();
        assert!(approx(wp, Vec3::new(2.0, 0.0, 0.0)), "实际 {wp:?}");
    }

    /// 层级是**树**：自挂、以及把更上层的节点挂到自己的后代下都必须报错。
    /// 反向验证：删掉 `WouldCycle` 判断 ⇒ 这条红（并且 `world_matrix` 会靠 guard 兜住）。
    #[test]
    fn scene_rejects_cycles() {
        let mut s = Scene::new();
        let a = s.spawn(None, Transform::IDENTITY);
        let b = s.spawn(Some(a), Transform::IDENTITY);
        let c = s.spawn(Some(b), Transform::IDENTITY);
        assert_eq!(s.set_parent(a, Some(a)), Err(SceneError::WouldCycle), "自挂必须被拒");
        assert_eq!(
            s.set_parent(a, Some(c)),
            Err(SceneError::WouldCycle),
            "把上一层挂到自己的深层后代下必须被拒"
        );
        // 合法的改上一层仍然可以
        assert!(s.set_parent(c, Some(a)).is_ok(), "改成兄弟层级应允许");
        assert_eq!(s.parent(c).unwrap(), Some(a));
        assert_eq!(s.children(b).unwrap().len(), 0, "改上层后旧上层的 children 要摘干净");
        assert_eq!(s.children(a).unwrap().len(), 2);
    }

    /// 删除上层节点必须级联删掉所有后代，且**旧句柄永久失效**（generation 守卫）。
    #[test]
    fn scene_despawn_cascades() {
        let mut s = Scene::new();
        let a = s.spawn(None, Transform::IDENTITY);
        let b = s.spawn(Some(a), Transform::IDENTITY);
        let c = s.spawn(Some(b), Transform::IDENTITY);
        assert_eq!(s.len(), 3);
        s.despawn(a).unwrap();
        assert_eq!(s.len(), 0, "级联后应无存活实体");
        for (name, id) in [("a", a), ("b", b), ("c", c)] {
            assert_eq!(
                s.world_matrix(id),
                Err(SceneError::NoSuchEntity(id)),
                "{name} 的旧句柄必须失效"
            );
        }
        // 槽位复用后，旧句柄仍然失效（generation 不同）
        let fresh = s.spawn(None, Transform::IDENTITY);
        assert_ne!(fresh, a, "槽位复用后句柄必须与旧的**不相等**（否则旧句柄会指向新实体）");
        assert_eq!(s.set_local(a, Transform::IDENTITY), Err(SceneError::NoSuchEntity(a)));
        assert!(s.set_local(fresh, Transform::IDENTITY).is_ok());
    }

    /// 🔴 **分层上限**：真引擎目录不许依赖玩法目录（见 `docs/ENGINE-PLAN.md` §3）。
    ///
    /// 今天 `src/engine/renderer.rs:18` 还 `use crate::engine` + `::game::{MapObstacle, ObstacleKind}`（故意拆开写，免得判据自己命中自己）
    /// —— 那是 P1 要消掉的目标（把这两个类型提到 `engine::scene`/`engine::geom`）。
    /// 所以在把那个数降到 0 之前，这条判据用**上限**约束：引用行数不许超过 1，
    /// 且**新增的 `engine/scene.rs` 自身必须是 0**。
    ///
    /// 反向验证：在 `engine/renderer/` 任一新文件里加一行 `use crate::engine` + `::game::Npc;`
    /// ⇒ 立刻红。
    #[test]
    fn engine_layer_does_not_grow_game_dependencies() {
        // P1a 已完成：renderer 对玩法目录的引用降到 **0**，这条从"上限"变成"硬零"。
        const RENDERER_BASELINE: usize = 0;
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if p.extension().is_some_and(|x| x == "rs") {
                        out.push(p);
                    }
                }
            }
        }
        let mut files = Vec::new();
        walk(std::path::Path::new("src/engine/renderer"), &mut files);
        files.push(std::path::PathBuf::from("src/engine/renderer.rs"));
        // 拼接而不是写死：判据文件里若出现完整路径，会**自己命中自己**（本会话已踩过一次）。
        const NEEDLE: &str = concat!("crate::engine", "::", "game");
        let mut hits = Vec::new();
        for f in &files {
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            for (i, line) in text.lines().enumerate() {
                if line.contains(NEEDLE) {
                    hits.push(format!("{}:{}", f.display(), i + 1));
                }
            }
        }
        assert!(
            hits.len() <= RENDERER_BASELINE,
            "引擎目录对玩法目录的引用从 {RENDERER_BASELINE} 行涨到 {} 行：{hits:?}\\\
             ；新代码请走 `engine::scene` 或 `engine::world`，别把玩法概念再引进来",

            hits.len()
        );
        // 场景层自己是硬 0：它是新代码，没有历史负担
        let scene_src = include_str!("scene.rs");
        assert!(
            !scene_src.contains(NEEDLE),
            "engine/scene.rs 不许引用 game 层（它就是用来切断这条依赖的）"
        );
    }
}
