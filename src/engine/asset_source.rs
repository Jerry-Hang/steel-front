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
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.to_string(), bytes);
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

    fn rel(path: &str) -> &str {
        path.strip_prefix("assets/").unwrap_or(path)
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