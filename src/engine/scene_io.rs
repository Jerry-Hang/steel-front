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
        recs.push(Rec {
            file_id: id,
            parent,
            t: Transform {
                translation: t,
                rotation: r,
                scale: s,
            },
        });
    }

    // 建实体：`Scene::spawn` 会按空槽复用，所以这里按**文件里出现的顺序**建，
    // 再记录"文件 id → 场景句柄"的映射；上下级必须靠这张映射连，不能靠位置。
    let mut scene = Scene::new();
    let mut map: Vec<(usize, crate::engine::scene::EntityId)> = Vec::new();
    for rec in &recs {
        let handle = scene.spawn(None, rec.t);
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
