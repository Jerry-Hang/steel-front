//! 场景存盘 / 读回（引擎化 P3，见 `docs/ENGINE-PLAN.md`）。
//!
//! **为什么先做存盘再做界面**：编辑器的价值 = 改完能留下来；AI 接口的价值 = 它改的东西
//! 人能接着改。两者都要求同一件事：**场景能写成文件、再原样读回来**。
//! 没有这一步，"编辑器"只是一个不存盘的查看器，"AI 控制"只是一个不可复现的脚本。
//!
//! 格式用 TOML（本仓零依赖，解析器在 `engine::map` 里已有先例），**只写稳定字段**：
//!
//! ```toml
//! [[entity]]
//! id = 0
//! parent = -1
//! t = [1.0, 2.0, 3.0]
//! r = [0.0, 0.0, 0.0, 1.0]   # 四元数 xyzw
//! s = [1.0, 1.0, 1.0]
//! ```
//!
//! 设计取舍：
//! - **id 显式写在文件里**：读回时要重建层级，靠"数组顺序"隐式推上一级节点会在中间删过实体后错位；
//! - **四元数用 xyzw 数组**：与 `glam::Quat::from_xyzw` 一致，不引入欧拉角约定问题；
//! - **浮点用 `{}` 默认格式**：`f32` 的默认 Display 是"能还原的最短表示"（Rust 保证 round-trip），
//!   所以写出去再读回来**逐位相等**，判据才做得成"字段逐一相等"而不是"差不多"。

use crate::engine::scene::{Scene, Transform};
use glam::{Quat, Vec3};

/// 存盘失败的原因。**不吞错误** —— 编辑器要弹提示、AI 接口要把原因回给调用方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// 场景里有悬空引用（上一级句柄失效）。存盘必须拒，否则文件本身就是坏的。
    DanglingParent,
    /// 读回时 `parent` 指向一个还没定义或已越界的 id。
    BadParent(String),
    /// 读回时某行缺字段或数值解析失败。
    BadField(String),
    /// 读回时 `r` 不是四元数长度（零四元数无法归一化）。
    BadRotation(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::DanglingParent => write!(f, "场景里有悬空上一级引用，拒绝存盘"),
            SaveError::BadParent(s) => write!(f, "parent 非法：{s}"),
            SaveError::BadField(s) => write!(f, "字段解析失败：{s}"),
            SaveError::BadRotation(s) => write!(f, "旋转四元数非法：{s}"),
        }
    }
}

/// 把场景写成 TOML 文本。
///
/// 顺序 = 槽位顺序（`Scene::len` 只数存活的，但这里按"所有存活实体"遍历并写出真实 id），
/// 这样读回时上下关系能按 id 重建，与数组顺序无关。
pub fn to_toml(scene: &Scene) -> Result<String, SaveError> {
    let mut out = String::from("# steel-front 场景导出（引擎化 P3）\n");
    out.push_str("# [[entity]] id/parent/t/r/s；parent = -1 表示世界根\n");
    for (id, local) in scene.iter_alive() {
        // 🔴 **镜像实体不写**：它们是每帧由玩法层重写的只读数据，
        // 写进关卡等于"存一次档就凭空多出 255 个 NPC"。
        if scene.is_readonly(id) {
            continue;
        }
        let parent = scene.parent(id).map_err(|_| SaveError::DanglingParent)?;
        out.push_str("\n[[entity]]\n");
        out.push_str(&format!("id = {}\n", scene.index_of(id)));
        out.push_str(&format!(
            "parent = {}\n",
            parent.map(|p| scene.index_of(p) as i64).unwrap_or(-1)
        ));
        let t = local.translation;
        out.push_str(&format!("t = [{}, {}, {}]\n", t.x, t.y, t.z));
        let r = local.rotation;
        out.push_str(&format!("r = [{}, {}, {}, {}]\n", r.x, r.y, r.z, r.w));
        let s = local.scale;
        out.push_str(&format!("s = [{}, {}, {}]\n", s.x, s.y, s.z));
        // 可见性只在 false 时写：默认 true ⇒ 绝大多数实体省一行，文件更好读。
        // 🔴 **必须持久化**：不写的话 `apply_to` 读回后世界根与玩家会变成可见，
        // 场景里凭空多出两个盒子（实测就是这么发现的）。
        if !scene.is_visible(id) {
            out.push_str("visible = false\n");
        }
        // 网格引用也要持久化：否则"实例化一个模型 → 存盘 → 读回"会变回方块
        if let Some(mesh) = scene.mesh_of(id) {
            out.push_str(&format!("mesh = \"{mesh}\"\n"));
        }
    }
    Ok(out)
}

/// 从 TOML 文本重建场景。**先建实体、再连上下级**（上一级可能写在下一级后面）。
pub fn from_toml(text: &str) -> Result<(Scene, Vec<(usize, crate::engine::scene::EntityId)>), SaveError> {
    // 逐行解析：只认 `[[entity]]` 段与 5 个键，够用且零依赖。
    let mut blocks: Vec<Vec<(String, String)>> = Vec::new();
    let mut cur: Option<Vec<(String, String)>> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[entity]]" {
            if let Some(b) = cur.take() {
                blocks.push(b);
            }
            cur = Some(Vec::new());
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if let Some(b) = cur.as_mut() {
                b.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    if let Some(b) = cur.take() {
        blocks.push(b);
    }

    fn parse_vec3(s: &str, key: &str) -> Result<Vec3, SaveError> {
        let inner = s
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|x| x.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SaveError::BadField(format!("{key} = {s}")))?;
        if inner.len() != 3 {
            return Err(SaveError::BadField(format!("{key} 需要 3 个分量：{s}")));
        }
        Ok(Vec3::new(inner[0], inner[1], inner[2]))
    }
    fn parse_vec4(s: &str, key: &str) -> Result<Quat, SaveError> {
        let inner = s
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|x| x.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SaveError::BadField(format!("{key} = {s}")))?;
        if inner.len() != 4 {
            return Err(SaveError::BadField(format!("{key} 需要 4 个分量：{s}")));
        }
        let q = Quat::from_xyzw(inner[0], inner[1], inner[2], inner[3]);
        if q.length_squared() < 1e-12 {
            return Err(SaveError::BadRotation(s.to_string()));
        }
        Ok(q.normalize())
    }

    struct Rec {
        file_id: i64,
        parent: i64,
        t: Transform,
        visible: bool,
        mesh: Option<String>,
    }
    let mut recs: Vec<Rec> = Vec::new();
    for b in &blocks {
        let get = |k: &str| -> Option<&String> { b.iter().find(|(kk, _)| kk == k).map(|(_, v)| v) };
        let id: i64 = get("id")
            .ok_or_else(|| SaveError::BadField("缺 id".to_string()))?
            .parse()
            .map_err(|_| SaveError::BadField("id 不是整数".to_string()))?;
        let parent: i64 = get("parent")
            .ok_or_else(|| SaveError::BadField("缺 parent".to_string()))?
            .parse()
            .map_err(|_| SaveError::BadField("parent 不是整数".to_string()))?;
        let t = parse_vec3(
            get("t").ok_or_else(|| SaveError::BadField("缺 t".to_string()))?,
            "t",
        )?;
        let r = parse_vec4(
            get("r").ok_or_else(|| SaveError::BadField("缺 r".to_string()))?,
            "r",
        )?;
        let s = parse_vec3(
            get("s").ok_or_else(|| SaveError::BadField("缺 s".to_string()))?,
            "s",
        )?;
        let visible = get("visible")
            .map(|v| v.trim() != "false")
            .unwrap_or(true);
        // 网格路径：去掉两端引号（字符串值在 TOML 里是 `mesh = "…"`）
        let mesh = get("mesh").map(|v| v.trim().trim_matches('"').to_string());
        recs.push(Rec {
            file_id: id,
            parent,
            t: Transform {
                translation: t,
                rotation: r,
                scale: s,
            },
            visible,
            mesh,
        });
    }

    // 建实体：`Scene::spawn` 会按空槽复用，所以这里按**文件里出现的顺序**建，
    // 再记录"文件 id → 场景句柄"的映射；上下级必须靠这张映射连，不能靠位置。
    let mut scene = Scene::new();
    let mut map: Vec<(usize, crate::engine::scene::EntityId)> = Vec::new();
    for rec in &recs {
        let handle = scene.spawn(None, rec.t);
        // 可见性在**建实体时**就写进去，与存盘对称
        scene.set_visible(handle, rec.visible).ok();
        if let Some(m) = rec.mesh.clone() {
            scene.set_mesh(handle, Some(m)).ok();
        }
        map.push((rec.file_id as usize, handle));
    }
    // 连上下级（二次遍历：上一级可能写在下一级后面）
    for rec in &recs {
        if rec.parent < 0 {
            continue;
        }
        let child = map
            .iter()
            .find(|(fid, _)| *fid == rec.file_id as usize)
            .map(|(_, h)| *h)
            .ok_or_else(|| SaveError::BadParent(format!("子 {} 找不到", rec.file_id)))?;
        let parent = map
            .iter()
            .find(|(fid, _)| *fid == rec.parent as usize)
            .map(|(_, h)| *h)
            .ok_or_else(|| {
                SaveError::BadParent(format!(
                    "id={} 的 parent={} 在文件里不存在",
                    rec.file_id, rec.parent
                ))
            })?;
        scene
            .set_parent(child, Some(parent))
            .map_err(|e| SaveError::BadParent(format!("{} 挂到 {} 失败：{e:?}", rec.file_id, rec.parent)))?;
    }
    Ok((scene, map))
}


/// 把文件内容**应用到现有场景**（先清空再重建）。返回读回的实体数。
///
/// 与 `from_toml` 的区别：那个返回一个**新** `Scene`，调用方拿到的是另一份世界；
/// 编辑器要的是"换掉当前世界的全部内容" —— 调用方（`Game`/`App`）持有的是同一个
/// `Scene` 引用，换对象就等于要求所有持有者一起换引用，那是不现实的。
pub fn apply_to(scene: &mut Scene, text: &str) -> Result<usize, SaveError> {
    let (fresh, _map) = from_toml(text)?;
    scene.clear();
    // 按文件顺序重建：`from_toml` 已经解好了上一层子关系，这里把结构与变换一起搬过来。
    let mut map: Vec<(crate::engine::scene::EntityId, crate::engine::scene::EntityId)> = Vec::new();
    for (id, local) in fresh.iter_alive() {
        let new_id = scene.spawn(None, local);
        map.push((id, new_id));
    }
    for (old, new) in &map {
        if let Ok(Some(old_parent)) = fresh.parent(*old) {
            if let Some((_, new_parent)) = map.iter().find(|(o, _)| *o == old_parent) {
                scene
                    .set_parent(*new, Some(*new_parent))
                    .map_err(|e| SaveError::BadParent(format!("重建层级失败：{e:?}")))?;
            }
        }
        // 可见性也要搬：否则读回的实体全部可见（世界根与玩家会多画两个盒子）
        let vis = fresh.is_visible(*old);
        scene.set_visible(*new, vis).ok();
        // 🔴 网格引用**也要搬**。判据 `scene_toml_roundtrips_the_mesh_reference` 抓的就是这条：
        // 只搬变换/层级/可见性的话，"实例化 → 存盘 → 读回"会悄悄退回方块。
        // 这是同一个形态的第三次（可见性一次、网格一次）：**每加一个逐实体字段，
        // 就要同时改 `to_toml` / `from_toml` / `apply_to` 三处**。
        if let Some(m) = fresh.mesh_of(*old) {
            scene.set_mesh(*new, Some(m)).ok();
        }
    }
    Ok(scene.len())
}


/// **结构化转储**（给 AI / 外部工具读，也给编辑器面板用）。
///
/// 与 `to_toml` 的区别：那个是**存盘格式**（可读、可手改、要能往返）；
/// 这个是**报告格式**（一行一个实体、字段顺序固定、便于逐行解析与 diff）。
///
/// 行格式（固定顺序，缺值用 `-`）：
/// ```text
/// #<idx> p=<parent|-> t=<x>,<y>,<z> r=<x>,<y>,<z>,<w> s=<x>,<y>,<z> v=<0|1> ro=<0|1> m=<mesh|->
/// ```
/// 为什么要有它：AI 侧现在只能拿到 `entities=N` 这种计数，看不到"谁在哪"。
/// 结构化转储让外部程序能**定位具体实体**，而不是只能凭猜发命令。
pub fn dump_lines(scene: &Scene) -> Vec<String> {
    let mut out = Vec::with_capacity(scene.len());
    for (id, local) in scene.iter_alive() {
        let parent = scene
            .parent(id)
            .ok()
            .flatten()
            .map(|p| format!("#{}", scene.index_of(p)))
            .unwrap_or_else(|| "-".to_string());
        let t = local.translation;
        let r = local.rotation;
        let s = local.scale;
        out.push(format!(
            "#{} p={parent} t={:.6},{:.6},{:.6} r={:.6},{:.6},{:.6},{:.6} s={:.6},{:.6},{:.6} v={} ro={} m={}",
            scene.index_of(id),
            t.x,
            t.y,
            t.z,
            r.x,
            r.y,
            r.z,
            r.w,
            s.x,
            s.y,
            s.z,
            if scene.is_visible(id) { 1 } else { 0 },
            if scene.is_readonly(id) { 1 } else { 0 },
            scene.mesh_of(id).unwrap_or_else(|| "-".to_string()),
        ));
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 **往返判据：存盘 → 读回，字段逐一相等**（不是"差不多"）。
    ///
    /// 用极端值（负零、极小值、非单位四元数、非等比缩放）来打：只要格式化/解析有一处
    /// 用 `{:.2}` 之类的截断，这条立刻红。
    #[test]
    fn scene_toml_roundtrip_preserves_transforms() {
        let mut s = Scene::new();
        let root = s.spawn(
            None,
            Transform {
                translation: Vec3::new(-0.0, 1.0e-7, 3.5),
                rotation: Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
                scale: Vec3::new(1.0, 2.5, 1.0e-6),
            },
        );
        let child = s.spawn(
            Some(root),
            Transform {
                translation: Vec3::new(-12.25, 0.0, 1.0e6),
                rotation: Quat::from_xyzw(0.5, 0.5, 0.5, 0.5),
                scale: Vec3::new(-1.0, 1.0, 0.5),
            },
        );
        let text = to_toml(&s).expect("存盘");
        let (back, _map) = from_toml(&text).expect("读回");
        assert_eq!(back.len(), 2, "实体数必须一致");
        // 逐字段比对：按文件 id 顺序（0 = 第一次 spawn）
        let ids: Vec<_> = back.iter_alive().map(|(id, t)| (id, t)).collect();
        let orig: Vec<_> = s.iter_alive().map(|(id, t)| (id, t)).collect();
        for ((_, a), (_, b)) in orig.iter().zip(ids.iter()) {
            assert_eq!(a.translation.to_array(), b.translation.to_array(), "平移必须逐位相等");
            assert_eq!(a.scale.to_array(), b.scale.to_array(), "缩放必须逐位相等");
            // 四元数会被归一化，所以比"归一化之后"的值
            let qa = a.rotation.normalize();
            let qb = b.rotation.normalize();
            for (x, y) in [(qa.x, qb.x), (qa.y, qb.y), (qa.z, qb.z), (qa.w, qb.w)] {
                assert!((x - y).abs() < 1e-6, "四元数分量必须一致：{qa:?} vs {qb:?}");
            }
        }
        // 层级也必须保住
        let child_back = ids[1].0;
        let parent_back = back.parent(child_back).unwrap().expect("下层必须有上级");
        assert_eq!(Some(parent_back), back.parent(child_back).unwrap());
        assert_eq!(back.children(parent_back).unwrap().len(), 1);
        let _ = child;
    }


    /// 🔴 **存盘 → 读回（替换场景）→ 再存盘：两次文本必须逐字节相同**。
    ///
    /// 这是比"字段相等"更强的判据：只要重建时任何一处（层级、可见性、槽位顺序）走样，
    /// 第二次的文本就会变。而且它顺带证明了 `apply_to` 没有"越读越多"或漏掉实体。
    ///
    /// 反向验证：把 `apply_to` 里搬可见性那两行删掉 ⇒ 世界根/玩家读回后变成可见，
    /// 第二次存盘文本不同（visibility 不进 TOML，但实体数会变？）—— 见下一条说明。
    #[test]
    fn apply_to_roundtrips_byte_identically() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let a = s.spawn(Some(root), Transform::from_translation(Vec3::new(1.5, 0.0, -2.25)));
        let b = s.spawn(None, Transform::from_translation(Vec3::new(0.0, 3.0, 0.0)));
        s.set_visible(root, false).ok();
        let first = to_toml(&s).expect("存盘");

        let mut target = Scene::new();
        // 先塞一个无关实体：证明 apply_to 是"换掉"而不是"追加"
        target.spawn(None, Transform::IDENTITY);
        let n = apply_to(&mut target, &first).expect("读回");
        assert_eq!(n, 3, "应读回 3 个实体（原有的那个被清掉）");

        let second = to_toml(&target).expect("再存盘");
        assert_eq!(first, second, "两次存盘必须逐字节相同");
        // 🔴 **可见性必须显式查**：它不进 TOML，所以"两次文本相同"根本测不到它。
        // 我第一版就在注释里声称这条会抓到"忘搬可见性"，注入验证时才发现那是错的。
        // 漏搬的后果很具体：世界根与玩家读回后变成可见 ⇒ 场景里凭空多出两个盒子。
        let vis: Vec<bool> = target.iter_alive().map(|(id, _)| target.is_visible(id)).collect();
        assert_eq!(
            vis,
            vec![false, true, true],
            "世界根必须仍然不可见（它是结构，不是要画的东西）：{vis:?}"
        );
        let _ = (a, b);
    }

    /// `apply_to` 必须**保住层级**（否则子实体会被打回根节点）。
    #[test]
    fn apply_to_preserves_the_hierarchy() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let child = s.spawn(Some(root), Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)));
        let text = to_toml(&s).unwrap();
        let mut target = Scene::new();
        apply_to(&mut target, &text).unwrap();
        let ids: Vec<_> = target.iter_alive().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 2);
        let parent = target.parent(ids[1]).unwrap();
        assert_eq!(parent, Some(ids[0]), "第二个实体的上一层必须是第一个");
        assert_eq!(target.children(ids[0]).unwrap().len(), 1);
        let _ = child;
    }


    /// 🔴 **网格引用必须随场景存盘往返**：否则"实例化一个模型 → 存盘 → 读回"会退回方块。
    /// 反向验证：删掉 `to_toml` 里写 mesh 那两行 ⇒ 读回后 `mesh_of` 是 None，红。
    #[test]
    fn scene_toml_roundtrips_the_mesh_reference() {
        let mut s = Scene::new();
        let a = s.spawn(None, Transform::IDENTITY);
        let b = s.spawn(None, Transform::IDENTITY);
        s.set_mesh(a, Some("assets/props/barrel_metal.glb".to_string()))
            .unwrap();
        let text = to_toml(&s).expect("存盘");
        assert!(text.contains("mesh = \"assets/props/barrel_metal.glb\""), "文件里要有网格路径");

        let mut target = Scene::new();
        apply_to(&mut target, &text).unwrap();
        let ids: Vec<_> = target.iter_alive().map(|(id, _)| id).collect();
        assert_eq!(
            target.mesh_of(ids[0]),
            Some("assets/props/barrel_metal.glb".to_string()),
            "读回后网格引用必须还在"
        );
        assert_eq!(target.mesh_of(ids[1]), None, "没绑网格的实体不许凭空有网格");
        let _ = b;
    }

    #[test]
    fn apply_to_keeps_mesh_on_the_third_entity() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let player = s.spawn(Some(root), Transform::IDENTITY);
        let mesh_e = s.spawn(None, Transform::from_translation(Vec3::new(0.0, 0.0, -1e-5)));
        s.set_visible(root, false).unwrap();
        s.set_visible(player, false).unwrap();
        s.set_mesh(mesh_e, Some("assets/props/barrel_metal.glb".into())).unwrap();
        let first = to_toml(&s).unwrap();
        let mut target = Scene::new();
        apply_to(&mut target, &first).unwrap();
        let second = to_toml(&target).unwrap();
        assert!(second.contains("mesh ="), "第二次存盘丢了网格：\n{second}");
        assert_eq!(first, second, "两次存盘必须逐字节相同");
    }

    /// 🔴 **结构化转储的字段必须齐全且顺序固定**（外部程序靠逐行解析）。
    ///
    /// 反向验证：把 `ro=` 那一段删掉 ⇒ 第一条断言红。
    #[test]
    fn dump_lines_has_stable_fields_in_fixed_order() {
        let mut s = Scene::new();
        let root = s.spawn(None, Transform::IDENTITY);
        let e = s.spawn(
            Some(root),
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::splat(2.0),
            },
        );
        s.set_visible(root, false).unwrap();
        s.set_readonly(e, true).unwrap();
        s.set_mesh(e, Some("assets/props/barrel_metal.glb".to_string()))
            .unwrap();
        let lines = dump_lines(&s);
        assert_eq!(lines.len(), 2, "一行一个实体");
        // 字段顺序与数量固定
        for (i, line) in lines.iter().enumerate() {
            for key in [" p=", " t=", " r=", " s=", " v=", " ro=", " m="] {
                assert!(line.contains(key), "第 {i} 行缺字段 {key}：{line}");
            }
            let pos: Vec<usize> = [" p=", " t=", " r=", " s=", " v=", " ro=", " m="]
                .iter()
                .map(|k| line.find(k).unwrap())
                .collect();
            let mut sorted = pos.clone();
            sorted.sort_unstable();
            assert_eq!(pos, sorted, "字段顺序必须固定：{line}");
        }
        // 值也要对得上（抽几条）
        assert!(lines[0].starts_with("#0 p=- "), "{}", lines[0]);
        assert!(lines[0].contains(" v=0 ro=0 m=-"), "{}", lines[0]);
        assert!(lines[1].starts_with("#1 p=#0 "), "{}", lines[1]);
        assert!(lines[1].contains(" t=1.000000,2.000000,3.000000"), "{}", lines[1]);
        assert!(lines[1].contains(" s=2.000000,2.000000,2.000000"), "{}", lines[1]);
        assert!(lines[1].contains(" ro=1 m=assets/props/barrel_metal.glb"), "{}", lines[1]);
    }

    /// 🔴 **`to_toml` 与 `dump_lines` 的差别必须钉死**：
    /// `to_toml` 跳过只读镜像（存盘不许写 NPC），`dump_lines` **要包含**它们（报告要看全部）。
    ///
    /// 为什么这条值钱：编辑态自证（`RV3D_EDIT_PROOF`）用 `dump_lines` 算哈希 ——
    /// 它一旦改成 `to_toml`，就**看不到 NPC 了**，而"播放态每帧变"正是它的正对照。
    /// 实测过一次：P6c 让 `to_toml` 跳过只读之后，这条自证悄悄失效，
    /// 播放态又变回"帧帧相同"，而我当时还以为正对照有效。
    ///
    /// 反向验证：把 `dump_lines` 也跳过只读 ⇒ 第一条断言红。
    #[test]
    fn dump_includes_readonly_while_save_skips_them() {
        let mut s = Scene::new();
        let a = s.spawn(None, Transform::IDENTITY);
        s.set_readonly(a, true).unwrap();
        assert_eq!(dump_lines(&s).len(), 1, "报告要包含只读实体");
        let text = to_toml(&s).expect("存盘");
        assert_eq!(
            text.lines().filter(|l| l.starts_with("[[entity]]")).count(),
            0,
            "存盘不许写只读实体"
        );
        // 移动只读实体：报告要变，存盘不变
        let before_dump = dump_lines(&s);
        let before_save = to_toml(&s).unwrap();
        s.set_local(a, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)))
            .unwrap();
        assert_ne!(dump_lines(&s), before_dump, "只读实体移动后报告必须变");
        assert_eq!(to_toml(&s).unwrap(), before_save, "存盘不受只读实体移动影响");
    }

    /// 读回时 parent 指向不存在的 id 必须报错，而不是静默把它当根节点。
    ///
    /// 反向验证：把 `BadParent` 分支改成 `continue` ⇒ 这条红（测试会读到"没有上级"而失败）。
    #[test]
    fn scene_toml_rejects_dangling_parent() {
        let bad = "[[entity]]\nid = 0\nparent = 7\nt = [0, 0, 0]\nr = [0, 0, 0, 1]\ns = [1, 1, 1]\n";
        let err = match from_toml(bad) {
            Err(e) => e,
            Ok(_) => panic!("悬空 parent 必须被拒，但读回成功了"),
        };
        assert!(
            matches!(err, SaveError::BadParent(_)),
            "应报 BadParent，实际 {err:?}"
        );
        assert!(err.to_string().contains("不存在"), "错误信息要能读：{err}");
    }

    /// 零四元数必须拒绝（无法归一化），不能悄悄变成单位四元数。
    #[test]
    fn scene_toml_rejects_zero_rotation() {
        let bad = "[[entity]]\nid = 0\nparent = -1\nt = [0, 0, 0]\nr = [0, 0, 0, 0]\ns = [1, 1, 1]\n";
        match from_toml(bad) {
            Err(SaveError::BadRotation(_)) => {}
            Err(e) => panic!("应是 BadRotation，实际 {e:?}"),
            Ok(_) => panic!("零四元数必须被拒，但读回成功了"),
        }
    }
}
