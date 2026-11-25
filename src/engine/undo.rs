//! 撤销 / 重做（引擎化 P3k）。
//!
//! 🔴 **不做逐操作逆运算，直接存整份场景快照**。理由：
//! 1. 存盘格式（`scene_io::to_toml`）已经**逐字节可往返**（判据 `apply_to_roundtrips_byte_identically`），
//!    撤销 = 把旧文本 `apply_to` 回去，不需要为 `spawn`/`despawn`/`set_parent`/`set_mesh`/缩放 各写一遍逆操作；
//! 2. 逆操作必须**逐个正确**才不出错，而快照只要"存 → 读回"这一条路径对就够了；
//! 3. 场景在编辑态很小（几十个实体），一份文本几 KB，存几十份没有压力。
//!
//! ⚠️ **代价**：撤销会**丢掉实体的 generation**（`apply_to` 是清空重建），
//! 所以撤销后旧句柄全部失效 —— 调用方必须把选中项清掉。这是刻意的取舍：
//! 保住句柄就要做"差量补丁"，那需要为每种操作写逆运算，正是上面刻意避开的复杂度。

/// 撤销栈。只存文本快照，不存语义。
#[derive(Debug, Clone)]
pub struct UndoStack {
    /// 过去的状态（越靠后越近）。
    past: Vec<String>,
    /// 被撤销掉、可以重做的状态。
    future: Vec<String>,
    /// 最多保留多少步。超出丢最旧的。
    capacity: usize,
}

impl UndoStack {
    /// `capacity` 至少为 1（0 会让"撤销"永远无效，而且 `remove(0)` 会 panic）。
    pub fn new(capacity: usize) -> Self {
        UndoStack {
            past: Vec::new(),
            future: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn past_len(&self) -> usize {
        self.past.len()
    }

    pub fn future_len(&self) -> usize {
        self.future.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// 记录一个"改动之前"的状态。
    ///
    /// **记录会清空重做栈** —— 这是编辑器的通行语义：改了新东西之后，
    /// 原来那条重做分支就不再是从"当前状态"出发的了，留着会让重做跳到一个不相干的世界。
    pub fn record(&mut self, before: String) {
        self.past.push(before);
        if self.past.len() > self.capacity {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// 撤销：传入**当前**状态，返回**要恢复成的**状态。
    ///
    /// 当前状态会被压进重做栈，所以紧接着的 `redo` 能回到这里。
    pub fn undo(&mut self, current: String) -> Option<String> {
        let prev = self.past.pop()?;
        self.future.push(current);
        if self.future.len() > self.capacity {
            self.future.remove(0);
        }
        Some(prev)
    }

    /// 重做：与 `undo` 镜像。
    pub fn redo(&mut self, current: String) -> Option<String> {
        let next = self.future.pop()?;
        self.past.push(current);
        if self.past.len() > self.capacity {
            self.past.remove(0);
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 撤销/重做必须**成对可逆**：undo 再 redo 要回到原状态。
    ///
    /// 反向验证：把 `redo` 里的 `self.past.push(current)` 删掉 ⇒ 再撤销一次会跳过中间状态，红。
    #[test]
    fn undo_then_redo_returns_to_the_same_state() {
        let mut u = UndoStack::new(16);
        u.record("s0".to_string());
        assert_eq!(u.undo("s1".to_string()), Some("s0".to_string()));
        assert_eq!(u.redo("s0".to_string()), Some("s1".to_string()));
        assert_eq!(u.past_len(), 1, "重做之后过去栈要复原");
        assert_eq!(u.future_len(), 0);
    }

    /// 🔴 **记录新改动会清空重做栈**：改了新东西之后，旧的重做分支已经不是从当前状态出发的了。
    /// 反向验证：删掉 `self.future.clear()` ⇒ 重做会跳到不相干的世界，红。
    #[test]
    fn recording_a_new_edit_clears_the_redo_stack() {
        let mut u = UndoStack::new(16);
        u.record("s0".to_string());
        u.undo("s1".to_string());
        assert!(u.can_redo());
        u.record("s2".to_string()); // 新的改动
        assert!(!u.can_redo(), "新改动之后不许还能重做旧分支");
        assert_eq!(u.future_len(), 0);
    }

    /// 🔴 容量上限：超出丢**最旧的**，而且 `past_len` 永远不超过容量。
    /// 反向验证：把 `remove(0)` 删掉 ⇒ 长度无上限，红。
    #[test]
    fn stack_is_capped_and_drops_the_oldest() {
        let mut u = UndoStack::new(3);
        for i in 0..10 {
            u.record(format!("s{i}"));
        }
        assert_eq!(u.past_len(), 3, "最多 3 步");
        // 丢掉最旧的之后，撤销只能回到最近三步
        assert_eq!(u.undo("now".to_string()), Some("s9".to_string()));
        assert_eq!(u.undo("s9".to_string()), Some("s8".to_string()));
        assert_eq!(u.undo("s8".to_string()), Some("s7".to_string()));
        assert_eq!(u.undo("s7".to_string()), None, "只能退三步");
    }

    /// 空栈上撤销/重做必须返回 `None`（不是 panic，也不是给空串）。
    #[test]
    fn empty_stack_returns_none() {
        let mut u = UndoStack::new(4);
        assert_eq!(u.undo("s".to_string()), None);
        assert_eq!(u.redo("s".to_string()), None);
        // 容量 0 也要能用（夹到 1），否则 remove(0) 会 panic
        let mut z = UndoStack::new(0);
        z.record("a".to_string());
        assert_eq!(z.past_len(), 1);
    }
}
