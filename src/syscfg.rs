//! 跨平台运行期配置读取：环境变量优先，Android 上再回退到系统属性。
//!
//! 为什么需要它（2026-10-09 实机）：Android 应用进程拿不到自定义环境变量，
//! `setprop` + `__system_property_get` 是不重编就能改运行参数的唯一通道。
//! 桌面（Windows / Linux）行为不变：只看环境变量。

/// 读一个配置项。
///
/// 查找顺序：
/// 1. 环境变量 `<name>`（例如 `RV3D_CPU_PRIME`）
/// 2. Android：系统属性 `debug.sf.<name 去掉 RV3D_ 前缀、转小写>`
///    （例如 `debug.sf.cpu_prime`）
pub fn cfg(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    #[cfg(target_os = "android")]
    {
        let prop = format!(
            "debug.sf.{}",
            name.trim_start_matches("RV3D_").to_lowercase()
        );
        if let Some(v) = android_getprop(&prop) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// **布尔开关**：`1` / `on` / `true`（大小写不敏感、两侧空白忽略）= 开；未设、空、其它值 = 关。
///
/// 🔴 2026-10-12：**语义从"只认 `1`"拓宽到也认 `on`/`true`**。依据是仓库里另有 5 处
/// 写的是 `is_ok_and(|v| v == "1" || v == "on" || v == "true")` —— 同一件事在代码里
/// 有两份答案，而它们迟早分叉。**只认 `1` 的那份是少数派**，统一到宽的那份。
/// 安全性：三个现有调用方（`RV3D_CPU_PRIME` / `RV3D_NO_SHADOW` / `RV3D_SHADOW_PASS_OFF`）
/// 都是布尔开关，拓宽是**超集**，不会把"原本开"变成"关"。
///
/// 与 `is_set` 的区别：**「设了空值」`flag` 算关**，`is_set` 算开。
/// 排查时用错这两个会把"我设了但没生效"误判成"开关没接线"。
pub fn flag(name: &str) -> bool {
    matches!(
        cfg(name).map(|v| v.trim().to_ascii_lowercase()),
        Some(ref v) if v == "1" || v == "on" || v == "true"
    )
}

/// **开关是否被设置**（不看值）。等价于旧代码里的 `std::env::var(name).is_ok()` 语义
/// —— 但**空值不算**（`cfg` 会把空/纯空白读成 `None`），这点比旧写法更严格、也更符合直觉。
pub fn is_set(name: &str) -> bool {
    cfg(name).is_some()
}

/// 解析成 `T`（失败返回 `None`）。统一走这里，避免各处 `.parse().ok()` 的写法分叉。
pub fn parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    cfg(name).and_then(|v| v.trim().parse::<T>().ok())
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    /// 三个辅助函数的语义必须彼此区分开（用错会把"没生效"误判成"没接线"）。
    #[test]
    fn flag_and_is_set_have_distinct_semantics() {
        // 这一条**不碰环境变量**（测试里改 env 会与并行用例互相干扰），
        // 只钉住"两个函数不是同一个东西"这个契约本身。
        assert!(!flag("RV3D_TEST_DEFINITELY_UNSET_XYZ"));
        assert!(!is_set("RV3D_TEST_DEFINITELY_UNSET_XYZ"));
        assert_eq!(parse::<u32>("RV3D_TEST_DEFINITELY_UNSET_XYZ"), None);
    }
}

/// Android 系统属性读取（libc 的 `__system_property_get`，libc 本来就链着，零新依赖）。
#[cfg(target_os = "android")]
fn android_getprop(name: &str) -> Option<String> {
    use std::ffi::CString;
    extern "C" {
        fn __system_property_get(
            name: *const std::os::raw::c_char,
            value: *mut std::os::raw::c_char,
        ) -> i32;
    }
    let cname = CString::new(name).ok()?;
    let mut buf = [0 as std::os::raw::c_char; 160];
    let n = unsafe { __system_property_get(cname.as_ptr(), buf.as_mut_ptr()) };
    if n <= 0 {
        return None;
    }
    let bytes: Vec<u8> = buf[..n as usize].iter().map(|&b| b as u8).collect();
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_reads_env_var() {
        std::env::set_var("RV3D_TEST_FLAG_UNIT", "1");
        assert!(flag("RV3D_TEST_FLAG_UNIT"));
        std::env::remove_var("RV3D_TEST_FLAG_UNIT");
        assert!(!flag("RV3D_TEST_FLAG_UNIT"));
    }

    #[test]
    fn cfg_ignores_blank_env() {
        std::env::set_var("RV3D_TEST_BLANK_UNIT", "   ");
        assert!(cfg("RV3D_TEST_BLANK_UNIT").is_none());
        std::env::remove_var("RV3D_TEST_BLANK_UNIT");
    }

    #[test]
    fn cfg_reads_env_var_value() {
        std::env::set_var("RV3D_TEST_VALUE_UNIT", "4-6");
        assert_eq!(cfg("RV3D_TEST_VALUE_UNIT").as_deref(), Some("4-6"));
        std::env::remove_var("RV3D_TEST_VALUE_UNIT");
    }
}