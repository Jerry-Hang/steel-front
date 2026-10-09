//! 资产来源抽象层（Android 移植前置，2026-10-09）
//!
//! 动机（`docs/HANDOFF-mobile.md` §4.3）：引擎现在按**相对路径**从磁盘读 `assets/`，
//! 而 Android 上 APK 里的资源要走 `AAssetManager`（`asset://` 不在文件系统里）。
//! 所以在真正接 `AAssetManager` 之前，先在 Linux 上把这条读取路径**过一层抽象**，
//! 让手机侧以后只是"加第三个实现"，而不必在真机上调试资源路径。
//!
//! 本层是**只读**的：写盘（日志 / 截图 / 配置）不走这里。
//!
//! 进度（增量式）：
//! - ✅ 本增量：抽出 `trait AssetSource` + `FsSource`，`load_spirv` / `load_map` / GLB 探测改走这里，
//!   桌面行为**逐字节等价**（仍走文件系统）。
//! - ⏳ 下一步：内存实现（`RV3D_ASSETS_FROM_MEMORY=1`）与全局 `install()`，
//!   那时本模块会加回 `OnceLock` 与 `MemSource`（现在加会触发 dead_code，违反 0 警告红线）。

/// 只读资产来源。桌面 = 文件系统；Android = `AAssetManager`；测试 = 内存。
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

static FS: FsSource = FsSource;

/// 取全局资产来源（当前恒为文件系统实现）。
///
/// 待接入 `AAssetManager` / 内存实现时，这里会改成从 `OnceLock` 取已安装的实现。
pub fn global() -> &'static dyn AssetSource {
    &FS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_source_reads_its_own_source_file() {
        // 本文件自身一定存在且为 UTF-8，可作为 fs 通路的最小判据。
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
    fn global_is_fs_source() {
        // 桌面/测试环境下全局来源即文件系统。
        assert!(global().exists("Cargo.toml"));
    }
}