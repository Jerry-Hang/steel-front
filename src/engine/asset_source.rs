//! 资产来源抽象层（Android 移植前置，2026-10-09）
//!
//! 动机（`docs/HANDOFF-mobile.md` §4.3）：引擎现在按**相对路径**从磁盘读 `assets/`，
//! 而 Android 上 APK 里的资源要走 `AAssetManager`（`asset://` 不在文件系统里）。
//! 所以在真正接 `AAssetManager` 之前，先在 Linux 上把这条读取路径**过一层抽象**，
//! 让手机侧以后只是"加第三个实现"，而不必在真机上调试资源路径。
//!
//! 本层是**只读**的：写盘（日志 / 截图 / 配置）不走这里。
//!
//! 实现进度：
//! - ✅ 文件系统实现 `FsSource`（桌面）。
//! - ✅ 内存实现 `MemSource` + 全局 `install()` + `RV3D_ASSETS_FROM_MEMORY=1`
//!   （用内存来源替代文件系统，用于在桌面上先把这条路跑通，见 `docs/HANDOFF-mobile.md` §4.3）。
//! - ⏳ 待接：Android `AAssetManager` 实现（真机阶段）。
//!
//! 覆盖范围：`load_spirv`（着色器）/ `load_map`（关卡 TOML）/ GLB 探测。
//! 尚未覆盖（仍直接读盘）：`PropSet::load_dir`（目录枚举）、GLB 实际字节读取。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 只读资产来源。桌面 = 文件系统；Android = `AAssetManager`；测试 / 内存模式 = 内存。
pub trait AssetSource: Send + Sync {
    /// 读取整个资产的原始字节。
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;

    /// 读取并校验为 UTF-8 文本（TOML / 配置等）。
    fn read_to_string(&self, path: &str) -> Result<String, String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| format!("资产 '{}' 不是合法 UTF-8: {}", path, e))
    }

    /// 资产是否存在。
    fn exists(&self, path: &str) -> bool;
}

/// 文件系统来源（桌面：Windows / Linux 原生）。
pub struct FsSource;

impl AssetSource for FsSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("读取资产失败 '{}': {}", path, e))
    }

    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
}

/// 内存来源：路径 → 字节。
///
/// 两个用途：单元测试；以及 `RV3D_ASSETS_FROM_MEMORY=1` 时把 `assets/` 预载进内存，
/// 在桌面上验证"资产不来自文件系统"这条路径（为 Android `AAssetManager` 铺路）。
pub struct MemSource {
    files: HashMap<String, Vec<u8>>,
}

impl MemSource {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// 插入一项（路径用 `/` 分隔，与读取时传入的字符串一致）。
    ///
    /// 🔴 **这里负责把平台原生分隔符归一化成 `/`**（2026-10-15，外部复核指出第 1 条，
    /// 已对着代码确认）：`preload_tree` 把 `PathBuf::to_str()` 的结果直接传进来，
    /// 而那是**平台原生**的 —— **Windows 上 `e.path()` 给出 `assets\maps\index.toml`**
    /// ⇒ 存进去的键全是反斜杠，而引擎与全部单测都按 `assets/maps/index.toml` 查表
    /// ⇒ **`HashMap::get` 100% 落空、所有资产都读不到**。
    /// （本机是 Linux，`to_str()` 本来就是 `/` ⇒ **这个 bug 只在 Windows 上现形**，
    ///  所以它能在 Linux 上一直绿着跑。）
    ///
    /// 归一化放在 `insert` 而不是 `preload_tree`：**这里是唯一的写入入口**，
    /// 放这儿才能让上面那句文档契约**对每一个调用方都成立**（而不是靠调用方自觉）。
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.replace('\\', "/"), bytes);
    }

    /// 已载入的条目数。
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// 递归把 `root` 下的所有文件读进内存（路径原样保留，含 `root` 前缀）。
    ///
    /// 读不到的子目录直接跳过——内存模式是"尽力而为"的取证通路，不是错误处理点。
    pub fn preload_tree(root: &str) -> Self {
        let mut m = Self::new();
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(bytes) = std::fs::read(&p) {
                    if let Some(s) = p.to_str() {
                        m.insert(s, bytes);
                    }
                }
            }
        }
        m
    }
}

impl Default for MemSource {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetSource for MemSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("内存资产不存在: {}", path))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}

static GLOBAL: OnceLock<Box<dyn AssetSource>> = OnceLock::new();

/// 安装全局资产来源。应在启动早期调用一次；已安装则忽略（首次安装生效）。
pub fn install(src: Box<dyn AssetSource>) {
    let _ = GLOBAL.set(src);
}

/// 取全局资产来源；未安装时为 [`FsSource`]（与旧行为逐字节一致）。
pub fn global() -> &'static dyn AssetSource {
    match GLOBAL.get() {
        Some(s) => s.as_ref(),
        None => &FS,
    }
}

static FS: FsSource = FsSource;

/// Android 资产来源：包在 `AAssetManager` 上（APK 内的 `assets/`）。
///
/// 路径约定：引擎传的是相对路径 `assets/foo`，而 `AAssetManager` 的路径相对
/// **`assets/` 根**（即 `foo`）—— 这里去掉 `assets/` 前缀。
#[cfg(target_os = "android")]
pub struct AndroidAssetSource {
    mgr: ndk::asset::AssetManager,
}

#[cfg(target_os = "android")]
impl AndroidAssetSource {
    pub fn new(mgr: ndk::asset::AssetManager) -> Self {
        Self { mgr }
    }

    fn rel(path: &str) -> String {
        // 🔴 归一化（2026-10-20 第 16 批复核第 6 条）：`./assets/...` 或 `assets\...`
        // 在桌面 FsSource 上能打开、到 APK 里 AAssetManager 直接失败。
        let p = path.replace('\\', "/");
        let p = p.trim_start_matches("./");
        p.strip_prefix("assets/").unwrap_or(p).to_string()
    }
}

#[cfg(target_os = "android")]
impl AssetSource for AndroidAssetSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let rel = Self::rel(path);
        let c = std::ffi::CString::new(rel)
            .map_err(|e| format!("资产路径含 NUL '{}': {}", path, e))?;
        let mut a = self
            .mgr
            .open(&c)
            .ok_or_else(|| format!("AAssetManager 打不开 '{}'（APK 内为 '{}'）", path, rel))?;
        let buf = a
            .buffer()
            .map_err(|e| format!("读取资产 '{}' 失败: {}", path, e))?;
        Ok(buf.to_vec())
    }

    fn exists(&self, path: &str) -> bool {
        let rel = Self::rel(path);
        std::ffi::CString::new(rel)
            .map(|c| self.mgr.open(&c).is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_source_reads_its_own_source_file() {
        let src = FsSource;
        assert!(src.exists("src/engine/asset_source.rs"));
        let text = src
            .read_to_string("src/engine/asset_source.rs")
            .expect("读取自身源文件");
        assert!(text.contains("trait AssetSource"));
    }

    #[test]
    fn fs_source_missing_path_is_err_not_panic() {
        let src = FsSource;
        assert!(!src.exists("assets/__definitely_missing__.spv"));
        assert!(src.read("assets/__definitely_missing__.spv").is_err());
    }

    #[test]
    fn mem_source_serves_what_it_was_given() {
        let mut m = MemSource::new();
        m.insert("assets/x.spv", vec![1, 2, 3]);
        assert!(m.exists("assets/x.spv"));
        assert_eq!(m.read("assets/x.spv").unwrap(), vec![1, 2, 3]);
        assert!(!m.exists("assets/y.spv"));
        assert!(m.read("assets/y.spv").is_err());
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn preload_tree_picks_up_known_assets() {
        // 仓库里一定存在的两处：着色器目录与关卡目录。
        let m = MemSource::preload_tree("assets");
        assert!(m.len() > 0, "assets/ 预载为空，判据失效");
        assert!(m.exists("assets/maps/index.toml"), "关卡索引未被预载");
    }
}
#[cfg(test)]
mod mem_source_path_tests {
    use super::*;

    /// 🔴 **`MemSource::insert` 必须把平台原生分隔符归一化成 `/`。**
    ///
    /// 存在理由（2026-10-15，外部复核指出第 1 条，已对着代码确认）：
    /// `preload_tree` 把 `PathBuf::to_str()` 的结果直接传给 `insert`，而那是**平台原生**的
    /// —— **Windows 上 `e.path()` 给出 `assets\maps\index.toml`**。
    /// 引擎与全部单测都按 `assets/maps/index.toml` 查表
    /// ⇒ **`HashMap::get` 100% 落空、`RV3D_ASSETS_FROM_MEMORY=1` 下所有资产都读不到**。
    ///
    /// ⚠️ **这个 bug 在 Linux 上永远不会现形**（`to_str()` 本来就是 `/`）⇒
    /// 判据不能依赖真实文件系统路径，必须**直接喂一个带反斜杠的键**，
    /// 否则它在开发机上恒绿（这正是它藏了这么久的原因）。
    #[test]
    fn insert_normalizes_backslash_paths_to_forward_slash() {
        let mut m = MemSource::new();
        // 模拟 Windows 上 `PathBuf::to_str()` 给出来的形态
        m.insert("assets\\maps\\index.toml", b"x".to_vec());
        // 引擎按 `/` 查
        assert!(
            m.exists("assets/maps/index.toml"),
            "反斜杠键没有被归一化 ⇒ Windows 上 preload_tree 载入的资产全部查不中"
        );
        assert_eq!(m.len(), 1, "归一化后应当只有【一个】键（而不是多出一个反斜杠副本）");

        // 正对照：本来就是 `/` 的键不受影响（别把正常路径改坏）
        let mut m2 = MemSource::new();
        m2.insert("assets/maps/index.toml", b"y".to_vec());
        assert!(m2.exists("assets/maps/index.toml"), "正常的 `/` 路径被改坏了");

        // 混合形态（Windows 上子目录拼接可能出现）
        let mut m3 = MemSource::new();
        m3.insert("assets\\guns/ak12.glb", b"z".to_vec());
        assert!(m3.exists("assets/guns/ak12.glb"), "混合分隔符没有被归一化");
    }
}

// ---------------------------------------------------------------------------
// Asset-path scan (ASCII on purpose: the CJK glyph table cannot be rebuilt).
//
// Invariant: production code must never reach an `assets/...` file through
// `std::fs::*`. Every asset read has to go through `asset_source::global()`, or
// Android (`AAssetManager`) and `RV3D_ASSETS_FROM_MEMORY=1` (`MemSource`) break
// silently -- the read simply fails and the caller falls back to a default.
//
// Why a subtree scan and not `include_str!`: this bug class is "one of two
// siblings was fixed" (load_map vs load_map_list, load_gun_glb vs
// load_soldier_glb_uncached, load_spirv vs PropSet::load_dir). A judge that
// only looks at the file it lives in cannot see the sibling. The walk is
// run-time so it follows future module splits without anyone remembering to
// update a file list, and it fails closed when the scan surface looks wrong
// (repo lesson 46: a scanner that reads nothing must not report OK).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod asset_path_scan_tests {
    use std::path::{Path, PathBuf};

    fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_rs(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// Production code only: strip every `#[cfg(test)]`-attributed item.
    ///
    /// 🔴 2026-10-20 第 16 批复核第 6 条：原来是"切到**第一个** `#[cfg(test)]` 为止"，
    /// 而本仓大量文件在**中段**放 `#[cfg(test)]` 测试辅助方法、**其后紧跟数百行生产代码**
    /// ⇒ 那些生产代码完全不在扫描面内（守卫有漏洞）。
    ///
    /// 这里用一个**能跳过字符串/注释/字符字面量**的扫描器按花括号配对定位每个
    /// `#[cfg(test)]` 项的结束位置并剔除；只认完整项，不破坏其余原文。
    fn production_part(src: &str) -> String {
        let bytes = src.as_bytes();
        let mut out = String::with_capacity(src.len());
        let mut i = 0usize;
        while i < bytes.len() {
            if let Some(attr_len) = cfg_test_attr_at(bytes, i) {
                // 跳过属性本身
                i += attr_len;
                // 跳过空白
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                // 定位该项的结束：第一个 `;`（声明）或配对 `{}`（块）
                let end = skip_item(bytes, i);
                i = end;
            } else {
                // 逐字节复制，但要跳过字符串/注释，避免误判 "#[cfg(test)]" 出现的位置
                let next = skip_non_code(bytes, i);
                out.push_str(&src[i..next]);
                i = next;
            }
        }
        out
    }

    /// 若 `i` 处是 `#[cfg(test)]` 属性，返回属性长度
    fn cfg_test_attr_at(bytes: &[u8], i: usize) -> Option<usize> {
        const ATTR: &[u8] = b"#[cfg(test)]";
        if bytes.len() >= i + ATTR.len() && &bytes[i..i + ATTR.len()] == ATTR {
            Some(ATTR.len())
        } else {
            None
        }
    }

    /// 跳过非代码片段（字符串/原始字符串/字符字面量/行注释/块注释），返回下一段代码起点。
    fn skip_non_code(bytes: &[u8], i: usize) -> usize {
        if i >= bytes.len() {
            return i;
        }
        match bytes[i] {
            b'"' => skip_string(bytes, i + 1),
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\n' {
                    j += 1;
                }
                j
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                let mut depth = 1;
                let mut j = i + 2;
                while j < bytes.len() && depth > 0 {
                    if bytes[j] == b'/' && j + 1 < bytes.len() && bytes[j + 1] == b'*' {
                        depth += 1;
                        j += 2;
                    } else if bytes[j] == b'*' && j + 1 < bytes.len() && bytes[j + 1] == b'/' {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                j
            }
            // 原始字符串 r"..." / r#"..."#
            b'r' => {
                let mut j = i + 1;
                let mut hashes = 0;
                while j < bytes.len() && bytes[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    let close: Vec<u8> = std::iter::once(b'"')
                        .chain(std::iter::repeat(b'#').take(hashes))
                        .collect();
                    let mut k = j + 1;
                    while k + close.len() <= bytes.len() {
                        if &bytes[k..k + close.len()] == close.as_slice() {
                            return k + close.len();
                        }
                        k += 1;
                    }
                    bytes.len()
                } else {
                    i + 1
                }
            }
            // 字符字面量：'x' / '\n' / '\''（避免把生命周期 'a 当字符）
            b'\'' => {
                if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                    i + 3
                } else if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                    // '\x' 形态：找到下一个引号
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' {
                        j += 1;
                    }
                    (j + 1).min(bytes.len())
                } else {
                    i + 1
                }
            }
            _ => i + 1,
        }
    }

    fn skip_string(bytes: &[u8], mut j: usize) -> usize {
        while j < bytes.len() {
            match bytes[j] {
                b'\\' => j += 2,
                b'"' => return j + 1,
                _ => j += 1,
            }
        }
        bytes.len()
    }

    /// 从项起点（已跳过属性与空白）跳到该项结束（含 `;` 或配对 `}`）。
    fn skip_item(bytes: &[u8], mut i: usize) -> usize {
        let mut depth = 0i32;
        while i < bytes.len() {
            let next = skip_non_code(bytes, i);
            if next != i + 1 || matches!(bytes[i], b'{' | b'}' | b';') {
                // 先处理代码字符
            }
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth <= 0 {
                        return i + 1;
                    }
                }
                b';' if depth == 0 => return i + 1,
                _ => {}
            }
            i = next;
        }
        bytes.len()
    }

    /// The complete set of `std::fs` read sites allowed in production code,
    /// each with the reason it must NOT go through `asset_source`.
    ///
    /// Snapshot on purpose. My first version of this judge only flagged lines
    /// that also contain the literal `assets/`, which made it blind to the very
    /// bug it was written for: `load_map_list` receives the path as a *variable*
    /// (`index_path`), so reverting it to `std::fs::read_to_string` kept the
    /// judge green. Pinning the whole set means any new site turns it red and
    /// the author has to state why the abstraction is wrong for that file.
    const ALLOWED_STD_FS_READS: &[(&str, &str)] = &[
        // config file (~/.steel_front.cfg) -- per-user, never shipped as an asset
        // config file (~/.steel_front.cfg) -- per-user, never shipped as an asset
        ("src/config.rs", "fs::read_to_string(path)"),
        // the abstraction itself: FsSource / MemSource::preload_tree
        // (these ARE the std::fs side of the trait; Android supplies its own impl)
        ("src/engine/asset_source.rs", "std::fs::read(path)"),
        ("src/engine/asset_source.rs", "std::fs::read_dir(&dir)"),
        ("src/engine/asset_source.rs", "std::fs::read(&p)"),
        // source-scanning test scaffolding: reads the checkout, not shipped assets
        ("src/engine/renderer/tests_support.rs", "std::fs::read_to_string(path)"),
        ("src/engine/renderer/tests_support.rs", "std::fs::read_dir(dir)"),
        // Windows-only dev helper (a path list next to the exe) -- not an asset
        ("src/lib.rs", "resource_paths.ini"),
        // WSL detection -- reads /proc, not the asset tree
        ("src/lib.rs", "/proc/version"),
        // CPU topology: reads Linux sysfs (`/sys/devices/system/cpu`), not assets
        ("src/engine/cpu.rs", "std::fs::read_to_string(&p)"),
        ("src/engine/cpu.rs", "std::fs::read_to_string(&path)"),
        ("src/engine/cpu.rs", "/sys/devices/system/cpu/online"),
        // GLB props: KNOWN GAP (documented in props.rs) -- `AssetSource` has no directory
        // listing, so `PropSet::load_dir` cannot be ported until the trait gains one.
        ("src/engine/props.rs", "std::fs::read_dir(dir)"),
        ("src/engine/props.rs", "std::fs::read(&path)"),
    ];

    /// 🔴 **剔除 `#[cfg(test)]` 项不能连带切掉它后面的生产代码**
    /// （2026-10-20 第 16 批复核第 6 条）：
    /// 原实现"切到第一个 `#[cfg(test)]` 为止"，本仓大量文件在中段放测试辅助方法、
    /// 其后紧跟数百行生产代码 ⇒ 扫描面有漏洞。
    /// 反向验证：把 `production_part` 改回 `src.find("#[cfg(test)]")` 截断 ⇒ 本判据红。
    #[test]
    fn production_part_keeps_code_after_a_mid_file_test_item() {
        let src = r#"
fn before() {}

#[cfg(test)]
mod tests {
    fn helper() { let _ = std::fs::read("x"); }
}

fn after() { let _ = std::fs::read("assets/y"); }
"#;
        let prod = production_part(src);
        assert!(prod.contains("fn before()"), "前段生产代码不能丢");
        assert!(
            prod.contains("fn after()"),
            "中段 `#[cfg(test)]` 之后的生产代码必须保留（旧实现会把它整段切掉）"
        );
        assert!(
            !prod.contains("fn helper()"),
            "测试模块本身必须被剔除"
        );
    }

    #[test]
    fn only_the_abstraction_and_non_asset_files_use_std_fs_in_production() {
        let mut files = Vec::new();
        collect_rs(Path::new("src"), &mut files);
        assert!(
            files.len() >= 50,
            "scan surface looks wrong: only {} .rs files under src/ (tests must run from the crate root)",
            files.len()
        );

        let mut found: Vec<(String, String)> = Vec::new();
        let mut prod_lines = 0usize;
        for f in &files {
            // 纯测试模块文件（`tests.rs` / `tests_*.rs`，由 `#[cfg(test)] mod tests;` 引入）
            // 里的源码读取是"读 checkout"而不是读资产 —— 跳过，不算生产代码。
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "tests.rs" || name.starts_with("tests_") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(f) else {
                continue;
            };
            let prod = production_part(&src);
            prod_lines += prod.lines().count();
            for line in prod.lines() {
                let t = line.trim();
                if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
                    continue;
                }
                if t.contains("std::fs::read") || t.contains("fs::read_to_string(") || t.contains("fs::read_dir(") {
                    found.push((f.display().to_string(), t.to_string()));
                }
            }
        }
        assert!(
            prod_lines > 20_000,
            "scan surface looks wrong: only {prod_lines} production lines read"
        );

        let unexpected: Vec<String> = found
            .iter()
            .filter(|pair| {
                // 🔴 路径分隔符归一：Windows 的 `f.display()` 给 `src\engine\x.rs`，而白名单写 `src/engine/x.rs`
                // ⇒ 直接用 ends_with 会**全不命中**，12 个合法站点全被报成 Unexpected（实测 2026-10-26）。
                let path = pair.0.replace('\\', "/");
                !ALLOWED_STD_FS_READS
                    .iter()
                    .any(|(file, frag)| path.ends_with(file) && pair.1.contains(frag))
            })
            .map(|(f, l)| format!("{}: {l}", f.replace('\\', "/")))
            .collect();

        assert!(
            unexpected.is_empty(),
            "production code reads files with std::fs outside the allowed set.\n\
             Asset reads must go through asset_source::global(): on Android (AAssetManager) and\n\
             with RV3D_ASSETS_FROM_MEMORY=1 (MemSource) a direct std::fs read fails and the\n\
             caller silently falls back to a default (that is how load_map_list, load_soldier_glb\n\
             and PropSet::load_dir stayed broken). If a NEW site is genuinely not an asset,\n\
             add it to ALLOWED_STD_FS_READS with the reason.\nUnexpected:\n{}",
            unexpected.join("\n")
        );
        // Fail closed: the snapshot must match reality exactly in the other
        // direction too, otherwise a stale entry hides a real regression.
        assert_eq!(
            found.len(),
            ALLOWED_STD_FS_READS.len(),
            "the allowlist has {} entries but {found} std::fs read sites were found -- \n\
             fix the list (a removed site means an entry is now stale)",
            ALLOWED_STD_FS_READS.len(),
            found = found.len()
        );
    }
}
