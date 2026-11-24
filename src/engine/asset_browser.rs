//! 资产列表（引擎化 P3h，见 `docs/ENGINE-PLAN.md`）。
//!
//! 编辑器要能"看见目录里有哪些模型"。这一层只做三件事，且都做成**纯函数 + 一个扫描入口**：
//! 排序、分屏、选中。绘制由 `editor` 负责（复用同一套 quad + 点阵文本）。
//!
//! 🔴 **目录列举必须走 `AssetSource`**，不许 `std::fs`：仓库有一条架构判据
//! （`only_the_abstraction_and_non_asset_files_use_std_fs_in_production`）看着这件事，
//! 而且 Android 上 `AAssetManager` 根本不走 `std::fs`。
//!
//! **三条三态约定**（仓库教训 46）：`scan` 返回 `Result`，
//! 目录不存在/读不了 ⇒ `Err`，**不是空列表** —— "扫到 0 个"和"没扫成"必须能分开，
//! 否则编辑器会安静地显示一个空列表，而人以为"这个目录就是空的"。

/// 一条资产条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    /// 文件名（不含目录），用于显示。
    pub name: String,
    /// 完整相对路径（`assets/props/x.glb`），用于加载。
    pub path: String,
}

/// 列表状态：条目表 + 当前屏 + 选中下标（全局下标，不是屏内下标）。
#[derive(Debug, Clone)]
pub struct AssetBrowser {
    entries: Vec<AssetEntry>,
    page: usize,
    per_page: usize,
    selected: usize,
}

impl AssetBrowser {
    /// 从一条路径列表构建（已排序，保证每次打开顺序一致 —— 文件系统的读目录顺序不保证）。
    pub fn from_paths(paths: Vec<String>, per_page: usize) -> Self {
        let mut entries: Vec<AssetEntry> = paths
            .into_iter()
            .filter(|p| p.to_ascii_lowercase().ends_with(".glb"))
            .map(|path| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                AssetEntry { name, path }
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        AssetBrowser {
            entries,
            page: 0,
            per_page: per_page.max(1),
            selected: 0,
        }
    }

    /// 扫描一个目录。目录读不了 ⇒ `Err`（**不是空表**）。
    pub fn scan(dir: &str, per_page: usize) -> Result<Self, String> {
        let paths = crate::engine::asset_source::global().list_dir(dir)?;
        Ok(Self::from_paths(paths, per_page))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn page_count(&self) -> usize {
        self.entries.len().div_ceil(self.per_page).max(1)
    }

    pub fn selected(&self) -> Option<&AssetEntry> {
        self.entries.get(self.selected)
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// 当前屏要显示的条目（屏内切片）。
    pub fn page_entries(&self) -> &[AssetEntry] {
        let start = (self.page * self.per_page).min(self.entries.len());
        let end = (start + self.per_page).min(self.entries.len());
        &self.entries[start..end]
    }

    /// 翻屏：**循环**（最后一屏的下一屏是第一屏）。空表时停在 0 屏。
    pub fn next_page(&mut self) {
        if self.entries.is_empty() {
            self.page = 0;
            return;
        }
        self.page = (self.page + 1) % self.page_count();
    }

    pub fn prev_page(&mut self) {
        if self.entries.is_empty() {
            self.page = 0;
            return;
        }
        let n = self.page_count();
        self.page = (self.page + n - 1) % n;
    }

    /// 在**全表**上移动选中项（跨屏，循环）。选中项始终跟着屏走 ——
    /// 否则会出现"选中在第 3 屏、屏幕显示第 1 屏"的错位。
    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.selected = 0;
            return;
        }
        let n = self.entries.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
        self.page = self.selected / self.per_page;
    }

    /// 面板要显示的行（含表头与屏码）。纯函数 ⇒ 判据能逐字比。
    pub fn panel_lines(&self) -> Vec<String> {
        if self.entries.is_empty() {
            return vec!["资产：空目录".to_string()];
        }
        let mut out = vec![format!(
            "资产 {}/{} 屏（共 {}）",
            self.page + 1,
            self.page_count(),
            self.entries.len()
        )];
        for (i, e) in self.page_entries().iter().enumerate() {
            let idx = self.page * self.per_page + i;
            let mark = if idx == self.selected { ">" } else { " " };
            // 名字截断到 22 字符：面板宽度固定，长文件名会把整行挤出去
            let name: String = e.name.chars().take(22).collect();
            out.push(format!("{mark}{name}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("assets/props/m{:02}.glb", i))
            .collect()
    }

    /// 🔴 排序必须确定：文件系统的读目录顺序**不保证**，不排序每次打开顺序都可能变。
    /// 反向验证：删掉 `entries.sort_by` ⇒ 这条红。注意输入要**故意乱序**。
    #[test]
    fn browser_sorts_entries_deterministically() {
        let mut p = paths(4);
        p.reverse(); // 故意乱序
        let b = AssetBrowser::from_paths(p, 2);
        let names: Vec<&str> = b.page_entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["m00.glb", "m01.glb"], "第一屏必须按名字升序");
        let b2 = AssetBrowser::from_paths(paths(4), 2);
        assert_eq!(
            b.page_entries(),
            b2.page_entries(),
            "同样的输入（哪担心顺序不同）必须得到同样的表"
        );
    }

    /// 只收 `.glb`：目录里可能有 LOD 的 `.glb` 与别的杂项，非 glb 一律过滤。
    #[test]
    fn browser_keeps_only_glb() {
        let p = vec![
            "assets/props/a.glb".to_string(),
            "assets/props/readme.txt".to_string(),
            "assets/props/b.GLB".to_string(),
        ];
        let b = AssetBrowser::from_paths(p, 10);
        assert_eq!(b.len(), 2, "应只留两个 glb（大小写不敏感）");
    }

    /// 🔴 翻屏是**循环**的，且 page_count 至少为 1（空表也要能算屏数，否则除零）。
    /// 反向验证：把 `% self.page_count()` 去掉（直接 +1）⇒ 最后一屏翻过头，红。
    #[test]
    fn browser_paging_wraps_and_never_divides_by_zero() {
        let mut b = AssetBrowser::from_paths(paths(5), 2);
        assert_eq!(b.page_count(), 3);
        b.next_page();
        b.next_page();
        assert_eq!(b.page(), 2, "第三屏");
        b.next_page();
        assert_eq!(b.page(), 0, "最后一屏的下一屏是第一屏");
        b.prev_page();
        assert_eq!(b.page(), 2, "第一屏的上一屏是最后一屏");

        let mut empty = AssetBrowser::from_paths(vec![], 2);
        assert_eq!(empty.page_count(), 1, "空表也要算 1 屏");
        empty.next_page();
        assert_eq!(empty.page(), 0);
    }

    /// 🔴 选中项跨屏移动时，**屏要跟着走** —— 否则会"选中在第 3 屏、屏幕显示第 1 屏"。
    #[test]
    fn browser_selection_drags_the_page_along() {
        let mut b = AssetBrowser::from_paths(paths(5), 2);
        assert_eq!(b.page(), 0);
        b.move_selection(3); // 选中第 4 个（下标 3）⇒ 第 2 屏
        assert_eq!(b.selected_index(), 3);
        assert_eq!(b.page(), 1, "屏必须跟着选中项走");
        b.move_selection(-4); // 跨 0 回绕到最后一个
        assert_eq!(b.selected_index(), 4);
        assert_eq!(b.page(), 2);
    }

    /// 面板文字必须标出选中项，且行数 = 表头 + 当屏条目数。
    #[test]
    fn browser_panel_lines_mark_the_selection() {
        let mut b = AssetBrowser::from_paths(paths(3), 2);
        b.move_selection(1);
        let lines = b.panel_lines();
        assert_eq!(lines.len(), 3, "表头 + 2 条：{lines:?}");
        assert!(lines[0].contains("1/2 屏"), "表头要有屏码：{}", lines[0]);
        assert!(lines[1].starts_with(' '), "未选中的行不带标记：{}", lines[1]);
        assert!(lines[2].starts_with('>'), "选中的行带 > ：{}", lines[2]);
    }
}
